//! A selectable fixed grid for the shared VT buffer and its source geometry.

use super::{terminal_colors::colors, terminal_surface::Buffer};
use crate::preferences::{current_context, tr};
use crate::terminal::GridCell;
use nsh_core::prelude::*;

/// Observers preserve source geometry. Only the active controller may fit a PTY to its viewport.
pub fn render(
    buffer: Buffer,
    size: Signal<Option<(u16, u16)>>,
    resize: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    let appearance = current_context();
    let viewport = signal((px(0.0), px(0.0)));
    let family = crate::fonts::terminal_family;
    let cell_width = computed(move || {
        let family = family();
        let font_size = appearance.font_size();
        let css = format!("{font_size}px {family}");
        measure_text(&css, "0000000000") / 10.0
    });
    let layout_buffer = buffer.clone();
    effect(move || {
        let (width, height) = viewport.get();
        if !resize() || width.0 <= 1.0 || height.0 <= 1.0 {
            return;
        }
        let cell = cell_width.get().0;
        if cell <= 0.0 {
            return;
        }
        let cols = (width.0 / cell).floor().clamp(2.0, 1000.0) as u16;
        let rows = (height.0 / appearance.line_height())
            .floor()
            .clamp(2.0, 1000.0) as u16;
        layout_buffer.resize(nsh_base::TerminalSize::new(cols, rows));
        size.set(Some((cols, rows)));
    });
    let count = buffer.clone();
    let rows = buffer.clone();
    let geometry = buffer.clone();
    let height = buffer.clone();
    let failures = buffer.clone();
    let failure = computed(move || failures.failure());
    let content_width = computed(move || {
        px((geometry.size.get().cols as f32 * cell_width.get().0).max(viewport.get().0.0))
    });
    let list = virtual_list(
        move || count.line_count(),
        move |position| row(rows.clone(), position, cell_width, content_width),
    )
    .test_id("terminal-observer")
    .label(move || tr("终端输出，可选择和复制"))
    .alignment(ListAlignment::Bottom)
    .follow_end(move || px(appearance.line_height()))
    .estimated_item_size(move || px(appearance.line_height()))
    .selectable(true)
    // This is the horizontal scroll's child: a zero flex basis collapses the
    // initial width and leaves GTK list cells without layout. Keep source columns.
    .w(move || Length::from(content_width.get()))
    .flex_shrink(0.0)
    // Bottom alignment follows real scrollback; capping the viewport at the
    // content height keeps a short fixed screen at the top of a taller window.
    .max_h(move || px(height.line_count() as f32 * appearance.line_height()))
    .min_h(px(0.0))
    .overflow_hidden()
    .font_family(family)
    .text_size(move || px(appearance.font_size() as f32))
    .text_color(move || appearance.foreground())
    .bg(move || appearance.background());
    view()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .bg(move || appearance.background())
        .on_layout(move |bounds: &Bounds<Points>| {
            if bounds.size.width.0 > 0.0 && bounds.size.height.0 > 0.0 {
                viewport.set((bounds.size.width, bounds.size.height));
            }
        })
        .child(when(
            move || failure.get().is_some(),
            move || {
                text(move || {
                    failure
                        .get()
                        .map(|failure| failure.to_string())
                        .unwrap_or_default()
                })
                .select_text()
                .role(AccessibilityRole::Alert)
                .text_color(move || nsh_base::theme().destructive)
            },
        ))
        .child(
            scroll()
                .horizontal()
                .test_id("terminal-observer-horizontal")
                .flex_1()
                .min_h(px(0.0))
                .child(list),
        )
}

fn row(
    buffer: Buffer,
    position: Position,
    width: Memo<Points>,
    content_width: Memo<Points>,
) -> impl IntoElement {
    let appearance = current_context();
    let lines = buffer.clone();
    let cells = computed(move || visible_cells(lines.cells(position.index())));
    let cursor = computed(move || {
        let cursor = buffer.cursor()?;
        let first = buffer
            .line_count()
            .saturating_sub(buffer.size.get().rows as usize);
        (position.index() == first + cursor.row as usize).then_some(cursor.col)
    });
    view()
        .w(move || Length::from(content_width.get()))
        .h(move || px(appearance.line_height()))
        .flex_shrink(0.0)
        .justify_center()
        .test_id("terminal-grid-row")
        .label(move || {
            cells
                .get()
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .child(each(
            move || cells.get(),
            |cell| cell.column,
            move |cell, _| cell_view(cell, width),
        ))
        .child(when(
            move || cursor.get().is_some(),
            move || {
                view()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(move || Length::from(width.get() * cursor.get().unwrap_or(0) as f32))
                    .w(move || Length::from(width.get()))
                    .test_id("terminal-observer-cursor")
                    .bg(move || appearance.cursor())
                    .opacity(0.25)
                    .pointer_events(PointerEvents::None)
            },
        ))
}

/// Omit unpainted trailing slots, while keeping internal spaces and all styled cells.
/// Every remaining text is a complete native grapheme, never a Rust `char` fragment.
fn visible_cells(mut cells: Vec<GridCell>) -> Vec<GridCell> {
    let end = cells
        .iter()
        .rposition(|cell| cell.text != " " || cell.style != nsh_base::TerminalStyle::default())
        .map_or(0, |index| index + 1);
    if end == 0 {
        // Empty lines still participate in native selection. An empty text
        // preserves the line break without copying padding spaces.
        cells.truncate(1);
        if let Some(cell) = cells.first_mut() {
            cell.text.clear();
        }
    } else {
        cells.truncate(end);
    }
    cells
}

fn cell_view(cell: ReadSignal<GridCell>, width: Memo<Points>) -> AnyElement {
    let appearance = current_context();
    let glyph = text(move || cell.with(|cell| cell.text.clone()))
        .continues_text(move || cell.with(|cell| cell.column != 0))
        .font_family(mono)
        .text_size(move || px(appearance.font_size() as f32))
        .text_left()
        .writing_direction(WritingDirection::Ltr)
        .number_of_lines(1u32)
        .text_overflow(nsh_core::EllipsizeMode::Clip)
        .w(move || Length::from(width.get() * cell.with(|cell| cell.width) as f32))
        .select_text()
        .selection_color(move || appearance.selection())
        .text_color(move || {
            colors(cell.with(|cell| cell.style))
                .0
                .unwrap_or_else(|| appearance.foreground())
        })
        .font_weight(FontWeight::NORMAL)
        .style_when(
            move || cell.with(|cell| cell.style.bold),
            |style| style.font_weight(FontWeight::BOLD),
        )
        .style_when(
            move || cell.with(|cell| cell.style.italic),
            |style| style.italic(),
        )
        .style_when(
            move || cell.with(|cell| cell.style.underline),
            |style| style.underline(),
        )
        .style_when(
            move || cell.with(|cell| cell.style.strikethrough),
            |style| style.line_through(),
        )
        .opacity(move || {
            cell.with(|cell| {
                if cell.style.hidden {
                    0.0_f32
                } else if cell.style.dim {
                    0.6_f32
                } else {
                    1.0_f32
                }
            })
        });
    view()
        .absolute()
        .top_0()
        .bottom_0()
        .left(move || Length::from(width.get() * cell.with(|cell| cell.column) as f32))
        .w(move || Length::from(width.get() * cell.with(|cell| cell.width) as f32))
        .overflow_hidden()
        .justify_center()
        .bg(move || {
            colors(cell.with(|cell| cell.style))
                .1
                .unwrap_or_else(|| appearance.background().opacity(0.0))
        })
        .child(glyph)
        .into_element()
}

fn mono() -> SharedString {
    crate::fonts::terminal_family()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::{TerminalEmulator, TerminalSize};
    use nsh_core::selection::{SelectionModel, SelectionTexts};

    struct Selection(Vec<(usize, GridCell)>);
    impl SelectionTexts for Selection {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn content(&self, index: usize) -> &str {
            &self.0[index].1.text
        }
        fn frame(&self, index: usize) -> Bounds<Points> {
            let (row, cell) = &self.0[index];
            Bounds::new(
                Point::new(px(cell.column as f32 * 8.0), px(*row as f32 * 18.0)),
                Size::new(px(cell.width as f32 * 8.0), px(18.0)),
            )
        }
        fn offset_at(&self, _: usize, _: Point<Points>) -> u32 {
            0
        }
        fn continues(&self, index: usize) -> bool {
            self.0[index].1.column != 0
        }
    }

    #[test]
    fn native_selection_copies_cells_with_wide_graphemes_and_preserves_blank_lines() {
        for blank_lines in [1, 2, 4] {
            let source = format!("one猫e\u{301}{}two", "\r\n".repeat(blank_lines + 1));
            let expected = format!("one猫e\u{301}{}two", "\n".repeat(blank_lines + 1));
            let mut emulator =
                crate::terminal::GhosttyEmulator::new(TerminalSize::new(80, 10), 0).unwrap();
            emulator.feed(source.as_bytes());
            let texts = Selection(
                (0..blank_lines + 2)
                    .flat_map(|row| {
                        visible_cells(emulator.cells(row))
                            .into_iter()
                            .map(move |cell| (row, cell))
                    })
                    .collect(),
            );
            let mut selection = SelectionModel::default();
            assert!(selection.select_all(&texts));
            assert_eq!(selection.text(&texts), expected);
        }
    }
}
