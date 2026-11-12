use std::collections::HashSet;

use anyhow::{Context, Result, bail, ensure};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use serde::{Deserialize, Serialize};
use webrtc::ice_transport::ice_candidate::RTCIceCandidateInit;
use zeroize::Zeroizing;

use crate::Role;

const MAX_SIGNAL: usize = 32_768;
pub(crate) const MAX_SIGNALS: usize = 128;
const NONCE_LEN: usize = 24;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub(crate) enum Signal {
    Offer(String),
    Answer(String),
    Candidate(RTCIceCandidateInit),
    RelayOnly,
}

/// The caller supplies a fresh connection-specific key over the established Noise channel.
/// Directional associated data prevents reflecting an offer back at its sender.
pub(crate) struct Cipher {
    key: Zeroizing<[u8; 32]>,
    role: Role,
    sent: usize,
    received: HashSet<[u8; NONCE_LEN]>,
}

impl Cipher {
    pub(crate) fn new(role: Role, key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
            role,
            sent: 0,
            received: HashSet::new(),
        }
    }

    pub(crate) fn seal(&mut self, signal: &Signal) -> Result<Vec<u8>> {
        ensure!(self.sent < MAX_SIGNALS, "signal count limit");
        let plain = serde_json::to_vec(signal)?;
        ensure!(plain.len() <= MAX_SIGNAL, "signal size limit");
        let mut nonce = [0; NONCE_LEN];
        getrandom::fill(&mut nonce).context("signal nonce randomness")?;
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("invalid signal key"))?;
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: aad(self.role),
                },
            )
            .map_err(|_| anyhow::anyhow!("signal encryption failed"))?;
        let mut out = Vec::with_capacity(NONCE_LEN + encrypted.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&encrypted);
        self.sent += 1;
        Ok(out)
    }

    /// Authenticated replay is ignored. Invalid ciphertext never reaches WebRTC.
    pub(crate) fn open(&mut self, bytes: &[u8]) -> Result<Option<Signal>> {
        ensure!(
            (NONCE_LEN + 16..=MAX_SIGNAL + NONCE_LEN + 16).contains(&bytes.len()),
            "signal size limit"
        );
        let nonce: [u8; NONCE_LEN] = bytes[..NONCE_LEN].try_into()?;
        if self.received.contains(&nonce) {
            return Ok(None);
        }
        ensure!(self.received.len() < MAX_SIGNALS, "signal count limit");
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| anyhow::anyhow!("invalid signal key"))?;
        let plain = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &bytes[NONCE_LEN..],
                    aad: aad(self.role.other()),
                },
            )
            .map_err(|_| anyhow::anyhow!("signal authentication failed"))?;
        let signal: Signal = serde_json::from_slice(&plain)?;
        if matches!(
            (&signal, self.role),
            (Signal::Offer(_), Role::Initiator) | (Signal::Answer(_), Role::Responder)
        ) {
            bail!("invalid signal direction");
        }
        self.received.insert(nonce);
        Ok(Some(signal))
    }
}

fn aad(role: Role) -> &'static [u8] {
    match role {
        Role::Initiator => b"neoko-direct-v1/initiator",
        Role::Responder => b"neoko-direct-v1/responder",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_direction_tampering_replay_and_random_nonce() {
        let mut a = Cipher::new(Role::Initiator, [7; 32]);
        let mut b = Cipher::new(Role::Responder, [7; 32]);
        let frame = a.seal(&Signal::Offer("private SDP".into())).unwrap();
        assert!(!frame.windows(11).any(|w| w == b"private SDP"));
        assert_ne!(frame, a.seal(&Signal::Offer("private SDP".into())).unwrap());
        assert!(a.open(&frame).is_err(), "reflection must fail");
        assert!(Cipher::new(Role::Responder, [8; 32]).open(&frame).is_err());
        let mut damaged = frame.clone();
        *damaged.last_mut().unwrap() ^= 1;
        assert!(b.open(&damaged).is_err());
        assert!(matches!(b.open(&frame).unwrap(), Some(Signal::Offer(_))));
        assert!(b.open(&frame).unwrap().is_none());
    }

    #[test]
    fn signal_and_replay_memory_are_bounded() {
        let mut a = Cipher::new(Role::Initiator, [3; 32]);
        assert!(a.seal(&Signal::Offer("x".repeat(MAX_SIGNAL))).is_err());
        assert!(a.open(&vec![0; MAX_SIGNAL + NONCE_LEN + 17]).is_err());
        for _ in 0..MAX_SIGNALS {
            a.seal(&Signal::RelayOnly).unwrap();
        }
        assert!(a.seal(&Signal::RelayOnly).is_err());
    }
}
