//! A Relay credit is returned when the peer removes its frame from the bounded inbox.
//! The queue of receipts is bounded by the Relay's outstanding frame windows; it
//! must not share a queue with bulk output, which can itself be waiting on credits.
use tokio::sync::mpsc;

pub(crate) struct Receipt {
    sequence: u64,
    completed: mpsc::UnboundedSender<u64>,
}

impl Receipt {
    pub(super) fn new(sequence: u64, completed: mpsc::UnboundedSender<u64>) -> Self {
        Self {
            sequence,
            completed,
        }
    }
}
impl Drop for Receipt {
    fn drop(&mut self) {
        let _ = self.completed.send(self.sequence);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn credits_wait_for_consumption_and_other_peers_complete_independently() {
        let (completed, mut acknowledgements) = mpsc::unbounded_channel();
        let (slow, mut slow_inbox) = mpsc::channel(8);
        let (fast, mut fast_inbox) = mpsc::channel(8);
        for sequence in 1..=8 {
            slow.try_send(Receipt::new(sequence, completed.clone()))
                .ok()
                .unwrap();
        }
        assert!(acknowledgements.try_recv().is_err());
        fast.try_send(Receipt::new(9, completed.clone()))
            .ok()
            .unwrap();
        drop(fast_inbox.recv().await.unwrap());
        assert_eq!(acknowledgements.recv().await, Some(9));
        assert!(acknowledgements.try_recv().is_err());
        drop(slow_inbox.recv().await.unwrap());
        assert_eq!(acknowledgements.recv().await, Some(1));
        // Removing a connection returns its remaining bounded slots without
        // requiring the now-closed peer to poll its business requests again.
        drop(slow_inbox);
        for sequence in 2..=8 {
            assert_eq!(acknowledgements.recv().await, Some(sequence));
        }
    }
}
