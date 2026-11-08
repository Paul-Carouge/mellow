//! The picker's contact sheet (docs/design/web-gif-picker.md "Layout concept"): rows of one height
//! where each tile takes a width from its shape, read in Klipy's order row by row. The web gets it
//! from `flex-wrap` with each tile's basis at the row's height times its shape and the same
//! number as its grow factor, plus a spacer (grow 1000) that keeps the last row from stretching;
//! this is that layout worked out by hand, since GPUI's flexbox does not wrap with grow.

use mellow_client_core::shared_logic::klipy::TileBounds;

/// The web's spacer after the last tile: it takes nearly all the last row's free space.
const LAST_ROW_SPACER_GROW: f32 = 1000.0;

/// Where each tile goes, in order, within a sheet `row_width` wide.
#[must_use]
pub fn justified_rows(
    aspect_ratios: &[f32],
    row_width: f32,
    row_height: f32,
    gap: f32,
) -> Vec<TileBounds> {
    let mut rows: Vec<Vec<f32>> = Vec::new();
    let mut used_width = 0.0;
    for &aspect_ratio in aspect_ratios {
        let basis = row_height * aspect_ratio;
        match rows.last_mut() {
            Some(row) if used_width + gap + basis <= row_width => {
                row.push(aspect_ratio);
                used_width += gap + basis;
            }
            _ => {
                rows.push(vec![aspect_ratio]);
                used_width = basis;
            }
        }
    }
    let row_count = rows.len();
    let mut placements = Vec::with_capacity(aspect_ratios.len());
    for (row_index, row) in rows.into_iter().enumerate() {
        let is_last_row = row_index + 1 == row_count;
        // `as` on a row count: a few dozen at most.
        #[allow(clippy::cast_precision_loss)]
        let gaps = gap * (row.len() - 1) as f32;
        let row_basis_width: f32 = row.iter().map(|ratio| row_height * ratio).sum();
        let grow_total: f32 = row.iter().sum();
        // The spacer joins the last row only where it fits, with a gap of its own.
        let has_spacer = is_last_row && row_basis_width + gaps + gap <= row_width;
        let (free, grow_total) = if has_spacer {
            (
                row_width - row_basis_width - gaps - gap,
                grow_total + LAST_ROW_SPACER_GROW,
            )
        } else {
            (row_width - row_basis_width - gaps, grow_total)
        };
        #[allow(clippy::cast_precision_loss)]
        let top = row_index as f32 * (row_height + gap);
        let mut left = 0.0;
        for aspect_ratio in row {
            let basis = row_height * aspect_ratio;
            let width = if free >= 0.0 {
                basis + free * aspect_ratio / grow_total
            } else {
                // Shrunk by its basis, as `flex-shrink: 1` does: a lone tile wider than the sheet.
                basis + free * basis / row_basis_width
            };
            let width = width.clamp(0.0, row_width);
            placements.push(TileBounds {
                left,
                top,
                width,
                height: row_height,
            });
            left += width + gap;
        }
    }
    placements
}

/// The sheet's height for `placements`.
#[must_use]
pub fn sheet_height(placements: &[TileBounds]) -> f32 {
    placements
        .iter()
        .map(|tile| tile.top + tile.height)
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(first: f32, second: f32) -> bool {
        (first - second).abs() < 0.01
    }

    #[test]
    fn full_rows_fill_the_width_and_keep_klipys_order() {
        // 96 px rows, 400 px wide: 1.5 + 1.5 fits (288 + 4), a third does not (436).
        let placements = justified_rows(&[1.5, 1.5, 1.5, 1.0], 400.0, 96.0, 4.0);
        assert_eq!(placements.len(), 4);
        let first_row: Vec<&TileBounds> =
            placements.iter().filter(|tile| tile.top == 0.0).collect();
        assert_eq!(first_row.len(), 2);
        let right_edge = first_row[1].left + first_row[1].width;
        assert!(close(right_edge, 400.0), "{right_edge}");
        assert!(
            close(first_row[0].width, first_row[1].width),
            "same shape, same width"
        );
        assert!(close(placements[2].top, 100.0));
        assert!(close(placements[2].left, 0.0));
    }

    #[test]
    fn the_last_row_keeps_its_tiles_near_their_shape() {
        let placements = justified_rows(&[1.0], 400.0, 96.0, 4.0);
        let width = placements[0].width;
        assert!((96.0..97.0).contains(&width), "{width}");
    }

    #[test]
    fn a_tile_wider_than_the_sheet_is_shrunk_to_it() {
        let placements = justified_rows(&[2.5], 200.0, 96.0, 4.0);
        assert!(close(placements[0].width, 200.0));
        assert!(close(sheet_height(&placements), 96.0));
        assert_eq!(justified_rows(&[], 400.0, 96.0, 4.0).len(), 0);
    }
}
