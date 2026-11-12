use neko_program_status::{BlockedKind, ProgramState, Tracker};
use neko_remoted_protocol::messages as protocol;

pub(super) struct ProgramStatus {
    tracker: Tracker,
    revision: Option<u64>,
    records: Vec<protocol::ProgramRecord>,
}

impl Default for ProgramStatus {
    fn default() -> Self {
        let mut tracker = Tracker::default();
        let (revision, _) = tracker.snapshot_since(None).unwrap();
        Self {
            tracker,
            revision: Some(revision),
            records: Vec::new(),
        }
    }
}

impl ProgramStatus {
    pub(super) fn foreground(&mut self, group: Option<u32>, cols: u16, rows: u16) {
        self.tracker.foreground(group, cols, rows);
    }

    pub(super) fn observe(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.tracker.observe(bytes)
    }

    pub(super) fn input(&mut self, bytes: &[u8]) {
        self.tracker.input(bytes);
    }

    pub(super) fn resize(&mut self, cols: u16, rows: u16) {
        self.tracker.resize(cols, rows);
    }

    pub(super) fn records(&self) -> &[protocol::ProgramRecord] {
        &self.records
    }

    pub(super) fn changed(&mut self) -> bool {
        let Some((revision, records)) = self.tracker.snapshot_since(self.revision) else {
            return false;
        };
        self.revision = Some(revision);
        self.records = records
            .into_iter()
            .map(|record| protocol::ProgramRecord {
                id: record.id,
                state: match record.state {
                    ProgramState::Idle => protocol::ProgramState::Idle,
                    ProgramState::Working => protocol::ProgramState::Working,
                    ProgramState::Done => protocol::ProgramState::Done,
                    ProgramState::Blocked => protocol::ProgramState::Blocked,
                    ProgramState::Error => protocol::ProgramState::Error,
                },
                kind: record.kind.map(|kind| match kind {
                    BlockedKind::Permission => protocol::BlockedKind::Permission,
                    BlockedKind::Question => protocol::BlockedKind::Question,
                    BlockedKind::Auth => protocol::BlockedKind::Auth,
                }),
                progress: record.progress,
                app: record.app,
                title: record.title,
                msg: record.msg,
            })
            .collect();
        true
    }
}
