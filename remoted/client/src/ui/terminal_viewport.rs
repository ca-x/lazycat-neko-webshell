//! Native scroll physics with a bounded canvas for the visible Ghostty rows.

use super::{Buffer, colors, drawing, selection::Interaction};
use crate::preferences::{current_context, tr};
use crate::ui::kitty_image_layer;
use neko_ghostty::kitty::Layer;
use nsh_core::prelude::*;

pub(super) fn render(
    buffer: Buffer,
    fonts: Memo<drawing::Fonts>,
    width: Memo<f32>,
    height: Memo<f32>,
    selection: Interaction,
    blink: Signal<bool>,
) -> impl IntoElement {
    let appearance = current_context();
    let reference = element_ref();
    let following = signal(true);
    let extent_buffer = buffer.clone();
    let extent = computed(move || {
        extent_buffer.line_count() as f32 * fonts.with(|fonts| fonts.grid.row_height)
    });
    let live_buffer = buffer.clone();
    let target = computed(move || {
        fonts.with(|fonts| {
            fonts.grid.follow_offset(
                live_buffer.line_count(),
                live_buffer.size.get().rows.into(),
                live_buffer.cursor().map(|cursor| cursor.row.into()),
                height.get(),
            )
        })
    });
    effect(move || {
        let height = height.get();
        let end = (extent.get() - height).max(0.0);
        let destination = if following.get() { target.get() } else { end };
        if height > 0.0 && (following.get() || selection.offset_y.get_untracked() > end) {
            selection.offset_y.set(destination);
            reference.scroll_to_offset(px(destination), false);
        }
    });
    let range_buffer = buffer.clone();
    let range = computed(move || {
        fonts.with(|fonts| {
            fonts.grid.visible_rows(
                selection.offset_y.get(),
                height.get(),
                range_buffer.line_count(),
            )
        })
    });
    let rows_buffer = buffer.clone();
    let rows = computed(move || {
        range
            .get()
            .map(|row| (row, rows_buffer.cells(row)))
            .collect::<Vec<_>>()
    });
    let image_buffer = buffer.clone();
    let images = computed(move || {
        let grid = fonts.with(|fonts| fonts.grid);
        image_buffer.images(
            range.get(),
            grid.cell_width.round().max(1.0) as u32,
            grid.row_height.round().max(1.0) as u32,
        )
    });
    let glyphs = canvas(move || {
        let grid = fonts.with(|fonts| fonts.grid);
        let cursor = buffer.cursor().filter(|_| blink.get()).map(|cursor| {
            (
                buffer
                    .line_count()
                    .saturating_sub(buffer.size.get().rows.into())
                    + usize::from(cursor.row),
                usize::from(cursor.col),
            )
        });
        let first = range.get().start;
        let canvas_height = range.get().len() as f32 * grid.row_height;
        let mut painter = Painter::new()
            .save()
            .begin_path()
            .rect(Bounds::new(
                Point::default(),
                Size::new(px(width.get()), px(canvas_height)),
            ))
            .clip();
        rows.with(|rows| {
            fonts.with(|fonts| {
                images.with(|images| {
                    // Images cross cell boundaries. Painting each placement once avoids
                    // antialiased seams at fractional device-pixel row boundaries.
                    for layer in [Layer::BelowBackground, Layer::BelowText, Layer::AboveText] {
                        painter = kitty_image_layer::draw(
                            std::mem::take(&mut painter),
                            images,
                            layer,
                            first,
                            grid.cell_width,
                            grid.row_height,
                            canvas_height,
                        );
                        if layer == Layer::AboveText {
                            continue;
                        }
                        for (row, cells) in rows {
                            let drawing = if layer == Layer::BelowBackground {
                                drawing::backgrounds(cells, grid, |style| {
                                    colors(style).1.map(color)
                                })
                            } else {
                                let paint = drawing::Paint {
                                    foreground: color(appearance.foreground()),
                                    background: color(appearance.background()),
                                    selection: appearance.selection(),
                                    cursor_color: color(appearance.cursor()),
                                    selected: selection
                                        .selected
                                        .get()
                                        .columns(*row, usize::from(buffer.size.get().cols)),
                                    cursor: cursor
                                        .filter(|(cursor_row, _)| cursor_row == row)
                                        .map(|(_, col)| col),
                                    cursor_style: appearance.cursor_style(),
                                };
                                drawing::foreground(cells, fonts, paint, |style| {
                                    let (fg, bg) = colors(style);
                                    (fg.map(color), bg.map(color))
                                })
                            };
                            painter = std::mem::take(&mut painter)
                                .save()
                                .translate(0.0, (row - first) as f32 * grid.row_height)
                                .begin_path()
                                .rect(Bounds::new(
                                    Point::default(),
                                    Size::new(px(width.get()), px(grid.row_height)),
                                ))
                                .clip();
                            for op in drawing.finish().ops {
                                painter = std::mem::take(&mut painter).op(op);
                            }
                            painter = std::mem::take(&mut painter).restore();
                        }
                    }
                })
            })
        });
        painter.restore()
    })
    // Retain the existing semantic target for selection/input consumers.
    .test_id("terminal-grid-row")
    .label(move || {
        rows.with(|rows| {
            rows.iter()
                .map(|(_, cells)| {
                    cells
                        .iter()
                        .map(|cell| cell.text.as_str())
                        .collect::<String>()
                        .trim_end()
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_owned()
        })
    })
    .absolute()
    .top(move || px(range.get().start as f32 * fonts.with(|fonts| fonts.grid.row_height)))
    .w(move || px(width.get()))
    .h(move || px(range.get().len() as f32 * fonts.with(|fonts| fonts.grid.row_height)))
    .overflow_hidden();
    scroll()
        .element_ref(&reference)
        .test_id("terminal-observer")
        .label(move || tr("终端输出，可选择和复制"))
        .w(move || px(width.get()))
        .h(move || px(height.get()))
        .min_h(px(0.0))
        .flex_shrink(0.0)
        .on_scroll(move |event: &ScrollEvent| {
            let end = (extent.get_untracked() - height.get_untracked()).max(0.0);
            let offset = event.offset.y.0.clamp(0.0, end);
            selection.offset_y.set(offset);
            following.set(
                (target.get_untracked() - offset).abs()
                    <= fonts.with(|fonts| fonts.grid.row_height),
            );
        })
        .on_layout(move |_: &Bounds<Points>| {
            if following.get_untracked() && height.get_untracked() > 0.0 {
                reference.scroll_to_offset(px(target.get_untracked()), false);
            }
        })
        .child(
            view()
                .w(move || px(width.get()))
                .h(move || px(extent.get()))
                .flex_shrink(0.0)
                .child(glyphs),
        )
}
