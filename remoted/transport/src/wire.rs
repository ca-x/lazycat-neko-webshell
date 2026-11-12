use anyhow::{Result, bail, ensure};

use crate::{MAX_WIRE_PAYLOAD, WINDOW};

const MAGIC: &[u8; 4] = b"NKT1";
const HEADER: usize = 13;

pub(crate) enum Frame<'a> {
    Data(u64, &'a [u8]),
    Ack(u64),
    Signal(&'a [u8]),
}

pub(crate) fn data(seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut out = prefix(0, seq);
    out.extend_from_slice(payload);
    out
}

pub(crate) fn ack(consumed: u64) -> Vec<u8> {
    prefix(1, consumed)
}

pub(crate) fn signal(payload: &[u8]) -> Vec<u8> {
    let mut out = prefix(2, 0);
    out.extend_from_slice(payload);
    out
}

fn prefix(kind: u8, number: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER);
    out.extend_from_slice(MAGIC);
    out.push(kind);
    out.extend_from_slice(&number.to_be_bytes());
    out
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Frame<'_>> {
    ensure!(bytes.len() <= MAX_WIRE_PAYLOAD, "oversized transport frame");
    ensure!(
        bytes.len() >= HEADER && &bytes[..4] == MAGIC,
        "invalid transport envelope"
    );
    let number = u64::from_be_bytes(bytes[5..HEADER].try_into()?);
    let payload = &bytes[HEADER..];
    match bytes[4] {
        0 => {
            ensure!(payload.len() <= crate::MAX_DATA, "oversized ciphertext");
            Ok(Frame::Data(number, payload))
        }
        1 if payload.is_empty() => Ok(Frame::Ack(number)),
        2 if number == 0 => Ok(Frame::Signal(payload)),
        _ => bail!("unknown transport frame"),
    }
}

/// Data at this distance can never be sent by a conforming window-limited peer.
pub(crate) fn in_window(seq: u64, consumed: u64) -> bool {
    seq >= consumed && seq - consumed < WINDOW as u64
}
