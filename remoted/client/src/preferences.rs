//! Persisted native appearance, terminal and new-session preferences, independent of connection state.
//!
//! Register this module in the crate root and mount `ui/preferences.rs` from the
//! host UI. See `preferences/README.md` for terminal and translation integration.

#[path = "preferences/catalog.rs"]
pub mod catalog;
mod cursor;
#[path = "preferences/font.rs"]
pub(crate) mod font;
#[path = "preferences/io.rs"]
mod io;
#[path = "preferences/locale.rs"]
pub mod locale;
#[path = "preferences/runtime.rs"]
mod runtime;
#[path = "preferences/terminal.rs"]
pub mod terminal;

pub use locale::Language;
pub use runtime::{SaveState, Ui};

/// Capture display preferences from the current NSH owner, with shared Web defaults as fallback.
pub fn current_context() -> terminal::Context {
    terminal::Context::new(use_context::<Ui>())
}

use anyhow::{Result, bail, ensure};
use neko_remoted_protocol::messages::SessionBackend;
use nsh_core::prelude::{SharedString, locale, use_context};
use serde::{Deserialize, Serialize};
use std::fmt::Display;

/// Load and validate protected preferences before constructing the application's UI owner.
pub async fn load() -> Result<Preferences> {
    let catalog = catalog::Catalog::shared()?;
    let value = io::read().await?;
    Preferences::decode(value.as_deref(), catalog)
}

/// Translate a Chinese literal or stable key using the installed reactive preference context.
///
/// Call inside a binding such as `text(move || preferences::tr("终端"))` so
/// language changes redraw existing views. Known formatted status templates preserve
/// their values; unknown literals pass through unchanged. Prefer `tr_format` for new code.
pub fn tr(source: &str) -> SharedString {
    use_context::<Ui>().map_or_else(
        || locale::translate(Language::System, locale().as_str(), source).into(),
        |ui| locale::translate(ui.preferences().language, locale().as_str(), source).into(),
    )
}

/// Translate an anonymous `{}` template before substituting display values in order.
///
/// Only anonymous placeholders are supported. Arguments must not include secrets;
/// use `tr_format("主机 {}", &[&number])` in a reactive binding. Named Rust format
/// specifiers must first be replaced by `{}` in the caller's template.
pub fn tr_format(template: &str, values: &[&dyn Display]) -> SharedString {
    let translated = tr(template);
    let mut parts = translated.split("{}");
    let mut output = parts.next().unwrap_or_default().to_owned();
    for (index, part) in parts.enumerate() {
        if let Some(value) = values.get(index) {
            output.push_str(&value.to_string());
        } else {
            output.push_str("{}");
        }
        output.push_str(part);
    }
    output.into()
}

/// The protected-storage key, separate from identity and connection preferences.
pub const STORAGE_KEY: &str = "remoted.appearance.v1";
/// The smallest supported terminal font size, in platform points.
pub const MIN_FONT_SIZE: u8 = 8;
/// The largest supported terminal font size, in platform points.
pub const MAX_FONT_SIZE: u8 = 40;

/// Saved terminal cursor shape, independent of the UI platform.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    /// Fill the terminal cell, preserving the previous native default.
    #[default]
    Block,
    /// A vertical insertion bar.
    Beam,
    /// A horizontal line at the bottom of the cell.
    Underline,
}

impl CursorStyle {
    /// Stable setting value for selection controls and renderer integrations.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Beam => "beam",
            Self::Underline => "underline",
        }
    }

    pub(crate) fn translation_key(self) -> &'static str {
        match self {
            Self::Block => "preferences.cursor_block",
            Self::Beam => "preferences.cursor_beam",
            Self::Underline => "preferences.cursor_underline",
        }
    }
}

/// Desktop window-close policy; actual quit and tray actions belong to the host.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseBehavior {
    /// Ask whether to quit or minimize to the tray.
    #[default]
    Ask,
    /// Quit the application.
    Quit,
    /// Keep the application running in the system tray.
    Tray,
}

impl CloseBehavior {
    /// Stable setting value for desktop window lifecycle integration.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Quit => "quit",
            Self::Tray => "tray",
        }
    }

    pub(crate) fn translation_key(self) -> &'static str {
        match self {
            Self::Ask => "preferences.close_ask",
            Self::Quit => "preferences.close_quit",
            Self::Tray => "preferences.close_tray",
        }
    }
}

/// An interface appearance override; terminal colors remain independently selected.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Follow platform appearance changes.
    #[default]
    System,
    /// Always use the application's light tokens.
    Light,
    /// Always use the application's dark tokens.
    Dark,
}

impl Appearance {
    pub(crate) fn native(self) -> nsh_core::platform::Appearance {
        match self {
            Self::System => nsh_core::platform::Appearance::System,
            Self::Light => nsh_core::platform::Appearance::Light,
            Self::Dark => nsh_core::platform::Appearance::Dark,
        }
    }
}

/// One validated document stores native appearance, terminal and new-session preferences.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// Document version; unsupported documents are never silently overwritten.
    pub version: u8,
    /// The exact WebShell theme ID, including `ghostty:` for Restty's catalog.
    pub theme_id: String,
    /// Primary terminal family; older documents retain the bundled Hack default.
    #[serde(default = "font::default_family")]
    pub font_family: String,
    /// Terminal font size in platform points.
    pub font_size: u8,
    /// Native cursor shape; missing fields retain the previous block cursor.
    #[serde(default)]
    pub cursor_style: CursorStyle,
    /// Whether the cursor blinks; older documents retain a steady cursor.
    #[serde(default)]
    pub cursor_blink: bool,
    /// Explicit #RRGGBB cursor color; absent uses a visible theme cursor.
    #[serde(default)]
    pub cursor_color: Option<String>,
    /// Interface appearance, independently of terminal theme.
    pub appearance: Appearance,
    /// Interface language; `system` keeps following the platform locale.
    pub language: Language,
    /// Backend for new local sessions; existing sessions retain their own backend.
    #[serde(default)]
    pub default_backend: SessionBackend,
    /// Request control of each remote terminal when opened; authorization still applies.
    #[serde(default)]
    pub remote_auto_control: bool,
    /// Desktop window-close policy; older documents ask before choosing an action.
    #[serde(default)]
    pub close_behavior: CloseBehavior,
}

impl Preferences {
    /// Use the WebShell defaults embedded in the generated catalog.
    pub fn defaults(catalog: &catalog::Catalog) -> Self {
        Self {
            version: 1,
            theme_id: catalog.defaults.theme_id.clone(),
            font_family: font::default_family(),
            font_size: catalog.defaults.font_size,
            cursor_style: CursorStyle::Block,
            cursor_blink: false,
            cursor_color: None,
            appearance: Appearance::System,
            language: Language::System,
            default_backend: SessionBackend::Native,
            remote_auto_control: false,
            close_behavior: CloseBehavior::Ask,
        }
    }

    /// Validate version, theme membership, and font names and bounds before applying or saving.
    pub fn validate(&self, catalog: &catalog::Catalog) -> Result<()> {
        ensure!(self.version == 1, "Unsupported display preferences version");
        ensure!(
            catalog.find(&self.theme_id).is_some(),
            "Unknown terminal theme: {}",
            self.theme_id
        );
        ensure!(
            (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&self.font_size),
            "Terminal font size must be {MIN_FONT_SIZE}–{MAX_FONT_SIZE}"
        );
        if let Some(color) = &self.cursor_color {
            cursor::validate(color)?;
        }
        if let Some(key) = font::validation_error(&self.font_family) {
            bail!("{}", locale::tr(Language::EnUs, "en-US", key));
        }
        Ok(())
    }

    /// Read an existing document without converting damaged storage into defaults.
    pub fn decode(value: Option<&str>, catalog: &catalog::Catalog) -> Result<Self> {
        let preferences = match value {
            Some(value) => serde_json::from_str(value)?,
            None => Self::defaults(catalog),
        };
        preferences.validate(catalog)?;
        Ok(preferences)
    }

    /// Encode a validated document for protected storage.
    pub fn encode(&self, catalog: &catalog::Catalog) -> Result<String> {
        self.validate(catalog)?;
        Ok(serde_json::to_string(self)?)
    }
}

#[cfg(test)]
#[path = "preferences/tests.rs"]
mod tests;
