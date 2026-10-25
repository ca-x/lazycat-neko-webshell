# Catflap physical clients v1

Contract: Catflap `docs/design/client-device-registration.md`, producer commit
`6c41ae548ee294acccac0f50fbc967cd3e9ba04d`.

- Discover physical clients independently from official LightOS and both fixed
  Catflap origins, with fresh user delegation and partial-failure isolation.
- Preserve the broker in a source-qualified selector through workspace actions,
  sessions and reconnects; launch hints must not choose arbitrary origins.
- Use Catflap v1 ticket validation and the dedicated credential header for HTTP
  and WebSocket, while keeping the official broker transport unchanged.
- Deduplicate stable identities, never names. Without a shared device ID, retain
  distinct broker registrations rather than silently equating unrelated IDs.
  Support optional `device_id` metadata for producers that expose that identity;
  the original v1 list does not provide it, so cross-broker physical-device
  deduplication cannot safely be inferred from that original response alone.
- Keep existing uncommitted interface changes intact. Validate source isolation,
  authorization, ticket expiry, transport credentials, launch persistence and
  duplicate handling before releasing the next WebShell version.

Real platform delegation and a device implementing the Catflap registration /
dynamic-service contract require deployment verification, not just HTTP fixtures.

Retrieval provenance: `nmem` CLI query `Catflap 物理机 API v1`, normal search
(server mode `fast_bm25_vector`), configured default scope, limit 1, no additional
filters. Rank 1: `e3b4f25e-9b94-4883-b637-eb34956b6bf7`,
`Catflap 物理机 API v1：Neko与设备端接入合同`; score was not returned.
Graph visualization was skipped because this CLI host cannot verify the browser
session's owner and Space restrictions.

## Local release verification (2026-10-25)

- `scripts/build-release.sh`: passed with Node 24 and Rust 1.97.1; 593 Rust
  tests passed (4 existing ignored tests), 605 frontend tests and 8 release
  tests passed. Static musl agent/provider builds and the Agent status hook
  checks passed. Agent compatibility versions are unchanged.
- `cargo fmt --all --check`, `git diff --check` and
  `cargo clippy --workspace --all-targets --locked`: passed; existing unrelated
  Clippy warnings remain.
- Browser `workspace-sync` and `mobile-keyboard` scenarios passed.
- The actual static provider served health and embedded version 0.10.1 assets.
  Local HTTP and browser smoke checks rejected unknown broker hints, preserved
  the Catflap registration source and required user delegation without fallback.
- Fixtures check current-ticket-only delegation, the versioned ticket body,
  owner/capability validation, credential headers for both transports, expiry,
  replay identity mapping and source-scoped duplicate isolation. They do not
  claim successful platform delegation or a real physical-device connection.
