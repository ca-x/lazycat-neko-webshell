# Shared relay/direct ciphertext transport

`start(Carrier) -> (Sender, Receiver, Guard)` starts one connection on a Tokio
runtime. The existing host/client adapters own the WebSocket and its carrier
credits. Keep the non-cloneable `Guard` until the connection ends. Its drop
cancels the actor and RTC attempt; `shutdown().await` waits for cleanup.

`Sender` is cloneable. `send(Vec<u8>).await` accepts a single already-encrypted
record; callers must not retry a record after acceptance or an ambiguous
cancellation. The queue and outstanding record window are bounded. Concurrent producers must
serialize Noise encryption and submission in the same order; the overlay assigns
sequence numbers in acceptance order and cannot inspect Noise nonces. `Receiver`
has one consumer; cancelling `recv` does not consume a record. Removing a record
atomically queues its consumption acknowledgement, with no second await.

## Integration contract

1. Send all Noise handshake and transport records through the overlay from the
   very first handshake record. Never alternate raw and wrapped records.
2. Complete Noise identity verification and the existing authorization checks.
   Generate a fresh random 32-byte signaling key for this connection and exchange
   it using the encrypted `EnableDirect` / `DirectReady` application messages.
3. Call `enable_direct(Role::Responder, key)` on the host and
   `enable_direct(Role::Initiator, key)` on the client. This is a synchronous,
   nonblocking call and is allowed once per connection. Signals that race ahead
   of the authorized call are retained as ciphertext, at most 128 records and 64 KiB in total, and are
   never decrypted beforehand.
4. Pump `Write.data` into opaque Relay Data. Resolve `Write.completed` after the
   existing carrier writer accepts/writes the record; do **not** wait for overlay
   consumption. Feed decoded carrier bytes or their original error into
   `Carrier.incoming`. Continue independent carrier delivery ACKs/credit handling
   for data, overlay ACKs and encrypted signaling alike.
5. Observe `kind()` or `subscribe_kind()`. `Connecting` uses the relay for business
   traffic while ICE is pending. `Direct` means a reliable ordered DataChannel
   is open. The relay must remain connected in all states.
6. On disconnection/revocation drop the guard, close Noise and preserve the
   application's existing reconnect/session-attach semantics. A new connection
   must use a new key and a new overlay; no uncertain application command is
   replayed on a new Noise connection.

Carrier errors retain their complete error/source chain through both send and
receive. Carrier EOF is `io::ErrorKind::UnexpectedEof`, closed writer is
`BrokenPipe` and the five-second carrier write deadline is `TimedOut`. Wire and
ordering violations remain protocol errors, with no manufactured IO cause.
A failed direct attempt alone preserves the working relay and Noise connection.

## Wire and bounds

Every carrier record starts with `NKT1`, a one-byte kind and a big-endian `u64`:

| Kind | Number | Remaining bytes |
| --- | --- | --- |
| 0 | Per-direction record sequence, starting at zero | Original Noise bytes |
| 1 | Exclusive cumulative consumed sequence | Empty |
| 2 | Zero | 24-byte random XChaCha nonce followed by ciphertext/tag |

There is a single sequence space per direction across both paths, covering
handshake and application ciphertext. The sender retains eight unacknowledged
original frames, and the receiver retains/reorders only that same eight-record
window. Duplicates never enter Noise. Consumption ACKs always use WSS, including
when business records use the DataChannel. Carrier credits are a separate layer.
ACKs and outer ordering metadata are not application authentication: trust still
comes from Noise, whose identity/revocation checks must remain active.

A direct close, ICE failure, send/buffer failure, 15-second negotiation deadline
or two seconds without consumption ACK of a direct record permanently switches
this connection to the relay. It replays the exact original outstanding frames,
without re-encryption, sequence changes or a second Noise nonce. The peer is
notified with an encrypted `relay_only` signal. There is no upgrade retry within
the same connection.

Maximum wire record: 65,536 bytes. Maximum Noise payload before this header:
61,456 bytes. Signaling JSON is limited to 32,768 bytes, 128 authenticated signals
and 64 remote ICE candidates per attempt; nonce replay tracking is bounded.
Signaling AEAD uses direction-specific associated data to prevent reflection.
The per-connection key authenticates SDP/DTLS fingerprints and ICE candidates.

## NAT and platform boundary

`webrtc = "=0.17.3"` uses reliable ordered SCTP/DTLS DataChannel, UDP4/UDP6 ICE and
`stun:stun.l.google.com:19302`. No TURN, static mapping, public peer interface or
manual inbound port is required. STUN can traverse many endpoint-independent
mapping NATs; incompatible/symmetric NATs, blocked UDP and STUN unavailability
retain WSS. No universal NAT success guarantee is implied.

The tests exercise actual ICE/DTLS/SCTP, actual Noise nonce progression, maximal
records, direct ACK timeout replay, cancellation/error propagation, and two
independent simulated private NATs. Host candidates are removed in the NAT
fixtures so those tests must use STUN-derived paths. The symmetric-NAT fixture
verifies continued relay delivery before and after failed negotiation. VNet
emulation is not a substitute for two real external networks or Android device
sleep/network-change validation.

The formal crate passed Android aarch64/API 28, macOS arm64/x64 and Windows GNU
cross checks on Linux. Production MSVC builds and native device runs
remain platform CI/integration checks. Android sleep may suspend timers/sockets;
on resume an expired direct attempt falls back, while a broken WSS connection
requires the existing authenticated reconnect flow.
