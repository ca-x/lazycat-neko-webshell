//! Selectable read-only terminal output that preserves the source terminal geometry.

use super::{terminal_colors::colors, terminal_surface::Buffer};
use crate::preferences::{current_context, tr};
use nsh_base::{TerminalRun, theme};
use nsh_core::prelude::*;
use unicode_width::UnicodeWidthChar;

/// Render and scroll output without resizing the local buffer or requesting a remote resize.
pub fn render(buffer: Buffer, _size: Signal<(u16, u16)>) -> impl IntoElement {
    let appearance = current_context();
    let viewport_width = signal(px(0.0));
    let family = move || {
        theme()
            .font_mono
            .clone()
            .unwrap_or_else(|| "monospace".into())
    };
    let cell_width = computed(move || {
        let family = family();
        let font_size = appearance.font_size();
        let css = if matches!(family.as_ref(), "monospace" | "ui-monospace") {
            format!("{font_size}px {family}")
        } else {
            format!("{font_size}px \"{}\"", family.replace('"', ""))
        };
        measure_text(&css, "0000000000") / 10.0
    });
    let count = buffer.clone();
    let rows = buffer.clone();
    let geometry = buffer.clone();
    let height = buffer.clone();
    let content_width = computed(move || {
        px((geometry.size.get().cols as f32 * cell_width.get().0).max(viewport_width.get().0))
    });
    let list = virtual_list(
        move || count.line_count(),
        move |position| row(rows.clone(), position, cell_width),
    )
    .test_id("terminal-observer")
    .label(move || tr("远程终端输出（只读，可选择和复制）"))
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
            if bounds.size.width.0 > 0.0 {
                viewport_width.set(bounds.size.width);
            }
        })
        .child(
            scroll()
                .horizontal()
                .test_id("terminal-observer-horizontal")
                .flex_1()
                .min_h(px(0.0))
                .child(list),
        )
}

fn row(buffer: Buffer, position: Position, width: Memo<Points>) -> impl IntoElement {
    let appearance = current_context();
    let lines = buffer.clone();
    let line = computed(move || appearance.terminal_line(lines.line(position.index())));
    let backgrounds = line;
    let cursor = computed(move || {
        let cursor = buffer.cursor()?;
        let first = buffer
            .line_count()
            .saturating_sub(buffer.size.get().rows as usize);
        (position.index() == first + cursor.row as usize).then_some(cursor.col)
    });
    view()
        .h(move || px(appearance.line_height()))
        .justify_center()
        .child(dynamic(move || {
            let mut column = 0;
            let mut fills = Vec::new();
            for run in backgrounds.get().runs {
                let cells: usize = run
                    .text
                    .chars()
                    .map(|character| character.width().unwrap_or(0))
                    .sum();
                if let (_, Some(background)) = colors(run.style) {
                    fills.push(
                        view()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(Length::from(width.get() * column as f32))
                            .w(Length::from(width.get() * cells as f32))
                            .bg(background)
                            .into_element(),
                    );
                }
                column += cells;
            }
            view()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .pointer_events(PointerEvents::None)
                .children(fills)
        }))
        .child(dynamic(move || {
            text("")
                .select_text()
                .selection_color(move || appearance.selection())
                .children(
                    line.get()
                        .runs
                        .into_iter()
                        .map(run_view)
                        .collect::<Vec<_>>(),
                )
        }))
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

fn run_view(run: TerminalRun) -> AnyElement {
    let style = run.style;
    let mut span = text(run.text);
    if let (Some(foreground), _) = colors(style) {
        span = span.text_color(foreground);
    }
    if style.bold {
        span = span.font_weight(FontWeight::BOLD);
    }
    if style.italic {
        span = span.italic();
    }
    if style.underline {
        span = span.underline();
    }
    if style.strikethrough {
        span = span.line_through();
    }
    if style.dim || style.hidden {
        span = span.opacity(if style.hidden { 0.0 } else { 0.6 });
    }
    span.into_element()
}
