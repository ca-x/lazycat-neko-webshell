//! Shared Ghostty colors generated from WebShell sources and pinned Restty themes.

use super::{MAX_FONT_SIZE, MIN_FONT_SIZE};
use anyhow::{Result, ensure};
use serde::Deserialize;
use std::{collections::HashSet, sync::OnceLock};

/// A generated terminal palette; colors are packed 24-bit RGB values.
#[derive(Clone, Debug, Deserialize)]
pub struct TerminalTheme {
    /// Stable WebShell ID.
    pub id: String,
    /// Theme's original display name.
    pub label: String,
    /// License association: `restty` or `inkstone`.
    pub origin: String,
    /// Original Restty catalog name when applicable.
    pub ghostty_name: Option<String>,
    /// Verbatim Ghostty source, retained for provenance and future renderers.
    pub ghostty_source: String,
    /// Default terminal background.
    pub background: u32,
    /// Default terminal foreground.
    pub foreground: u32,
    /// Cursor fill.
    pub cursor: u32,
    /// Text under the cursor.
    pub cursor_text: u32,
    /// Selection fill.
    pub selection_background: u32,
    /// Selected text.
    pub selection_foreground: u32,
    /// All 256 indexed colors, with the xterm ramp filling unspecified entries.
    pub palette: Vec<u32>,
}

/// Defaults exported directly from WebShell's `DEFAULT_SETTINGS`.
#[derive(Clone, Debug, Deserialize)]
pub struct Defaults {
    /// Initial terminal theme ID.
    pub theme_id: String,
    /// Initial terminal font size.
    pub font_size: u8,
}

/// Attribution shipped with the catalog and compiled into the native application.
#[derive(Clone, Debug, Deserialize)]
pub struct License {
    /// Theme origin associated with this license.
    pub id: String,
    /// Repository path of the original license.
    pub path: String,
    /// Unmodified license text, including copyright notices.
    pub text: String,
}

/// The validated application catalog; no network or JavaScript runtime is required.
#[derive(Clone, Debug, Deserialize)]
pub struct Catalog {
    /// Catalog format version.
    pub version: u8,
    /// Version of the pinned Restty theme provider.
    pub restty_version: String,
    /// Web defaults shared with the native client.
    pub defaults: Defaults,
    /// Redistributable attribution texts.
    pub licenses: Vec<License>,
    /// Ordered WebShell themes followed by all Restty builtins.
    pub themes: Vec<TerminalTheme>,
}

impl Catalog {
    /// Parse and validate the generated catalog once per process.
    pub fn shared() -> Result<&'static Self> {
        static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
        CATALOG
            .get_or_init(|| {
                Self::parse(include_str!("../../../themes/catalog.json"))
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| anyhow::anyhow!(error.clone()))
    }

    /// Parse catalog JSON, rejecting incomplete palettes and missing attribution.
    pub fn parse(source: &str) -> Result<Self> {
        let catalog: Self = serde_json::from_str(source)?;
        ensure!(catalog.version == 1, "Unsupported terminal catalog version");
        ensure!(!catalog.themes.is_empty(), "Empty terminal catalog");
        ensure!(!catalog.restty_version.is_empty(), "Missing Restty version");
        ensure!(
            catalog
                .licenses
                .iter()
                .all(|license| !license.path.is_empty()),
            "Missing license source path"
        );
        ensure!(
            (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&catalog.defaults.font_size),
            "Invalid catalog default font size"
        );
        ensure!(
            catalog.find(&catalog.defaults.theme_id).is_some(),
            "Missing default theme"
        );
        let mut ids = HashSet::new();
        for theme in &catalog.themes {
            ensure!(ids.insert(&theme.id), "Duplicate theme ID: {}", theme.id);
            ensure!(
                !theme.label.is_empty() && !theme.ghostty_source.is_empty(),
                "Incomplete theme: {}",
                theme.id
            );
            ensure!(
                theme.palette.len() == 256,
                "Incomplete palette: {}",
                theme.id
            );
            ensure!(
                theme
                    .palette
                    .iter()
                    .chain([
                        &theme.background,
                        &theme.foreground,
                        &theme.cursor,
                        &theme.cursor_text,
                        &theme.selection_background,
                        &theme.selection_foreground
                    ])
                    .all(|color| *color <= 0xffffff),
                "Invalid RGB value: {}",
                theme.id
            );
            ensure!(
                catalog
                    .licenses
                    .iter()
                    .any(|license| license.id == theme.origin && !license.text.is_empty()),
                "Missing theme license: {}",
                theme.id
            );
        }
        Ok(catalog)
    }

    /// Resolve an exact ID without silently changing the requested selection.
    pub fn find(&self, id: &str) -> Option<&TerminalTheme> {
        self.themes.iter().find(|theme| theme.id == id)
    }
}
