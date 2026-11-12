//! Resolve terminal colors against the shared selected theme, independently of app appearance.

use crate::preferences::{current_context, terminal::color};
use nsh_base::{TerminalColor, TerminalStyle};
use nsh_core::prelude::*;

fn resolve(value: TerminalColor) -> Option<AdaptiveColor> {
    match value {
        TerminalColor::Rgb(red, green, blue) => Some(color(
            (red as u32) << 16 | (green as u32) << 8 | blue as u32,
        )),
        TerminalColor::Indexed(index) => current_context()
            .theme()
            .map(|theme| color(theme.palette[index as usize])),
    }
}

/// Resolve indexed and inverse colors without substituting interface foreground or background.
pub fn colors(style: TerminalStyle) -> (Option<AdaptiveColor>, Option<AdaptiveColor>) {
    let style = current_context().style(style);
    (style.fg.and_then(resolve), style.bg.and_then(resolve))
}
