//! The widths the reader gives the columns of the wide layout (`web/src/components/column-widths.ts`,
//! DESIGN.md §4.5): the channel list in the navigator and the docked member sheet. Remembered in
//! the preferences, checked on the way in, and fitted to the window so the conversation keeps its
//! minimum width. Pure: widths in logical pixels at 100 % zoom.

use crate::theme::tokens::{
    COMPONENT_NAVIGATOR_STRIP_WIDTH, SIZE_CHAT_MIN_WIDTH, SIZE_MEMBER_LIST_WIDTH,
    SIZE_MEMBER_LIST_WIDTH_MAX, SIZE_MEMBER_LIST_WIDTH_MIN, SIZE_SHEET_INSET, SIZE_SIDEBAR_WIDTH,
    SIZE_SIDEBAR_WIDTH_MAX, SIZE_SIDEBAR_WIDTH_MIN,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColumnBounds {
    pub minimum: f32,
    pub maximum: f32,
    /// The width a double-click puts back.
    pub default: f32,
}

pub const CHANNELS_COLUMN_BOUNDS: ColumnBounds = ColumnBounds {
    minimum: SIZE_SIDEBAR_WIDTH_MIN,
    maximum: SIZE_SIDEBAR_WIDTH_MAX,
    default: SIZE_SIDEBAR_WIDTH,
};

pub const MEMBERS_COLUMN_BOUNDS: ColumnBounds = ColumnBounds {
    minimum: SIZE_MEMBER_LIST_WIDTH_MIN,
    maximum: SIZE_MEMBER_LIST_WIDTH_MAX,
    default: SIZE_MEMBER_LIST_WIDTH,
};

/// The strip, a conversation at its minimum and the row's four insets (two ends, two gaps).
pub const WIDTH_BESIDE_COLUMNS: f32 =
    COMPONENT_NAVIGATOR_STRIP_WIDTH + SIZE_CHAT_MIN_WIDTH + 4.0 * SIZE_SHEET_INSET;

/// The widest the column may be now: its maximum, or less when the window leaves less room.
#[must_use]
pub fn largest_column_width(bounds: ColumnBounds, room: f32) -> f32 {
    bounds.minimum.max(bounds.maximum.min(room))
}

/// The width kept within the column's bounds and within `room`, though never under its minimum,
/// on a whole pixel.
#[must_use]
pub fn clamp_column_width(width: f32, bounds: ColumnBounds, room: f32) -> f32 {
    width
        .max(bounds.minimum)
        .min(largest_column_width(bounds, room))
        .round()
}

/// The widths laid out for the channel list and the docked side sheet (`None` when none is
/// docked), given the room the window leaves beside a conversation of minimum width. The side
/// sheet gives way first, down to its minimum, so opening it moves the channel list only when even
/// that does not fit. Below the breakpoints every column stays at its minimum: the drawers take
/// over there.
#[must_use]
pub fn fit_column_widths(
    room: f32,
    channels: (f32, ColumnBounds),
    side_sheet: Option<(f32, ColumnBounds)>,
) -> (f32, Option<f32>) {
    let side_minimum = side_sheet.map_or(0.0, |(_, bounds)| bounds.minimum);
    let fitted_channels = clamp_column_width(channels.0, channels.1, room - side_minimum);
    (
        fitted_channels,
        side_sheet.map(|(width, bounds)| clamp_column_width(width, bounds, room - fitted_channels)),
    )
}

/// The width after a key on a handle (WAI-ARIA window splitter): the arrows move the edge one step
/// left or right, Home and End to the narrowest and widest. `None` for any other key.
/// `grows_rightwards` says whether moving the edge right widens the column: the channel list's
/// trailing edge does, the side sheet's leading edge does not.
#[must_use]
pub fn width_after_key(
    key: &str,
    width: f32,
    bounds: ColumnBounds,
    grows_rightwards: bool,
    step: f32,
) -> Option<f32> {
    let towards_wider = if grows_rightwards { step } else { -step };
    match key {
        "right" => Some(clamp_column_width(
            width + towards_wider,
            bounds,
            bounds.maximum,
        )),
        "left" => Some(clamp_column_width(
            width - towards_wider,
            bounds,
            bounds.maximum,
        )),
        "home" => Some(bounds.minimum),
        "end" => Some(bounds.maximum),
        _ => None,
    }
}

/// A stored width within the column's bounds; the default when none is stored.
#[must_use]
pub fn stored_column_width(stored: Option<f32>, bounds: ColumnBounds) -> f32 {
    stored.map_or(bounds.default, |width| {
        clamp_column_width(width, bounds, bounds.maximum)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNELS: ColumnBounds = ColumnBounds {
        minimum: 240.0,
        maximum: 360.0,
        default: 240.0,
    };
    const MEMBERS: ColumnBounds = ColumnBounds {
        minimum: 240.0,
        maximum: 320.0,
        default: 240.0,
    };
    const SEARCH: ColumnBounds = ColumnBounds {
        minimum: 400.0,
        maximum: 560.0,
        default: 400.0,
    };
    const STEP: f32 = 16.0;

    #[test]
    fn a_width_stays_within_the_bounds_and_the_room_on_whole_pixels() {
        assert_eq!(clamp_column_width(300.0, CHANNELS, 1_000.0), 300.0);
        assert_eq!(clamp_column_width(100.0, CHANNELS, 1_000.0), 240.0);
        assert_eq!(clamp_column_width(900.0, CHANNELS, 1_000.0), 360.0);
        assert_eq!(clamp_column_width(360.0, CHANNELS, 300.0), 300.0);
        assert_eq!(clamp_column_width(360.0, CHANNELS, 100.0), 240.0);
        assert_eq!(clamp_column_width(360.0, CHANNELS, -50.0), 240.0);
        assert_eq!(largest_column_width(CHANNELS, 300.0), 300.0);
        assert_eq!(largest_column_width(CHANNELS, 100.0), 240.0);
        assert_eq!(clamp_column_width(250.6, CHANNELS, 1_000.0), 251.0);
    }

    #[test]
    fn the_side_sheet_narrows_first_then_the_channel_list() {
        assert_eq!(
            fit_column_widths(1_000.0, (360.0, CHANNELS), Some((320.0, MEMBERS))),
            (360.0, Some(320.0))
        );
        assert_eq!(
            fit_column_widths(620.0, (360.0, CHANNELS), Some((320.0, MEMBERS))),
            (360.0, Some(260.0))
        );
        assert_eq!(
            fit_column_widths(560.0, (360.0, CHANNELS), Some((320.0, MEMBERS))),
            (320.0, Some(240.0))
        );
        assert_eq!(
            fit_column_widths(200.0, (360.0, CHANNELS), Some((500.0, SEARCH))),
            (240.0, Some(400.0))
        );
        assert_eq!(
            fit_column_widths(330.0, (360.0, CHANNELS), None),
            (330.0, None)
        );
    }

    #[test]
    fn keys_move_the_edge_and_stop_at_the_bounds() {
        assert_eq!(
            width_after_key("right", 240.0, CHANNELS, true, STEP),
            Some(256.0)
        );
        assert_eq!(
            width_after_key("left", 256.0, CHANNELS, true, STEP),
            Some(240.0)
        );
        // The side sheet's leading edge: left widens it.
        assert_eq!(
            width_after_key("left", 240.0, MEMBERS, false, STEP),
            Some(256.0)
        );
        assert_eq!(
            width_after_key("right", 256.0, MEMBERS, false, STEP),
            Some(240.0)
        );
        assert_eq!(
            width_after_key("left", 240.0, CHANNELS, true, STEP),
            Some(240.0)
        );
        assert_eq!(
            width_after_key("right", 350.0, CHANNELS, true, STEP),
            Some(360.0)
        );
        assert_eq!(
            width_after_key("home", 300.0, CHANNELS, true, STEP),
            Some(240.0)
        );
        assert_eq!(
            width_after_key("end", 300.0, CHANNELS, true, STEP),
            Some(360.0)
        );
        assert_eq!(width_after_key("up", 300.0, CHANNELS, true, STEP), None);
        assert_eq!(width_after_key("enter", 300.0, CHANNELS, true, STEP), None);
    }

    #[test]
    fn stored_widths_come_back_within_the_bounds() {
        assert_eq!(stored_column_width(None, CHANNELS), 240.0);
        assert_eq!(stored_column_width(Some(9_000.0), CHANNELS), 360.0);
        assert_eq!(stored_column_width(Some(-4.0), MEMBERS), 240.0);
        assert_eq!(stored_column_width(Some(450.4), SEARCH), 450.0);
    }

    #[test]
    fn the_generated_bounds_are_ordered() {
        for bounds in [CHANNELS_COLUMN_BOUNDS, MEMBERS_COLUMN_BOUNDS] {
            assert!(bounds.minimum <= bounds.default && bounds.default <= bounds.maximum);
        }
    }
}
