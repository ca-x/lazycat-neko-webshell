//! Observe taps without claiming the native selection/scroll gesture.
use nsh_core::{TouchEvent, TouchPhase, prelude::*};
use std::cell::Cell;

#[derive(Clone, Copy)]
struct Tap {
    id: u32,
    x: f32,
    y: f32,
    started: f64,
}

pub fn focus_on_tap(reference: ElementRef, enabled: Memo<bool>) -> impl Fn(&TouchEvent) {
    let tap = Cell::new(None::<Tap>);
    move |event| {
        if !enabled.get_untracked() || event.points.len() != 1 {
            tap.set(None);
            return;
        }
        let point = &event.points[0];
        match event.phase {
            TouchPhase::Start => tap.set(Some(Tap {
                id: point.id,
                x: point.page.x.0,
                y: point.page.y.0,
                started: event.timestamp,
            })),
            TouchPhase::Move | TouchPhase::End => {
                let Some(start) = tap.get() else {
                    return;
                };
                let elapsed = event.timestamp - start.started;
                let moved = (point.page.x.0 - start.x).hypot(point.page.y.0 - start.y);
                if point.id != start.id || moved > 6.0 || !(0.0..250.0).contains(&elapsed) {
                    tap.set(None);
                } else if event.phase == TouchPhase::End {
                    tap.set(None);
                    reference.focus();
                }
            }
            TouchPhase::Cancel => tap.set(None),
        }
    }
}
