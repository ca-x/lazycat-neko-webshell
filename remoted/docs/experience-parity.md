# WebShell and neoko-terminal experience parity

Status: implementation in progress, 2026-11-12. This document records the user's acceptance target; it does not claim the current release implements it.

## Product target

The WebShell and native clients must provide the same terminal and Agent workflow. NSH remains the native interface on desktop and Android. Shared Ghostty emulation and theme data support that target, but are not substitutes for testing input, layout, history and reconnect behavior.

## Standalone desktop and remote-only mobile

Windows, macOS and Linux are complete terminal applications: local shells, local workspaces and tabs work without a paired host, Relay address, Relay connectivity or LazyCat client. Remote devices are additional workspaces in the same application. Optional hosting allows explicit pairing from another desktop or Android, independently of local usage. Android is remote-only and must not show a local-shell or host-server entry. The fixed terminal grid, optional native input composer, themes, language and Agent state belong to this shared workflow. There is no Chat mode.

## Backend preference

Keep Native PTY, hc (herdr-core), and Herdr as separate backends. Desktop preferences select the default for new local sessions; an existing session and its reconnect always retain the original backend. The optional backend must report missing installation or connection errors explicitly. Protected-storage acknowledgment determines the active choice; an incomplete or failed save cannot replace it. Android remains remote-only and has no local backend selector.

## Two views of a session

- **Terminal**: full VT interaction, keyboard/IME, selection, bracketed paste, scrolling, application cursor modes, and explicit control ownership. Ordinary shell sessions default to this view.
- **Conversation**: a compact message-oriented view with an IME-friendly composer, clearly attributed user submissions, genuine structured Agent responses where supported, and Agent activity. Herdr and Agent-oriented sessions expose this view alongside Terminal. A terminal transcript without structured roles stays labelled terminal output; a screen redraw is not a new Agent message.

Changing views does not create a PTY, resend a prompt, claim control, resize a Herdr-owned pane, or reconnect the transport. Drafts, scroll position and the preferred view are scoped to device/resource/session; input is never replayed after reconnect. Unsupported structured history is explicit and retains an accessible terminal view.

## Navigation and status

A device sidebar selects local or paired hosts. Within the device, a workspace owns a single row of tabs. Navigation and common actions use labelled icons with native focus/disabled/pressed states and platform-appropriate touch targets. Herdr and ordinary terminals have distinct icons, including the exact Herdr SVG supplied by the user.

Each tab displays the Agent identity and activity when reported. Herdr status comes from its authoritative snapshot/events and joins by actual pane membership. Ordinary terminal status comes from the server's replacement program-status table. Native OSC 7501 status wins over inferred process status. Missing status remains unknown rather than guessed from output. Detection and PTY query replies are not rerun in any renderer or replay.

## Appearance and language

WebShell Ghostty theme sources, including bundled Inkstone themes, feed one generated native catalog. Preserve stable theme IDs and license attribution. Foreground, background, cursor, selection and ANSI palette must agree; true-color output retains its requested colors. Terminal theme and app appearance (system/light/dark) are separate preferences. Font size, appearance, locale and theme restore after restart.

Language supports system, Simplified Chinese and English, including navigation, settings, session actions, empty/error states and accessibility labels. Machine protocol identifiers are not translated.

## Connectivity and verification

Use the public Relay for real pairing, control, terminal and reconnect acceptance. Desktop-to-desktop hosting is independently enabled and explicitly approved. Preserve read-only grants, revocation and control arbitration. Native keepalive must be investigated against existing timers rather than adding a second uncoordinated heartbeat.

The user confirmed on 2026-11-12 that an OS reboot resolved the reported Super+V/Ctrl+V freeze. No application paste fix is claimed. See `paste-diagnosis-2026-11-12.md` for the bounded evidence and its limits.

## Release evidence required

- Shared emulator tests: CJK/graphemes, style/palette, alternate screen, scrollback, resize, paste modes, and no renderer probe replies.
- Native interaction checks on a narrow Android view and desktop, light/dark themes, both locales, device/workspace/tab switching and both session views.
- User and Agent attribution reflects actual source data; no transcript duplication on screen refresh or reconnect.
- Settings save/load and per-session preference isolation; stale generations cannot affect a newly selected device.
- Actual public Relay acceptance and focused existing WebShell regression checks.
- Private builds and verified public publishing of Android, Windows, macOS and Linux deb. Public source remains Relay plus publishing support.
