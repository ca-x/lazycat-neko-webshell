//! Capture actual terminal canvas mount payloads for behavioral consumer tests.
use nsh_core::{
    MountItem, NodeId,
    canvas::{CanvasFont, CanvasOp, Drawing, Paint},
    prelude::*,
};
use nsh_testing::Headless;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
fn rgb(value: u32) -> nsh_core::Color {
    nsh_core::color::rgb(value).into()
}
// Capture the actual mount payload handed to the native Canvas, not inferred styles.
#[derive(Clone, Default)]
pub(crate) struct Canvases(Arc<Mutex<BTreeMap<NodeId, Arc<Drawing>>>>);

#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub color: nsh_core::Color,
    pub font: CanvasFont,
    pub x: f32,
    pub baseline: f32,
    pub width: Points,
}

impl Canvases {
    pub fn observe(app: &mut Headless) -> Self {
        let canvases = Self::default();
        let record = canvases.clone();
        app.observe(move |_, batch| {
            let mut rows = record.0.lock().expect("canvas payloads");
            for item in &batch.items {
                match item {
                    MountItem::Canvas { id, drawing } => {
                        rows.insert(*id, drawing.clone());
                    }
                    MountItem::Remove { id } => {
                        rows.remove(id);
                    }
                    _ => {}
                }
            }
        });
        canvases
    }

    pub fn rows(&self, app: &Headless) -> Vec<(Bounds<Points>, Arc<Drawing>)> {
        let drawings = self.0.lock().expect("canvas payloads");
        app.screen()
            .layout_nodes()
            .into_iter()
            .filter(|node| node.test_id.as_deref() == Some("terminal-grid-row"))
            .filter_map(|node| {
                drawings
                    .get(&node.id)
                    .map(|drawing| (node.bounds, drawing.clone()))
            })
            .collect()
    }

    pub fn glyph(&self, app: &Headless, wanted: &str) -> Glyph {
        for (bounds, drawing) in self.rows(app) {
            let mut current = (rgb(0), CanvasFont::default());
            let mut stack = Vec::new();
            for op in &drawing.ops {
                match op {
                    CanvasOp::Save => stack.push(current.clone()),
                    CanvasOp::Restore => {
                        current = stack.pop().expect("balanced native canvas state")
                    }
                    CanvasOp::FillStyle(Paint::Color(color)) => current.0 = *color,
                    CanvasOp::Font(font) => current.1 = font.clone(),
                    CanvasOp::FillText {
                        text,
                        x,
                        y,
                        max_width,
                    } if text.as_ref() == wanted => {
                        let glyph = Glyph {
                            color: current.0,
                            font: current.1.clone(),
                            x: x.get(|_| 0.0),
                            baseline: y.get(|_| 0.0),
                            width: max_width.expect("native cell width"),
                        };
                        assert!(
                            glyph.baseline > 0.0 && glyph.baseline < bounds.size.height.0,
                            "baseline inside row: {glyph:?}"
                        );
                        assert!(glyph.width.0 > 0.0);
                        return glyph;
                    }
                    _ => {}
                }
            }
        }
        panic!("no native canvas glyph {wanted:?}");
    }

    pub fn text(&self, app: &Headless) -> String {
        self.rows(app)
            .iter()
            .flat_map(|(_, drawing)| drawing.ops.iter())
            .filter_map(|op| {
                if let CanvasOp::FillText { text, .. } = op {
                    Some(text.as_ref())
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn has_fill(&self, app: &Headless, color: nsh_core::Color, min_width: f32) -> bool {
        self.rows(app).iter().any(|(_, drawing)| {
            let mut fill = rgb(0);
            drawing.ops.iter().any(|op| {
                match op {
                    CanvasOp::FillStyle(Paint::Color(color)) => fill = *color,
                    CanvasOp::FillRect(rect)
                        if fill == color
                            && rect[2].get(|_| 0.0) >= min_width
                            && rect[3].get(|_| 0.0) > 0.0 =>
                    {
                        return true;
                    }
                    _ => {}
                }
                false
            })
        })
    }
}
