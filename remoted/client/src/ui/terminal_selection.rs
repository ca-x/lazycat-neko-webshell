//! Grid selection retains complete native graphemes, including wide-cell tails.

use super::{Buffer, geometry::Grid};
use crate::terminal::GridCell;
use nsh_core::{MouseButton, PointerType, TouchEvent, TouchPhase, prelude::*};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Cell {
    pub row: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Selection {
    anchor: Cell,
    head: Cell,
    pub active: bool,
    pub dragging: bool,
}

impl Selection {
    fn same_range(self, other: Self) -> bool {
        self.anchor == other.anchor && self.head == other.head && self.active == other.active
    }
    pub fn begin(&mut self, cell: Cell) {
        *self = Self {
            anchor: cell,
            head: cell,
            dragging: true,
            active: false,
        };
    }

    pub fn extend(&mut self, cell: Cell) {
        if self.dragging {
            self.head = cell;
            self.active |= self.head != self.anchor;
        }
    }

    pub fn word(&mut self, row: usize, col: usize, cells: &[GridCell]) {
        let Some(index) = cells
            .iter()
            .position(|cell| cell.column <= col && col < cell.column + cell.width)
        else {
            return;
        };
        let class = |cell: &GridCell| {
            if cell.text.chars().all(char::is_whitespace) {
                0
            } else if cell.text.chars().any(|c| c.is_alphanumeric() || c == '_') {
                1
            } else {
                2
            }
        };
        let kind = class(&cells[index]);
        let mut start = index;
        let mut end = index;
        while start > 0 && class(&cells[start - 1]) == kind {
            start -= 1;
        }
        while end + 1 < cells.len() && class(&cells[end + 1]) == kind {
            end += 1;
        }
        *self = Self {
            anchor: Cell {
                row,
                col: cells[start].column,
            },
            head: Cell {
                row,
                col: cells[end].column + cells[end].width.saturating_sub(1),
            },
            active: true,
            dragging: true,
        };
    }

    pub fn columns(self, row: usize, cols: usize) -> Option<(usize, usize)> {
        if !self.active || cols == 0 {
            return None;
        }
        let (first, last) = (self.anchor.min(self.head), self.anchor.max(self.head));
        if row < first.row || row > last.row {
            return None;
        }
        Some((
            if row == first.row {
                first.col.min(cols - 1)
            } else {
                0
            },
            if row == last.row {
                last.col.min(cols - 1)
            } else {
                cols - 1
            },
        ))
    }

    pub fn text(
        self,
        count: usize,
        cols: usize,
        mut cells: impl FnMut(usize) -> Vec<GridCell>,
    ) -> String {
        if !self.active || count == 0 {
            return String::new();
        }
        let first = self.anchor.min(self.head).row.min(count - 1);
        let last = self.anchor.max(self.head).row.min(count - 1);
        let mut text = String::new();
        for row in first..=last {
            if row != first {
                text.push('\n');
            }
            let Some((start, end)) = self.columns(row, cols) else {
                continue;
            };
            let line_start = text.len();
            for cell in cells(row) {
                if intersects(&cell, start, end) {
                    text.push_str(&cell.text);
                }
            }
            // Strip grid padding, preserving selected internal spaces and blank rows.
            let trimmed = text[line_start..].trim_end_matches(' ').len();
            text.truncate(line_start + trimmed);
        }
        text
    }
}

pub(super) fn intersects(cell: &GridCell, start: usize, end: usize) -> bool {
    cell.column <= end && cell.column + cell.width > start
}

/// Only viewport offsets are mutable; all hit tests use the painted grid.
#[derive(Clone, Copy)]
pub(super) struct Interaction {
    pub selected: Signal<Selection>,
    verified: Signal<Option<(Selection, String)>>,
    pub offset_x: Signal<f32>,
    pub offset_y: Signal<f32>,
    // The native event's local/page relationship stays valid during a captured drag.
    pub origin: Signal<Point<Points>>,
    pub grid: Memo<Grid>,
}

impl Interaction {
    pub fn new(grid: Memo<Grid>) -> Self {
        Self {
            selected: signal(Selection::default()),
            verified: signal(None),
            offset_x: signal(0.0),
            offset_y: signal(0.0),
            origin: signal(Point::default()),
            grid,
        }
    }

    /// Keep an existing selection only while those same source cells still contain its text.
    pub fn observe(self, buffer: Buffer) {
        effect(move || {
            buffer.revision.get();
            let selected = self.selected.get();
            if !selected.active {
                self.verified.set(None);
                return;
            }
            let text = selected.text(buffer.line_count(), buffer.size.get().cols.into(), |row| {
                buffer.cells(row)
            });
            let replaced = self.verified.with_untracked(|previous| {
                previous
                    .as_ref()
                    .is_some_and(|(range, before)| range.same_range(selected) && before != &text)
            });
            if replaced {
                self.verified.set(None);
                self.selected.set(Selection::default());
            } else {
                self.verified.set(Some((selected, text)));
            }
        });
    }

    pub fn hit(self, buffer: &Buffer, point: Point<Points>) -> Cell {
        let (row, col) = self.grid.get_untracked().hit(
            point.x.0 + self.offset_x.get_untracked(),
            point.y.0 + self.offset_y.get_untracked(),
            buffer.size.get_untracked().cols.into(),
            buffer.line_count(),
        );
        Cell { row, col }
    }

    pub fn copy(self, buffer: &Buffer) {
        let selected = self.selected.get_untracked();
        let text = selected.text(
            buffer.line_count(),
            buffer.size.get_untracked().cols.into(),
            |row| buffer.cells(row),
        );
        // An output event may precede the reactive validation in this turn.
        if self.verified.with_untracked(|previous| {
            previous
                .as_ref()
                .is_some_and(|(range, before)| range.same_range(selected) && before != &text)
        }) {
            self.selected.set(Selection::default());
            self.verified.set(None);
            return;
        }
        if !text.is_empty() {
            set_clipboard(text);
        }
    }

    pub fn wrap(self, buffer: Buffer, content: impl IntoElement) -> impl IntoElement {
        let down = buffer.clone();
        let held = buffer.clone();
        let moved = buffer.clone();
        let touch = buffer.clone();
        view()
            .flex_1()
            .min_h(px(0.0))
            .test_id("terminal-selection")
            .on_press_down(move |event: &PressEvent| {
                self.origin.set(event.page_position - event.position);
                if !is_desktop() {
                    return;
                }
                let cell = self.hit(&down, event.position);
                self.selected.update(|selected| {
                    if event.clicks >= 2 {
                        selected.word(cell.row, cell.col, &down.cells(cell.row));
                    } else if event.modifiers.shift && selected.active {
                        selected.dragging = true;
                        selected.extend(cell);
                    } else {
                        selected.begin(cell);
                    }
                });
            })
            .on_long_press(move |event: &PressEvent| {
                if is_desktop() {
                    return;
                }
                self.origin.set(event.page_position - event.position);
                let cell = self.hit(&held, event.position);
                self.selected
                    .update(|selected| selected.word(cell.row, cell.col, &held.cells(cell.row)));
            })
            .on_mouse_move(move |event: &MouseEvent| {
                if event.pressed_button == Some(MouseButton::Left) {
                    let cell = self.hit(&moved, event.position);
                    self.selected.update(|selected| selected.extend(cell));
                }
            })
            // PressUp also means "click cancelled after moving past slop".
            // Selection ends on the physical release, not that cancellation.
            .on_mouse_up(move |_: &MouseEvent| {
                self.selected.update(|selected| selected.dragging = false)
            })
            .on_touch(move |event: &TouchEvent| {
                let Some(point) = event
                    .points
                    .first()
                    .filter(|point| point.pointer_type != PointerType::Mouse)
                else {
                    return;
                };
                match event.phase {
                    TouchPhase::Move if self.selected.get_untracked().dragging => {
                        let cell = self.hit(&touch, point.page - self.origin.get_untracked());
                        self.selected.update(|selected| selected.extend(cell));
                    }
                    TouchPhase::End | TouchPhase::Cancel => {
                        self.selected.update(|selected| selected.dragging = false)
                    }
                    _ => {}
                }
            })
            .claim_touches(TouchClaim::on_move().ask(move |request: &ClaimRequest| {
                if self.selected.get_untracked().dragging {
                    request.claim();
                }
            }))
            .keep_touches(true)
            .on_responder_move(move |event: &ResponderEvent| {
                let cell = self.hit(&buffer, event.page - self.origin.get_untracked());
                self.selected.update(|selected| selected.extend(cell));
            })
            .on_responder_release(move |_: &ResponderEvent| {
                self.selected.update(|selected| selected.dragging = false)
            })
            .on_responder_terminate(move |_: &ResponderEvent| {
                self.selected.update(|selected| selected.dragging = false)
            })
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::{TerminalEmulator, TerminalSize};

    #[test]
    fn dragging_in_either_direction_copies_complete_graphemes_and_empty_rows() {
        let mut vt = crate::terminal::GhosttyEmulator::new(TerminalSize::new(24, 6), 0).unwrap();
        vt.feed("x猫e\u{301}\r\n\r\n\r\nend".as_bytes());
        for (start, end) in [
            (Cell { row: 0, col: 2 }, Cell { row: 3, col: 1 }),
            (Cell { row: 3, col: 1 }, Cell { row: 0, col: 2 }),
        ] {
            let mut selection = Selection::default();
            selection.begin(start);
            selection.extend(end);
            assert_eq!(
                selection.text(6, 24, |row| vt.cells(row)),
                "猫e\u{301}\n\n\nen"
            );
        }
    }

    #[test]
    fn long_press_on_wide_tail_selects_the_word_and_short_click_does_not() {
        let mut vt = crate::terminal::GhosttyEmulator::new(TerminalSize::new(24, 2), 0).unwrap();
        vt.feed("a 猫e\u{301}!".as_bytes());
        let mut selection = Selection::default();
        selection.begin(Cell { row: 0, col: 3 });
        assert!(!selection.active);
        selection.word(0, 3, &vt.cells(0));
        assert_eq!(selection.text(2, 24, |row| vt.cells(row)), "猫e\u{301}");
    }
}
