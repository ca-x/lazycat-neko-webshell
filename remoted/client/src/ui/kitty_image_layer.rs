//! Native Kitty image compositing for row canvases, including fractional cell metrics.

use neko_ghostty::kitty::{Layer, Placement};
use nsh_core::{
    canvas::{CanvasImage, CanvasOp, Num},
    geometry::ScaledPixels,
    prelude::*,
};

/// Paint one native image layer in the canvas of a global scrollback row.
pub(super) fn draw(
    mut painter: Painter,
    placements: &[Placement],
    layer: Layer,
    row: usize,
    cell_width: f32,
    row_height: f32,
) -> Painter {
    if !cell_width.is_finite() || !row_height.is_finite() || cell_width <= 0.0 || row_height <= 0.0
    {
        return painter;
    }
    for placement in placements
        .iter()
        .filter(|placement| placement.layer == layer)
    {
        if placement.cell_width == 0
            || placement.cell_height == 0
            || placement.width == 0
            || placement.height == 0
        {
            continue;
        }
        let sx = cell_width / placement.cell_width as f32;
        let sy = row_height / placement.cell_height as f32;
        let x = placement.x as f32 * sx;
        let y = placement.y as f32 * sy - row as f32 * row_height;
        let width = placement.width as f32 * sx;
        let height = placement.height as f32 * sy;
        if !x.is_finite()
            || !y.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || y >= row_height
            || y + height <= 0.0
        {
            continue;
        }
        let [source_x, source_y, source_width, source_height] = placement.source;
        let source = Bounds::new(
            Point::new(ScaledPixels(source_x as f32), ScaledPixels(source_y as f32)),
            Size::new(
                ScaledPixels(source_width as f32),
                ScaledPixels(source_height as f32),
            ),
        );
        painter = painter.op(CanvasOp::DrawImage {
            image: CanvasImage {
                uri: placement.image.data_uri.clone(),
            },
            source: Some(source),
            dest: [x, y, width, height].map(Num::Const),
        });
    }
    painter
}

#[cfg(test)]
mod tests {
    use super::*;
    use neko_ghostty::kitty::Image;
    use std::sync::Arc;

    #[test]
    fn fractional_cells_crop_layers_and_global_rows_reach_the_native_canvas() {
        let image = Arc::new(Image {
            id: 1,
            generation: 1,
            width: 2,
            height: 2,
            rgba: vec![255; 16].into(),
            data_uri: "data:image/png;base64,test".into(),
        });
        let placement = Placement {
            image,
            x: 18,
            y: 200,
            width: 18,
            height: 40,
            source: [1, 0, 1, 2],
            cell_width: 9,
            cell_height: 20,
            layer: Layer::BelowText,
            z: -1,
        };
        let drawing = draw(
            Painter::new(),
            std::slice::from_ref(&placement),
            Layer::BelowText,
            11,
            8.5,
            20.0,
        )
        .finish();
        let CanvasOp::DrawImage {
            image,
            source,
            dest,
        } = &drawing.ops[0]
        else {
            panic!("native drawImage expected");
        };
        assert!(
            Arc::ptr_eq(&image.uri, &placement.image.data_uri),
            "each row shares encoded image bytes"
        );
        assert_eq!(source.unwrap().origin.x.0, 1.0);
        assert_eq!(
            (*dest).map(|num| num.get(|_| 0.0)),
            [17.0, -20.0, 17.0, 40.0]
        );
        assert!(
            draw(
                Painter::new(),
                std::slice::from_ref(&placement),
                Layer::AboveText,
                11,
                8.5,
                20.0
            )
            .finish()
            .ops
            .is_empty()
        );
        assert!(
            draw(
                Painter::new(),
                &[placement],
                Layer::BelowText,
                12,
                8.5,
                20.0
            )
            .finish()
            .ops
            .is_empty()
        );
    }
}
