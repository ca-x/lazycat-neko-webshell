//! One measured grid for terminal painting, pointer coordinates, and PTY sizing.

use nsh_base::TerminalSize;
use nsh_core::canvas::TextMetrics;

/// The terminal's content inset, from the app's eight-point spacing tier.
pub(super) const PADDING: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Grid {
    pub cell_width: f32,
    pub row_height: f32,
    pub baseline: f32,
}

impl Grid {
    /// Follow the live screen and its cursor, excluding an unused blank tail.
    pub fn follow_offset(
        self,
        count: usize,
        screen_rows: usize,
        cursor: Option<usize>,
        height: f32,
    ) -> f32 {
        let screen_start = count.saturating_sub(screen_rows) as f32 * self.row_height;
        let cursor_bottom = (cursor.unwrap_or(0) + 1) as f32 * self.row_height;
        let limit = (count as f32 * self.row_height - height).max(0.0);
        (screen_start + (cursor_bottom - height).max(0.0)).min(limit)
    }
    /// At most one viewport and two boundary rows; never paint the whole history.
    pub fn visible_rows(self, offset: f32, height: f32, count: usize) -> std::ops::Range<usize> {
        let limit = (count as f32 * self.row_height - height.max(0.0)).max(0.0);
        let first = ((offset.clamp(0.0, limit) / self.row_height).floor() as usize).min(count);
        let length = (height.max(0.0) / self.row_height).ceil() as usize + 2;
        first..first.saturating_add(length).min(count)
    }
    pub fn measured(width: f32, preferred_height: f32, metrics: &[TextMetrics]) -> Self {
        let ascent = metrics.iter().map(|m| m.ascent.0).fold(0.0, f32::max);
        let descent = metrics.iter().map(|m| m.descent.0).fold(0.0, f32::max);
        let height = metrics.iter().map(|m| m.height.0).fold(0.0, f32::max);
        let row_height = preferred_height.max(height).max(ascent + descent).ceil();
        Self {
            cell_width: width.max(1.0),
            row_height: row_height.max(1.0),
            baseline: ascent + (row_height - ascent - descent).max(0.0) / 2.0,
        }
    }

    pub fn fit(self, width: f32, height: f32) -> Option<TerminalSize> {
        let width = width - 2.0 * PADDING;
        let height = height - 2.0 * PADDING;
        // Hidden and transitional layouts must not resize a live PTY to 1x1.
        if width < 2.0 * self.cell_width || height < 2.0 * self.row_height {
            return None;
        }
        Some(TerminalSize::new(
            (width / self.cell_width).floor().clamp(2.0, 1000.0) as u16,
            (height / self.row_height).floor().clamp(2.0, 1000.0) as u16,
        ))
    }

    /// Content coordinates; a captured drag may clamp to the first/last cell.
    pub fn hit(self, x: f32, y: f32, cols: usize, rows: usize) -> (usize, usize) {
        (
            ((y / self.row_height).floor().max(0.0) as usize).min(rows.saturating_sub(1)),
            ((x / self.cell_width).floor().max(0.0) as usize).min(cols.saturating_sub(1)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_core::geometry::px;

    #[test]
    fn following_keeps_a_cropped_live_screen_and_cursor_visible_without_blank_tail() {
        let grid = Grid {
            cell_width: 8.5,
            row_height: 20.0,
            baseline: 15.0,
        };
        assert_eq!(grid.follow_offset(24, 24, Some(0), 300.0), 0.0);
        assert_eq!(grid.follow_offset(24, 24, Some(23), 300.0), 180.0);
        assert_eq!(grid.follow_offset(100, 24, Some(0), 300.0), 1520.0);
        assert_eq!(grid.follow_offset(100, 24, Some(23), 300.0), 1700.0);
        assert_eq!(grid.follow_offset(100, 24, None, 300.0), 1520.0);
        assert_eq!(grid.follow_offset(24, 24, Some(0), 600.0), 0.0);
    }

    #[test]
    fn visible_canvas_work_is_bounded_by_the_viewport_even_with_large_history() {
        let grid = Grid {
            cell_width: 8.5,
            row_height: 20.0,
            baseline: 15.0,
        };
        assert_eq!(grid.visible_rows(205.0, 400.0, 100_000), 10..32);
        assert_eq!(grid.visible_rows(199_990.0, 400.0, 10_000), 9980..10_000);
        assert_eq!(grid.visible_rows(0.0, 400.0, 3), 0..3);
        assert_eq!(grid.visible_rows(-20.0, 400.0, 0), 0..0);
        assert_eq!(
            grid.visible_rows(480.0, 480.0, 24),
            0..24,
            "stale zero-height offset must not hide the source screen"
        );
    }

    #[test]
    fn baseline_keeps_every_font_variant_inside_the_first_row() {
        let metrics = [(12.0, 4.0, 18.0), (15.0, 5.0, 21.0)].map(|(a, d, h)| {
            let mut metrics = TextMetrics::default();
            metrics.ascent = px(a);
            metrics.descent = px(d);
            metrics.height = px(h);
            metrics
        });
        let grid = Grid::measured(8.4, 17.0, &metrics);
        for metrics in metrics {
            assert!(grid.baseline >= metrics.ascent.0);
            assert!(grid.baseline + metrics.descent.0 <= grid.row_height);
        }
        assert_eq!(grid.row_height, 21.0);
    }

    #[test]
    fn fitted_grid_is_the_largest_painted_grid_inside_the_insets() {
        let grid = Grid {
            cell_width: 8.4,
            row_height: 21.0,
            baseline: 15.0,
        };
        for width in [320.0, 799.5, 1200.0] {
            for height in [240.0, 599.5, 900.0] {
                let size = grid.fit(width, height).unwrap();
                for (count, cell, limit) in [
                    (size.cols, grid.cell_width, width),
                    (size.rows, grid.row_height, height),
                ] {
                    assert!(2.0 * PADDING + f32::from(count) * cell <= limit);
                    assert!(2.0 * PADDING + (f32::from(count) + 1.0) * cell > limit);
                }
                assert_eq!(
                    grid.hit(
                        (f32::from(size.cols) - 0.5) * grid.cell_width,
                        (f32::from(size.rows) - 0.5) * grid.row_height,
                        size.cols.into(),
                        size.rows.into()
                    ),
                    (usize::from(size.rows) - 1, usize::from(size.cols) - 1)
                );
            }
        }
        assert!(grid.fit(1.0, 1.0).is_none());
        assert!(grid.fit(20.0, 20.0).is_none());
    }
}
