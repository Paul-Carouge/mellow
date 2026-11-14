//! What marks a text channel's name in its row and in the chat header (DESIGN.md v0.18, §5.3,
//! §6.54): its icon, as a rounded square (circles are people) and always still, in place of `#`;
//! without one, `#`, or the lock of a channel the reader cannot write in. With an icon, that lock
//! moves after the name.

use gpui::{
    AnyElement, App, Hsla, IntoElement, ObjectFit, Styled as _, StyledImage as _, Window, img,
};
use mellow_client_core::DownloadableFile;
use mellow_protocol::ProfileImage;

use super::{icon_picture, square_picture_request};
use crate::components::icon;
use crate::pictures::{Animation, PictureStatus, picture_status};
use crate::theme::icons::IconName;
use crate::theme::length;
use crate::theme::tokens::COMPONENT_CHANNEL_ROW_IMAGE_RADIUS;

/// What leads a channel's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelLeadingMark {
    Picture(ProfileImage),
    Glyph(IconName),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelNameMarks {
    pub leading: ChannelLeadingMark,
    /// The lock of a read-only channel, after the name because the icon took its place.
    pub has_lock_after_name: bool,
}

/// The marks of a text channel with `icon`, which the reader can write in or not.
#[must_use]
pub(crate) fn channel_name_marks(icon: Option<ProfileImage>, can_send: bool) -> ChannelNameMarks {
    let glyph = if can_send {
        IconName::Hash
    } else {
        IconName::Lock
    };
    match icon {
        Some(picture) => ChannelNameMarks {
            leading: ChannelLeadingMark::Picture(picture),
            has_lock_after_name: !can_send,
        },
        None => ChannelNameMarks {
            leading: ChannelLeadingMark::Glyph(glyph),
            has_lock_after_name: false,
        },
    }
}

/// The marks once the icon's picture failed to load: the glyph alone, the lock back in front
/// (`failedIconIds` on the web), so a read-only channel never shows two locks.
#[must_use]
pub(crate) fn with_failed_icon_dropped(
    marks: ChannelNameMarks,
    has_icon_failed: bool,
) -> ChannelNameMarks {
    match marks.leading {
        ChannelLeadingMark::Picture(_) if has_icon_failed => {
            channel_name_marks(None, !marks.has_lock_after_name)
        }
        _ => marks,
    }
}

/// [`with_failed_icon_dropped`] for the icon drawn at `size` token pixels in this window.
pub(crate) fn shown_channel_marks(
    marks: ChannelNameMarks,
    size: f32,
    window: &Window,
    cx: &mut App,
) -> ChannelNameMarks {
    let ChannelLeadingMark::Picture(picture) = marks.leading else {
        return marks;
    };
    let request = square_picture_request(
        DownloadableFile::Icon(picture.id),
        size,
        Animation::StillFrame,
        window,
    );
    with_failed_icon_dropped(marks, picture_status(request, cx) == PictureStatus::Failed)
}

/// The leading mark at `size` token pixels in `ink`; a picture that fails shows the glyph.
pub(crate) fn channel_leading_mark(
    marks: ChannelNameMarks,
    size: f32,
    ink: Hsla,
    window: &Window,
) -> AnyElement {
    match marks.leading {
        ChannelLeadingMark::Glyph(glyph) => icon(glyph, size, ink).into_any_element(),
        ChannelLeadingMark::Picture(picture) => {
            let fallback_glyph = if marks.has_lock_after_name {
                IconName::Lock
            } else {
                IconName::Hash
            };
            // Rounded itself: GPUI's `overflow_hidden` does not clip to rounded corners.
            img(icon_picture(picture, size, Animation::StillFrame, window))
                .flex_none()
                .size(length(size))
                .rounded(length(COMPONENT_CHANNEL_ROW_IMAGE_RADIUS))
                .object_fit(ObjectFit::Cover)
                .with_fallback(move || icon(fallback_glyph, size, ink).into_any_element())
                .into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Snowflake;

    use super::*;

    fn picture() -> ProfileImage {
        ProfileImage {
            id: Snowflake::from_raw(9),
            is_animated: true,
        }
    }

    #[test]
    fn a_channel_icon_takes_the_place_of_the_hash() {
        assert_eq!(
            channel_name_marks(Some(picture()), true),
            ChannelNameMarks {
                leading: ChannelLeadingMark::Picture(picture()),
                has_lock_after_name: false,
            }
        );
        assert_eq!(
            channel_name_marks(None, true),
            ChannelNameMarks {
                leading: ChannelLeadingMark::Glyph(IconName::Hash),
                has_lock_after_name: false,
            }
        );
    }

    #[test]
    fn an_icon_that_failed_leaves_the_glyph_alone_without_a_second_lock() {
        let read_only = channel_name_marks(Some(picture()), false);
        assert_eq!(
            with_failed_icon_dropped(read_only, true),
            ChannelNameMarks {
                leading: ChannelLeadingMark::Glyph(IconName::Lock),
                has_lock_after_name: false,
            }
        );
        let writable = channel_name_marks(Some(picture()), true);
        assert_eq!(
            with_failed_icon_dropped(writable, true).leading,
            ChannelLeadingMark::Glyph(IconName::Hash)
        );
        assert_eq!(
            with_failed_icon_dropped(read_only, false),
            read_only,
            "still loading or shown"
        );
    }

    #[test]
    fn the_lock_of_a_read_only_channel_moves_after_the_name_behind_an_icon() {
        assert_eq!(
            channel_name_marks(Some(picture()), false),
            ChannelNameMarks {
                leading: ChannelLeadingMark::Picture(picture()),
                has_lock_after_name: true,
            }
        );
        assert_eq!(
            channel_name_marks(None, false),
            ChannelNameMarks {
                leading: ChannelLeadingMark::Glyph(IconName::Lock),
                has_lock_after_name: false,
            }
        );
    }
}
