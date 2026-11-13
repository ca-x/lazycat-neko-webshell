use super::*;
use crate::state::Device;
use neko_remoted_protocol::{
    self as protocol,
    messages::{Event, Operation, Permission, Request, Response},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::mpsc,
};

async fn device(core: &Core, parent: String, permission: Permission) -> Principal {
    let mut state = core.state.lock().await;
    state.saved.enabled = true;
    let device = Device {
        id: uuid::Uuid::new_v4().to_string(),
        name: "creator".into(),
        key: protocol::encode(&protocol::random32().unwrap()),
        permission,
        resources: vec![parent],
        generation: 1,
    };
    let principal = Principal::Device {
        id: device.id.clone(),
        generation: 1,
    };
    state.saved.devices.push(device);
    core.persist(&state.saved).await.unwrap();
    principal
}

async fn connect(
    core: Arc<Core>,
    principal: Principal,
) -> (
    mpsc::Sender<Request>,
    mpsc::Receiver<Response>,
    CancellationToken,
) {
    let (requests, incoming) = mpsc::channel(8);
    let (responses, mut replies) = mpsc::channel(32);
    let cancel = CancellationToken::new();
    tokio::spawn(crate::session::serve(
        core,
        principal,
        uuid::Uuid::new_v4().to_string(),
        incoming,
        responses,
        cancel.clone(),
    ));
    assert!(matches!(
        replies.recv().await.unwrap().event,
        Event::Ready { .. }
    ));
    (requests, replies, cancel)
}

async fn reply(replies: &mut mpsc::Receiver<Response>, id: u64) -> Event {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = replies.recv().await.unwrap();
            if response.id == id {
                return response.event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn herdr_only_grant_creates_usable_native_shell_and_reconnect_only_discovers_it() {
    if crate::tests::controlled_shell_child(
        "terminal_creation::tests::herdr_only_grant_creates_usable_native_shell_and_reconnect_only_discovers_it",
    ) {
        return;
    }
    let (save, saved) = crate::tests::store();
    let mut core = Core::start(None, save).await.unwrap();
    let root = std::env::temp_dir().join(format!("neko-create-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let socket = root.join("herdr.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let program = root.join("herdr");
    crate::tests::write_test_executable(
        &program,
        &format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'herdr test'; else echo '{{\"running\":true,\"socket\":\"{}\"}}'; fi\n",
            socket.display()
        ),
    );
    Arc::get_mut(&mut core).unwrap().herdr = Arc::new(crate::herdr::Herdr::with_program(program));
    let ping = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream)
            .read_line(&mut line)
            .await
            .unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], "ping");
        stream.write_all(format!("{}\n", serde_json::json!({"id":request["id"],"result":{"protocol":22,"version":"test"}})).as_bytes()).await.unwrap();
    });
    core.herdr.refresh().await.unwrap();
    ping.await.unwrap();
    assert!(core.terminals.list().unwrap().is_empty());
    let parent = core.herdr.resource().unwrap().id;
    let principal = device(&core, parent, Permission::Operate).await;
    let Principal::Device {
        id: device_id,
        generation,
    } = &principal
    else {
        unreachable!()
    };
    let device_id = device_id.clone();
    let generation = *generation;
    let (requests, mut replies, cancel) = connect(core.clone(), principal).await;
    requests
        .send(Request {
            id: 1,
            operation: Operation::TerminalCreate {
                grant: core.herdr.resource().unwrap().id,
            },
        })
        .await
        .unwrap();
    let Event::TerminalCreated { resource } = reply(&mut replies, 1).await else {
        panic!("create failed")
    };
    assert_eq!(resource.backend, "native");
    assert!(resource.terminal && !resource.herdr && !resource.files);
    let persisted: serde_json::Value =
        serde_json::from_str(saved.lock().unwrap().as_ref().unwrap()).unwrap();
    assert!(
        persisted["devices"][0]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == &resource.id)
    );
    requests
        .send(Request {
            id: 2,
            operation: Operation::TerminalAttach {
                grant: resource.id.clone(),
                pane: None,
            },
        })
        .await
        .unwrap();
    let Event::TerminalAttached {
        stream,
        read_only: false,
        ..
    } = reply(&mut replies, 2).await
    else {
        panic!("new shell not attachable")
    };
    requests
        .send(Request {
            id: 3,
            operation: Operation::TerminalControl {
                stream: stream.clone(),
                take: true,
            },
        })
        .await
        .unwrap();
    assert!(matches!(
        reply(&mut replies, 3).await,
        Event::TerminalControl {
            controller: true,
            ..
        }
    ));
    requests
        .send(Request {
            id: 4,
            operation: Operation::TerminalInput {
                stream,
                data: protocol::encode(b"printf '%s%s\\n' created_ shell_ok\r"),
            },
        })
        .await
        .unwrap();
    let mut output = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::TerminalOutput { data, .. } = replies.recv().await.unwrap().event {
                output.extend(protocol::decode(&data, protocol::MAX_PLAINTEXT).unwrap());
                if output
                    .windows(b"created_shell_ok".len())
                    .any(|part| part == b"created_shell_ok")
                {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let (requests, mut replies, cancel) = connect(
        core.clone(),
        Principal::Device {
            id: device_id,
            generation,
        },
    )
    .await;
    requests
        .send(Request {
            id: 1,
            operation: Operation::Resources {},
        })
        .await
        .unwrap();
    assert!(
        matches!(reply(&mut replies, 1).await, Event::Resources { resources } if resources.iter().any(|item| item.id == resource.id))
    );
    assert_eq!(
        core.terminals.list().unwrap().len(),
        1,
        "reconnect must not repeat creation"
    );
    cancel.cancel();
    core.close_terminal(&resource.id).await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn readonly_unknown_parent_stale_generation_and_revoke_never_create() {
    if crate::tests::controlled_shell_child(
        "terminal_creation::tests::readonly_unknown_parent_stale_generation_and_revoke_never_create",
    ) {
        return;
    }
    let (save, _) = crate::tests::store();
    let core = Core::start(None, save).await.unwrap();
    let parent = core.create_terminal("parent".into()).await.unwrap();
    for denial in [
        "readonly",
        "wrong_parent",
        "generation",
        "revoked",
        "cancelled",
    ] {
        let principal = device(
            &core,
            parent.clone(),
            if denial == "readonly" {
                Permission::ReadOnly
            } else {
                Permission::Operate
            },
        )
        .await;
        let cancel = CancellationToken::new();
        if denial == "cancelled" {
            cancel.cancel();
        }
        let mut state = core.state.lock().await;
        if denial == "generation" {
            state.saved.devices.last_mut().unwrap().generation += 1;
        }
        if denial == "revoked" {
            state.saved.devices.pop();
        }
        drop(state);
        let mut unknown = false;
        let result = core
            .create_granted_terminal(
                &principal,
                if denial == "wrong_parent" {
                    "unapproved"
                } else {
                    &parent
                },
                &cancel,
                &mut unknown,
            )
            .await;
        assert!(result.is_err(), "{denial} must be denied");
        assert!(!unknown, "{denial} is known not created");
        assert_eq!(
            core.terminals.list().unwrap().len(),
            1,
            "{denial} spawned a shell"
        );
    }
    core.close_terminal(&parent).await.unwrap();
}

#[tokio::test]
async fn failed_child_grant_save_does_not_publish_or_leave_live_shell() {
    if crate::tests::controlled_shell_child(
        "terminal_creation::tests::failed_child_grant_save_does_not_publish_or_leave_live_shell",
    ) {
        return;
    }
    use std::sync::atomic::{AtomicBool, Ordering};
    let failing = Arc::new(AtomicBool::new(false));
    let fail = failing.clone();
    let save: crate::Save = Arc::new(move |_| {
        let fail = fail.clone();
        Box::pin(async move {
            ensure!(!fail.load(Ordering::SeqCst), "store failed");
            Ok(())
        })
    });
    let core = Core::start(None, save).await.unwrap();
    let parent = core.create_terminal("parent".into()).await.unwrap();
    let principal = device(&core, parent.clone(), Permission::Operate).await;
    failing.store(true, Ordering::SeqCst);
    let mut unknown = false;
    assert!(
        core.create_granted_terminal(&principal, &parent, &CancellationToken::new(), &mut unknown)
            .await
            .is_err()
    );
    assert!(unknown, "mutation began before a persistence error");
    assert_eq!(core.terminals.list().unwrap().len(), 1);
    assert_eq!(
        core.state.lock().await.saved.devices[0].resources,
        vec![parent.clone()]
    );
    failing.store(false, Ordering::SeqCst);
    core.close_terminal(&parent).await.unwrap();
}
