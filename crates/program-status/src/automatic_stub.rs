use crate::ProgramRecord;

#[derive(Default)]
pub(crate) struct Automatic;

// Keep the same receiver-based API as the Unix detector on unsupported targets.
#[allow(clippy::unused_self)]
impl Automatic {
    pub fn foreground(&mut self, _process_group: Option<u32>, _cols: u16, _rows: u16) -> bool {
        false
    }

    pub fn output(&mut self, _data: &[u8]) {}

    pub fn input(&mut self, _data: &[u8]) {}

    pub fn resize(&mut self, _cols: u16, _rows: u16) {}

    pub fn record(&mut self) -> Option<ProgramRecord> {
        None
    }
}

pub fn hook_identity(_agent: &str, _helper_pid: u32, _group: u32) -> Option<(u32, u64)> {
    None
}
