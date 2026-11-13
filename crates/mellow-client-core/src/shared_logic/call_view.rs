//! What the call view and a watched screen show (`call-view.ts` and `screen-share-look.ts`,
//! ADR-0035, ADR-0037): which people get a tile and in which order, how large each picture is
//! asked for, the grid's shape, who the speaker view puts large, the live chip beside a sharer,
//! and the state of the screen being watched.

use std::collections::HashSet;

use mellow_protocol::{Snowflake, VideoTileSize, VoiceState};

/// The grid, or one person large over a strip of the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CallLayout {
    #[default]
    Grid,
    Speaker,
}

/// What a person sends: a camera, a shared screen (ADR-0037: never both), or nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallVideo {
    Camera,
    Screen,
}

/// One person of the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallPerson {
    pub user_id: Snowflake,
    pub video: Option<CallVideo>,
}

impl CallPerson {
    /// What `state` says the person sends.
    #[must_use]
    pub fn of_voice_state(state: &VoiceState) -> Self {
        let video = if state.self_stream {
            Some(CallVideo::Screen)
        } else if state.self_video {
            Some(CallVideo::Camera)
        } else {
            None
        };
        Self {
            user_id: state.user_id,
            video,
        }
    }
}

/// A picture to ask the instance for, and how large.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchedPicture {
    pub user_id: Snowflake,
    pub size: VideoTileSize,
}

/// Nine tiles at most, as many as the video receive slots (ADR-0035: a 3 × 3 grid).
pub const MAXIMUM_CALL_TILES: usize = 9;

/// The 16:9 of cameras and of most screens.
const TILE_ASPECT_RATIO: f32 = 16.0 / 9.0;

/// The people with a tile, and those shown as avatars under them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallArrangement {
    pub tiles: Vec<CallPerson>,
    pub overflow: Vec<CallPerson>,
}

/// People with a picture first, then the others, each in join order; the first `maximum` get a
/// tile and the rest show as avatars under them. In the speaker view the featured person comes
/// first.
#[must_use]
pub fn arrange_call_tiles(
    people: &[CallPerson],
    featured_user_id: Option<Snowflake>,
    maximum: usize,
) -> CallArrangement {
    let is_featured = |person: &&CallPerson| Some(person.user_id) == featured_user_id;
    let featured = people.iter().filter(is_featured);
    let rest = || people.iter().filter(|person| !is_featured(person));
    let mut ordered: Vec<CallPerson> = featured
        .chain(rest().filter(|person| person.video.is_some()))
        .chain(rest().filter(|person| person.video.is_none()))
        .copied()
        .collect();
    let overflow = ordered.split_off(maximum.min(ordered.len()));
    CallArrangement {
        tiles: ordered,
        overflow,
    }
}

/// ADR-0035: a grid of 1 is large, 2 to 4 medium, 5 to 9 small; the speaker view's large tile
/// is large, the strip small.
#[must_use]
pub fn tile_size_of(layout: CallLayout, tile_count: usize, is_featured: bool) -> VideoTileSize {
    match layout {
        CallLayout::Speaker if is_featured => VideoTileSize::Large,
        CallLayout::Grid if tile_count <= 1 => VideoTileSize::Large,
        CallLayout::Grid if tile_count <= 4 => VideoTileSize::Medium,
        CallLayout::Speaker | CallLayout::Grid => VideoTileSize::Small,
    }
}

/// The pictures to ask the instance for: everyone with a camera or a screen on a tile, you
/// aside (your own picture is local).
#[must_use]
pub fn watched_pictures_of(
    tiles: &[CallPerson],
    layout: CallLayout,
    own_user_id: Option<Snowflake>,
) -> Vec<WatchedPicture> {
    tiles
        .iter()
        .enumerate()
        .filter(|(_, person)| person.video.is_some() && Some(person.user_id) != own_user_id)
        .map(|(index, person)| WatchedPicture {
            user_id: person.user_id,
            size: tile_size_of(
                layout,
                tiles.len(),
                layout == CallLayout::Speaker && index == 0,
            ),
        })
        .collect()
}

/// The watched screen large first, then the cameras shown, nine at most (ADR-0035): the list
/// the session sends.
#[must_use]
pub fn watch_list_of(
    watched_stream_user_id: Option<Snowflake>,
    cameras: &[WatchedPicture],
) -> Vec<WatchedPicture> {
    watched_stream_user_id
        .map(|user_id| WatchedPicture {
            user_id,
            size: VideoTileSize::Large,
        })
        .into_iter()
        .chain(
            cameras
                .iter()
                .filter(|camera| Some(camera.user_id) != watched_stream_user_id)
                .copied(),
        )
        .take(MAXIMUM_CALL_TILES)
        .collect()
}

/// The grid's column count and its tiles' width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridShape {
    pub columns: usize,
    pub tile_width: f32,
}

/// The column count that gives `count` 16:9 tiles the largest size in `width` × `height`, and
/// that size.
#[must_use]
pub fn grid_shape_for(count: usize, width: f32, height: f32, gap: f32) -> GridShape {
    let mut best = GridShape {
        columns: 1,
        tile_width: 0.0,
    };
    for columns in 1..=count.max(1) {
        let rows = count.div_ceil(columns);
        // At most nine tiles: exact in `f32`.
        #[allow(clippy::cast_precision_loss)]
        let (columns_f32, rows_f32) = (columns as f32, rows as f32);
        let width_per_tile = (width - gap * (columns_f32 - 1.0)) / columns_f32;
        let height_per_tile = (height - gap * (rows_f32 - 1.0)) / rows_f32;
        let tile_width = width_per_tile
            .min(height_per_tile * TILE_ASPECT_RATIO)
            .max(0.0)
            .floor();
        if tile_width > best.tile_width {
            best = GridShape {
                columns,
                tile_width,
            };
        }
    }
    best
}

/// Who the speaker view puts large when nobody is picked: whoever is talking (someone else than
/// you, as on Discord), staying on the same person while they still talk, and on the last one
/// through silences. `None` when that person left.
#[must_use]
pub fn next_speaker(
    current: Option<Snowflake>,
    speaking_user_ids: &HashSet<Snowflake, impl std::hash::BuildHasher>,
    own_user_id: Option<Snowflake>,
    present_user_ids: &[Snowflake],
) -> Option<Snowflake> {
    let is_present = |user_id: &Snowflake| present_user_ids.contains(user_id);
    if let Some(current) = current
        && speaking_user_ids.contains(&current)
        && is_present(&current)
    {
        return Some(current);
    }
    present_user_ids
        .iter()
        .find(|user_id| Some(**user_id) != own_user_id && speaking_user_ids.contains(user_id))
        .copied()
        .or(current.filter(is_present))
}

/// The person the speaker view puts large: the one picked, else the last speaker, else the
/// first picture, else the first person.
#[must_use]
pub fn featured_person_of(
    people: &[CallPerson],
    picked_user_id: Option<Snowflake>,
    speaker_user_id: Option<Snowflake>,
    own_user_id: Option<Snowflake>,
) -> Option<Snowflake> {
    let is_present = |user_id: &Snowflake| people.iter().any(|person| person.user_id == *user_id);
    if let Some(picked) = picked_user_id.filter(is_present) {
        return Some(picked);
    }
    if let Some(speaker) = speaker_user_id.filter(is_present) {
        return Some(speaker);
    }
    let mut others = people
        .iter()
        .filter(|person| Some(person.user_id) != own_user_id);
    others
        .clone()
        .find(|person| person.video.is_some())
        .or_else(|| others.next())
        .or_else(|| people.first())
        .map(|person| person.user_id)
}

/// What the live chip beside someone who shares their screen does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveBadge {
    /// A button: watch their screen large.
    Watch,
    /// Your own share: shown, never opened, since watching yourself would loop the picture back.
    Own,
    /// They share in a channel you are not connected to: shown, with how to watch.
    JoinToWatch,
    /// In your call, but this client cannot show their picture.
    CannotWatch,
}

/// The call you are in, as far as watching goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Watcher {
    /// The channel of your call once connected; `None` otherwise.
    pub connected_channel_id: Option<Snowflake>,
    pub can_watch_streams: bool,
}

/// `None` when they do not share their screen.
#[must_use]
pub fn live_badge_of(
    state: &VoiceState,
    own_user_id: Option<Snowflake>,
    watcher: Watcher,
) -> Option<LiveBadge> {
    if !state.self_stream {
        return None;
    }
    Some(if Some(state.user_id) == own_user_id {
        LiveBadge::Own
    } else if Some(state.channel_id) != watcher.connected_channel_id {
        LiveBadge::JoinToWatch
    } else if watcher.can_watch_streams {
        LiveBadge::Watch
    } else {
        LiveBadge::CannotWatch
    })
}

/// What the stream view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchedStreamStatus {
    /// They share, and their picture has not arrived yet.
    Connecting,
    Live,
    /// They stopped sharing, left your call, or you left theirs: the view says so, then closes.
    Ended,
}

/// `state` is the watched person's voice state, `call_channel_id` the channel of your call
/// (`None` when you are in none) and `has_picture` whether their picture has arrived.
#[must_use]
pub fn watched_stream_status(
    state: Option<&VoiceState>,
    call_channel_id: Option<Snowflake>,
    has_picture: bool,
) -> WatchedStreamStatus {
    let is_sharing_here =
        state.is_some_and(|state| call_channel_id == Some(state.channel_id) && state.self_stream);
    if !is_sharing_here {
        WatchedStreamStatus::Ended
    } else if has_picture {
        WatchedStreamStatus::Live
    } else {
        WatchedStreamStatus::Connecting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn person(user_id: u64, video: Option<CallVideo>) -> CallPerson {
        CallPerson {
            user_id: id(user_id),
            video,
        }
    }

    fn ids(people: &[CallPerson]) -> Vec<u64> {
        people.iter().map(|person| person.user_id.raw()).collect()
    }

    #[test]
    fn pictures_come_first_each_group_in_join_order_and_the_featured_person_before_everyone() {
        let people = [
            person(1, None),
            person(2, Some(CallVideo::Camera)),
            person(3, None),
            person(4, Some(CallVideo::Screen)),
        ];
        let arranged = arrange_call_tiles(&people, None, MAXIMUM_CALL_TILES);
        assert_eq!(ids(&arranged.tiles), [2, 4, 1, 3]);
        let arranged = arrange_call_tiles(&people, Some(id(3)), MAXIMUM_CALL_TILES);
        assert_eq!(ids(&arranged.tiles), [3, 2, 4, 1]);
    }

    #[test]
    fn nine_tiles_are_kept_and_the_others_go_under_them() {
        let people: Vec<CallPerson> = (0..12)
            .map(|index| person(index, (index == 11).then_some(CallVideo::Camera)))
            .collect();
        let arranged = arrange_call_tiles(&people, None, MAXIMUM_CALL_TILES);
        assert_eq!(arranged.tiles.len(), 9);
        assert_eq!(arranged.tiles[0].user_id, id(11));
        assert_eq!(ids(&arranged.overflow), [8, 9, 10]);
    }

    #[test]
    fn tile_sizes_follow_adr_0035() {
        assert_eq!(
            tile_size_of(CallLayout::Grid, 1, false),
            VideoTileSize::Large
        );
        assert_eq!(
            tile_size_of(CallLayout::Grid, 2, false),
            VideoTileSize::Medium
        );
        assert_eq!(
            tile_size_of(CallLayout::Grid, 4, false),
            VideoTileSize::Medium
        );
        assert_eq!(
            tile_size_of(CallLayout::Grid, 5, false),
            VideoTileSize::Small
        );
        assert_eq!(
            tile_size_of(CallLayout::Speaker, 5, true),
            VideoTileSize::Large
        );
        assert_eq!(
            tile_size_of(CallLayout::Speaker, 5, false),
            VideoTileSize::Small
        );
    }

    #[test]
    fn every_picture_on_a_tile_is_asked_for_but_your_own() {
        let tiles = [
            person(2, Some(CallVideo::Camera)),
            person(1, Some(CallVideo::Camera)),
            person(4, Some(CallVideo::Screen)),
            person(3, None),
        ];
        let watched = |user_id, size| WatchedPicture {
            user_id: id(user_id),
            size,
        };
        assert_eq!(
            watched_pictures_of(&tiles, CallLayout::Grid, Some(id(1))),
            [
                watched(2, VideoTileSize::Medium),
                watched(4, VideoTileSize::Medium)
            ]
        );
        assert_eq!(
            watched_pictures_of(&tiles, CallLayout::Speaker, Some(id(1))),
            [
                watched(2, VideoTileSize::Large),
                watched(4, VideoTileSize::Small)
            ]
        );
    }

    #[test]
    fn the_watched_screen_goes_first_and_large_and_the_list_stops_at_nine() {
        let cameras: Vec<WatchedPicture> = (1..=10)
            .map(|user_id| WatchedPicture {
                user_id: id(user_id),
                size: VideoTileSize::Small,
            })
            .collect();
        let list = watch_list_of(Some(id(3)), &cameras);
        assert_eq!(list.len(), 9);
        assert_eq!(
            list[0],
            WatchedPicture {
                user_id: id(3),
                size: VideoTileSize::Large
            }
        );
        assert_eq!(
            list.iter().filter(|camera| camera.user_id == id(3)).count(),
            1
        );
        assert_eq!(watch_list_of(None, &cameras[..2]), cameras[..2]);
    }

    #[test]
    fn the_grid_takes_the_columns_that_make_the_largest_tiles() {
        let shape = grid_shape_for(1, 1000.0, 600.0, 8.0);
        assert_eq!(shape.columns, 1);
        assert!((shape.tile_width - 1000.0).abs() < f32::EPSILON);
        assert_eq!(grid_shape_for(4, 1000.0, 600.0, 8.0).columns, 2);
        assert_eq!(grid_shape_for(3, 1600.0, 400.0, 8.0).columns, 3);
        // A narrow, tall window stacks them.
        assert_eq!(grid_shape_for(3, 360.0, 700.0, 8.0).columns, 1);
        assert_eq!(grid_shape_for(9, 1200.0, 700.0, 8.0).columns, 3);
        assert!(grid_shape_for(9, 0.0, 0.0, 8.0).tile_width.abs() < f32::EPSILON);
    }

    #[test]
    fn the_speaker_follows_whoever_else_talks_and_keeps_them_through_silences() {
        let present = [id(1), id(2), id(3)];
        let speaking = |raw: &[u64]| -> HashSet<Snowflake> {
            raw.iter().map(|user_id| id(*user_id)).collect()
        };
        let me = Some(id(1));
        assert_eq!(next_speaker(None, &speaking(&[1]), me, &present), None);
        assert_eq!(
            next_speaker(None, &speaking(&[2]), me, &present),
            Some(id(2))
        );
        assert_eq!(
            next_speaker(Some(id(2)), &speaking(&[2, 3]), me, &present),
            Some(id(2))
        );
        assert_eq!(
            next_speaker(Some(id(2)), &speaking(&[3]), me, &present),
            Some(id(3))
        );
        assert_eq!(
            next_speaker(Some(id(3)), &speaking(&[]), me, &present),
            Some(id(3))
        );
        assert_eq!(
            next_speaker(Some(id(4)), &speaking(&[]), me, &present),
            None
        );
    }

    #[test]
    fn the_featured_person_is_the_one_picked_then_the_speaker_then_someone_else_s_picture() {
        let people = [
            person(1, Some(CallVideo::Camera)),
            person(2, None),
            person(3, Some(CallVideo::Camera)),
        ];
        let me = Some(id(1));
        assert_eq!(
            featured_person_of(&people, Some(id(2)), Some(id(3)), me),
            Some(id(2))
        );
        assert_eq!(
            featured_person_of(&people, None, Some(id(2)), me),
            Some(id(2))
        );
        assert_eq!(
            featured_person_of(&people, Some(id(9)), None, me),
            Some(id(3))
        );
        assert_eq!(
            featured_person_of(&[person(1, None)], None, None, me),
            Some(id(1))
        );
        assert_eq!(featured_person_of(&[], None, None, me), None);
    }

    const ME: u64 = 1;
    const ANA: u64 = 2;
    const SALON: u64 = 21;
    const JEUX: u64 = 22;

    fn voice_state(user_id: u64, channel_id: u64, is_streaming: bool) -> VoiceState {
        VoiceState {
            user_id: id(user_id),
            guild_id: id(100),
            channel_id: id(channel_id),
            self_mute: false,
            self_deaf: false,
            suppressed: false,
            join_sound_id: None,
            self_video: false,
            self_stream: is_streaming,
            video_codec: None,
        }
    }

    #[test]
    fn the_live_chip_says_what_watching_their_screen_takes() {
        let in_salon = Watcher {
            connected_channel_id: Some(id(SALON)),
            can_watch_streams: true,
        };
        let me = Some(id(ME));
        assert_eq!(
            live_badge_of(&voice_state(ANA, SALON, false), me, in_salon),
            None
        );
        let mut camera = voice_state(ANA, SALON, false);
        camera.self_video = true;
        assert_eq!(live_badge_of(&camera, me, in_salon), None);
        assert_eq!(
            live_badge_of(&voice_state(ANA, SALON, true), me, in_salon),
            Some(LiveBadge::Watch)
        );
        assert_eq!(
            live_badge_of(&voice_state(ME, SALON, true), me, in_salon),
            Some(LiveBadge::Own)
        );
        assert_eq!(
            live_badge_of(&voice_state(ANA, JEUX, true), me, in_salon),
            Some(LiveBadge::JoinToWatch)
        );
        let not_connected = Watcher {
            connected_channel_id: None,
            can_watch_streams: false,
        };
        assert_eq!(
            live_badge_of(&voice_state(ANA, SALON, true), me, not_connected),
            Some(LiveBadge::JoinToWatch)
        );
        let without_video = Watcher {
            connected_channel_id: Some(id(SALON)),
            can_watch_streams: false,
        };
        assert_eq!(
            live_badge_of(&voice_state(ANA, SALON, true), me, without_video),
            Some(LiveBadge::CannotWatch)
        );
    }

    #[test]
    fn a_watched_screen_connects_goes_live_and_ends_when_they_stop_or_leave() {
        let salon = Some(id(SALON));
        let sharing = voice_state(ANA, SALON, true);
        assert_eq!(
            watched_stream_status(Some(&sharing), salon, false),
            WatchedStreamStatus::Connecting
        );
        assert_eq!(
            watched_stream_status(Some(&sharing), salon, true),
            WatchedStreamStatus::Live
        );
        assert_eq!(
            watched_stream_status(Some(&voice_state(ANA, SALON, false)), salon, true),
            WatchedStreamStatus::Ended
        );
        assert_eq!(
            watched_stream_status(None, salon, false),
            WatchedStreamStatus::Ended
        );
        assert_eq!(
            watched_stream_status(Some(&voice_state(ANA, JEUX, true)), salon, true),
            WatchedStreamStatus::Ended
        );
        assert_eq!(
            watched_stream_status(Some(&sharing), None, true),
            WatchedStreamStatus::Ended
        );
    }

    #[test]
    fn a_voice_state_says_what_its_person_sends() {
        let mut state = voice_state(ANA, SALON, false);
        assert_eq!(CallPerson::of_voice_state(&state).video, None);
        state.self_video = true;
        assert_eq!(
            CallPerson::of_voice_state(&state).video,
            Some(CallVideo::Camera)
        );
        state.self_stream = true;
        assert_eq!(
            CallPerson::of_voice_state(&state).video,
            Some(CallVideo::Screen)
        );
    }
}
