//! The client's SDP offer, in the exact shape the server validates
//! (`crates/mellow-server/src/domain/voice_rules.rs`): the voice line and 16 voice slots, the
//! camera line (VP8, simulcast `h`, `l`, `q`) and 9 camera slots, the screen-sound line and 3
//! screen-sound slots. Order is all that tells voice lines from screen-sound lines, so the mids
//! of each group are kept here.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mellow_protocol::{
    MAXIMUM_SCREEN_SOUND_RECEIVE_SLOTS, MAXIMUM_VIDEO_RECEIVE_SLOTS, MAXIMUM_VOICE_RECEIVE_SLOTS,
};
use str0m::change::SdpPendingOffer;
use str0m::format::{Codec, FormatParams};
use str0m::media::{Direction, Frequency, MediaKind, Mid, Pt, Simulcast, SimulcastLayer};
use str0m::{Candidate, Rtc};

/// Opus's payload type in every browser's offer, which the server answers with (ADR-0030).
pub(crate) const OPUS_PAYLOAD_TYPE: u8 = 111;
/// How often the round trip to the server is reported, as the web samples it.
const STATS_INTERVAL: Duration = Duration::from_secs(2);
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
    pub(crate) camera_receive: Vec<Mid>,
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
/// candidate: no STUN or TURN is needed), and its offer.
pub(crate) fn new_peer(local_address: SocketAddr, now: Instant) -> Result<PendingPeer, OfferError> {
    str0m::crypto::from_feature_flags().install_process_default();
    let mut builder = Rtc::builder()
        .clear_codecs()
        .enable_vp8(true)
        .set_stats_interval(Some(STATS_INTERVAL));
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
    // The camera and screen-sound send lines carry nothing until video comes; the server
    // requires them for the receive lines after them.
    change.add_media(
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
    add_lines(MediaKind::Audio, Direction::SendOnly, 1);
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
    Ok(PendingPeer {
        rtc,
        pending,
        sdp_offer: offer.to_sdp_string(),
        media_lines: MediaLines {
            voice_send,
            voice_receive,
            camera_receive,
            screen_sound_receive,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> PendingPeer {
        new_peer("127.0.0.1:40000".parse().unwrap(), Instant::now()).unwrap()
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
        assert!(!sdp.contains("H264") && !sdp.contains("VP9"), "VP8 only");
    }

    #[test]
    fn the_camera_line_is_simulcast_with_the_three_rids() {
        let sdp = offer().sdp_offer;
        for rid in SIMULCAST_RIDS {
            assert!(sdp.contains(&format!("a=rid:{rid} send")), "{sdp}");
        }
        assert!(sdp.contains("a=simulcast:send q;l;h"), "{sdp}");
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
        assert_eq!(lines.audio_receive().count(), 19);
    }
}
