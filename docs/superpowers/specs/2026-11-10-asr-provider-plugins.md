# ASR provider plugins

## Approved scope

The user specified that a plugin registers an ASR provider into the existing voice input flow, rather than creating another voice tool. Keep existing API providers and saved profiles. Add a discoverable PC voice entry and retain the mobile entry. The user also requested the Doubao IME provider from memory and publication after review and tests.

## Provider boundary

Built-in plugin manifests declare `asrProvider=true`. ASR profiles store `provider=plugin` and the registered `pluginId`. Only registered, enabled ASR plugins receive transcription calls. The host retains multipart upload limits and routes raw WAV bytes through the existing plugin runtime, preserving configuration gates, task draining and disable semantics. Plugins do not own terminal handles, PTYs, agent upgrades or account authorization.

The WeChat provider uses the pinned WowType voice library without microphone/hotkey features. The browser captures WAV; the plugin validates PCM format, bounds duration, resamples to 16 kHz mono and performs inference on the provider server. No cloud ASR request is made. Mobile clients still require a connection to the provider. Models are downloaded only by an explicit action from the pinned Tencent CDN resource, verified, extracted using an allowlist into a temporary directory, and atomically installed into the plugin's persistent model directory. Models are not bundled with the application. Disable releases the model; removal is explicit. Downloads/inference are serialized and bounded.

Plugin settings show model state and download/remove actions. No separate plugin tool is created. Existing API form fields do not apply to plugin providers. Disabling/removing a plugin preserves saved profiles but makes their unavailability explicit; no silent provider fallback or cloud upload is allowed.

## Doubao evidence and remaining limitation

Nowledge memories `db3b7586-0856-4011-8943-d86bad8864c5` and `57139a9a-51b7-4665-8bef-762844076f38` define the SAMI token endpoint, WS framing and audio parameters. Their 9/9 protocol checks do not establish working ASR. The access probe repeated on 2026-11-10 obtained a token but StartTask returned TaskFailed/40000000 with ACL 3003 for original.sami.sami. Do not advertise functioning Doubao IME transcription or substitute Volcengine without user agreement. A complete authorized ASR handshake and audio payload/result contract are required before shipping that provider.

## Acceptance and commands

- API profiles round trip unchanged; plugin profiles round trip without API credentials.
- Only declared and enabled providers may transcribe; TTS remains API only.
- Model download has fixed HTTPS source, byte/hash bounds, safe archive entry allowlist and atomic installation. Invalid/oversize audio, missing model, disable and cleanup have explicit tested outcomes.
- PC/mobile use the existing recording flow and selected provider, retain focus and microphone cleanup, and never execute a terminal command automatically.
- Run `npm test`, `npm run typecheck`, `npm run build`, focused Rust tests followed by workspace tests and strict Clippy; exercise settings and PC/mobile voice with browser fixtures and actual model inference where feasible.
- Run `git diff --check`, audit final changes, update application versions only after acceptance, then follow private tag build -> explicit public publish workflow -> verify Release assets and store outcomes. Target-local agent versions remain unchanged.

## Review and validation (2026-11-10)

Reviewed the application diff against rust-best-practices and rust-review, edition 2024 / Rust 1.97.1. Fixed installation rollback so an unsuccessful restore cannot delete the last valid backup (rust-review §3 correctness/resource cleanup). Removed the unreachable operation panic in favor of the existing typed plugin error (rust-review §3 expected failures). The strict pedantic Clippy findings were corrected. Native capability checks consult the built-in manifest, and lifecycle draining remains in the shared plugin runtime.

Browser review found clipped model actions in the original layout; the final view uses one compact model row and wrapping actions. API fields are explicitly hidden for plugin profiles. Recording binds its provider at start, cancels on plugin disable or Esc, releases microphone tracks, and inserts text without Enter. The final ASR browser scenario covers profile save/reopen, download failure/retry, confirmed removal, keyboard recording, provider changes, microphone cleanup, and the existing mobile entry.

Validation: 741 frontend tests and 8 release-script tests; 679 Rust workspace tests passed (8 explicitly ignored fixtures). Both new real-model fixtures were also run explicitly: verified installation/loading with silence and Chinese speech. The generated phrase “你好，这是小橘终端的离线语音识别测试。” was recognized in full. Strict all-features/all-targets pedantic Clippy, format checks, typecheck and production frontend build passed. ASR, settings and terminal-diagnostics browser scenarios passed. The existing frontend bundle-size advisory remains.

Release scope is v0.12.0 with the WeChat provider. The unavailable Doubao IME provider remains an explicit upstream ACL blocker rather than an advertised feature. No target-local agent version changes are required. WowType's Apache-2.0 license, original wtlocal notice, and pinned-source attribution accompany the package; no model bytes accompany it.
