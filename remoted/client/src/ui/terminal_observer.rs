//! A native viewport canvas observes Ghostty without answering VT probes.

use super::{terminal_colors::colors, terminal_surface::Buffer};
use crate::preferences::current_context;
use nsh_core::prelude::*;
use std::{rc::Rc, time::Duration};

#[path = "terminal_drawing.rs"]
mod drawing;
#[path = "terminal_geometry.rs"]
mod geometry;
#[path = "terminal_menu.rs"]
mod menu;
#[path = "terminal_selection.rs"]
mod selection;
#[path = "terminal_viewport.rs"]
mod viewport;

#[cfg(test)]
#[path = "terminal_observer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "terminal_test_support.rs"]
pub(crate) mod test_support;

use selection::Interaction;

/// Observe source geometry without terminal input actions (also used by fixtures).
#[cfg(test)]
pub fn render(
    buffer: Buffer,
    size: Signal<Option<(u16, u16)>>,
    resize: impl Fn() -> bool + 'static,
) -> impl IntoElement {
    render_interactive(buffer, size, resize, computed(|| false), |_| {})
}

/// Only the active controller may fit a PTY; paste remains owned by its input adapter.
pub fn render_interactive(
    buffer: Buffer,
    size: Signal<Option<(u16, u16)>>,
    resize: impl Fn() -> bool + 'static,
    writable: Memo<bool>,
    paste: impl Fn(String) + 'static,
) -> impl IntoElement {
    let appearance = current_context();
    let viewport = signal((0.0, 0.0));
    let fonts = computed(drawing::Fonts::measure);
    let grid = computed(move || fonts.with(|fonts| fonts.grid));
    let selection = Interaction::new(grid);
    selection.observe(buffer.clone());
    let layout = buffer.clone();
    effect(move || {
        let (width, height) = viewport.get();
        if resize()
            && let Some(fitted) = grid.get().fit(width, height)
        {
            layout.resize(fitted);
            size.set(Some((fitted.cols, fitted.rows)));
        }
    });
    let blink = signal(true);
    let updates = buffer.clone();
    effect(move || {
        updates.revision.get();
        blink.set(true);
        if appearance.cursor_blink() && !reduce_motion() {
            // One clock per terminal, disposed/restarted on output or preference changes.
            let task = set_interval(Duration::from_millis(600), move || {
                blink.update(|on| *on = !*on)
            });
            cleanup(move || task.cancel());
        }
    });
    let geometry = buffer.clone();
    let content_width = computed(move || {
        (f32::from(geometry.size.get().cols) * grid.get().cell_width)
            .max(viewport.get().0 - 2.0 * geometry::PADDING)
    });
    let height = buffer.clone();
    let viewport_height = computed(move || {
        (viewport.get().1 - 2.0 * geometry::PADDING)
            .max(0.0)
            .min(height.line_count() as f32 * grid.get().row_height)
    });
    let failures = buffer.clone();
    let failure = computed(move || failures.failure());
    let content_rows = viewport::render(
        buffer.clone(),
        fonts,
        content_width,
        viewport_height,
        selection,
        blink,
    );
    let content = scroll()
        .horizontal()
        .test_id("terminal-observer-horizontal")
        .flex_1()
        .min_h(px(0.0))
        .on_scroll(move |event: &ScrollEvent| selection.offset_x.set(event.offset.x.0))
        .child(content_rows);
    let content = selection.wrap(buffer.clone(), content);
    let copy = buffer.clone();
    view()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .bg(move || appearance.background())
        .on_layout(move |bounds: &Bounds<Points>| {
            viewport.set((bounds.size.width.0, bounds.size.height.0));
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
            view()
                .flex_1()
                .min_h(px(0.0))
                .p(px(geometry::PADDING))
                .child(menu::wrap(
                    buffer,
                    selection,
                    writable,
                    Rc::new(paste),
                    content,
                )),
        )
        .child(when(
            move || selection.selected.get().active,
            move || {
                view()
                    .test_id("terminal-selection-actions")
                    .absolute()
                    .top_2()
                    .right_2()
                    .flex_row()
                    .gap_1()
                    .child(super::controls::action("复制", false, || false, {
                        let copy = copy.clone();
                        move || selection.copy(&copy)
                    }))
                    .child(super::controls::action(
                        "取消",
                        false,
                        || false,
                        move || {
                            selection.selected.set(selection::Selection::default());
                        },
                    ))
            },
        ))
}
