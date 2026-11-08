//! Messages as the web client draws them (DESIGN.md §5.4): `MessageItem.svelte`,
//! `MessageMarkdown.svelte`, `MessageCodeBlock.svelte`, `MessageReactions.svelte`, the pictures of
//! `MessageAttachments.svelte`, the reply line of `reply-references.ts` and the day and unread
//! dividers of `MessageList.svelte`.
//!
//! These are standalone components: the message list (virtualised) builds one row per entry of
//! client-core's `build_message_rows` and keeps a [`MessageBodyCache`], so a message's text is
//! parsed once per version, not on every frame.

mod attachments;
mod audio_attachment;
mod body;
mod cards;
mod code_block;
mod dividers;
mod forward;
mod gif;
mod item;
mod link_previews;
mod markdown_view;
mod reactions;
mod reply;
mod rich_text;
pub(crate) mod samples;
mod time;

use gpui::{App, ImageSource, Window};

pub use attachments::MessageAttachments;
pub use body::{MessageBody, MessageBodyCache, MessageBodyKey, message_summary};
pub use cards::{ChecklistCard, PollCard, ProgressBarCard, poll_time_left_label};
pub use code_block::MessageCodeBlock;
pub use dividers::{DayDivider, UnreadDivider};
pub use forward::ForwardOrigin;
pub use gif::{MessageGif, message_gif_url};
pub use item::{MessageAuthor, MessageItem, MessageItemContent};
pub use link_previews::MessageLinkPreviews;
pub use markdown_view::{EditedMarker, MessageMarkdown};
pub use reactions::{
    AddReactionButton, MessageReactions, ReactionPill, ReactionTooltipBuilder, reaction_pills,
};
pub use reply::{ReplyLine, ReplyReference, reply_reference};
pub use time::{
    day_divider_label, full_timestamp_label, local_date_time, local_day_of,
    message_timestamp_label, time_of_day_label, upcoming_time_label,
};

use crate::pictures::{Animation, DisplaySize, PictureFile, PictureRequest, picture_source};

/// Lengths in tokens are logical pixels at the default 16 px root size; zoom changes the root.
pub(crate) const TOKEN_PIXELS_PER_REM: f32 = 16.0;

/// Animated pictures play unless motion is reduced, as `isPictureAnimationWanted` decides.
pub(crate) fn picture_animation(cx: &App) -> Animation {
    if cx.reduce_motion() {
        Animation::StillFrame
    } else {
        Animation::Animate
    }
}

/// The request for `file` decoded for a box of `width` × `height` token pixels in this window.
pub(crate) fn picture_request_for_box(
    file: impl Into<PictureFile>,
    width: f32,
    height: f32,
    animation: Animation,
    window: &Window,
) -> PictureRequest {
    let device_pixels_per_token_pixel =
        f32::from(window.rem_size()) / TOKEN_PIXELS_PER_REM * window.scale_factor();
    // Rounded up and at least one pixel, so the cast cannot truncate a positive size to zero.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let device = |side: f32| (side * device_pixels_per_token_pixel).ceil().max(1.0) as u32;
    PictureRequest {
        file: file.into(),
        animation,
        display_size: DisplaySize {
            width: device(width),
            height: device(height),
        },
    }
}

/// The picture of `file` decoded for a box of `width` × `height` token pixels in this window.
pub(crate) fn picture_for_box(
    file: impl Into<PictureFile>,
    width: f32,
    height: f32,
    animation: Animation,
    window: &Window,
) -> ImageSource {
    picture_source(picture_request_for_box(
        file, width, height, animation, window,
    ))
}
