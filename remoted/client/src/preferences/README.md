# Native preferences integration

This module owns display and default local-session settings. The host registers `pub mod preferences;`
in `lib.rs`, registers `mod preferences;` in `ui.rs`, and chooses where to mount
`ui::preferences::render(preferences_handle)`. Terminal color, surface and read-only observer integration lives in
`ui/terminal_colors.rs`, `ui/terminal_surface.rs` and `ui/terminal_observer.rs`.
Startup, navigation and transport wiring belong to the host.

## Startup and persistence

`preferences::load().await -> anyhow::Result<Preferences>` reads
`remoted.appearance.v1` through the existing `platform::secure_get` API. In the UI
owner, call `preferences::Ui::from_loaded(saved) -> anyhow::Result<Ui>` before
building children. This validates the document, installs the NSH appearance and
locale overrides, and provides the context used by `preferences::tr`.

Alternatively call `preferences::Ui::start() -> anyhow::Result<Ui>` once in the
app owner. It starts with WebShell defaults, loads storage asynchronously, and
keeps editing disabled until the load succeeds. Keep this handle alive at app
scope; opening settings must not recreate it. `Ui::retry_load()` retries failed
reads. Unknown versions, unknown theme IDs, invalid font sizes and corrupt JSON
are explicit errors. Failed reads never overwrite storage.

Use `handle.update(|preferences| { ... })` to save changes. Writes use the
existing `platform::secure_set` API. The view disables editing during a pending
write, and the handle applies values only after storage acknowledges them.
Failures retain the previous values and expose `SaveState::SaveFailed`; retry
by making the change again. Startup and retry both call `preferences::load()`,
so synchronous startup and mounted loading share one validation path.
The storage APIs use the same platform protection
as existing identity/binding storage without touching those documents.
Platform calls run on a dedicated Tokio worker: NSH's UI executor does not
provide the Tokio context required by desktop `spawn_blocking` or Android
timeouts. UI completion remains attached to the app owner, so closing a select
popup or the settings page does not cancel an accepted storage operation.

The persisted fields are `version: 1`, `theme_id: String`, `font_size: u8`
(8–40 platform points), `appearance: Appearance::{System,Light,Dark}`, and
`language: Language::{System,ZhCn,EnUs}`, and
`default_backend: SessionBackend::{Native,Hc,Herdr}`. The serialized values are
`system/light/dark` and `system/zh-CN/en-US`. System settings remain overrides
of `System`, rather than snapshots of the current locale or color scheme.
The default backend serializes as `native/hc/herdr`. Older documents without the
field use Native; invalid values remain explicit load errors. The desktop setting
only selects the initial type on entering the app or local workspace and is hidden on remote-only Android. New sessions follow the currently selected section and ordinary backend.

Use `Ui::startup_backend() -> Option<SessionBackend>` only for initial navigation selection and to check whether settings have loaded. `Loading` and `LoadFailed` return `None`, so creation can stay disabled
until storage is readable. `Ready`, `Saving` and `SaveFailed` return the last
acknowledged value; saving a preference never overrides an explicit current selection.
Existing sessions keep their own backend.

## Translation API

```rust,ignore
use crate::preferences::{tr, tr_format};

text(move || tr("终端"));
text(move || tr("nav.terminal"));
text(move || tr_format("主机 {}", &[&host_number.get()]));
```

Final signatures:

```rust,ignore
pub fn tr(source: &str) -> nsh_core::element::SharedString;
pub fn tr_format(template: &str, values: &[&dyn std::fmt::Display])
    -> nsh_core::element::SharedString;
```

Use them **inside reactive bindings**, including button text and accessibility
labels; calling once while composing a view captures a snapshot. `Ui::tr(key)`
returns a borrowed `&str` for an explicit handle. The pure
`locale::tr(language, system_locale, source) -> &str` works outside the UI engine.
`locale::TRANSLATIONS` exposes stable keys and bilingual copy. `literals.rs`
contains host Chinese literal mappings. `tr_format` translates first, then
substitutes anonymous `{}` placeholders; convert named Rust formatting templates
to anonymous placeholders at call sites. Values are never translated. Unknown
strings pass through. `tr` also recognizes anchored, catalogued formatted statuses
(such as `已退出（3）` and `本设备指纹：...`) and preserves their captured values.
It does not translate arbitrary resource names or error details. Use `tr_format`
for new dynamic labels; format values are never recursively translated.
Unsupported system languages use English; all Chinese system locales use the
Simplified Chinese vocabulary. Explicit English/Chinese also sets NSH's locale
for native formatting and components; System clears that override.

Existing host UI becomes bilingual only as its hardcoded labels are replaced
with these calls. This module alone does not localize the host or arbitrary
backend notices/errors. Known connection, authorization and transfer notices are
in the literal catalog; their displaying bindings must still call `tr`.

## Terminal API

`preferences::current_context().terminal() -> nsh_base::TerminalElement` returns a normal NSH terminal
with reactive font size, line height, foreground, background and cursor color.
The host keeps all its existing component refs, transport callbacks, read-only
behavior and emulator ownership. Plain read-only output can use
`handle.preferences().font_size` and `terminal::color(handle.theme().foreground)`
in bindings. The theme's background is independent of interface appearance.

`preferences::current_context() -> preferences::terminal::Context` captures the
current app preferences handle. Its reactive methods include `theme`, `font_size`,
`line_height`, `background`, `foreground`, `cursor`, `selection`, `style`,
`terminal_line`, and `terminal`. Capture the context during component construction,
then call its methods inside reactive bindings. Before startup provides a handle,
it falls back to the generated Web default theme.

NSH's built-in terminal has no palette property. `SharedEmulator::line()` now
returns `current_context().terminal_line(original_line)` on each read, outside the
untracked raw buffer read. This tracks the selected theme for existing row redraws,
resolves all indexed colors, preserves explicit RGB, and resolves inverse defaults
against terminal colors. Converted lines never enter the original emulator;
Ghostty's palette mapping and the stored indexed colors remain intact.

`terminal_observer::render(buffer, size)` preserves its call signature and is
strictly a read-only projection. Layout and font changes never resize the buffer,
change `size`, or issue a remote resize. It uses the same context for theme/font,
selection highlight and cursor fill. A native virtual list preserves vertical
scrollback and selectable text; a horizontal scroll container preserves terminal
columns when the viewport is narrower than its fixed geometry. Main's existing
terminal view remains responsible for deciding when a writable ordinary terminal
may send a resize. Herdr geometry is not changed here.

The catalog retains cursor text and selection foreground/background colors. The
read-only observer applies the selection background through NSH's supported text
selection API. The pinned NSH terminal exposes cursor fill but no terminal
selection color or cursor-text property; those retained values are available to a
future renderer. This module does not claim to apply unsupported colors.

## Catalog and verification

Run from the repository root with Node supporting `node:module`'s
`stripTypeScriptTypes` and the repository's pinned npm dependencies installed:

```sh
node scripts/export-native-terminal-themes.mjs
node scripts/export-native-terminal-themes.mjs --check
node remoted/client/src/preferences/check.mjs
```

The exporter reads the actual `THEMES` and `DEFAULT_SETTINGS` declarations from
`config.ts`, `INKSTONE_THEMES` from `inkstone-themes.ts`, and Restty's ID prefix
from `theme-registry.ts`. It uses the pinned Restty Ghostty parser and builtin
source API, retains verbatim theme sources, source hashes, Restty version and
full existing license notices. A missing source, duplicate ID, unsupported
declaration shape or package-version mismatch fails generation. `--check` compares
the complete output without writing. No timestamps or network requests are used.

`remoted/themes/catalog.json` is included at compile time, so the native app
requires neither npm nor a JavaScript engine at runtime. Missing palette values
use the real Ghostty default's ANSI values and the standard xterm cube/ramp for
indices 16–255. User-created browser themes and fonts are outside this module.

`check.mjs` builds an isolated temporary test crate using the host's dependency
declarations. It compiles the actual new modules and existing shared controls,
uses a fake protected-storage service (never real credentials), runs pure and
headless tests including the actual Ghostty adapter and both terminal renderers, then Clippy with warnings denied. It does not require adding host
module declarations or modifying Cargo files. Once wired, the pure tests are also
part of the host's normal `cargo test` run. Device and visual checks remain
separate from headless behavior verification.

When another worker has temporarily broken shared controls, `--baseline-controls`
checks the actual new view against `HEAD`'s shared controls in the temporary
crate, without modifying their working files. Report this isolation explicitly;
the default command still checks current controls. `--model-only` compiles the
preference model, runtime and terminal adapter and runs their pure tests without
the host's control layer. The harness copies the current client lockfile into its
temporary directory so dependency resolution starts from the client's versions.

Ghostty's upstream build script needs its pinned source and Zig toolchain. An
existing source/package cache can be supplied without changing any dependency:

```sh
GHOSTTY_SOURCE_DIR=/path/to/pinned/ghostty \
GHOSTTY_ZIG_SYSTEM_DIR=/path/to/zig-packages \
node remoted/client/src/preferences/check.mjs
```

`--skip-controls-tests` keeps compiling the current shared controls but skips
only their own `ui::controls::translation_tests`. This is useful while that
worker is fixing an unrelated test fixture. The default still runs those tests;
report this option explicitly when used.

Theme selection shows the selected source's original theme name and Restty version.
Catalog validation requires a provider version and license source paths as well as
the complete license notices. The terminal display API is `current_context()`;
redundant `Ui::terminal()` and `Ui::terminal_line()` wrappers were removed.
