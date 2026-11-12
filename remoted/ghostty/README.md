# Ghostty emulator for the NSH client

The client adapter is `../client/src/terminal.rs`. Both the application factory
and the shared `ui/terminal_surface.rs` buffer use this emulator. Views share one
terminal; the existing hidden 1×1 resize guard and stale viewport cursor guard
remain in place.

## Integration

The controlling task must add this dependency to **client/Cargo.toml** and update
its lockfile; this change deliberately does not edit either file:

```toml
neko-ghostty = { path = "../ghostty" }
```

Startup must call `crate::terminal::initialize()` and route its `Err` to the
application error page before creating terminal views. It validates terminal
creation, a render snapshot, cursor and modes and returns the factory on success.
The existing infallible `factory()` remains for NSH's factory interface and
existing fixtures. Startup/error-page orchestration is outside this task's write
set and must be connected by the controlling task. `GhosttyEmulator::new` also
returns `Result`, including for invalid dimensions.

This crate pins `libghostty-vt` with default features disabled to upstream Git
revision `cd4d497e8c698b27054a677ce19d1bd1a1a4d8bf`. Its package version is
0.2.1. GitHub API verification on 2026-11-12 found that every one of the 74 blobs
in the supplied `/tmp/neoko-libghostty-review` tree matches that revision.
The repository also has a v0.2.2 tag; the pin here identifies the inspected source,
rather than relying on the package version or a moving branch.

The binding's build script pins Ghostty itself to
`108cacb17ed8990840cd14abfce43d8ed8503ca1` (VT 1.0.1). Building requires Rust
1.96 or later, Zig 0.17, and network access for the Ghostty source and Zig
packages, unless those are supplied in advance. `GHOSTTY_SOURCE_DIR` can point to
an extracted copy of that exact Ghostty revision. Validation used Zig 0.17.1,
Rust/Cargo 1.97.1, and Android NDK 27.3.13750724.

For Android, export `ANDROID_NDK_HOME` and select the NDK linker matching the
application's minimum API. Ghostty otherwise selects Android API 29 with this
Zig toolchain, whereas this app declares API 28. Set `ZIG` to this crate's
executable `tools/zig-android.sh` to pin the Ghostty Android target to API 28.
`NEKO_GHOSTTY_ANDROID_API` overrides 28; `NEKO_GHOSTTY_ZIG` overrides the real
Zig executable. The wrapper passes all other arguments through unchanged.

```sh
export ANDROID_NDK_HOME=/path/to/android-ndk
export ZIG="$PWD/remoted/ghostty/tools/zig-android.sh"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android28-clang"
cargo test --manifest-path remoted/ghostty/Cargo.toml --no-run --target aarch64-linux-android
```

## Pinned CI toolchain and private artifacts

`toolchain.json` is the build-toolchain pin: binding Git revision
`cd4d497e8c698b27054a677ce19d1bd1a1a4d8bf`, Ghostty source revision
`108cacb17ed8990840cd14abfce43d8ed8503ca1`, Zig **0.17.1**, Android NDK
**27.3.13750724**, and Android API **28**. The installer checks the binding
revision against this crate's manifest and both binding entries in the client
lockfile. Updating a pin requires updating the matching manifest/lock entries;
the installer does not rewrite dependencies.

The private `.github/workflows/neoko-terminal-desktop.yml` matrix covers Linux
x86_64 GNU, Windows x86_64 MSVC, and macOS ARM64/Intel. Zig is selected for the
runner CPU, while Cargo selects the application target (including the macOS
Intel cross build). `.github/workflows/neoko-terminal-android.yml` installs the
same toolchain in Linux verification and Android arm64 packaging jobs.

Both use `../scripts/ghostty-toolchain.py setup` to download fixed source and
compiler archives, verify committed SHA-256 values before extraction, and check
the actual Zig version. The source download uses the GitHub API tarball at the
fixed revision, avoiding an unbounded Git clone in the dependency build script.
Network retries and timeouts are bounded. Only checked download archives are
cached; installed source/compiler trees are re-extracted each run. Cargo caches
use a separate prefix derived from the pins and build wrappers so an older
`DOCS_RS=1` sys build cannot satisfy this workflow's native-link checks.

For local builds, use Python 3.12 or later:

```sh
python3 remoted/scripts/ghostty-toolchain.py setup \
  --directory /tmp/neoko-ghostty-toolchain --cache /tmp/neoko-ghostty-archives
python3 remoted/scripts/ghostty-toolchain.py run \
  --directory /tmp/neoko-ghostty-toolchain -- \
  cargo test --locked --manifest-path remoted/client/Cargo.toml --lib
# With ANDROID_NDK_HOME set to the pinned NDK:
python3 remoted/scripts/ghostty-toolchain.py run --android \
  --directory /tmp/neoko-ghostty-toolchain -- \
  bash remoted/client/scripts/build-android.sh
```

`setup` records `NEKO_GHOSTTY_TOOLCHAIN` through `GITHUB_ENV` on Actions, so CI
can omit `--directory` for `run`. The command runner removes `DOCS_RS`, supplies
the extracted `GHOSTTY_SOURCE_DIR`, sets `ZIG` to the installed compiler, and
uses a baseline CPU. `--android` additionally checks the NDK pin, selects the
existing API-28 Zig wrapper, and supplies API-28 NDK linkers for aarch64 and
x86_64. It leaves Cargo's static Ghostty feature selection unchanged.

These workflows only build in `ca-x/lazycat-neko-webshell` when it is private.
They retain the existing immutable release-tag validation, tests, APK signing
and verification, artifact names, package/provenance layout, and read-only
repository permissions. They upload private Actions artifacts; they do not
publish releases. The public repository must continue to retrieve and verify
those artifacts through its existing verified-artifact process rather than
rebuilding native source. No deployment is part of this toolchain setup.

### Packaged notices and metadata contract

`licenses/manifest.json` pins the source/wrapper revisions and the SHA-256 of
21 attribution files. The complete set plus that manifest (22 files) is copied
under the existing package license directory as `ghostty/`: desktop ZIP
`licenses/ghostty/`, macOS `neoko-terminal.app/Contents/Resources/licenses/ghostty/`,
Linux deb `usr/share/doc/neoko-terminal/licenses/ghostty/`, and Android APK
`assets/licenses/ghostty/`. It includes both project MIT licenses, the pinned VT
third-party inventory/notices (including uucode's supplemental Unicode/Bjoern
notices), and Zig/LLVM runtime notices. Original license bytes are preserved;
the directory's Git attributes disable newline conversion on Windows.

`../scripts/native_build_inputs.py` validates those digests before packaging.
Desktop ZIPs and signed APKs are read back and compared byte-for-byte; package
tests also extract and compare the deb contents. Existing NEKO/NSH/ZXing license
handling and existing provenance checks remain in place.

Desktop inner and outer `build-info.json` retain all existing fields and add
three flat keys: `ghostty_source_revision`, `ghostty_wrapper_revision`, and
`zig_version`, with values from `toolchain.json`. Android `build-info.json`
adds the same three keys plus `source_sha` and `source_dirty`; those five fields
are also embedded as `assets/build-inputs.json`, verified after signing.
The separate public `provenance.json` schema is unchanged by these scripts.

`source_dirty` checks tracked changes and untracked inputs in client, protocol,
host, themes, ghostty, program-status, native build scripts/workflows, workspace
manifests/locks, and the native theme exporter. It no longer treats a clean
client/protocol directory alone as proof of clean native build inputs.

## Behavior and limits

- Ghostty owns parsing, primary/alternate screens, reflow, scrollback, UTF-8 and
  combining graphemes. The adapter skips the trailing cell of wide characters.
- Rendering uses Ghostty's render snapshots, caching a viewport at a time.
  Historical line reads restore the viewport to the active screen before any
  subsequent feed. Feed and resize invalidate the cache.
- Indexed and RGB colors, OSC palette/default foreground/background changes,
  and all flags expressible by `TerminalStyle` are mapped to NSH. Unset colors
  continue to use NSH's theme. Ordinary ANSI/256 palette indices remain indexed,
  including background-only cells, so the theme's palette is not baked into RGB.
  Only explicit OSC changes to an individual palette entry resolve that entry
  to RGB; the current trait cannot transmit palette replacements separately.
- Cursor position/visibility, application cursor mode, bracketed paste, title
  changes and bells cross the NSH trait boundary.
- No PTY writer, clipboard, or report callback is registered. This crate exposes
  only title/bell events, so live and replay data have no reply channel. The
  surface additionally retains its defensive `Reply` filter. The server remains
  the only PTY reply authority.
- NSH's current trait cannot express cursor shape/blink/color, mouse tracking
  and encoding, Kitty keyboard negotiation, hyperlinks, images, distinct
  underline styles/colors, text blink/overline, or render hold scheduling.
  These are not claimed as supported. Disabling the binding's default feature
  also disables Kitty graphics. Matching the parser does not establish identical
  font shaping or the full Restty renderer/input experience.
- Ghostty prunes scrollback at page granularity, so its configured line limit
  can retain additional physical rows. This crate removes the default byte cap
  and uses the caller's requested line limit; it does not promise an exact cap.
- The NSH trait has no error return. Query/resize failures enter a permanent
  stopped state without changing kernels. Previously rendered rows and the
  last input modes are retained, the cursor is hidden, and an extra error row
  is displayed. Unavailable rows explicitly show the failure. `Buffer::failure()`
  exposes the structured operation/message, and one title/bell notification is
  queued. Further feed/resize calls cannot overwrite the frozen display. The
  retained kernel is not destroyed; recovery requires an explicit session
  rebuild/resync. Ghostty's VT-processing error flag is checked after feed.
  Tests inject a returned out-of-memory error into the state machine; they do
  not force operating-system-wide memory exhaustion or claim to catch Rust's
  global allocation abort.

## Validation and measured cost

All validation used temporary manifests under `/tmp`, leaving the client's
manifest and lockfile untouched. The standalone harness used the inspected
local upstream path after checking it against the fixed Git tree, avoiding a
second network clone. The NSH surface harness compiled the actual client source
files against the actual pinned NSH dependencies, not substitute trait types.

| Check | Result |
| --- | --- |
| Linux x86_64 GNU core tests | 6 passed: history/alternate/reflow, split UTF-8/CJK/combining, styles/palette, modes/events/probes, OSC default colors/cache, retained ANSI indices/background-only cells |
| Linux real NSH adapter/surface tests | 6 passed: no replies on replay, trait mapping, hidden resize, shared views/stale cursor, frozen error state/one notification, invalid dimensions/fallible startup |
| Android aarch64, API 28 | Both unit and integration test executables compiled and linked; no device execution |
| Core Clippy, all targets, warnings denied | Passed; upstream binding emits four existing dead-code warnings |
| Core rustdoc | Passed |
| Full client tests through temporary manifest | Initial attempt blocked by a `controller.rs` match missing `Event::TerminalProgramStatus`; the controlling task subsequently reported fixing this and owns the full-client rerun |

The first Linux native build/link took about 171 seconds, including Zig package
fetches; that run exposed an incorrect test assumption about cursor position on
alternate-screen entry, fixed by explicitly homing the fixture. A warm successful
test run took 0.18 seconds. The first Android build after setting the NDK took
86 seconds; rebuilding for API 28 took 50 seconds with dependencies cached.
The real NSH surface harness took about 3 seconds with the existing client cache.
These builds ran concurrently and are observations, not performance guarantees.
Downloading the 39 MiB Ghostty source archive separately took about ten minutes
on this connection.

Unstripped debug Ghostty static archives were 67.1 MiB (Linux) and 33.0 MiB
(Android). Core integration test executables were 48.9 MiB and 32.2 MiB.
These are debug development artifacts, not release APK or application size.
Release size, throughput, memory profiling, Android device execution, and a
visual comparison against WebShell were not measured. Android x86_64 and Apple/
Windows targets were not compiled in the initial emulator delivery; see the
subsequent build-toolchain validation below.

Temporary evidence: `/tmp/neko-ghostty-linux.log`,
`/tmp/neko-ghostty-surface.log`, `/tmp/neko-ghostty-android-api28.log`,
`/tmp/neko-ghostty-clippy.log`, `/tmp/neko-ghostty-doc.log`, and
`/tmp/neko-ghostty-client.log`.

The fixed Git revision is now present in Cargo's own cache at
`~/.cargo/git/checkouts/libghostty-rs-28fee7453bdb2b25/cd4d497` and was resolved
successfully by `cargo metadata --offline` with a fresh temporary manifest.
`CARGO_NET_OFFLINE=true` avoids additional Cargo Git fetches on this machine;
keep `GHOSTTY_SOURCE_DIR=/tmp/neko-ghostty-source` to avoid the binding build
script independently attempting a Ghostty Git clone. The Zig packages required
by the tested targets are also cached. Cache paths and temporary files are local
validation conveniences, not portable build prerequisites.

### Build-toolchain validation (2026-11-12)

The new command runner was exercised with `DOCS_RS=1` intentionally present in
the parent environment, a SHA-256-verified source archive, the existing local
Zig 0.17.1 installation, and a temporary manifest using the real fixed Git
dependency. Linux compiled/linked and passed all six core integration tests
(44.74 seconds cold target directory). Android arm64/API 28 compiled and linked
both test executables (67 seconds), without executing them on a device.

Direct Zig cross builds of the pinned kernel also produced ARM64 and x86_64
macOS static archives and Mach-O dylibs. These are kernel builds on Linux, not
native macOS app/package execution. A Linux-to-Windows MSVC kernel attempt
failed on missing MSVC C/C++ headers; the Windows workflow enables the pinned
MSVC environment action and still requires actual Windows-runner validation.
No hosted Actions job or production application release was built in this
toolchain-edit task.

The build-script suite passed 22 tests (11 toolchain contracts, seven packaging
contracts, and four existing artifact contracts), including archive corruption,
extraction, pin drift, removal of DOCS_RS, actual child environment, Android
argument rewriting, expanded dirty detection, Windows ZIP packaging and actual
deb creation/extraction using a fixture executable. Actionlint 1.7.12 with
ShellCheck 0.11.0 passed both workflows.
Installer unit tests use local fixtures. A subsequent real setup also completed:
the official Zig 0.17.1 Linux archive (57,411,036 bytes) was downloaded in bounded
byte ranges to work around this connection's slow single-stream transfer. Its
full SHA-256 matched `3581863e3cbcfd0fce0efc682ce7e9e93da96668a977791d356a26d2a7e9b17a`.
The actual `setup` command verified and extracted this archive plus the previously
downloaded pinned Ghostty source archive into `/tmp/neoko-ghostty-setup-real`.
The actual `run` command then built/linked and passed all six core tests using
that newly installed compiler, a fresh Cargo target directory, and the fixed
Git dependency (47.67 seconds). `DOCS_RS=1` was intentionally set in the parent
environment. This final check did not reuse the existing compiler installation.

Download, installation and Cargo evidence: `/tmp/neoko-ghostty-zig-download-real.log`,
`/tmp/neoko-ghostty-setup-real.log`, and `/tmp/neoko-ghostty-setup-real-kernel.log`.
The full-client `--locked --lib` attempt stopped before compilation because its
manifest/lockfile were not synchronized during concurrent host dependency work;
the controlling task owns that lockfile and the complete client rerun. Its log
is `/tmp/neoko-ghostty-setup-real-cargo.log`.

The Android packaging script also completed a real 0.3.0/build-8 APK using the
existing native library and a temporary test certificate. APK signing and
16-KiB alignment checks passed; every new license and embedded build-inputs
record was read back exactly. This is a packaging regression, not a rebuild of
the current client or a production-signed release. Evidence:
`/tmp/neko-ghostty-package-android.log` and
`/tmp/neko-ghostty-build-scripts.log`.

Evidence: `/tmp/neko-ghostty-ci-linux.log`,
`/tmp/neko-ghostty-ci-android.log`, and
`/tmp/neko-ghostty-ci-{aarch64-macos-none,x86_64-macos-none,x86_64-windows-msvc}.log`.
