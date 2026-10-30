# Protocol Notes

## Data Plane

`/ws/terminal` is the terminal hot path. It is intentionally not protobuf:

- binary WebSocket frames carry PTY bytes from backend to frontend;
- binary or text frames carry user input from frontend to backend;
- JSON text frames carry resize and close controls;
- backend sends JSON text frames for `ready`, `error`, `process-exit`, `output-sequence`, and `replay-complete`.

Reconnect replay uses `/ws/terminal?...&replay=true&after=<last_sequence>`. PTY bytes still travel as raw binary frames; the following `output-sequence` control frame identifies the monotonic output sequence that was just delivered. The frontend queues input while replay is active and flushes it after `replay-complete`.

Accepted client text messages:

```json
{"type":"input","data":"ls\r"}
{"type":"resize","cols":120,"rows":32}
{"type":"close"}
```

Compatibility shorthands are also accepted:

```text
input:<bytes>
resize:<cols>,<rows>
```

## HC terminal transport

`/ws/terminal?backend=hc&session_id=...&pane_id=...&hc_history_lines=1000` attaches an existing provider-owned HC session on a LightOS target. The saved session and pane must match; selecting another backend cannot reinterpret the session. The target must already have a compatible `hc` executable. HC is launched as the authorized login user through non-PTY `lightosctl exec -i`, in a dedicated provider/selector/user scope.

Each attach replaces the browser screen: `replay-start`, `ready`, and `replay-complete` identify the session, pane and selector before HC's ordered initial paint and live bytes. HC has no paint-end marker; its sequence is not a native output cursor. The browser suppresses generated terminal replies throughout the HC connection because HC owns probe responses. `hc_history_lines` is bounded to 0–10000. Finished sessions use `terminal.read` for a final plain-text snapshot, followed by `process-exit`; its `exit_code` may be null and its `signal` and `reason` preserve termination information.

Program status is an app-owned replacement table from the atomic event subscription snapshot, followed by changed tables decoded from newline-delimited HC events. Event sequence numbers are scoped to the current core process. Missed events or transport loss require another attach and fresh snapshot. Credits acknowledge bounded WebSocket delivery, not renderer consumption.

Creation intent, concrete terminal identity and confirmed-close tombstones live in a separate per-session SQLite journal, insulated from replaceable workspace/session snapshots. Intent is persisted before the single `terminal.run` request. Reconnect reconciles exact provider/session labels and the saved core/address, including finished terminals; an ambiguous creation result never repeats the command. Browser disconnect only detaches. Explicit close confirms `terminal.kill` or an already finished terminal before deleting local layout/session state. HC has no native history-export, terminal MCP or single-controller interface in this release.

## Control Plane

`lazycat.webshell.v1.CapabilityService` owns typed control APIs:

- `ListInstances`
- `GetProvider`
- `CreateSession`
- `CloseSession`
- `ListSessions`
- `ListPlugins`
- `ConfigurePlugin`
- `InvokePlugin`
- `RequestControl`
- `ReleaseControl`

The browser uses Connect over HTTP for these APIs. This avoids forcing terminal byte streams through browser-limited request streaming while keeping platform operations typed.

`CloseSession` accepts `session_id` and an optional `selector`. Provider-local workspace sessions can be closed with the session ID alone. Agent-managed sessions require the selector when the provider has no local workspace record, so the close request is routed to the correct instance-local agent instead of scanning or waking unrelated instances.

Agent-managed close is explicit lifecycle cleanup: the agent resolves `session_id -> pane -> owning tab`, removes that pane from workspace state, closes the pane PTY, and repairs orphan panes that are no longer referenced by any tab. Non-active tabs or panes are not treated as idle and are not cleaned up unless they become unreachable orphan objects.

## Generic Plugin Protocol

Plugins are described with:

- stable `id`;
- generic `kind`;
- `scopes`;
- accepted and produced content types;
- JSON schema strings for input/output payloads;
- enablement state;
- string metadata.

Plugin invocation is opaque:

```protobuf
message InvokePluginRequest {
  string plugin_id = 1;
  string session_id = 2;
  string operation = 3;
  string content_type = 4;
  bytes payload = 5;
  map<string, string> metadata = 6;
}
```

This keeps the protocol stable while future implementations add concrete handlers for file transfer, remote shell adapters, AI control, or human collaboration workflows.
