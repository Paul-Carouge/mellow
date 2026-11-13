//! The client's SDP offer, in the exact shape the server validates
//! (`crates/mellow-server/src/domain/voice_rules.rs`): the voice line and 16 voice slots, the
//! camera line (VP8, simulcast `h`, `l`, `q`; H.264 offered too where the system decodes it,
//! ADR-0052) and 9 camera slots, the screen-sound line and 3
//! screen-sound slots. Order is all that tells voice lines from screen-sound lines, so the mids
//! of each group are kept here. While an H.264 share runs, the camera line offers H.264
//! Constrained Baseline only and the receive lines stay as they were (ADR-0052, ADR-0054).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mellow_protocol::{
    MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS, MAXIMUM_VIDEO_RECEIVE_SLOTS, MAXIMUM_VOICE_RECEIVE_SLOTS,
    VideoCodec,
};
use str0m::change::SdpPendingOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, MediaKind, Mid, Pt, Simulcast, SimulcastLayer};
use str0m::{Candidate, Rtc};

/// Opus's payload type in every browser's offer, which the server answers with (ADR-0030).
pub(crate) const OPUS_PAYLOAD_TYPE: u8 = 111;
/// How often the round trip to the server is reported, as the web samples it.
const STATS_INTERVAL: Duration = Duration::from_secs(2);
/// H.264 Constrained Baseline in packetisation mode 1 with its retransmissions, the payload
/// types the server answers with (ADR-0052).
const H264_PAYLOAD_TYPE: u8 = 108;
const H264_RETRANSMISSION_PAYLOAD_TYPE: u8 = 109;
const H264_CONSTRAINED_BASELINE_PROFILE_LEVEL_ID: u32 = 0x42_e0_1f;
/// The camera's simulcast RIDs, lowest first, as the web client declares them.
const SIMULCAST_RIDS: [&str; 3] = ["q", "l", "h"];

/// What a receive m-line carries, from its place in the offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveSlotKind {
    Voice,
    Camera,
    ScreenSound,
}

/// The mids of the offer's m-lines, by role.
#[derive(Debug, Clone)]
pub(crate) struct MediaLines {
    pub(crate) voice_send: Mid,
    pub(crate) voice_receive: Vec<Mid>,
    /// The camera or shared screen, simulcast `q`, `l`, `h`.
    pub(crate) camera_send: Mid,
    pub(crate) camera_receive: Vec<Mid>,
    /// The shared screen's sound (ADR-0037).
    pub(crate) screen_sound_send: Mid,
    pub(crate) screen_sound_receive: Vec<Mid>,
}

impl MediaLines {
    pub(crate) fn receive_slot_kind(&self, mid: Mid) -> Option<ReceiveSlotKind> {
        if self.voice_receive.contains(&mid) {
            Some(ReceiveSlotKind::Voice)
        } else if self.camera_receive.contains(&mid) {
            Some(ReceiveSlotKind::Camera)
        } else if self.screen_sound_receive.contains(&mid) {
            Some(ReceiveSlotKind::ScreenSound)
        } else {
            None
        }
    }

    /// The audio receive lines, voices first: the slots the audio engine plays.
    pub(crate) fn audio_receive(&self) -> impl Iterator<Item = (Mid, ReceiveSlotKind)> + '_ {
        self.voice_receive
            .iter()
            .map(|mid| (*mid, ReceiveSlotKind::Voice))
            .chain(
                self.screen_sound_receive
                    .iter()
                    .map(|mid| (*mid, ReceiveSlotKind::ScreenSound)),
            )
    }
}

/// A peer with its offer made, waiting for the answer.
pub(crate) struct PendingPeer {
    pub(crate) rtc: Rtc,
    pub(crate) pending: SdpPendingOffer,
    pub(crate) sdp_offer: String,
    pub(crate) media_lines: MediaLines,
}

impl std::fmt::Debug for PendingPeer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingPeer")
            .field("media_lines", &self.media_lines)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the WebRTC offer could not be made: {0}")]
pub struct OfferError(String);

/// A peer whose only candidate is `local_address` (the server is ICE-lite with one public host
/// candidate: no STUN or TURN is needed), and its offer, whose camera line sends
/// `camera_codec`.
pub(crate) fn new_peer(
    local_address: SocketAddr,
    now: Instant,
    camera_codec: VideoCodec,
) -> Result<PendingPeer, OfferError> {
    str0m::crypto::from_feature_flags().install_process_default();
    let mut builder = Rtc::builder()
        .clear_codecs()
        .enable_vp8(true)
        .set_stats_interval(Some(STATS_INTERVAL));
    // Video lines take H.264 too where the system decodes it (ADR-0052): the server then sends
    // native sharers' pictures here. The camera line sends VP8, which the server picks first,
    // unless an H.264 share runs (ADR-0054).
    let is_h264_camera = camera_codec == VideoCodec::H264;
    let receive_lines_take_h264 = crate::h264::decodes_h264();
    if is_h264_camera || receive_lines_take_h264 {
        builder.codec_config().add_h264(
            Pt::new_with_value(H264_PAYLOAD_TYPE),
            Some(Pt::new_with_value(H264_RETRANSMISSION_PAYLOAD_TYPE)),
            true,
            H264_CONSTRAINED_BASELINE_PROFILE_LEVEL_ID,
        );
    }
    builder.codec_config().add_config(
        Pt::new_with_value(OPUS_PAYLOAD_TYPE),
        None,
        Codec::Opus,
        Frequency::FORTY_EIGHT_KHZ,
        Some(2),
        FormatParams {
            min_p_time: Some(10),
            use_inband_fec: Some(true),
            use_dtx: Some(true),
            ..FormatParams::default()
        },
    );
    let mut rtc = builder.build(now);
    let candidate =
        Candidate::host(local_address, "udp").map_err(|error| OfferError(error.to_string()))?;
    rtc.add_local_candidate(candidate);
    let mut change = rtc.sdp_api();
    let mut add_lines = |kind, direction, count| -> Vec<Mid> {
        (0..count)
            .map(|_| change.add_media(kind, direction, None, None, None))
            .collect()
    };
    let voice_send = add_lines(MediaKind::Audio, Direction::SendOnly, 1);
    let voice_receive = add_lines(
        MediaKind::Audio,
        Direction::RecvOnly,
        MAXIMUM_VOICE_RECEIVE_SLOTS,
    );
    let mut simulcast = Simulcast::new();
    for rid in SIMULCAST_RIDS {
        simulcast.add_send_layer(SimulcastLayer::new(rid));
    }
    let camera_send = change.add_media(
        MediaKind::Video,
        Direction::SendOnly,
        None,
        None,
        Some(simulcast),
    );
    let mut add_lines = |kind, direction, count| -> Vec<Mid> {
        (0..count)
            .map(|_| change.add_media(kind, direction, None, None, None))
            .collect()
    };
    let camera_receive = add_lines(
        MediaKind::Video,
        Direction::RecvOnly,
        MAXIMUM_VIDEO_RECEIVE_SLOTS,
    );
    let screen_sound_send = add_lines(MediaKind::Audio, Direction::SendOnly, 1);
    let screen_sound_receive = add_lines(
        MediaKind::Audio,
        Direction::RecvOnly,
        MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS,
    );
    let (offer, pending) = change
        .apply()
        .ok_or_else(|| OfferError("no media lines".into()))?;
    let Some(&voice_send) = voice_send.first() else {
        return Err(OfferError("the voice line is missing".into()));
    };
    let Some(&screen_sound_send) = screen_sound_send.first() else {
        return Err(OfferError("the screen sound line is missing".into()));
    };
    let sdp_offer = offer.to_sdp_string();
    let sdp_offer = if is_h264_camera {
        let h264 = [H264_PAYLOAD_TYPE, H264_RETRANSMISSION_PAYLOAD_TYPE];
        restrict_video_payload_types(&sdp_offer, &camera_send, &h264, receive_lines_take_h264)
    } else {
        sdp_offer
    };
    Ok(PendingPeer {
        rtc,
        pending,
        sdp_offer,
        media_lines: MediaLines {
            voice_send,
            voice_receive,
            camera_send,
            camera_receive,
            screen_sound_send,
            screen_sound_receive,
        },
    })
}

/// The offer with the video m-line `camera_mid` carrying only `payload_types` and every other
/// video m-line carrying all but them, unless `receive_lines_keep_them`. `str0m` offers its
/// whole codec configuration on every line of a kind, while the server reads the camera line's
/// codecs as what the participant sends and the receive lines' as what it decodes (ADR-0052).
/// The answer only ever names payload types of this offer, which `str0m` accepts.
fn restrict_video_payload_types(
    sdp: &str,
    camera_mid: &Mid,
    payload_types: &[u8],
    receive_lines_keep_them: bool,
) -> String {
    let mut sections: Vec<Vec<&str>> = vec![Vec::new()];
    for line in sdp.lines() {
        if line.starts_with("m=") {
            sections.push(Vec::new());
        }
        if let Some(section) = sections.last_mut() {
            section.push(line);
        }
    }
    let camera_mid_line = format!("a=mid:{camera_mid}");
    let mut restricted = String::with_capacity(sdp.len());
    for section in sections {
        let is_video = section
            .first()
            .is_some_and(|line| line.starts_with("m=video "));
        let is_camera = section.iter().any(|line| *line == camera_mid_line);
        let keeps = |payload_type: u8| {
            if is_camera {
                payload_types.contains(&payload_type)
            } else {
                receive_lines_keep_them || !payload_types.contains(&payload_type)
            }
        };
        for line in section {
            let kept = if !is_video {
                Some(line.to_owned())
            } else if line.starts_with("m=video ") {
                // `m=video <port> <proto> <payload types…>`
                let mut fields: Vec<&str> = line.split(' ').collect();
                let kept_formats: Vec<&str> = fields
                    .split_off(3.min(fields.len()))
                    .into_iter()
                    .filter(|format| format.parse().is_ok_and(keeps))
                    .collect();
                fields.extend(kept_formats);
                Some(fields.join(" "))
            } else {
                let payload_type = ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"]
                    .iter()
                    .find_map(|prefix| line.strip_prefix(prefix))
                    .and_then(|rest| rest.split([' ', ':']).next())
                    .and_then(|payload_type| payload_type.parse::<u8>().ok());
                payload_type.is_none_or(keeps).then(|| line.to_owned())
            };
            if let Some(kept) = kept {
                restricted.push_str(&kept);
                restricted.push_str("\r\n");
            }
        }
    }
    restricted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> PendingPeer {
        offer_with(VideoCodec::Vp8)
    }

    fn offer_with(camera_codec: VideoCodec) -> PendingPeer {
        new_peer(
            "127.0.0.1:40000".parse().unwrap(),
            Instant::now(),
            camera_codec,
        )
        .unwrap()
    }

    /// The `m=` line and attribute lines of the section whose mid is `mid`.
    fn section_of<'a>(sdp: &'a str, mid: &Mid) -> Vec<&'a str> {
        let mut sections: Vec<Vec<&str>> = Vec::new();
        for line in sdp.lines() {
            if line.starts_with("m=") {
                sections.push(Vec::new());
            }
            if let Some(section) = sections.last_mut() {
                section.push(line);
            }
        }
        let mid_line = format!("a=mid:{mid}");
        sections
            .into_iter()
            .find(|section| section.contains(&mid_line.as_str()))
            .unwrap()
    }

    #[test]
    fn an_h264_share_offers_h264_alone_on_the_camera_line_and_nothing_else_changes() {
        let peer = offer_with(VideoCodec::H264);
        let sdp = &peer.sdp_offer;
        let camera = section_of(sdp, &peer.media_lines.camera_send);
        assert!(camera[0].ends_with(" 108 109"), "{camera:?}");
        let rtpmaps: Vec<&&str> = camera
            .iter()
            .filter(|line| line.starts_with("a=rtpmap:"))
            .collect();
        assert_eq!(
            rtpmaps,
            [&"a=rtpmap:108 H264/90000", &"a=rtpmap:109 rtx/90000"]
        );
        let fmtp = camera
            .iter()
            .find(|line| line.starts_with("a=fmtp:108 "))
            .unwrap();
        for parameter in ["packetization-mode=1", "profile-level-id=42e01f"] {
            assert!(fmtp.contains(parameter), "{fmtp}");
        }
        assert!(camera.contains(&"a=simulcast:send q;l;h"));
        for mid in &peer.media_lines.camera_receive {
            let receive = section_of(sdp, mid);
            assert_eq!(
                receive.join("\n").contains("H264"),
                crate::h264::decodes_h264(),
                "{receive:?}"
            );
            assert!(receive.iter().any(|line| line.contains("VP8/90000")));
        }
        // Still the server's shape.
        let sections = media_sections(sdp);
        assert_eq!(sections.len(), 31);
    }

    /// Where the system decodes H.264, an H.264 sharer's receive lines keep it, so others'
    /// H.264 shares still reach it; elsewhere they lose it.
    #[test]
    fn receive_lines_keep_h264_only_when_the_system_decodes_it() {
        let section = |mid: &str| {
            format!(
                "m=video 9 UDP/TLS/RTP/SAVPF 96 97 108 109\r\na=mid:{mid}\r\n\
                 a=rtpmap:96 VP8/90000\r\na=rtpmap:97 rtx/90000\r\n\
                 a=rtpmap:108 H264/90000\r\na=fmtp:108 packetization-mode=1\r\n\
                 a=rtcp-fb:108 nack\r\na=rtpmap:109 rtx/90000\r\n"
            )
        };
        let sdp = format!("v=0\r\n{}{}", section("0"), section("1"));
        let h264 = [H264_PAYLOAD_TYPE, H264_RETRANSMISSION_PAYLOAD_TYPE];
        for receive_lines_keep_them in [false, true] {
            let restricted =
                restrict_video_payload_types(&sdp, &Mid::from("0"), &h264, receive_lines_keep_them);
            let camera = section_of(&restricted, &Mid::from("0"));
            assert_eq!(camera[0], "m=video 9 UDP/TLS/RTP/SAVPF 108 109");
            assert!(!camera.join("\n").contains("VP8"));
            let receive = section_of(&restricted, &Mid::from("1"));
            assert!(receive.contains(&"a=rtpmap:96 VP8/90000"));
            if receive_lines_keep_them {
                assert_eq!(receive[0], "m=video 9 UDP/TLS/RTP/SAVPF 96 97 108 109");
                assert!(receive.contains(&"a=rtcp-fb:108 nack"));
            } else {
                assert_eq!(receive[0], "m=video 9 UDP/TLS/RTP/SAVPF 96 97");
                assert!(!receive.join("\n").contains("H264"));
            }
        }
    }

    /// The `m=` lines with their direction, in order.
    fn media_sections(sdp: &str) -> Vec<(String, String)> {
        let mut sections = Vec::new();
        for line in sdp.lines() {
            if let Some(media) = line.strip_prefix("m=") {
                let kind = media.split(' ').next().unwrap().to_owned();
                sections.push((kind, String::new()));
            } else if let Some(direction) = ["a=sendonly", "a=recvonly", "a=sendrecv"]
                .iter()
                .find(|direction| line == **direction)
                && let Some(section) = sections.last_mut()
            {
                section.1 = direction.trim_start_matches("a=").to_owned();
            }
        }
        sections
    }

    #[test]
    fn the_offer_has_the_server_s_m_line_order() {
        let peer = offer();
        let sections = media_sections(&peer.sdp_offer);
        let mut expected = vec![("audio", "sendonly")];
        expected.extend([("audio", "recvonly"); 16]);
        expected.push(("video", "sendonly"));
        expected.extend([("video", "recvonly"); 9]);
        expected.push(("audio", "sendonly"));
        expected.extend([("audio", "recvonly"); 3]);
        let sections: Vec<(&str, &str)> = sections
            .iter()
            .map(|(kind, direction)| (kind.as_str(), direction.as_str()))
            .collect();
        assert_eq!(sections, expected);
        assert_eq!(peer.media_lines.voice_receive.len(), 16);
        assert_eq!(peer.media_lines.camera_receive.len(), 9);
        assert_eq!(peer.media_lines.screen_sound_receive.len(), 3);
    }

    #[test]
    fn audio_is_opus_111_with_fec_dtx_and_the_audio_level_extension() {
        let sdp = offer().sdp_offer;
        assert!(sdp.contains("a=rtpmap:111 opus/48000/2"), "{sdp}");
        let fmtp = sdp
            .lines()
            .find(|line| line.starts_with("a=fmtp:111 "))
            .unwrap();
        for parameter in ["minptime=10", "useinbandfec=1", "usedtx=1"] {
            assert!(fmtp.contains(parameter), "{fmtp}");
        }
        assert!(sdp.contains("urn:ietf:params:rtp-hdrext:ssrc-audio-level"));
        assert!(
            sdp.contains("transport-wide-cc"),
            "the server's allocation needs it"
        );
        // Only codec lines: the random ICE credentials may spell "VP9" or "AV1".
        assert!(
            sdp.lines()
                .filter(|line| line.starts_with("a=rtpmap:"))
                .all(|line| !line.contains("VP9") && !line.contains("AV1")),
            "VP8 and H.264 only"
        );
        assert_eq!(
            sdp.contains("a=rtpmap:108 H264/90000"),
            crate::h264::decodes_h264(),
            "{sdp}"
        );
        if crate::h264::decodes_h264() {
            assert!(
                sdp.contains("profile-level-id=42e01f") && sdp.contains("packetization-mode=1"),
                "{sdp}"
            );
        }
    }

    #[test]
    fn the_camera_line_is_simulcast_with_the_three_rids() {
        let sdp = offer().sdp_offer;
        for rid in SIMULCAST_RIDS {
            assert!(sdp.contains(&format!("a=rid:{rid} send")), "{sdp}");
        }
        assert!(sdp.contains("a=simulcast:send q;l;h"), "{sdp}");
    }

    /// `str0m` draws each new line's mid at random (3 characters) and once checked it only
    /// against the lines already negotiated, not those added in the same offer: about one offer
    /// in 500 named two of its 31 lines alike, and the server refused it (`InvalidOffer`), which
    /// failed a join or a reconnection. 3 000 lines in one offer collided almost surely.
    #[test]
    fn every_line_of_one_offer_gets_its_own_mid() {
        let mut rtc = Rtc::builder().build(Instant::now());
        let mut change = rtc.sdp_api();
        let mids: Vec<Mid> = (0..3_000)
            .map(|_| change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None))
            .collect();
        let distinct: std::collections::HashSet<&Mid> = mids.iter().collect();
        assert_eq!(distinct.len(), mids.len());
    }

    #[test]
    fn mids_are_told_apart_by_their_place() {
        let peer = offer();
        let lines = &peer.media_lines;
        assert_eq!(
            lines.receive_slot_kind(lines.voice_receive[3]),
            Some(ReceiveSlotKind::Voice)
        );
        assert_eq!(
            lines.receive_slot_kind(lines.camera_receive[0]),
            Some(ReceiveSlotKind::Camera)
        );
        assert_eq!(
            lines.receive_slot_kind(lines.screen_sound_receive[2]),
            Some(ReceiveSlotKind::ScreenSound)
        );
        assert_eq!(lines.receive_slot_kind(lines.voice_send), None);
        assert_eq!(lines.receive_slot_kind(lines.camera_send), None);
        assert_eq!(lines.receive_slot_kind(lines.screen_sound_send), None);
        assert_eq!(lines.audio_receive().count(), 19);
    }
}
