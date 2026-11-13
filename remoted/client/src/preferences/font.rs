//! Persisted terminal font names, independent of the platform font resolver.

pub(crate) const HACK: &str = "Hack Nerd Font Mono";
pub(crate) const MAPLE: &str = "Maple Mono NF CN";
pub(crate) const SYSTEM: &str = "monospace";
/// Bound stored names and custom input without restricting names to ASCII.
pub(crate) const MAX_FAMILY_BYTES: usize = 128;

pub(super) fn default_family() -> String {
    HACK.to_owned()
}

/// Return a translation key for invalid single-family names.
pub(crate) fn validation_error(family: &str) -> Option<&'static str> {
    if family.is_empty() || family.trim() != family {
        Some("preferences.font_name_required")
    } else if family.len() > MAX_FAMILY_BYTES {
        Some("preferences.font_name_length")
    } else if family
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
    {
        Some("preferences.font_name_controls")
    } else if family.contains([',', '\'', '"', '\\']) {
        Some("preferences.font_name_single")
    } else {
        None
    }
}
