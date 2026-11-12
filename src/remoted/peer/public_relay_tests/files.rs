//! Real Python transfer helper under a fresh fd-scoped temporary root.
use super::*;
use crate::remoted::store::RootIdentity;
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;

fn digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}

async fn download(client: &mut Client, grant: &Grant, expected: &[u8], hash: &str) {
    let id = client
        .request(Operation::FileDownload {
            grant: grant.id.clone(),
            path: "artifact.bin".into(),
        })
        .await;
    let Event::TransferStarted { transfer, size } = client.response(id).await else {
        panic!("real helper download must start")
    };
    assert_eq!(size, u64::try_from(expected.len()).unwrap());
    let mut bytes = Vec::new();
    loop {
        let id = client
            .request(Operation::TransferAcknowledge {
                transfer: transfer.clone(),
                offset: u64::try_from(bytes.len()).unwrap(),
            })
            .await;
        match client.response(id).await {
            Event::FileChunk {
                transfer: actual,
                offset,
                data,
            } => {
                assert_eq!(actual, transfer);
                assert_eq!(offset, u64::try_from(bytes.len()).unwrap());
                bytes.extend(protocol::decode(&data, protocol::FILE_CHUNK).unwrap());
            }
            Event::TransferComplete {
                transfer: actual,
                size,
                sha256,
            } => {
                assert_eq!(actual, transfer);
                assert_eq!(size, u64::try_from(expected.len()).unwrap());
                assert_eq!(sha256, hash);
                break;
            }
            other => panic!("unexpected real file event: {other:?}"),
        }
    }
    assert_eq!(bytes, expected);
    assert_eq!(digest(&bytes), hash);
}

async fn upload(
    client: &mut Client,
    grant: &Grant,
    root: &std::path::Path,
    bytes: &[u8],
    hash: &str,
) {
    let id = client
        .request(Operation::FileUpload {
            grant: grant.id.clone(),
            path: "artifact.bin".into(),
            size: u64::try_from(bytes.len()).unwrap(),
            sha256: hash.into(),
            overwrite: false,
        })
        .await;
    let Event::TransferStarted { transfer, .. } = client.response(id).await else {
        panic!("real helper upload must start")
    };
    for (index, chunk) in bytes.chunks(protocol::FILE_CHUNK).enumerate() {
        assert!(
            !root.join("artifact.bin").exists(),
            "uncommitted upload must not be visible"
        );
        let offset = u64::try_from(index * protocol::FILE_CHUNK).unwrap();
        let id = client
            .request(Operation::FileChunk {
                transfer: transfer.clone(),
                offset,
                data: protocol::encode(chunk),
            })
            .await;
        assert!(
            matches!(client.response(id).await, Event::TransferProgress { offset: actual, .. } if actual == offset + u64::try_from(chunk.len()).unwrap())
        );
    }
    let id = client
        .request(Operation::FileCommit {
            transfer: transfer.clone(),
        })
        .await;
    assert!(
        matches!(client.response(id).await, Event::TransferComplete { transfer: actual, size, sha256 } if actual == transfer && size == u64::try_from(bytes.len()).unwrap() && sha256 == hash)
    );
    assert_eq!(std::fs::read(root.join("artifact.bin")).unwrap(), bytes);
}

pub(super) async fn run(
    state: &Arc<AppState>,
    relay: &RelayAddress,
    adapter: crate::plugin::http::terminal_mcp_herdr_adapter::HerdrTerminalAdapter,
) {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("neko-public-files-{}", uuid::Uuid::new_v4())));
    let root = scratch.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    let info = std::fs::metadata(&root).unwrap();
    let grant = Grant {
        id: uuid::Uuid::new_v4().to_string(),
        session: uuid::Uuid::new_v4().to_string(),
        selector: "isolated@owner".into(),
        login: "test-user".into(),
        backend: "webshell".into(),
        name: "temporary file root".into(),
        target_signature: String::new(),
        root: Some(root.to_str().unwrap().into()),
        root_identity: Some(RootIdentity {
            device: info.dev(),
            inode: info.ino(),
            uid: info.uid(),
        }),
    };
    let target = test_target::Scope::new(state.clone(), grant.clone(), adapter);
    target.enable_local_files();
    let bytes: Vec<u8> = (0..37_001)
        .map(|i| u8::try_from(i % 256).unwrap())
        .collect();
    let hash = digest(&bytes);
    let mut operator = pair_and_approve(state, relay, &grant, Permission::Operate).await;
    upload(&mut operator, &grant, &root, &bytes, &hash).await;
    let id = operator
        .request(Operation::FileList {
            grant: grant.id.clone(),
            path: String::new(),
        })
        .await;
    assert!(
        matches!(operator.response(id).await, Event::FileList { entries, .. } if entries.iter().any(|entry| entry.name == "artifact.bin" && entry.size == u64::try_from(bytes.len()).unwrap()))
    );
    download(&mut operator, &grant, &bytes, &hash).await;
    let id = operator
        .request(Operation::FileDownload {
            grant: grant.id.clone(),
            path: "../escape.bin".into(),
        })
        .await;
    assert!(matches!(
        operator.response(id).await,
        Event::Error {
            outcome_unknown: false,
            ..
        }
    ));
    let _ = operator.socket.close(None).await;
    let mut observer = pair_and_approve(state, relay, &grant, Permission::ReadOnly).await;
    download(&mut observer, &grant, &bytes, &hash).await;
    let id = observer
        .request(Operation::FileUpload {
            grant: grant.id.clone(),
            path: "forbidden.bin".into(),
            size: 0,
            sha256: digest(&[]),
            overwrite: false,
        })
        .await;
    assert!(
        matches!(observer.response(id).await, Event::Error { code, outcome_unknown: false, .. } if code == "AUTHORIZATION_FAILED")
    );
    assert!(!root.join("forbidden.bin").exists());
    assert_eq!(std::fs::read(root.join("artifact.bin")).unwrap(), bytes);
    let _ = observer.socket.close(None).await;
    eprintln!(
        "PASS real public encrypted Python file upload (2 chunks, atomic commit) -> list -> download; readonly download and upload denial; path escape denied; bytes=37001 SHA256={hash}"
    );
    eprintln!(
        "BOUNDARY files: exact production Python helper and server transfer state machine; only platform target dispatch substitutes a local child inside a scoped test target"
    );
}
