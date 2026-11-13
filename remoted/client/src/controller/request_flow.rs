//! Pace terminal input before the host's bounded business FIFO.
//! Unsent input is queued once, never replayed after a connection changes.
use super::*;

const IN_FLIGHT: usize = 4;
const MAX_QUEUED: usize = 128;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const INPUT_BYTES: usize = 16 * 1024;

pub(super) struct Queued {
    operation: Operation,
    pub kind: Pending,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct Requests {
    queue: VecDeque<Queued>,
    bytes: usize,
}

impl Requests {
    pub fn pending(&self) -> impl Iterator<Item = &Pending> {
        self.queue.iter().map(|request| &request.kind)
    }

    fn push(&mut self, operation: Operation, kind: Pending) -> Result<()> {
        let bytes = serde_json::to_vec(&operation)?.len();
        ensure!(
            self.bytes + bytes <= MAX_BYTES,
            "输入尚未发送：终端输入缓冲已满，请等待当前输入完成"
        );
        if matches!(kind, Pending::Ordinary)
            && let Some(last) = self.queue.back_mut()
            && matches!(last.kind, Pending::Ordinary)
        {
            match (&mut last.operation, &operation) {
                (
                    Operation::TerminalInput { stream, data },
                    Operation::TerminalInput {
                        stream: next_stream,
                        data: next,
                    },
                ) if stream == next_stream => {
                    let mut combined = protocol::decode(data, INPUT_BYTES)?;
                    let next = protocol::decode(next, INPUT_BYTES)?;
                    if combined.len() + next.len() <= INPUT_BYTES {
                        combined.extend(next);
                        *data = protocol::encode(&combined);
                        let next_bytes = serde_json::to_vec(&last.operation)?.len();
                        self.bytes = self.bytes - last.bytes + next_bytes;
                        last.bytes = next_bytes;
                        return Ok(());
                    }
                }
                (
                    Operation::TerminalResize { stream, cols, rows },
                    Operation::TerminalResize {
                        stream: next_stream,
                        cols: next_cols,
                        rows: next_rows,
                    },
                ) if stream == next_stream => {
                    *cols = *next_cols;
                    *rows = *next_rows;
                    self.bytes = self.bytes - last.bytes + bytes;
                    last.bytes = bytes;
                    return Ok(());
                }
                _ => {}
            }
        }
        ensure!(
            self.queue.len() < MAX_QUEUED,
            "操作尚未发送：终端操作队列已满，请等待当前操作完成"
        );
        self.bytes += bytes;
        self.queue.push_back(Queued {
            operation,
            kind,
            bytes,
        });
        Ok(())
    }

    fn pop(&mut self) -> Option<Queued> {
        let request = self.queue.pop_front()?;
        self.bytes -= request.bytes;
        Some(request)
    }

    pub fn forget_stream(&mut self, stream: &str) -> Vec<u64> {
        let mut rejected = Vec::new();
        self.queue.retain(|request| !matches!(&request.operation,
            Operation::TerminalInput { stream: target, .. } | Operation::TerminalResize { stream: target, .. } | Operation::TerminalControl { stream: target, .. } if target == stream && {
                if let Pending::Input(id) = request.kind { rejected.push(id); }
                true
            }));
        self.bytes = self.queue.iter().map(|request| request.bytes).sum();
        rejected
    }
}

impl Actor {
    pub(super) fn pending_requests(&self) -> impl Iterator<Item = &Pending> {
        self.outstanding
            .values()
            .map(|request| &request.kind)
            .chain(self.requests.pending())
    }
    pub(super) async fn request(&mut self, operation: Operation, kind: Pending) -> Result<()> {
        ensure!(self.session.is_some(), "尚未连接主机");
        // Cancellation has its own reserved host slot and must pass a busy data lane.
        if matches!(operation, Operation::Cancel { .. }) {
            return self.transmit(operation, kind).await;
        }
        let input = if let Pending::Input(id) = kind {
            Some(id)
        } else {
            None
        };
        if let Err(error) = self.requests.push(operation, kind) {
            if let Some(id) = input {
                self.model.interaction.reject(id);
            }
            return Err(error);
        }
        self.flush_requests().await
    }

    pub(super) fn opening_panes(&self) -> usize {
        self.pending_requests()
            .filter(|kind| {
                matches!(
                    kind,
                    Pending::Create { .. } | Pending::Attach(_) | Pending::Restore { .. }
                )
            })
            .count()
    }

    pub(super) async fn flush_requests(&mut self) -> Result<()> {
        while self.outstanding.len() < IN_FLIGHT {
            let Some(request) = self.requests.pop() else {
                break;
            };
            // Permissions and stream liveness can change while input waits for
            // a host slot. Known-unsent input must not cross that transition.
            if matches!(
                request.operation,
                Operation::TerminalInput { .. } | Operation::TerminalResize { .. }
            ) && let Err(error) = self.validate(&request.operation)
            {
                if let Pending::Input(id) = request.kind {
                    self.model.interaction.reject(id);
                }
                self.model.notice = format!("输入未发送：{error}");
                continue;
            }
            self.transmit(request.operation, request.kind).await?;
        }
        Ok(())
    }

    pub(super) fn reject_stream_unsent(&mut self, stream: &str) {
        for id in self.requests.forget_stream(stream) {
            self.model.interaction.reject(id);
        }
    }

    pub(super) fn reject_unsent(&mut self) {
        while let Some(request) = self.requests.pop() {
            if let Pending::Input(id) = request.kind {
                self.model.interaction.reject(id);
            }
        }
    }
}
