//! Terminal appearance adapters for NSH's fixed-palette terminal renderer.

use super::{Ui, catalog::TerminalTheme};
use nsh_base::{TerminalColor, TerminalElement, TerminalLine, TerminalStyle, terminal};
use nsh_core::prelude::*;

/// Reactive terminal appearance captured from the app's installed preference handle.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    ui: Option<Ui>,
}

impl Context {
    pub(crate) fn new(ui: Option<Ui>) -> Self {
        Self { ui }
    }

    /// Resolve the current theme, or the generated Web default before startup installs preferences.
    pub fn theme(self) -> Option<&'static TerminalTheme> {
        self.ui.map(Ui::theme).or_else(|| {
            let catalog = super::catalog::Catalog::shared().ok()?;
            catalog.find(&catalog.defaults.theme_id)
        })
    }

    /// Terminal font size in platform points, tracking saved changes.
    pub fn font_size(self) -> u8 {
        self.ui
            .map(|ui| ui.preferences().font_size)
            .unwrap_or_else(|| {
                super::catalog::Catalog::shared().map_or(14, |catalog| catalog.defaults.font_size)
            })
    }

    /// Row height used by both interactive and read-only terminal surfaces.
    pub fn line_height(self) -> f32 {
        self.font_size() as f32 * 1.22
    }

    /// Default terminal background, independently of interface appearance.
    pub fn background(self) -> AdaptiveColor {
        self.theme().map_or_else(
            || nsh_base::theme().background,
            |theme| color(theme.background),
        )
    }

    /// Default terminal foreground, independently of interface appearance.
    pub fn foreground(self) -> AdaptiveColor {
        self.theme().map_or_else(
            || nsh_base::theme().foreground,
            |theme| color(theme.foreground),
        )
    }

    /// Theme cursor fill for editable and read-only surfaces.
    pub fn cursor(self) -> AdaptiveColor {
        self.theme()
            .map_or_else(|| self.foreground(), |theme| color(theme.cursor))
    }

    /// Selection highlight for NSH selectable text; native selection keeps its own text color.
    pub fn selection(self) -> nsh_core::color::Hsla {
        self.theme().map_or_else(
            || {
                let accent = nsh_base::theme().accent;
                if dark() { accent.dark } else { accent.light }
            },
            |theme| nsh_core::color::Hsla::from(nsh_core::color::rgb(theme.selection_background)),
        )
    }

    /// Resolve palette indices and inverse colors in the current theme.
    pub fn style(self, style: TerminalStyle) -> TerminalStyle {
        self.theme().map_or(style, |theme| theme.style(style))
    }

    /// Resolve a copied raw emulator line without losing its stored palette indices.
    pub fn terminal_line(self, line: TerminalLine) -> TerminalLine {
        match self.theme() {
            Some(theme) => theme.line(line),
            None => line,
        }
    }

    /// Build an NSH terminal with reactive font metrics, text, background, and cursor fill.
    pub fn terminal(self) -> TerminalElement {
        terminal()
            .font_size(move || px(self.font_size() as f32))
            .line_height(move || px(self.line_height()))
            .bg(move || self.background())
            .text_color(move || self.foreground())
            .cursor_style(move |style| style.bg(move || self.cursor()))
    }
}

/// Convert a catalog RGB value into an appearance-independent NSH data color.
pub fn color(rgb: u32) -> AdaptiveColor {
    nsh_core::color::Hsla::from(nsh_core::color::rgb(rgb)).into()
}

fn terminal_color(rgb: u32) -> TerminalColor {
    TerminalColor::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl TerminalTheme {
    /// Resolve indexed colors and inverse defaults while preserving explicit truecolor.
    pub fn style(&self, mut style: TerminalStyle) -> TerminalStyle {
        let resolve = |value| match value {
            TerminalColor::Indexed(index) => terminal_color(self.palette[index as usize]),
            rgb => rgb,
        };
        style.fg = style.fg.map(resolve);
        style.bg = style.bg.map(resolve);
        if style.inverse {
            let foreground = style.fg.unwrap_or(terminal_color(self.foreground));
            let background = style.bg.unwrap_or(terminal_color(self.background));
            style.fg = Some(background);
            style.bg = Some(foreground);
            style.inverse = false;
        }
        style
    }

    /// Convert a copied emulator line for rendering; never modify the emulator's palette indices.
    pub fn line(&self, mut line: TerminalLine) -> TerminalLine {
        for run in &mut line.runs {
            run.style = self.style(run.style);
        }
        line
    }
}
