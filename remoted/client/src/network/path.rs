//! Adapts Relay delivery credits and application sends without blocking receive.
use super::{DEADLINE, Transport};
use anyhow::{Context, Result};
use neko_remoted_protocol::messages::ConnectionKind;
use neko_remoted_transport::{Carrier, Guard, Receiver, Role, Sender, Write};
use tokio::sync::{mpsc, oneshot};

// The controller admits at most 64 pending business requests. Reserve another
// transport window for authentication/heartbeat control records. The wire
// consumer window remains eight; this queue only decouples its writer from UI receive.
const REQUEST_QUEUE: usize = 64 + neko_remoted_transport::WINDOW;

pub(super) enum Connection {
    Live(Path),
    #[cfg(test)]
    Raw(Transport),
}

pub(super) struct Path {
    sender: Sender,
    receiver: Receiver,
    requests: mpsc::Sender<Vec<u8>>,
    failure: mpsc::Receiver<anyhow::Error>,
    bridge: tokio::task::JoinHandle<()>,
    shutdown: Option<oneshot::Sender<()>>,
}
impl Drop for Path {
    fn drop(&mut self) {
        self.bridge.abort();
    }
}
impl Connection {
    pub fn start(relay: Transport) -> Self {
        let (outgoing, writes) = mpsc::channel::<Write>(8);
        let (events, incoming) = mpsc::channel(8);
        let (shutdown, stopping) = oneshot::channel();
        let (requests, pending) = mpsc::channel(REQUEST_QUEUE);
        let (failed, failure) = mpsc::channel(1);
        let (sender, receiver, guard) =
            neko_remoted_transport::start(Carrier { outgoing, incoming });
        let bridge = tokio::spawn(
            Bridge {
                relay,
                writes,
                events,
                pending,
                sender: sender.clone(),
                guard,
                stopping,
                failure: failed,
            }
            .run(),
        );
        Self::Live(Path {
            sender,
            receiver,
            requests,
            failure,
            bridge,
            shutdown: Some(shutdown),
        })
    }
    /// Reserve before assigning an ID or encrypting. A full/closed queue has not
    /// accepted this request and must not advance its Noise send nonce.
    pub fn reserve_request(&self) -> Result<Option<mpsc::OwnedPermit<Vec<u8>>>> {
        match self {
            Self::Live(path) => {
                path.requests
                    .clone()
                    .try_reserve_owned()
                    .map(Some)
                    .map_err(|error| match error {
                        mpsc::error::TrySendError::Full(_) => {
                            anyhow::anyhow!("发送队列繁忙，操作未入队")
                        }
                        mpsc::error::TrySendError::Closed(_) => {
                            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "发送队列已关闭")
                                .into()
                        }
                    })
            }
            #[cfg(test)]
            Self::Raw(_) => Ok(None),
        }
    }
    /// Handshake records must be physically accepted before opening the Session.
    pub async fn send(&mut self, data: Vec<u8>) -> Result<()> {
        match self {
            Self::Live(path) => {
                let mut guard = super::AbortOnDrop(Some(path.bridge.abort_handle()));
                let result = tokio::time::timeout(DEADLINE, path.sender.send(data))
                    .await
                    .context("发送超时，操作结果未知，请勿自动重试")?;
                if result.is_ok() {
                    guard.0 = None;
                }
                result
            }
            #[cfg(test)]
            Self::Raw(raw) => raw.send(data).await,
        }
    }
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        match self {
            Self::Live(path) => {
                // Drain validated records before surfacing a saved writer/socket
                // error, including a final authenticated denial followed by EOF.
                match path.receiver.recv().await {
                    Ok(data) => Ok(data),
                    Err(error) => Err(path.failure.try_recv().unwrap_or(error)),
                }
            }
            #[cfg(test)]
            Self::Raw(raw) => raw.receive().await,
        }
    }
    pub fn enable_direct(&self, key: [u8; 32]) -> Result<()> {
        match self {
            Self::Live(path) => path.sender.enable_direct(Role::Initiator, key),
            #[cfg(test)]
            Self::Raw(_) => Ok(()),
        }
    }
    pub fn kind(&self) -> ConnectionKind {
        match self {
            Self::Live(path) => path.sender.kind(),
            #[cfg(test)]
            Self::Raw(_) => ConnectionKind::Relay,
        }
    }
    pub fn fail(&mut self) {
        match self {
            Self::Live(path) => path.bridge.abort(),
            #[cfg(test)]
            Self::Raw(raw) => {
                raw.failed = true;
                raw.task.abort();
            }
        }
    }
    pub async fn close(mut self) {
        match &mut self {
            Self::Live(path) => {
                if let Some(stop) = path.shutdown.take() {
                    let _ = stop.send(());
                }
                if tokio::time::timeout(DEADLINE, &mut path.bridge)
                    .await
                    .is_err()
                {
                    path.bridge.abort();
                }
            }
            #[cfg(test)]
            Self::Raw(raw) => raw.close().await,
        }
    }
}

struct Bridge {
    relay: Transport,
    writes: mpsc::Receiver<Write>,
    events: mpsc::Sender<Result<Vec<u8>>>,
    pending: mpsc::Receiver<Vec<u8>>,
    sender: Sender,
    guard: Guard,
    stopping: oneshot::Receiver<()>,
    failure: mpsc::Sender<anyhow::Error>,
}
impl Bridge {
    async fn run(mut self) {
        let raw_out = self.relay.outgoing.clone();
        let result: Result<()> = tokio::select! {
            biased;
            _ = &mut self.stopping => Ok(()),
            // Retain the original socket/protocol error before observing actor
            // termination caused by its writer disappearing.
            result = read_relay(&mut self.relay, self.events, &self.failure) => result,
            result = self.guard.join() => result,
            result = write_relay(self.writes, raw_out) => result,
            result = write_requests(self.pending, self.sender) => result,
        };
        if let Err(error) = result {
            let _ = self.failure.try_send(error);
        }
        self.relay.close().await;
    }
}

async fn read_relay(
    relay: &mut Transport,
    events: mpsc::Sender<Result<Vec<u8>>>,
    failure: &mpsc::Sender<anyhow::Error>,
) -> Result<()> {
    loop {
        let received = relay.receive().await;
        let failed = received.is_err();
        let received = match received {
            Ok(data) => Ok(data),
            Err(error) => {
                let _ = failure.try_send(error);
                Err(
                    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Relay接收已结束")
                        .into(),
                )
            }
        };
        // Put EOF/error after earlier wire records, then let the actor finish
        // naturally. Cancelling its Guard here could lose the final response.
        if events.send(received).await.is_err() || failed {
            return std::future::pending().await;
        }
    }
}

async fn write_relay(
    mut writes: mpsc::Receiver<Write>,
    outgoing: mpsc::Sender<super::Write>,
) -> Result<()> {
    loop {
        let Some(write) = writes.recv().await else {
            return std::future::pending().await;
        };
        let result = tokio::time::timeout(DEADLINE, async {
            let (completed, ack) = oneshot::channel();
            outgoing
                .send(super::Write {
                    data: write.data,
                    completed,
                })
                .await
                .map_err(|_| super::LinkClosed)?;
            ack.await.map_err(|_| super::LinkClosed)?
        })
        .await
        .context("Relay发送超时，操作结果未知")
        .and_then(|result| result);
        let failed = result.is_err();
        let _ = write.completed.send(result);
        // The actor retains this actual carrier failure. Let its join complete
        // rather than cancelling it before that terminal cause has been saved.
        if failed {
            return std::future::pending().await;
        }
    }
}

async fn write_requests(mut pending: mpsc::Receiver<Vec<u8>>, sender: Sender) -> Result<()> {
    while let Some(data) = pending.recv().await {
        tokio::time::timeout(DEADLINE, sender.send(data))
            .await
            .context("发送超时，操作结果未知，请勿自动重试")??;
    }
    Ok(())
}

#[cfg(test)]
impl From<Transport> for Connection {
    fn from(raw: Transport) -> Self {
        Self::Raw(raw)
    }
}
#[cfg(test)]
mod tests;
