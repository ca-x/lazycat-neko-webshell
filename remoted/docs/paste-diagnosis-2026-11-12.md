# Ordinary WebShell paste investigation — 2026-11-12

Resolved by the user on 2026-11-12: restarting the operating system restored pasting. The user identified this incident as an operating-system issue. No application root cause was reproduced; no production paste change is justified by this incident. The separate native Relay consumer early-ACK defect is tracked and fixed independently.

## Evidence

- Installed Omarchy clipboard binding maps Super+V into Ctrl+V for ordinary browsers. The current execution session is X11 on :100 with no live Hyprland instance or Wayland socket; this is not a test of the user’s actual Hyprland clipboard seat.
- Real Restty frontend plus clipboard controller with isolated echo transport: Meta+V, Ctrl+V, ClipboardEvent, 20 B / 64 KiB / 1 MiB, and stalled image-read promise did not freeze animation. `/tmp/neoko-paste-probe-results.json`.
- Real provider ordinary-terminal WebSocket loop and managed `/bin/sh` PTY: 120 sentinel commands actually executed, followed by 120 successful same-socket control ping/pong round trips; maximum observed output latency approximately 48 ms. Evidence: `/tmp/neoko-paste-ws-diagnosis.log`. The temporary diagnostic test was removed after the user resolved the incident.
- Chromium with actual Restty/clipboard controller attached to that real WebSocket and PTY: 60 paste input events; output and animation continued; no socket closure or browser error. This uses a controlled clipboard and transport adapter, not the complete production application or real Hyprland clipboard. `/tmp/neoko-paste-live-ws-results.json` and `/tmp/neoko-paste-live-ws-diagnosis.log`.
- Managed PTY matrix with remote subscriber absent, consuming, or paused at full eight-slot queue: 36 final scenarios all passed, with 20 B / 64 KiB / 1 MiB legal 16 KiB fragments, single-line and 80-column wrapping, and 500 ms status polling. Local output, control notifications and live status continued even while the remote queue remained full. One MiB completion 1.289–1.670 seconds without subscriber, 1.339–1.626 seconds consuming, 1.292–1.626 seconds paused. Status queries occasionally took up to 118 ms and single input calls up to 208 ms; no sustained freeze was observed. `/tmp/neko-managed-paste-contention-1900284.json` and `/tmp/neko-paste-run.log`.
- A single input frame larger than 16 KiB is rejected. This is distinct from a browser freeze; the frontend’s direct send path currently does not split oversized input. Do not assume it explains small-text pastes.

## Closure and limits

The user confirmed the failure affected all page controls and repainting, then reported that an OS reboot fixed it. Stop paste-specific investigation and remove the temporary diagnostic test modules. Keep program-status and Relay flow-control work independently justified by their own tests.

These probes did not reproduce the complete application on the user's actual Hyprland seat. They do not prove a particular OS component was at fault. Do not present an application paste fix, or treat the earlier Relay hypothesis as the cause of this resolved incident.
