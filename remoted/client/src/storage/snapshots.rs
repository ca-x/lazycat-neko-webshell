//! Confirmed shared preferences, ordered by this process's binding writer.
use super::{Binding, HostAliases, HostNames, SavedBindings};
use anyhow::{Context, Result};
use std::sync::{Arc, OnceLock};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub revision: u64,
    pub bindings: Vec<Binding>,
    pub aliases: HostAliases,
    pub host_names: HostNames,
}

pub(crate) type Receiver = watch::Receiver<Option<Arc<Snapshot>>>;

struct Publisher {
    sender: watch::Sender<Option<Arc<Snapshot>>>,
}

impl Publisher {
    fn new() -> Self {
        Self {
            sender: watch::channel(None).0,
        }
    }

    // The caller holds BINDING_WRITER across load, persistence, and publication.
    // Allocate before polling persistence, so overflow cannot save an unpublishable
    // state. Failed/ambiguous writes never publish a confirmed snapshot.
    async fn persist(
        &self,
        saved: &SavedBindings,
        persist: impl std::future::Future<Output = Result<()>>,
    ) -> Result<()> {
        let revision = self
            .sender
            .borrow()
            .as_ref()
            .map_or(0, |snapshot| snapshot.revision)
            .checked_add(1)
            .context("设备偏好同步世代已耗尽，请重新启动客户端")?;
        let snapshot = Arc::new(Snapshot {
            revision,
            bindings: saved.bindings.clone(),
            aliases: saved.aliases.clone(),
            host_names: saved.host_names.clone(),
        });
        persist.await?;
        // Retain the latest value even if no router is currently subscribed.
        let _ = self.sender.send_replace(Some(snapshot));
        Ok(())
    }
}

fn publisher() -> &'static Publisher {
    static PUBLISHER: OnceLock<Publisher> = OnceLock::new();
    PUBLISHER.get_or_init(Publisher::new)
}

pub(super) async fn persist(
    saved: &SavedBindings,
    persist: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    publisher().persist(saved, persist).await
}

pub(crate) fn subscribe() -> Receiver {
    publisher().sender.subscribe()
}

/// A closed fixture/source stays disabled rather than spinning the router.
pub(crate) async fn changed(receiver: &mut Receiver) {
    if receiver.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{BindingIdentity, HostAlias};
    use neko_remoted_protocol::{self as protocol, RelayAddress};
    use std::sync::atomic::{AtomicBool, Ordering};

    fn saved(seed: u8) -> SavedBindings {
        let binding = Binding {
            relay: RelayAddress::parse("wss://snapshots-test.invalid").unwrap(),
            route: protocol::encode(&[seed; 32]),
            host_key: protocol::encode(&[seed.wrapping_add(1); 32]),
            device_id: format!("test-{seed}"),
        };
        let entry = |name: &str| HostAlias {
            identity: BindingIdentity::of(&binding),
            name: name.into(),
        };
        SavedBindings {
            version: 1,
            bindings: vec![binding.clone()],
            selected: None,
            auto_connect: false,
            local: None,
            aliases: HostAliases(vec![entry("alias")]),
            host_names: serde_json::from_value(serde_json::json!([entry("hostname")])).unwrap(),
        }
    }

    #[tokio::test]
    async fn only_acknowledged_writes_publish_complete_monotonic_snapshots() {
        let publisher = Publisher::new();
        let mut receiver = publisher.sender.subscribe();
        let first = saved(1);
        assert!(
            publisher
                .persist(&first, async { anyhow::bail!("ambiguous write") })
                .await
                .is_err()
        );
        assert!(receiver.borrow().is_none());
        assert!(!receiver.has_changed().unwrap());
        publisher
            .persist(&first, async {
                assert!(
                    receiver.borrow().is_none(),
                    "must wait for persistence acknowledgement"
                );
                Ok(())
            })
            .await
            .unwrap();
        let confirmed = receiver.borrow_and_update().clone().unwrap();
        assert_eq!(confirmed.revision, 1);
        assert_eq!(confirmed.bindings, first.bindings);
        assert_eq!(confirmed.aliases, first.aliases);
        assert_eq!(confirmed.host_names, first.host_names);
        let second = saved(5);
        publisher.persist(&second, async { Ok(()) }).await.unwrap();
        let confirmed = receiver.borrow_and_update().clone().unwrap();
        assert_eq!(confirmed.revision, 2);
        assert_eq!(confirmed.bindings, second.bindings);
    }

    #[tokio::test]
    async fn coalesced_snapshots_include_every_table_and_survive_absent_subscribers() {
        let publisher = Publisher::new();
        let first = saved(1);
        publisher.persist(&first, async { Ok(()) }).await.unwrap();
        let mut receiver = publisher.sender.subscribe();
        assert_eq!(receiver.borrow().as_ref().unwrap().revision, 1);
        let mut second = saved(5);
        second.bindings.extend(first.bindings.clone());
        publisher.persist(&second, async { Ok(()) }).await.unwrap();
        let mut third = second;
        third.bindings.remove(0);
        third.aliases = first.aliases.clone();
        third.host_names = first.host_names.clone();
        publisher.persist(&third, async { Ok(()) }).await.unwrap();
        let latest = receiver.borrow_and_update().clone().unwrap();
        assert_eq!(latest.revision, 3);
        assert_eq!(latest.bindings, third.bindings);
        assert_eq!(latest.aliases, third.aliases);
        assert_eq!(latest.host_names, third.host_names);
        assert!(
            !receiver.has_changed().unwrap(),
            "one complete latest snapshot supersedes intermediate writes"
        );
    }

    #[tokio::test]
    async fn revision_exhaustion_does_not_attempt_persistence() {
        let publisher = Publisher::new();
        let _ = publisher.sender.send_replace(Some(Arc::new(Snapshot {
            revision: u64::MAX,
            bindings: Vec::new(),
            aliases: HostAliases::default(),
            host_names: HostNames::default(),
        })));
        let persisted = AtomicBool::new(false);
        assert!(
            publisher
                .persist(&saved(1), async {
                    persisted.store(true, Ordering::Release);
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(!persisted.load(Ordering::Acquire));
    }
}
