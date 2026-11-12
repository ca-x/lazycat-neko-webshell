# Portable OSC 7501 modules

`osc7501.rs` and `stream.rs` are vendored from the published `w1` 0.3.0
crate by krhougs (https://github.com/krhougs/w1). They contain only protocol,
text sanitization, and byte-stream logic. The MIT and Apache 2.0 licenses are
preserved alongside these files.

The stream module's `crate::osc7501` import is changed to `super::osc7501`
so the modules remain private to this adapter. Formatting follows this crate;
nested encoder conditions use equivalent let chains, UTF-8 continuation uses
`if let`, and a reused string uses `clone_into` to satisfy Clippy. The byte
state machine keeps its upstream layout with a local line-count lint allowance.
No Unix process, PTY, filesystem inspection, or session-store code is vendored.

Unix builds continue using the upstream `w1` dependency. Non-Unix builds use
these modules with the same `Tracker` adapter and no automatic process
detection. Unix tests compare the portable parser and stream with upstream
across valid, malformed, bounded, split, and reset inputs. Keep that comparison
when refreshing the vendored source.
