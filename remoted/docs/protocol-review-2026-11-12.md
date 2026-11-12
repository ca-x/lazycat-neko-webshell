# Protocol flow review, 2026-11-12

Status: reviewed and implemented for application/Relay v2. Uses DDIA reliability, bounded stream processing, authoritative state and failure isolation principles. The user resolved the separate paste incident by rebooting the operating system; no application paste fix is claimed.

## Authority and data paths

Browser WebShell connects directly to its provider WebSocket. Remote clients connect to the provider through the CF Relay using an authenticated end-to-end Noise channel. Both paths may attach the same provider-owned terminal. The PTY/reply authority remains singular; renderers are read models and must not replay input or reply to historical probes.

Pairing grants are durable metadata. Output subscriptions, control ownership and transfers are connection resources. Disconnect/revoke must release the latter without destroying a shared persistent PTY. Client-side forgetting is not host-side revocation. Saved pairings currently have no inactivity expiry.

## Verified and actionable findings

1. **Transport credit is acknowledged before consumption.** Client transport enqueues a Data frame in an eight-slot queue and immediately ACKs it. The sender can consume the returned credit while the queue is still full. A fixture reproduces connection closure on frame nine with only one frame unacknowledged at any time. Fix: return credit when the consumer dequeues a frame, while keeping receive/credit/control processing independent. Retain the terminal error outside the full data queue. Do not increase queue capacity to conceal the incorrect accounting.
2. **Provider and desktop host have the same early-ACK pattern.** Their per-peer inbound queues use try_send followed by an immediate Relay ACK. A noisy or temporarily busy peer can therefore lose its connection even if the Relay observes its nominal window. This needs the same bounded-consumer accounting, not another keepalive timer.
3. **Host ACKs are cumulative across multiplexed peers.** A cumulative global watermark cannot express that peer B consumed a frame while peer A has not consumed an earlier frame. Before delaying host ACKs, either preserve a bounded global completion watermark with explicit head-of-line tradeoffs, or version the transport to exact per-frame ACK tokens. Never return credit for another peer's unconsumed frame. Prefer exact acknowledgement if changing the only deployed instance and all endpoints together.
4. **New status messages require an application protocol version change.** Unknown enum variants are rejected by published clients. Move the authenticated protocol version from 1 to 2 so incompatible peers fail at handshake instead of failing during terminal output. Relay envelope version is separate; do not conflate a schema change with browser WebShell protocol.
5. **Status is a replacement view, terminal output is an ordered log.** Send the current replacement program table after replay and only changed tables during live operation. Status should not be inferred from replayed output. Include status events in per-stream attachment ordering and stale-stream filtering.

## Failure and ordering requirements

- A slow remote subscriber may lose its own subscription with an explicit output-gap/error, but must not block PTY output collection, other subscribers, or the browser connection.
- Flow-control ACK means a bounded local buffer slot is released. It is not confirmation that terminal input executed or a file mutation committed.
- Timeout after mutation dispatch means outcome unknown. Do not retry shell input, file commits, terminal creation or destructive operations automatically.
- Bound queue lengths, bytes, outstanding requests, per-peer frames and transfer state. Preserve terminal errors independently from data queue saturation.
- Revoke persists grant removal before cancelling active connections. Cancellation cannot undo already dispatched target operations; report that boundary honestly.
- Keep heartbeats independently serviceable from long business work and bulk output. Existing timeouts already exist; an additional heartbeat does not fix a credit error.
- Metrics should identify the stream and connection generation, queue depth/bytes, send stall duration, last receive/Pong age and output-gap/reconnect reasons. Never include tickets, keys, pairing codes, clipboard or terminal contents.

## Verification before release

Use small windows and a paused consumer to reproduce credit exhaustion. Resume consumption and assert ordered delivery; concurrently prove outgoing delivery receipts and heartbeat still work. Run two peers, one stalled, to test isolation and exact ACK semantics. Test stale/repeated ACKs, peer removal with pending frames, revoke during blocked output and browser input concurrent with a slow remote subscriber. Validate normal and read-only clients through the public Relay, then deploy/publish compatible endpoints together. The separate paste incident is closed as an operating-system issue confirmed by the user; it is not a release fix.


## Completed verification

The public Relay was deployed from public commit
`b576ca98b523e9eebd7bc1204a86d4cb826313ce`, package 0.2.0 and wire protocol
`neko.remoted.v2`. Its root GET exposes the package version, protocol version,
source commit and build time without caching. Relay validation passed 26 unit
and nine actual workerd/Durable Object integration cases, including exact ACKs,
slow-peer isolation, peer removal and idempotent late receipts.

Provider tests exercised registration/RTT and encrypted file transfer through
the public Relay. A separate native-host/client test exercised actual public
TLS/WebSocket/Noise, explicit approval, a real shared PTY, control arbitration,
read-only rejection, reconnect to the same PTY, resource closure and revocation.
It ran the exact ignored child test with `--include-ignored` and checked that
one test really passed; the earlier child launcher that silently skipped it is
not accepted as verification. See the current acceptance record for commands
and platform limits.
