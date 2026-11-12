# neoko-terminal native desktop host

Implementation for the WebShell 0.14.2, Android 0.3.0 and desktop 0.4.1 release candidate, 2026-11-12. Actual deployment and artifact evidence are recorded separately.

## Intended behavior

Desktop neoko-terminal can own local terminals and independently serve explicitly approved remote devices. Android remains a remote client. Existing client identity, bindings, application ID and signed upgrade chain remain unchanged. Native desktop terminals work without a Relay and without self-pairing.

A user selects “本机” in the device sidebar, creates a terminal and explicitly acquires control before typing. Switching devices only detaches the current view. Returning to 本机 reattaches the same live PTY. Closing a session is a separate confirmed action. Ordinary terminals and Herdr remain separate resource kinds.

Preferences choose the backend for new local sessions: Native PTY, hc (herdr-core), or Herdr. Existing sessions keep their backend when the default changes or the view reconnects. The default comes only from acknowledged protected storage; failed loads block creation, and failed or pending saves keep the previous default. Missing optional backends report installation or connection guidance rather than falling back. Android does not expose a local-backend setting.

Remote control is off by default. Desktop settings accept a Relay address before enabling; connection status, invitation, pending approvals and paired devices appear progressively. Pairing requires a fresh device code and explicit host approval with fingerprint comparison, read-only/operate choice and selected resources. Revocation closes active encrypted sessions and removes grants immediately. Local workspace selection and remote serving have separate lifecycles.

## Architecture

- `remoted/host`: desktop-only Rust library, independent of NSH, WebShell AppState, LightOS and application databases. Existing protocol wire types and Noise handshake use application protocol v2 and exact-credit Relay v2.
- `host::terminals`: portable-pty real Unix PTYs / Windows ConPTY, bounded output replay, subscriber streams, single controller and real exit status. Detach preserves the PTY; explicit close terminates it.
- `crates/hc-client`: provider/native shared bounded framed codec. The host owns its explicit namespace/core identity and durable create intent; hc owns persistent terminal lifetime. Local and authorized remote attachments share control arbitration, with terminal-scoped snapshots and events. hc output never enters the PTY probe/detection authority. A lost channel or missing numeric exit status produces `TerminalUnavailable`, which keeps the last screen and disables only that attachment; it cannot disconnect another session or invent a successful process exit.
- Host runtime: private host identity and routing credential, persisted settings and device grants, separate Relay generation, bounded connection queues and reconnect. Peer identity comes from authenticated Noise static keys, never Relay peer IDs.
- Client controller: Local and Remote workspace channel enum with the same request/receive interface. Local commands cannot be invoked through remote protocol. Workspace generations prevent stale responses or output from entering a newly selected device.
- Client platform bridge: existing protected credential store, using a new host-specific key. Failure to save approval must fail approval, not create an ephemeral trusted device.
- UI: existing semantic tokens, controls and NSH native components. Sidebar keeps the terminal as the primary task; hosting controls live in a dedicated expandable settings section.

## Required boundaries

1. Existing `remoted.identity.v1` and client bindings remain independent from host identity/settings.
2. Default hosting is disabled; local terminals do not depend on hosting or Relay health.
3. One controller per actual PTY across local and remote subscribers. Read-only devices cannot take control, input, resize or mutate files.
4. Device grants bind explicit host-owned resource IDs. No caller-supplied path becomes a terminal or a Herdr endpoint.
5. Disconnect and app suspend detach views. No input, file transfer or shell creation is replayed on reconnect.
6. Persisted devices survive process restart; PTYs survive switching and network loss while the application remains running. App exit is not described as persistent daemon hosting. hc terminals have their separate core-owned persistence; restart resolves saved UUID/core/terminal identities and never repeats an ambiguous create.
7. File sharing requires an explicitly selected root, handle-relative resolution and bounded/atomic transfers. Unimplemented platform capabilities are absent.
8. Herdr uses an explicitly selected current-user endpoint and the existing allowlisted method contract. No platform ticket forwarding. Shared program-status portability changes follow the independent lightweight agent version policy.
9. Bounded pending approvals, devices, frames, output buffers and transfers. Secrets, device codes and terminal contents are not logged.
10. Public repository retains Actions/publishing support and Relay only. New native code is private; releases follow private builds → public verified publication.

## Implementation and acceptance

- [x] Real local PTY registry and control arbitration; tests for detach, replay, competing control, read-only rejection and exit.
- [x] Native host identity/configuration, Relay responder and explicit approval/revocation; tests for durable approval and stale revision rejection.
- [x] Local/Remote controller integration; tests for device switches, stale generation, no network retry for local sessions and persisted workspace preference.
- [ ] Native sidebar/session actions and hosting settings, audited with NSH guides, ui-ux-pro-max, apple-design and animate; inspect actual light/dark desktop UI.
- [x] Real two-client public Relay acceptance: pair, approve, attach an actual native PTY, type a unique sentinel, read output, arbitrate control and revoke.
- [ ] Re-run existing remote-client regression checks; publish all native desktop packages through private builds and public verification. Android only changes when its shared client behavior changes.
