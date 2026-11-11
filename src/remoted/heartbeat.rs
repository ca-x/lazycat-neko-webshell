use anyhow::{Result, ensure};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Heartbeat {
    pending: Option<(String, Instant)>,
}
impl Heartbeat {
    pub fn tick(&mut self) -> Result<Option<String>> {
        self.tick_at(Instant::now())
    }
    fn tick_at(&mut self, now: Instant) -> Result<Option<String>> {
        if let Some((_, sent)) = &self.pending {
            ensure!(
                now.duration_since(*sent) < Duration::from_secs(30),
                "heartbeat timed out"
            );
            return Ok(None);
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        self.pending = Some((nonce.clone(), now));
        Ok(Some(nonce))
    }
    pub fn pong(&mut self, nonce: &str) -> Option<u64> {
        self.pong_at(nonce, Instant::now())
    }
    fn pong_at(&mut self, nonce: &str, now: Instant) -> Option<u64> {
        let (expected, sent) = self.pending.as_ref()?;
        if expected != nonce {
            return None;
        }
        let elapsed = now.duration_since(*sent);
        if elapsed >= Duration::from_secs(30) {
            return None;
        }
        self.pending = None;
        Some(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_and_mismatched_pongs_do_not_replace_pending_nonce() {
        let mut heartbeat = Heartbeat::default();
        let now = Instant::now();
        let nonce = heartbeat.tick_at(now).unwrap().unwrap();
        assert!(
            heartbeat
                .tick_at(now + Duration::from_secs(10))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            heartbeat.pong_at("wrong", now + Duration::from_secs(11)),
            None
        );
        assert!(
            heartbeat
                .tick_at(now + Duration::from_secs(20))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            heartbeat.pong_at(&nonce, now + Duration::from_secs(25)),
            Some(25000)
        );
        assert!(
            heartbeat
                .tick_at(now + Duration::from_secs(30))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn missing_response_expires_at_thirty_seconds() {
        let mut heartbeat = Heartbeat::default();
        let now = Instant::now();
        let nonce = heartbeat.tick_at(now).unwrap().unwrap();
        assert!(heartbeat.tick_at(now + Duration::from_secs(30)).is_err());
        assert_eq!(
            heartbeat.pong_at(&nonce, now + Duration::from_secs(30)),
            None
        );
    }
}
