//! Real relay carrier used by the acceptance fixtures, sharing production framing.
use super::*;
use neko_remoted_transport::{Carrier, Guard, Receiver, Sender, Write};

pub(super) struct WireSocket {
    tx: Sender,
    rx: Receiver,
    _guard: Guard,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for WireSocket {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl WireSocket {
    pub fn new(mut socket: Socket, peer: String) -> Self {
        let (outgoing, mut writes) = mpsc::channel::<Write>(8);
        let (events, incoming) = mpsc::channel(32);
        let (tx, rx, guard) = neko_remoted_transport::start(Carrier { outgoing, incoming });
        let task = tokio::spawn(async move {
            let mut outstanding = 0u8;
            loop {
                tokio::select! {
                    write = writes.recv(), if outstanding < 8 => {
                        let Some(write) = write else { return; };
                        let frame = RelayFrame::Data { peer: peer.clone(), data: protocol::encode(&write.data), sequence: None };
                        let result = async {
                            timeout(Duration::from_secs(5), socket.send(Message::Text(serde_json::to_string(&frame)?.into()))).await??;
                            Ok::<(), anyhow::Error>(())
                        }.await;
                        let failed = result.is_err();
                        let _ = write.completed.send(result);
                        if failed { return; }
                        outstanding += 1;
                    }
                    message = socket.next() => {
                        let Some(Ok(message)) = message else { return; };
                        match message {
                            Message::Text(text) => match serde_json::from_str::<RelayFrame>(&text).unwrap() {
                                RelayFrame::Data { data, sequence: Some(sequence), .. } => {
                                    let data = protocol::decode(&data, protocol::MAX_WIRE_PAYLOAD).unwrap();
                                    if events.try_send(Ok(data)).is_err() { return; }
                                    send_frame(&mut socket, RelayFrame::Acknowledge { peer: peer.clone(), sequence }).await;
                                }
                                RelayFrame::Delivered { count, .. } => { outstanding = outstanding.checked_sub(count).unwrap(); }
                                RelayFrame::Pong { .. } => {}
                                _ => return,
                            }
                            Message::Ping(data) => { if socket.send(Message::Pong(data)).await.is_err() { return; } }
                            Message::Pong(_) => {}
                            _ => return,
                        }
                    }
                }
            }
        });
        Self {
            tx,
            rx,
            _guard: guard,
            task,
        }
    }
    pub async fn send(&self, bytes: Vec<u8>) {
        self.tx.send(bytes).await.unwrap();
    }
    pub async fn receive(&mut self) -> Vec<u8> {
        self.rx.recv().await.unwrap()
    }
    pub async fn close(&mut self, _: Option<()>) -> Result<()> {
        self.task.abort();
        let _ = (&mut self.task).await;
        Ok(())
    }
}
