use super::*;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

struct Fixture {
    link: Link,
    input: mpsc::Sender<Input>,
    output: mpsc::Receiver<Input>,
    writes: mpsc::Receiver<RelayFrame>,
}
impl Fixture {
    fn new(credits: usize) -> Self {
        let (outgoing, writes) = mpsc::channel(8);
        let (input, incoming) = mpsc::channel(9);
        let link = Link {
            transport: Arc::new(OnceLock::new()),
            peer: "fixture".into(),
            outgoing,
            credits: Arc::new(Semaphore::new(credits)),
            cancel: CancellationToken::new(),
        };
        let output = link.start_transport(incoming).unwrap();
        Self {
            link,
            input,
            output,
            writes,
        }
    }
    async fn ack(&mut self, consumed: u64) {
        let frame = timeout(Duration::from_secs(1), self.writes.recv())
            .await
            .unwrap()
            .expect("adapter closed before ACK");
        let RelayFrame::Data { data, .. } = frame else {
            panic!("expected overlay ACK")
        };
        let bytes = protocol::decode(&data, protocol::MAX_WIRE_PAYLOAD).unwrap();
        assert_eq!(&bytes[..5], b"NKT1\x01");
        assert_eq!(u64::from_be_bytes(bytes[5..].try_into().unwrap()), consumed);
        self.link.credits.add_permits(1);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.link.cancel.cancel();
    }
}
fn packet(sequence: u64) -> Vec<u8> {
    let mut bytes = b"NKT1\0".to_vec();
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes
}

#[tokio::test]
async fn slow_consumer_backpressures_before_committing_overlay_ack() {
    let mut fixture = Fixture::new(8);
    for sequence in 0..10 {
        fixture
            .input
            .send(Input::Data(packet(sequence)))
            .await
            .ok()
            .unwrap();
        if sequence < 9 {
            fixture.ack(sequence + 1).await;
        }
    }
    // The app inbox holds nine. The tenth must stay in the transport without
    // being consumed/ACKed or cancelling the otherwise healthy connection.
    assert!(
        timeout(Duration::from_millis(50), fixture.writes.recv())
            .await
            .is_err(),
        "tenth record was ACKed without application capacity, or bridge exited"
    );
    assert!(!fixture.link.cancel.is_cancelled());
    for sequence in 0u64..10 {
        let frame = timeout(Duration::from_secs(1), fixture.output.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(frame, Input::Data(data) if data == sequence.to_be_bytes()));
        if sequence == 0 {
            fixture.ack(10).await;
        }
    }
    assert!(!fixture.link.cancel.is_cancelled());
}

#[tokio::test]
async fn waiting_write_credit_does_not_block_inbound_receipt_or_cancellation() {
    let mut fixture = Fixture::new(0);
    let sender = fixture.link.transport.get().unwrap().clone();
    let mut sending = tokio::spawn(async move { sender.send(b"waiting".to_vec()).await });
    // Let the writer reach its missing-credit wait before submitting inbound ACK.
    assert!(
        timeout(Duration::from_millis(20), &mut sending)
            .await
            .is_err()
    );
    let (completed, mut receipts) = mpsc::unbounded_channel();
    let mut ack = b"NKT1\x01".to_vec();
    ack.extend_from_slice(&0u64.to_be_bytes());
    fixture
        .input
        .send(Input::CreditedData {
            data: ack,
            receipt: super::super::Receipt {
                sequence: 1,
                completed,
            },
        })
        .await
        .ok()
        .unwrap();
    assert_eq!(
        timeout(Duration::from_millis(250), receipts.recv())
            .await
            .expect("inbound receipt blocked behind outbound credit"),
        Some(1)
    );
    assert!(!sending.is_finished());
    fixture.link.cancel.cancel();
    assert!(
        timeout(Duration::from_millis(250), sending)
            .await
            .expect("credit wait did not cancel promptly")
            .unwrap()
            .is_err()
    );
    assert!(
        fixture.writes.try_recv().is_err(),
        "cancelled write reached Relay"
    );
    assert!(fixture.output.recv().await.is_none());
}
