use super::store::{Device, Grant, Remoted, Saved};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub device_id: String,
    pub generation: u64,
    pub peer: String,
    pub request_id: u64,
    pub operation: &'static str,
    pub submitted: bool,
    pub mutating: bool,
    pub interrupted: bool,
    #[serde(flatten)]
    pub outcome: Outcome,
    #[serde(skip)]
    cancel: CancellationToken,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub outcome_unknown: bool,
    pub result: Option<String>,
}

#[derive(Default)]
pub struct Operations {
    active: HashMap<String, Record>,
    recent: VecDeque<Record>,
}

#[derive(Debug)]
pub struct Busy;
impl std::fmt::Display for Busy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("operation capacity reached")
    }
}
impl std::error::Error for Busy {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub in_flight_unknown: usize,
    pub active: Vec<Record>,
    pub recent: Vec<Record>,
}

fn current<'a>(saved: &'a Saved, device: &str, generation: u64) -> Result<&'a Device> {
    ensure!(saved.enabled, "远程控制已关闭");
    saved
        .devices
        .iter()
        .find(|item| item.id == device && item.generation == generation)
        .context("设备授权已撤销或变化")
}

pub struct Admission {
    manager: Arc<Remoted>,
    id: String,
    submitted: AtomicBool,
    finished: bool,
}

impl Remoted {
    pub fn admit(
        self: &Arc<Self>,
        device: &Device,
        peer: &str,
        request_id: u64,
        cancel: CancellationToken,
    ) -> Result<Admission> {
        let saved = self
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("remote settings unavailable"))?;
        current(&saved, &device.id, device.generation)?;
        ensure!(!cancel.is_cancelled(), "connection cancelled");
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("operation gate unavailable"))?;
        if operations.active.len() >= neko_remoted_protocol::MAX_PEERS {
            return Err(Busy).context("host operation capacity reached");
        }
        if operations
            .active
            .values()
            .any(|record| record.device_id == device.id)
        {
            return Err(Busy).context("device already has a pending operation");
        }
        let id = uuid::Uuid::new_v4().to_string();
        operations.active.insert(
            id.clone(),
            Record {
                id: id.clone(),
                device_id: device.id.clone(),
                generation: device.generation,
                peer: peer.to_owned(),
                request_id,
                operation: "business",
                submitted: false,
                mutating: false,
                interrupted: false,
                outcome: Outcome::default(),
                cancel,
            },
        );
        Ok(Admission {
            manager: self.clone(),
            id,
            submitted: AtomicBool::new(false),
            finished: false,
        })
    }

    pub fn operation_status(&self) -> Result<Status> {
        let saved = self
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("remote settings unavailable"))?;
        let operations = self
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("operation gate unavailable"))?;
        let active: Vec<_> = operations
            .active
            .values()
            .cloned()
            .map(|mut record| {
                record.interrupted = record.cancel.is_cancelled()
                    || current(&saved, &record.device_id, record.generation).is_err();
                record.outcome.outcome_unknown =
                    record.submitted && record.mutating && record.interrupted;
                record
            })
            .collect();
        Ok(Status {
            in_flight_unknown: active
                .iter()
                .filter(|record| record.outcome.outcome_unknown)
                .count(),
            active,
            recent: operations.recent.iter().cloned().collect(),
        })
    }
}

impl Admission {
    pub fn describe(&self, operation: &'static str) -> Result<()> {
        let mut operations = self
            .manager
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("operation gate unavailable"))?;
        operations
            .active
            .get_mut(&self.id)
            .context("operation admission expired")?
            .operation = operation;
        Ok(())
    }
    pub fn submit(&self, mutating: bool) -> Result<()> {
        self.submit_checked(mutating, None)
    }

    pub(super) fn submit_creation(&self, parent: &Grant) -> Result<()> {
        self.submit_checked(true, Some(parent))
    }

    fn submit_checked(&self, mutating: bool, parent: Option<&Grant>) -> Result<()> {
        let saved = self
            .manager
            .saved
            .lock()
            .map_err(|_| anyhow::anyhow!("remote settings unavailable"))?;
        let mut operations = self
            .manager
            .operations
            .lock()
            .map_err(|_| anyhow::anyhow!("operation gate unavailable"))?;
        let record = operations
            .active
            .get_mut(&self.id)
            .context("operation admission expired")?;
        let device = current(&saved, &record.device_id, record.generation)?;
        ensure!(
            !record.cancel.is_cancelled(),
            "connection cancelled before dispatch"
        );
        ensure!(
            !mutating || device.permission.can_write(),
            "只读设备不能执行写操作"
        );
        if let Some(parent) = parent {
            ensure!(
                device
                    .grants
                    .iter()
                    .any(|grant| grant.same_authority(parent)),
                "parent grant changed before creation"
            );
            ensure!(device.grants.len() < 32, "device resource limit reached");
        }
        record.submitted = true;
        record.mutating |= mutating;
        self.submitted.store(true, Ordering::Release);
        Ok(())
    }

    pub fn mutation_submitted(&self) -> bool {
        self.manager.operations.lock().map_or(true, |operations| {
            operations
                .active
                .get(&self.id)
                .is_none_or(|record| record.submitted && record.mutating)
        })
    }

    pub fn submitted(&self) -> bool {
        self.submitted.load(Ordering::Acquire)
    }

    pub fn finish(&mut self, success: bool, outcome_unknown: bool) {
        let saved = self.manager.saved.lock().ok();
        if let Ok(mut operations) = self.manager.operations.lock()
            && let Some(mut record) = operations.active.remove(&self.id)
        {
            record.interrupted = record.cancel.is_cancelled()
                || saved.as_ref().is_none_or(|saved| {
                    current(saved, &record.device_id, record.generation).is_err()
                });
            record.outcome.outcome_unknown =
                !success && record.submitted && record.mutating && outcome_unknown;
            record.outcome.result = Some(
                if success {
                    "confirmed"
                } else if record.outcome.outcome_unknown {
                    "unknown"
                } else {
                    "failed"
                }
                .into(),
            );
            operations.recent.push_front(record);
            operations.recent.truncate(64);
        }
        self.finished = true;
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(false, self.submitted());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neko_remoted_protocol::messages::Permission;

    fn fixture() -> (Arc<Remoted>, Device) {
        let database = Arc::new(crate::database::AppDatabase::open(":memory:".into()).unwrap());
        let manager = Arc::new(Remoted::load(database).unwrap());
        let device = Device {
            id: "device".into(),
            name: "device".into(),
            public_key: String::new(),
            permission: Permission::Operate,
            grants: Vec::new(),
            generation: 1,
        };
        let mut saved = manager.snapshot().unwrap();
        saved.enabled = true;
        saved.devices.push(device.clone());
        manager.persist(saved).unwrap();
        (manager, device)
    }

    #[test]
    fn one_slot_per_device_across_connections_and_stale_admissions_fail() {
        let (manager, device) = fixture();
        let admission = manager
            .admit(&device, "first", 1, CancellationToken::new())
            .unwrap();
        assert!(
            manager
                .admit(&device, "second", 2, CancellationToken::new())
                .is_err()
        );
        let mut saved = manager.snapshot().unwrap();
        saved.devices[0].generation += 1;
        manager.persist(saved).unwrap();
        assert!(admission.submit(true).is_err());
        assert!(!manager.operation_status().unwrap().active[0].submitted);
        drop(admission);
        assert!(
            manager
                .admit(&device, "second", 2, CancellationToken::new())
                .is_err()
        );
        let current = manager.snapshot().unwrap().devices[0].clone();
        assert!(
            manager
                .admit(&current, "second", 3, CancellationToken::new())
                .is_ok()
        );
    }

    #[test]
    fn revoke_reports_dispatched_write_without_waiting_and_confirmation_resolves_unknown() {
        let (manager, device) = fixture();
        let mut admission = manager
            .admit(&device, "peer", 9, CancellationToken::new())
            .unwrap();
        admission.submit(true).unwrap();
        let mut saved = manager.snapshot().unwrap();
        saved.devices.clear();
        manager.persist(saved).unwrap();
        let status = manager.operation_status().unwrap();
        assert_eq!(status.in_flight_unknown, 1);
        assert_eq!(status.active[0].request_id, 9);
        assert!(status.active[0].interrupted);
        assert!(admission.submit(true).is_err());
        admission.finish(true, true);
        let status = manager.operation_status().unwrap();
        assert!(status.active.is_empty());
        assert!(!status.recent[0].outcome.outcome_unknown);
        assert_eq!(
            status.recent[0].outcome.result.as_deref(),
            Some("confirmed")
        );
    }

    #[test]
    fn disable_readonly_and_cancel_before_dispatch_never_enter_target() {
        for action in ["disable", "readonly", "cancel"] {
            let (manager, device) = fixture();
            let cancel = CancellationToken::new();
            let admission = manager.admit(&device, "peer", 1, cancel.clone()).unwrap();
            let mut saved = manager.snapshot().unwrap();
            match action {
                "disable" => saved.enabled = false,
                "readonly" => saved.devices[0].permission = Permission::ReadOnly,
                _ => cancel.cancel(),
            }
            manager.persist(saved).unwrap();
            assert!(admission.submit(true).is_err());
            assert_eq!(manager.operation_status().unwrap().in_flight_unknown, 0);
        }
    }

    #[test]
    fn interrupted_ambiguous_write_keeps_gate_until_finish_and_retains_unknown_history() {
        let (manager, device) = fixture();
        let cancel = CancellationToken::new();
        let mut admission = manager.admit(&device, "peer", 1, cancel.clone()).unwrap();
        admission.submit(true).unwrap();
        cancel.cancel();
        assert!(
            manager
                .admit(&device, "reconnect", 2, CancellationToken::new())
                .is_err()
        );
        assert_eq!(manager.operation_status().unwrap().in_flight_unknown, 1);
        admission.finish(false, true);
        assert!(
            manager.operation_status().unwrap().recent[0]
                .outcome
                .outcome_unknown
        );
        assert!(
            manager
                .admit(&device, "reconnect", 2, CancellationToken::new())
                .is_ok()
        );
    }

    #[test]
    fn concurrent_generation_change_and_submission_have_one_order() {
        for _ in 0..32 {
            let (manager, device) = fixture();
            let admission = manager
                .admit(&device, "peer", 1, CancellationToken::new())
                .unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let started = barrier.clone();
            let task = std::thread::spawn(move || {
                started.wait();
                let submitted = admission.submit(true).is_ok();
                (admission, submitted)
            });
            barrier.wait();
            let mut saved = manager.snapshot().unwrap();
            saved.devices[0].generation += 1;
            manager.persist(saved).unwrap();
            let (admission, submitted) = task.join().unwrap();
            assert_eq!(
                manager.operation_status().unwrap().in_flight_unknown,
                usize::from(submitted)
            );
            assert!(admission.submit(true).is_err());
        }
    }

    #[test]
    fn abandoned_workers_and_history_are_bounded_without_forgetting_active_dispatches() {
        let (manager, device) = fixture();
        let mut saved = manager.snapshot().unwrap();
        for index in 1..=neko_remoted_protocol::MAX_PEERS {
            let mut other = device.clone();
            other.id = format!("device-{index}");
            saved.devices.push(other);
        }
        manager.persist(saved.clone()).unwrap();
        let held: Vec<_> = saved
            .devices
            .iter()
            .take(neko_remoted_protocol::MAX_PEERS)
            .map(|device| {
                manager
                    .admit(device, "peer", 1, CancellationToken::new())
                    .unwrap()
            })
            .collect();
        assert!(
            manager
                .admit(
                    saved.devices.last().unwrap(),
                    "peer",
                    1,
                    CancellationToken::new()
                )
                .err()
                .unwrap()
                .is::<Busy>()
        );
        drop(held);
        for request in 0..70 {
            drop(
                manager
                    .admit(&device, "peer", request, CancellationToken::new())
                    .unwrap(),
            );
        }
        let status = manager.operation_status().unwrap();
        assert!(status.active.is_empty());
        assert_eq!(status.recent.len(), 64);
        assert_eq!(status.recent[0].request_id, 69);
    }
}
