//! Native terminal actions share the same selection and input permission gate.

use super::{
    Buffer,
    selection::{Cell, Interaction},
};
use crate::preferences::tr;
use nsh_base::{context_menu, menu_item, theme};
use nsh_core::prelude::*;
use std::rc::Rc;

pub(super) fn wrap(
    buffer: Buffer,
    selection: Interaction,
    writable: Memo<bool>,
    paste: Rc<dyn Fn(String)>,
    content: impl IntoElement,
) -> impl IntoElement {
    let copy = buffer.clone();
    let all = buffer.clone();
    context_menu()
        .flex_1()
        .min_h(px(0.0))
        // Touch selection uses hold-and-drag; its actions stay reachable
        // without opening a sheet over the finger being used to select text.
        .disabled(move || !is_desktop())
        .popup_class(
            "min-w-44 bg-popover text-popover-foreground border border-border rounded-md p-1",
        )
        .trigger(view().flex_1().min_h(px(0.0)).child(content))
        .child(item(
            "复制",
            "terminal-menu-copy",
            move || !selection.selected.get().active,
            move || selection.copy(&copy),
        ))
        .child(item(
            "粘贴",
            "terminal-menu-paste",
            move || !writable.get(),
            move || {
                if !writable.get_untracked() {
                    return;
                }
                let paste = paste.clone();
                spawn(async move {
                    let text = clipboard_string().await;
                    // Revalidate: permission can change while the clipboard request is pending.
                    if writable.get_untracked() && !text.is_empty() {
                        paste(text);
                    }
                });
            },
        ))
        .child(item(
            "全选",
            "terminal-menu-select-all",
            || false,
            move || {
                let count = all.line_count();
                let cols = all.size.get_untracked().cols;
                selection.selected.update(|selected| {
                    selected.begin(Cell { row: 0, col: 0 });
                    selected.extend(Cell {
                        row: count.saturating_sub(1),
                        col: usize::from(cols.saturating_sub(1)),
                    });
                    selected.active = count != 0;
                    selected.dragging = false;
                });
            },
        ))
}

fn item(
    label: &'static str,
    id: &'static str,
    disabled: impl Fn() -> bool + 'static,
    action: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .disabled(disabled)
        .on_select(action)
        .min_h(super::super::controls::control_size)
        .px_3()
        .py_1()
        .rounded(move || theme().radius)
        .hover(|s| s.bg(move || theme().accent))
        .focus(|s| {
            s.bg(move || theme().accent)
                .outline(px(1.0))
                .outline_color(move || theme().ring)
        })
        .disabled_style(|s| s.opacity(0.45))
        .child(text(move || tr(label)).test_id(id).text_sm())
}
