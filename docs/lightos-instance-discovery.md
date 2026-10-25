# Instance discovery

LightOS and Catflap are independent, optional discovery sources. Installing
only either application or installing both is supported. Neko requests each
source concurrently in the current platform user's context and merges by
instance kind and the complete selector. Equal names with different owners
remain distinct. A repeated selector retains all source associations.

With `lzcapp.user_delegate`, each request sends the current platform-injected
`X-HC-User-Ticket` only to these fixed origins:

- `http://app.cloud.lazycat.lightos.entry.lzcx`: `/api/webshell/instances` and
  `/api/client-instances`, independently.
- `http://app.community.lazycat.app.catflap.lzcx`: `/api/webshell/targets`.
- `http://app.community.lazycat.app.catflap.dev.lzcx`: `/api/webshell/targets`.

The production and development Catflap endpoints are probed independently;
the returned owner deployment ID is preserved. A missing endpoint (404) is
optional when another endpoint succeeds. Other failures accompany successful
results as diagnostics. If every source fails, discovery returns an error;
it does not report a successful empty list. Remote-client discovery failure
does not discard LightOS instances.

Catflap typed discovery uses the version-1 `targets`/`sources` envelope.
Only a 404 on `/api/webshell/targets` falls back to the older
`/api/webshell/instances` bare array; authentication errors, malformed responses,
and server failures do not trigger this fallback. Extra fields and unknown
target kinds are ignored. Known unavailable sources preserve other successful
targets with diagnostics; unknown error codes are never echoed. Username metadata is retained, with a
nonempty Catflap username preferred when merging. Connections continue to
resolve the current username and authorize the target through `lightosctl`;
no browser-supplied username or cross-user metadata cache grants access.

The delegated HTTP client has a bounded timeout and response size, bypasses
environment proxies, and rejects redirects. Browser Cookie/Authorization and
incoming identity claims never cross the application boundary. Tickets are
sensitive and never stored or echoed. Legacy embedded/development credential
forwarding remains restricted to the original LightOS origin; it does not
probe Catflap without a delegation ticket.

The REST response remains an array, with partial failures carried in the
`X-Neko-Discovery-Warnings` JSON header. The Connect response exposes
`discovery_warnings`. The UI keeps valid rows visible and displays partial
errors separately; a failed initial load is distinct from a successful empty
list. Source markers are small colored dots without visible labels or hover
explanations.

## MicroVM terminal sessions

MicroVM targets appear in a separate group with their actual UUID in `target_id`
and the `SMOLVM` kind. Their fixed `provider_app_id` comes from the queried
origin, never an upstream-supplied URL. They have no LightOS selector or username.
When the target advertises `terminal_available`, Neko opens a dedicated terminal
view using the shared terminal renderer and current font settings.

The browser calls Neko's `/api/catflap/{app}/...` routes. Neko forwards a fresh
platform ticket to the allowlisted Catflap application for each operation:

- POST `/api/smolvm/instances/{uuid}/terminal/sessions` with a saved, newly
  generated session UUID and initial dimensions (1..500).
- WebSocket `/api/smolvm/terminal/sessions/{session}/ws`, no query, subprotocol
  `catflap.terminal.v1`. Binary frames carry original PTY bytes; JSON text carries
  resize, ready, error and real process-exit events.
- DELETE `/api/smolvm/terminal/sessions/{session}` for explicit termination;
  only 204 confirms cleanup. POST and DELETE include `X-Catflap-Request: 1`.

The platform checks user identity and Catflap revalidates VM/session ownership.
Neko forwards no browser cookies, authorization, Origin or identity claims. It
rejects cross-origin browser requests and applies bounded handshake/write time,
frame size and HTTP response limits.

Disconnect detaches; reconnect reuses the session ID and never starts another
shell. An ambiguous create response also retains that ID without automatically
reissuing POST. Ended sessions retain their exit code in browser session storage.
Explicit New closes the previous session first; an inaccessible/expired session
may be forgotten only for that explicit action, never reported as cleaned up.

Catflap keeps detached sessions for 120 seconds with a one-hour maximum lifetime;
process restart loses sessions. There is no replay: `ready.output_gap` is shown
as missing output, not fabricated recovery. Only one connection per session is
supported. This transport does not provide Herdr, file management or desktop.

Local HTTP/WebSocket fixtures and browser tests are separate from real platform
user-ticket verification over `.lzcx`; they cannot establish deployment success.

## Default login account

Agent v21 refreshes the workspace's default username from each non-empty,
authorized request. Missing or blank metadata leaves a confirmed username
intact. Only future tabs and splits use the updated default; existing pane
objects, process identities, and sessions are preserved.

The Agent protocol remains v4 and the minimum supported implementation remains
v11. Installing a new provider does not replace a compatible running Agent.
The account-refresh fix takes effect when Agent v21 or later starts; the Agent
update tool can prepare the bundled payload for the next launch without
terminating current PTYs.
