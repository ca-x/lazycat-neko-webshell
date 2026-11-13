use super::*;
use neko_remoted_protocol::messages::Request;

fn actor_with_wire() -> (Actor, mpsc::Receiver<Request>) {
    let (updates, _receiver) = mpsc::channel(16);
    let mut actor = Actor::new(updates);
    actor.model.phase = Phase::Ready;
    actor.model.permission = Some(Permission::Operate);
    let (session, wire) = Session::observed();
    actor.session = Some(session.into());
    (actor, wire)
}

#[tokio::test]
async fn remembered_control_overrides_default_but_never_readonly_authorization() {
    for (choice, auto, readonly, permission, expected) in [
        (Some(true), false, false, Permission::Operate, true),
        (Some(false), true, false, Permission::Operate, false),
        (None, true, false, Permission::Operate, true),
        (None, false, false, Permission::Operate, false),
        (Some(true), true, true, Permission::Operate, false),
        (Some(true), true, false, Permission::ReadOnly, false),
    ] {
        let (mut actor, mut wire) = actor_with_wire();
        actor.remote_auto_control = auto;
        actor.model.permission = Some(permission);
        let attachment = Attachment {
            grant: "terminal".into(),
            pane: None,
        };
        if let Some(choice) = choice {
            actor.control_choices.insert(attachment.clone(), choice);
        }
        actor.outstanding.insert(
            99,
            Outstanding {
                kind: Pending::Attach(attachment),
                expires: Instant::now() + Duration::from_secs(30),
            },
        );
        actor
            .response(Response {
                id: 99,
                event: Event::TerminalAttached {
                    stream: "stream".into(),
                    name: "shell".into(),
                    read_only: readonly,
                },
            })
            .await
            .unwrap();
        let request = wire.try_recv().ok();
        assert_eq!(
            request.is_some(),
            expected,
            "choice={choice:?} auto={auto} readonly={readonly}"
        );
        if let Some(request) = request {
            assert!(matches!(
                request.operation,
                Operation::TerminalControl { take: true, .. }
            ));
        }
        assert!(
            !actor.model.panes[0].controller,
            "only server acknowledgement grants control"
        );
    }
}

#[tokio::test]
async fn only_confirmed_explicit_choice_changes_preference_and_unsolicited_loss_keeps_it() {
    let (mut actor, _wire) = actor_with_wire();
    let attachment = Attachment {
        grant: "terminal".into(),
        pane: Some("pane".into()),
    };
    actor.outstanding.insert(
        99,
        Outstanding {
            kind: Pending::Attach(attachment.clone()),
            expires: Instant::now() + Duration::from_secs(30),
        },
    );
    actor
        .response(Response {
            id: 99,
            event: Event::TerminalAttached {
                stream: "s".into(),
                name: "shell".into(),
                read_only: false,
            },
        })
        .await
        .unwrap();
    for (id, take, controller, remembered) in [
        (100, true, false, None),
        (101, true, true, Some(true)),
        (102, false, false, Some(false)),
    ] {
        actor.outstanding.insert(
            id,
            Outstanding {
                kind: Pending::Control {
                    stream: "s".into(),
                    take,
                },
                expires: Instant::now() + Duration::from_secs(30),
            },
        );
        actor
            .response(Response {
                id,
                event: Event::TerminalControl {
                    stream: "s".into(),
                    controller,
                },
            })
            .await
            .unwrap();
        assert_eq!(actor.control_choices.get(&attachment).copied(), remembered);
        actor
            .response(Response {
                id: 0,
                event: Event::TerminalControl {
                    stream: "s".into(),
                    controller: false,
                },
            })
            .await
            .unwrap();
        assert_eq!(actor.control_choices.get(&attachment).copied(), remembered);
    }
    assert!(!actor.control_choices.contains_key(&Attachment {
        grant: "terminal".into(),
        pane: Some("another".into())
    }));
}
