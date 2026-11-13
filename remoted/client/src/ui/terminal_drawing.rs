//! Row paint commands for a bounded native viewport; Ghostty owns grapheme bounds.

use super::geometry::Grid;
use crate::{
    preferences::{CursorStyle, current_context},
    terminal::GridCell,
};
use nsh_base::TerminalStyle;
use nsh_core::{
    canvas::{CanvasFont, CanvasOp, CanvasTextAlign, CanvasTextBaseline},
    prelude::*,
};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fonts {
    variants: [CanvasFont; 4],
    pub grid: Grid,
}

impl Fonts {
    pub fn measure() -> Self {
        let appearance = current_context();
        let size = appearance.font_size();
        let family = crate::fonts::terminal_family();
        let styles = ["", "bold ", "italic ", "italic bold "];
        let variants = std::array::from_fn(|index| CanvasFont {
            size: px(f32::from(size)),
            weight: if index % 2 == 1 { 700 } else { 400 },
            italic: index >= 2,
            // Painter::font's CSS parser drops fallback families in NSH f4d68f7.
            // Native backends accept a complete family list through CanvasFont.
            family: Some(family.as_ref().into()),
            ..Default::default()
        });
        let width = measure_text(&format!("{size}px {family}"), "0000000000").0 / 10.0;
        let metrics: Vec<_> = family
            .split(',')
            .flat_map(|family| {
                styles.map(|style| {
                    text_metrics(
                        &format!("{style}{size}px {}", family.trim()),
                        "Mg猫e\u{301}\u{e0b0}",
                    )
                })
            })
            .collect();
        Self {
            variants,
            grid: Grid::measured(width, appearance.line_height(), &metrics),
        }
    }

    fn font(&self, style: TerminalStyle) -> &CanvasFont {
        &self.variants[usize::from(style.bold) + 2 * usize::from(style.italic)]
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Paint {
    pub foreground: Hsla,
    pub background: Hsla,
    pub selection: Hsla,
    pub cursor_color: Hsla,
    pub selected: Option<(usize, usize)>,
    pub cursor: Option<usize>,
    pub cursor_style: CursorStyle,
}

pub(super) fn cursor_cell(cells: &[GridCell], column: usize) -> (usize, usize) {
    cells
        .iter()
        .find(|cell| cell.column <= column && column < cell.column + cell.width)
        .map_or((column, 1), |cell| (cell.column, cell.width))
}

/// Explicit cell backgrounds are painted before the viewport's below-text images.
pub(super) fn backgrounds(
    cells: &[GridCell],
    grid: Grid,
    resolve: impl Fn(TerminalStyle) -> Option<Hsla>,
) -> Painter {
    let mut painter = Painter::new();
    for cell in cells {
        if let Some(background) = resolve(cell.style) {
            painter = painter.fill_color(background).fill_rect(
                cell.column as f32 * grid.cell_width,
                0.0,
                cell.width as f32 * grid.cell_width,
                grid.row_height,
            );
        }
    }
    painter
}

/// Selection, cursor, and glyphs remain above the below-text image layer.
pub(super) fn foreground(
    cells: &[GridCell],
    fonts: &Fonts,
    paint: Paint,
    resolve: impl Fn(TerminalStyle) -> (Option<Hsla>, Option<Hsla>),
) -> Painter {
    let grid = fonts.grid;
    let mut painter = Painter::new()
        .text_align(CanvasTextAlign::Left)
        .text_baseline(CanvasTextBaseline::Alphabetic);
    if let Some((start, end)) = paint.selected {
        let (start, _) = cursor_cell(cells, start);
        let (end, width) = cursor_cell(cells, end);
        painter = painter.fill_color(paint.selection).fill_rect(
            start as f32 * grid.cell_width,
            0.0,
            (end + width - start) as f32 * grid.cell_width,
            grid.row_height,
        );
    }
    let cursor = paint.cursor.map(|col| cursor_cell(cells, col));
    if let Some((col, width)) = cursor.filter(|_| paint.cursor_style == CursorStyle::Block) {
        let (x, y, w, h) = cursor_bounds(grid, col, width, paint.cursor_style);
        painter = painter.fill_color(paint.cursor_color).fill_rect(x, y, w, h);
    }
    for cell in cells {
        // Hidden glyphs and their decorations stay hidden under selection/cursor.
        if cell.style.hidden
            || (cell.text.trim().is_empty() && !cell.style.underline && !cell.style.strikethrough)
        {
            continue;
        }
        let (foreground, background) = resolve(cell.style);
        let foreground = if paint.cursor_style == CursorStyle::Block
            && cursor.is_some_and(|(col, _)| col == cell.column)
        {
            background.unwrap_or(paint.background)
        } else {
            foreground.unwrap_or(paint.foreground)
        };
        let x = cell.column as f32 * grid.cell_width;
        let width = cell.width as f32 * grid.cell_width;
        painter = painter
            .save()
            .begin_path()
            .rect(Bounds::new(
                Point::new(px(x), px(0.0)),
                Size::new(px(width), px(grid.row_height)),
            ))
            .clip()
            .fill_color(foreground)
            .global_alpha(if cell.style.dim { 0.6 } else { 1.0 });
        if !cell.text.trim().is_empty() {
            painter = painter
                .op(CanvasOp::Font(fonts.font(cell.style).clone()))
                .fill_text_max_width(&cell.text, x, grid.baseline, px(width));
        }
        let thickness = (grid.row_height / 16.0).round().max(1.0);
        if cell.style.underline {
            painter = painter.fill_rect(
                x,
                (grid.baseline + thickness).min(grid.row_height - thickness),
                width,
                thickness,
            );
        }
        if cell.style.strikethrough {
            painter = painter.fill_rect(x, grid.baseline * 0.65, width, thickness);
        }
        painter = painter.restore();
    }
    if paint.cursor_style != CursorStyle::Block
        && let Some((col, width)) = cursor
    {
        let (x, y, w, h) = cursor_bounds(grid, col, width, paint.cursor_style);
        painter = painter
            .global_alpha(1.0)
            .fill_color(paint.cursor_color)
            .fill_rect(x, y, w, h);
    }
    painter
}

fn cursor_bounds(
    grid: Grid,
    column: usize,
    width: usize,
    style: CursorStyle,
) -> (f32, f32, f32, f32) {
    let x = column as f32 * grid.cell_width;
    let width = width as f32 * grid.cell_width;
    let stroke = (grid.row_height / 12.0).round().max(2.0);
    match style {
        CursorStyle::Block => (x, 0.0, width, grid.row_height),
        CursorStyle::Beam => (x, 0.0, stroke.min(width), grid.row_height),
        CursorStyle::Underline => (x, grid.row_height - stroke, width, stroke),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::{canvas::CanvasOp, color::rgb};

    fn draw(
        cells: &[GridCell],
        fonts: &Fonts,
        paint: Paint,
        resolve: impl Fn(TerminalStyle) -> (Option<Hsla>, Option<Hsla>),
    ) -> Painter {
        let mut painter = backgrounds(cells, fonts.grid, |style| resolve(style).1);
        for op in foreground(cells, fonts, paint, resolve).finish().ops {
            painter = painter.op(op);
        }
        painter
    }

    fn fonts() -> Fonts {
        Fonts {
            variants: [
                "14px monospace",
                "bold 14px monospace",
                "italic 14px monospace",
                "italic bold 14px monospace",
            ]
            .map(|font| CanvasFont::parse(font).unwrap()),
            grid: Grid {
                cell_width: 8.0,
                row_height: 20.0,
                baseline: 15.0,
            },
        }
    }

    fn paint() -> Paint {
        Paint {
            foreground: rgb(0xffffff).into(),
            background: rgb(0).into(),
            selection: rgb(0x333333).into(),
            cursor_color: rgb(0xffffff).into(),
            selected: None,
            cursor: None,
            cursor_style: CursorStyle::Block,
        }
    }

    #[test]
    fn native_wide_grapheme_and_styles_reach_the_canvas_without_text_controls() {
        let mut style = TerminalStyle::default();
        style.bold = true;
        style.italic = true;
        style.underline = true;
        style.strikethrough = true;
        let cells = [GridCell {
            column: 2,
            width: 2,
            text: "猫\u{301}".into(),
            style,
        }];
        let drawing = draw(&cells, &fonts(), paint(), |_| (None, None)).finish();
        assert!(
            drawing
                .ops
                .iter()
                .any(|op| matches!(op, CanvasOp::Font(font) if font.weight == 700 && font.italic))
        );
        assert!(drawing.ops.iter().any(|op| matches!(op, CanvasOp::FillText {text, x, y, max_width} if text.as_ref() == "猫\u{301}" && x.get(|_|0.0) == 16.0 && y.get(|_|0.0) == 15.0 && *max_width == Some(px(16.0)))));
        assert_eq!(
            drawing
                .ops
                .iter()
                .filter(|op| matches!(op, CanvasOp::FillRect(_)))
                .count(),
            2
        );
    }

    #[test]
    fn hidden_text_has_background_but_no_glyph_or_decoration() {
        let mut style = TerminalStyle::default();
        style.hidden = true;
        style.underline = true;
        let cells = [GridCell {
            column: 0,
            width: 1,
            text: "secret".into(),
            style,
        }];
        let drawing = draw(&cells, &fonts(), paint(), |_| {
            (None, Some(rgb(0x333333).into()))
        })
        .finish();
        assert!(
            !drawing
                .ops
                .iter()
                .any(|op| matches!(op, CanvasOp::FillText { .. }))
        );
        assert_eq!(
            drawing
                .ops
                .iter()
                .filter(|op| matches!(op, CanvasOp::FillRect(_)))
                .count(),
            1
        );
    }

    #[test]
    fn every_cursor_shape_on_a_wide_tail_uses_the_head_and_complete_width() {
        let cells = [GridCell {
            column: 2,
            width: 2,
            text: "猫".into(),
            style: TerminalStyle::default(),
        }];
        assert_eq!(cursor_cell(&cells, 3), (2, 2));
        let grid = fonts().grid;
        assert_eq!(
            cursor_bounds(grid, 2, 2, CursorStyle::Block),
            (16.0, 0.0, 16.0, 20.0)
        );
        assert_eq!(
            cursor_bounds(grid, 2, 2, CursorStyle::Beam),
            (16.0, 0.0, 2.0, 20.0)
        );
        assert_eq!(
            cursor_bounds(grid, 2, 2, CursorStyle::Underline),
            (16.0, 18.0, 16.0, 2.0)
        );
        for style in [
            CursorStyle::Block,
            CursorStyle::Beam,
            CursorStyle::Underline,
        ] {
            let mut paint = paint();
            paint.cursor = Some(3);
            paint.cursor_style = style;
            let drawing = draw(&cells, &fonts(), paint, |_| (None, None)).finish();
            let rect = drawing
                .ops
                .iter()
                .find_map(|op| match op {
                    CanvasOp::FillRect(rect) => Some(rect.map(|n| n.get(|_| 0.0))),
                    _ => None,
                })
                .expect("native cursor rectangle");
            let (x, y, width, height) = cursor_bounds(grid, 2, 2, style);
            assert_eq!(rect, [x, y, width, height], "native cursor {style:?}");
        }
    }
}
