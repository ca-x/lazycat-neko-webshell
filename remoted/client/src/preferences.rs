//! Persisted native appearance, terminal and new-session preferences, independent of connection state.
//!
//! Register this module in the crate root and mount `ui/preferences.rs` from the
//! host UI. See `preferences/README.md` for terminal and translation integration.

#[path = "preferences/catalog.rs"]
pub mod catalog;
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

use anyhow::{Result, ensure};
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
    /// Terminal font size in platform points.
    pub font_size: u8,
    /// Interface appearance, independently of terminal theme.
    pub appearance: Appearance,
    /// Interface language; `system` keeps following the platform locale.
    pub language: Language,
    /// Backend for new local sessions; existing sessions retain their own backend.
    #[serde(default)]
    pub default_backend: SessionBackend,
}

impl Preferences {
    /// Use the WebShell defaults embedded in the generated catalog.
    pub fn defaults(catalog: &catalog::Catalog) -> Self {
        Self {
            version: 1,
            theme_id: catalog.defaults.theme_id.clone(),
            font_size: catalog.defaults.font_size,
            appearance: Appearance::System,
            language: Language::System,
            default_backend: SessionBackend::Native,
        }
    }

    /// Validate version, theme membership, and font bounds before applying or saving.
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
