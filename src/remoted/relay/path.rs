//! Per-peer adaptation; carrier writes never suspend inbound credit/overlay ACKs.
use super::{Input, Link};
use anyhow::{Context, Result, ensure};
use neko_remoted_protocol::{self as protocol, messages::RelayFrame};
use neko_remoted_transport::{Carrier, Receiver, Role, Write};
use std::{sync::Arc, time::Duration};
use tokio::sync::{Semaphore, mpsc};

impl Link {
    pub fn start_transport(
        &self,
        raw_input: mpsc::Receiver<Input>,
    ) -> Result<mpsc::Receiver<Input>> {
        let (outgoing, writes) = mpsc::channel::<Write>(8);
        let (wire_input, incoming) = mpsc::channel(32);
        let (sender, receiver, guard) =
            neko_remoted_transport::start(Carrier { outgoing, incoming });
        ensure!(
            self.transport.set(sender).is_ok(),
            "peer transport already initialized"
        );
        let (output, app_input) = mpsc::channel(9);
        let peer = self.peer.clone();
        let raw_out = self.outgoing.clone();
        let credits = self.credits.clone();
        let cancel = self.cancel.clone();
        tokio::spawn(async move {
            // All workers are futures owned by this supervisor. Cancellation,
            // revocation or failure drops them together, including the RTC guard.
            tokio::select! {
                biased;
                () = cancel.cancelled() => {},
                _ = guard.join() => {},
                _ = read_wire(raw_input, wire_input, output.clone()) => {},
                _ = write_wire(writes, raw_out, credits, peer) => {},
                _ = deliver(receiver, output) => {},
            }
            cancel.cancel();
        });
        Ok(app_input)
    }
    pub fn enable_direct(&self, key: [u8; 32]) -> Result<()> {
        self.transport
            .get()
            .context("peer transport is not initialized")?
            .enable_direct(Role::Responder, key)
    }
}

async fn write_wire(
    mut writes: mpsc::Receiver<Write>,
    outgoing: mpsc::Sender<RelayFrame>,
    credits: Arc<Semaphore>,
    peer: String,
) -> Result<()> {
    while let Some(write) = writes.recv().await {
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            let permit = credits.clone().acquire_owned().await?;
            outgoing
                .send(RelayFrame::Data {
                    peer: peer.clone(),
                    data: protocol::encode(&write.data),
                    sequence: None,
                })
                .await?;
            permit.forget();
            Ok(())
        })
        .await
        .context("carrier send timed out")
        .and_then(|result| result);
        let failed = result.is_err();
        let _ = write.completed.send(result);
        // Let the actor save the carrier's real failure before its guard ends.
        if failed {
            return std::future::pending().await;
        }
    }
    std::future::pending().await
}

async fn read_wire(
    mut input: mpsc::Receiver<Input>,
    wire: mpsc::Sender<Result<Vec<u8>>>,
    output: mpsc::Sender<Input>,
) -> Result<()> {
    loop {
        match input.recv().await.context("carrier disconnected")? {
            // Hello precedes wire data. It never waits behind application data.
            Input::Hello(mode) => output
                .try_send(Input::Hello(mode))
                .context("unexpected Hello overflow")?,
            Input::Data(data) => wire.send(Ok(data)).await?,
            Input::CreditedData { data, receipt } => {
                wire.send(Ok(data)).await?;
                // This is the carrier receipt, independent of application ACKs.
                drop(receipt);
            }
        }
    }
}

async fn deliver(mut receiver: Receiver, output: mpsc::Sender<Input>) -> Result<()> {
    loop {
        // recv commits the overlay's consumption ACK. Reserve first so a slow
        // business consumer applies bounded backpressure without losing records.
        let permit = output.reserve().await?;
        let data = receiver.recv().await?;
        permit.send(Input::Data(data));
    }
}

#[cfg(test)]
mod tests;
