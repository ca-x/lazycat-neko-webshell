# hc Backend Implementation Plan

> **For agentic workers:** Use subagent-driven-development for the independent protocol and frontend tasks; controller integrates workspace lifecycle and release.

**Goal:** Add a selectable hc persistence backend while retaining native and its settings.
**Architecture:** standalone hc + bounded provider protocol client; existing optional workspace store and shared pane UI.
**Tech Stack:** Rust/Tokio/Axum, TypeScript/Restty, installed hc 0.1.14 API generation 1.
**Spec:** docs/superpowers/specs/2026-10-30-hc-backend-design.md

## Global Constraints
Follow AGENTS.md. Default backend webshell, hc optional; LightOS only. Do not change native Agent implementation/minimum/protocol: target hc is a separate process launched by provider. No new frontend algorithms in main.ts. No automatic installs, runtime upgrades, live migration, raw caller RPC or credentials in logs. All commands preserve target UID/GID and controlled environment. Preserve the untracked reference clone in the source checkout.

## Review Focus
Unknown create outcome must not duplicate a command; close failures must retain local state; framing cancellation must not lose bytes; replay must not answer queries; default changes must not alter existing session ownership.

### Task 1: hc protocol client
- [x] Add src/hc/protocol.rs and tests: cancellation-safe bounded framed client, generation/features negotiation, request correlation and early attach DATA preservation, close process cleanup.
- [x] RED/GREEN: split headers/payloads, oversize/unknown frames, RPC errors, response id mismatch, early data, truncated EOF, bounded queues and credit.

### Task 2: frontend settings and pane integration
- [x] Add hc to SessionBackendId, settings normalization, labels and existing settings option. Default stays webshell.
- [x] Add focused hc frontend adapter for URL, reset/repaint, reply authority, reconnect and split; main.ts only orchestration.
- [x] RED/GREEN focused Node tests for persisted hc, native compatibility, changed default not changing session ownership, reconnect and paint reset.

### Task 3: provider lifecycle and workspace integration
- [x] Add src/hc module with target launch, core scope, session create/reconcile/attach/close and WebSocket adaptation.
- [x] Add hc backend to existing optional workspace path, preserving creation intents and close-before-delete semantics. hc routes never enter native ManagedTerminal.
- [x] Verify fake core lifecycle tests including ambiguous creation, browser detach and failed explicit termination; validate target authorization and identities.

### Task 4: integration, review, release
- [x] Integrate independent patches, run full Rust/frontend checks and isolated hc smoke if available.
- [x] Independent whole-diff review and fix confirmed findings; update docs, settings explanations and version consistently.
- [ ] Commit, tag and push v0.11.0; private build then public workflow dispatch for that exact source run. Verify artifact digest and actual MiaoMiao/official outcomes; update release notes and save durable record.

## Verification record

- Full workspace: 667 Rust tests passed (six environment-dependent tests remain ignored in the ordinary suite).
- Official hc0.1.14: both isolated tests explicitly enabled and passed, covering PTY detach/same-PID reconnect, resize, status, nonzero exit and final WebSocket replay.
- Frontend: 738 tests; release scripts: eight tests; TypeScript and production build passed.
- Strict workspace Clippy, rustfmt and rustdoc passed. Independent final review found no remaining release blocker.
- Regression fixes include canonical HC IDs, whitespace-normalized transport identity, immutable target login/backend, retry before ready, no silent HC fallback, and a separate durable HC journal surviving stale snapshots.
- Validation boundary: target launcher scripts and official HC were exercised locally; an authenticated live LightOS gateway was not available for a deployment smoke. Browser delivery credits do not claim renderer consumption.
