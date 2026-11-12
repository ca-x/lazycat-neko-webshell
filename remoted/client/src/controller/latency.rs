//! Bounded application Ping/Pong over the authenticated Noise session.
use super::*;

const INTERVAL: Duration = Duration::from_secs(10);
const EXPIRY: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Path {
    generation: u64,
    kind: ConnectionKind,
}

struct Probe {
    id: u64,
    nonce: String,
    sent: Instant,
}

#[derive(Default)]
pub(super) struct Latency {
    path: Option<Path>,
    pending: Option<Probe>,
    sent: Option<Instant>,
    sample: Option<(Instant, u64)>,
}

impl Latency {
    fn sync(&mut self, path: Option<Path>, now: Instant) {
        if self.path != path {
            *self = Self {
                path,
                ..Self::default()
            };
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|probe| now.saturating_duration_since(probe.sent) >= EXPIRY)
        {
            self.pending = None;
        }
        if self
            .sample
            .is_some_and(|(at, _)| now.saturating_duration_since(at) >= EXPIRY)
        {
            self.sample = None;
        }
    }

    fn due(&self, now: Instant) -> bool {
        self.path.is_some()
            && self.pending.is_none()
            && self
                .sent
                .is_none_or(|at| now.saturating_duration_since(at) >= INTERVAL)
    }

    fn sent(&mut self, id: u64, nonce: String, now: Instant) {
        self.sent = Some(now);
        self.pending = Some(Probe {
            id,
            nonce,
            sent: now,
        });
    }

    fn pong(&mut self, id: u64, nonce: &str, now: Instant) -> bool {
        let Some(probe) = &self.pending else {
            return false;
        };
        let elapsed = now.saturating_duration_since(probe.sent);
        if probe.id != id || probe.nonce != nonce || elapsed >= EXPIRY {
            return false;
        }
        self.pending = None;
        self.sample = Some((now, elapsed.as_millis() as u64));
        true
    }

    fn millis(&self, path: Option<Path>, now: Instant) -> Option<u64> {
        (path.is_some() && self.path == path)
            .then_some(self.sample)
            .flatten()
            .filter(|(at, _)| now.saturating_duration_since(*at) < EXPIRY)
            .map(|(_, millis)| millis)
    }
}

impl Actor {
    fn latency_path(&self) -> Option<Path> {
        if !self.model.ready() || self.model.local || self.model.generation != self.scope.current()
        {
            return None;
        }
        let kind = self.session.as_ref()?.connection_kind();
        (kind != ConnectionKind::Local).then_some(Path {
            generation: self.model.generation,
            kind,
        })
    }

    // Publishing can observe a path change before the next tick/response. Never
    // show a sample from the old path, even in that short interval.
    pub(super) fn reported_latency(&self, now: Instant) -> Option<u64> {
        self.latency.millis(self.latency_path(), now)
    }

    pub(super) fn refresh_latency(&mut self, now: Instant) -> bool {
        let previous_kind = self.model.connection_kind;
        let previous_latency = self.model.latency_ms;
        if let Some(session) = &self.session {
            self.model.connection_kind = session.connection_kind();
        }
        self.latency.sync(self.latency_path(), now);
        self.model.latency_ms = self.reported_latency(now);
        previous_kind != self.model.connection_kind || previous_latency != self.model.latency_ms
    }

    pub(super) async fn latency_tick(&mut self, now: Instant) -> Result<bool> {
        let changed = self.refresh_latency(now);
        // Let business replies and restoration replay drain before adding a
        // probe to the shared bounded transport. Probes never enter outstanding:
        // a missed Pong must not become an unknown operation or force a retry.
        if !self.outstanding.is_empty() || !self.latency.due(now) {
            return Ok(changed);
        }
        let nonce = protocol::encode(&protocol::random32()?);
        let scope = self.scope.clone();
        let id = scope
            .during(
                self.model.generation,
                self.session
                    .as_mut()
                    .context("连接已关闭")?
                    .request(Operation::Ping {
                        nonce: nonce.clone(),
                    }),
            )
            .await?;
        self.latency.sent(id, nonce, now);
        Ok(changed)
    }

    pub(super) fn latency_pong(&mut self, id: u64, nonce: &str, now: Instant) -> bool {
        if !self.latency.pong(id, nonce, now) {
            return false;
        }
        self.model.latency_ms = self.reported_latency(now);
        true
    }

    pub(super) fn clear_latency(&mut self) {
        self.latency = Latency::default();
        self.model.latency_ms = None;
    }
}

#[cfg(test)]
mod tests;
