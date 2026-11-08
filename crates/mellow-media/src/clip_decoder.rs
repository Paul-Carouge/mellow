//! Audio attachments decoded for playback (ADR-0038, ADR-0045): the instance stores most sounds
//! as Ogg Opus, read here with the `ogg` crate and libopus; the types it leaves as uploaded (low
//! bitrate MP3 and M4A, ADTS AAC, and files from before ADR-0045) go through symphonia, with the
//! instance's own guards (`domain::audio_transcoding`): only the codec the sniffed container
//! usually holds, bounded packets, one or two channels. Ogg Vorbis and anything else answers
//! [`ClipError::Unsupported`], which readers see as "download it instead".
//!
//! Decoding is incremental: one packet at a time, so memory follows the file, never its decoded
//! length.

use std::io::Cursor;
use std::sync::Arc;

use ogg::reading::PacketReader;
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_PCM_ALAW, CODEC_ID_PCM_F32LE,
    CODEC_ID_PCM_F64LE, CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24LE,
    CODEC_ID_PCM_S32LE, CODEC_ID_PCM_U8,
};
use symphonia::core::codecs::audio::{AudioCodecId, AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

use crate::audio_format::{MAXIMUM_SAMPLES_PER_OPUS_PACKET, VOICE_SAMPLE_RATE};

/// The audio types the instance serves inline (ADR-0038's `PLAYABLE_MEDIA_TYPES`), the web's
/// `PLAYABLE_AUDIO_TYPES`.
pub const PLAYABLE_ATTACHMENT_AUDIO_TYPES: [&str; 6] = [
    "audio/mpeg",
    "audio/aac",
    "audio/mp4",
    "audio/ogg",
    "audio/wav",
    "audio/flac",
];

/// Rates a recorder or a phone writes; anything else is refused before decoding.
const SUPPORTED_SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;
/// The most frames one packet may declare or decode to: FLAC's ceiling, as on the instance.
const MAXIMUM_FRAMES_PER_PACKET: u64 = 65_536;
/// No real packet comes near this; a larger one is a hostile file.
const MAXIMUM_PACKET_BYTES: usize = 1024 * 1024;
/// `OpusHead` (RFC 7845): magic, version, channels, pre-skip, rate, gain, mapping family.
const OPUS_HEAD_BYTES: usize = 19;
const OGG_CAPTURE_PATTERN: &[u8; 4] = b"OggS";
/// A page header's granule position sits after the pattern, version and header type.
const OGG_GRANULE_OFFSET: usize = 6;
/// "No packet ends on this page" (RFC 3533).
const OGG_NO_GRANULE: u64 = u64::MAX;

/// Why a file is not played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    /// A type, codec or layout this app does not decode: the reader downloads it instead.
    #[error("this audio file cannot be played here")]
    Unsupported,
    /// The file stops making sense part way.
    #[error("this audio file is damaged")]
    Corrupt,
}

impl From<symphonia::core::errors::Error> for ClipError {
    fn from(error: symphonia::core::errors::Error) -> Self {
        match error {
            symphonia::core::errors::Error::Unsupported(_) => Self::Unsupported,
            _ => Self::Corrupt,
        }
    }
}

/// A sound being decoded, one packet at a time.
pub(crate) trait ClipDecoder: Send {
    fn sample_rate(&self) -> u32;
    /// One or two.
    fn channels(&self) -> usize;
    /// Frames (samples per channel) in the whole sound, when the file says.
    fn duration_frames(&self) -> Option<u64>;
    /// Appends the next interleaved samples to `samples` (possibly none, while a seek skips
    /// ahead); false at the end of the sound.
    fn decode_next(&mut self, samples: &mut Vec<f32>) -> Result<bool, ClipError>;
    /// The next samples decoded start at frame `frame`.
    fn seek(&mut self, frame: u64) -> Result<(), ClipError>;
}

/// Checks the type and the file's first bytes, then reads its headers; nothing is decoded yet.
///
/// # Errors
/// `Unsupported` for a type, codec or layout not played here; `Corrupt` for unreadable headers.
pub(crate) fn open_clip(
    bytes: Arc<[u8]>,
    content_type: &str,
) -> Result<Box<dyn ClipDecoder>, ClipError> {
    if !PLAYABLE_ATTACHMENT_AUDIO_TYPES.contains(&content_type)
        || !starts_like(&bytes, content_type)
    {
        return Err(ClipError::Unsupported);
    }
    if content_type == "audio/ogg" {
        Ok(Box::new(OggOpusClip::open(bytes)?))
    } else {
        Ok(Box::new(SymphoniaClip::open(bytes, content_type)?))
    }
}

/// Whether the first bytes are those of the type the instance sniffed (ADR-0038): a file whose
/// type was set by something else is never handed to a decoder.
fn starts_like(bytes: &[u8], content_type: &str) -> bool {
    let is_mpeg_frame = |at: usize| {
        bytes.get(at) == Some(&0xFF) && bytes.get(at + 1).is_some_and(|byte| byte & 0xE0 == 0xE0)
    };
    match content_type {
        "audio/ogg" => bytes.starts_with(OGG_CAPTURE_PATTERN),
        "audio/wav" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "audio/flac" => bytes.starts_with(b"fLaC"),
        "audio/mpeg" => bytes.starts_with(b"ID3") || is_mpeg_frame(0),
        "audio/aac" => {
            bytes.first() == Some(&0xFF) && bytes.get(1).is_some_and(|byte| byte & 0xF6 == 0xF0)
        }
        "audio/mp4" => bytes.get(4..8) == Some(b"ftyp"),
        _ => false,
    }
}

/// Keeps the frames of `decoded` (interleaved, `channels` wide, starting at frame `first`) that
/// fall in `wanted`, appending them to `samples`.
fn append_frames_within(
    decoded: &[f32],
    channels: usize,
    first: u64,
    wanted: std::ops::Range<u64>,
    samples: &mut Vec<f32>,
) {
    let count = (decoded.len() / channels) as u64;
    let start = wanted.start.clamp(first, first + count) - first;
    let end = wanted.end.clamp(first, first + count) - first;
    if start < end {
        let (start, end) = (
            usize::try_from(start).unwrap_or(usize::MAX),
            usize::try_from(end).unwrap_or(usize::MAX),
        );
        samples.extend_from_slice(&decoded[start * channels..end * channels]);
    }
}

// ---- Ogg Opus ----

/// RFC 7845 with channel mapping family 0 (mono or stereo), what the instance writes.
struct OggOpusClip {
    bytes: Arc<[u8]>,
    reader: PacketReader<Cursor<Arc<[u8]>>>,
    decoder: opus::Decoder,
    serial: u32,
    channels: usize,
    pre_skip: u64,
    duration_frames: Option<u64>,
    /// Frames decoded since the start of the stream, pre-skip included.
    decoded_frames: u64,
    /// The first frame (after pre-skip) to hand out: the target of the last seek.
    first_wanted_frame: u64,
    output: Vec<f32>,
}

impl OggOpusClip {
    fn open(bytes: Arc<[u8]>) -> Result<Self, ClipError> {
        let mut reader = PacketReader::new(Cursor::new(Arc::clone(&bytes)));
        let head = reader
            .read_packet()
            .map_err(|_| ClipError::Corrupt)?
            .ok_or(ClipError::Corrupt)?;
        // Vorbis, or an Opus stream with another channel mapping, is not decoded here.
        let data = &head.data;
        if data.len() < OPUS_HEAD_BYTES
            || !data.starts_with(b"OpusHead")
            || data[8] >> 4 != 0
            || data[18] != 0
        {
            return Err(ClipError::Unsupported);
        }
        let (channels, opus_channels) = match data[9] {
            1 => (1, opus::Channels::Mono),
            2 => (2, opus::Channels::Stereo),
            _ => return Err(ClipError::Unsupported),
        };
        let pre_skip = u64::from(u16::from_le_bytes([data[10], data[11]]));
        let serial = head.stream_serial();
        // The comment header follows; its content is not needed.
        reader.read_packet().map_err(|_| ClipError::Corrupt)?;
        let duration_frames = last_granule(&bytes, serial)
            .map(|granule| granule.saturating_sub(pre_skip))
            .filter(|frames| *frames > 0);
        Ok(Self {
            decoder: opus::Decoder::new(VOICE_SAMPLE_RATE, opus_channels)
                .map_err(|_| ClipError::Corrupt)?,
            bytes,
            reader,
            serial,
            channels,
            pre_skip,
            duration_frames,
            decoded_frames: 0,
            first_wanted_frame: 0,
            output: vec![0.0; MAXIMUM_SAMPLES_PER_OPUS_PACKET * channels],
        })
    }
}

/// The granule position of the stream's last page: its length in 48 kHz frames plus pre-skip.
fn last_granule(bytes: &[u8], serial: u32) -> Option<u64> {
    let mut end = bytes.len();
    while let Some(start) = bytes[..end]
        .windows(OGG_CAPTURE_PATTERN.len())
        .rposition(|window| window == OGG_CAPTURE_PATTERN)
    {
        let header = bytes.get(start..start + 18)?;
        let granule = u64::from_le_bytes(
            header[OGG_GRANULE_OFFSET..OGG_GRANULE_OFFSET + 8]
                .try_into()
                .ok()?,
        );
        let page_serial = u32::from_le_bytes(header[14..18].try_into().ok()?);
        if page_serial == serial && granule != OGG_NO_GRANULE {
            return Some(granule);
        }
        end = start;
    }
    None
}

impl ClipDecoder for OggOpusClip {
    fn sample_rate(&self) -> u32 {
        VOICE_SAMPLE_RATE
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn duration_frames(&self) -> Option<u64> {
        self.duration_frames
    }

    fn decode_next(&mut self, samples: &mut Vec<f32>) -> Result<bool, ClipError> {
        loop {
            let Some(packet) = self.reader.read_packet().map_err(|_| ClipError::Corrupt)? else {
                return Ok(false);
            };
            if packet.stream_serial() != self.serial {
                continue;
            }
            let frames = self
                .decoder
                .decode_float(&packet.data, &mut self.output, false)
                .map_err(|_| ClipError::Corrupt)?;
            let first = self.decoded_frames;
            self.decoded_frames += frames as u64;
            // Pre-skip is the encoder's priming; past the last granule is padding.
            let wanted_end = self
                .duration_frames
                .map_or(u64::MAX, |duration| self.pre_skip + duration);
            append_frames_within(
                &self.output[..frames * self.channels],
                self.channels,
                first,
                self.pre_skip + self.first_wanted_frame..wanted_end,
                samples,
            );
            return Ok(self.decoded_frames < wanted_end);
        }
    }

    // shortcut: a seek decodes again from the start (libopus decodes a 10-minute file in well under
    // a second); bisect pages with `PacketReader::seek_absgp` if long Opus files show up.
    fn seek(&mut self, frame: u64) -> Result<(), ClipError> {
        let mut reopened = Self::open(Arc::clone(&self.bytes))?;
        reopened.first_wanted_frame = frame;
        *self = reopened;
        Ok(())
    }
}

// ---- symphonia: WAV, FLAC, MP3, AAC, M4A ----

struct SymphoniaClip {
    bytes: Arc<[u8]>,
    content_type: String,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
    duration_frames: Option<u64>,
    /// Frames still to drop after a seek landed before its target.
    frames_to_skip: u64,
    decoded: Vec<f32>,
}

impl SymphoniaClip {
    fn open(bytes: Arc<[u8]>, content_type: &str) -> Result<Self, ClipError> {
        let source = MediaSourceStream::new(
            Box::new(Cursor::new(Arc::clone(&bytes))),
            MediaSourceStreamOptions::default(),
        );
        let format = symphonia::default::get_probe().probe(
            &Hint::new(),
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(ClipError::Unsupported)?;
        let track_id = track.id;
        let duration_frames = track.num_frames;
        let parameters = track
            .codec_params
            .as_ref()
            .and_then(|parameters| parameters.audio())
            .ok_or(ClipError::Unsupported)?
            .clone();
        let sample_rate = parameters.sample_rate.ok_or(ClipError::Unsupported)?;
        let channels = parameters
            .channels
            .as_ref()
            .map_or(0, symphonia::core::audio::Channels::count);
        // The instance's rule: the usual codec of the sniffed container, bounded packets. An M4A
        // declaring raw PCM could size symphonia's buffers from any `u32`.
        let is_taken = (1..=2).contains(&channels)
            && SUPPORTED_SAMPLE_RATES.contains(&sample_rate)
            && is_codec_expected_for_type(parameters.codec, content_type)
            && parameters
                .max_frames_per_packet
                .is_none_or(|frames| frames <= MAXIMUM_FRAMES_PER_PACKET);
        if !is_taken {
            return Err(ClipError::Unsupported);
        }
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&parameters, &AudioDecoderOptions::default())?;
        Ok(Self {
            bytes,
            content_type: content_type.to_owned(),
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            duration_frames,
            frames_to_skip: 0,
            decoded: Vec::new(),
        })
    }
}

/// The codec the sniffed container type normally holds (`is_codec_expected_for_sniffed_type`
/// on the instance).
fn is_codec_expected_for_type(codec: AudioCodecId, content_type: &str) -> bool {
    match content_type {
        "audio/wav" => [
            CODEC_ID_PCM_U8,
            CODEC_ID_PCM_S16LE,
            CODEC_ID_PCM_S24LE,
            CODEC_ID_PCM_S32LE,
            CODEC_ID_PCM_F32LE,
            CODEC_ID_PCM_F64LE,
            CODEC_ID_PCM_ALAW,
            CODEC_ID_PCM_MULAW,
        ]
        .contains(&codec),
        "audio/flac" => codec == CODEC_ID_FLAC,
        "audio/mpeg" => codec == CODEC_ID_MP3,
        "audio/aac" | "audio/mp4" => codec == CODEC_ID_AAC,
        _ => false,
    }
}

impl ClipDecoder for SymphoniaClip {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn duration_frames(&self) -> Option<u64> {
        self.duration_frames
    }

    fn decode_next(&mut self, samples: &mut Vec<f32>) -> Result<bool, ClipError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(false),
                // A file of undeclared length ends by running out of bytes.
                Err(symphonia::core::errors::Error::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            if packet.data.len() > MAXIMUM_PACKET_BYTES {
                return Err(ClipError::Corrupt);
            }
            let buffer = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                // A damaged frame is skipped, as browsers play the frames they can read.
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(error) => return Err(error.into()),
            };
            // A layout changing midway (HE-AAC decoded as its core, MP3 frames turning mono).
            if buffer.spec().rate() != self.sample_rate
                || buffer.spec().channels().count() != self.channels
                || buffer.frames() as u64 > MAXIMUM_FRAMES_PER_PACKET
            {
                return Err(ClipError::Corrupt);
            }
            buffer.copy_to_vec_interleaved(&mut self.decoded);
            append_frames_within(
                &self.decoded,
                self.channels,
                0,
                self.frames_to_skip..u64::MAX,
                samples,
            );
            let frames = (self.decoded.len() / self.channels) as u64;
            self.frames_to_skip = self.frames_to_skip.saturating_sub(frames);
            return Ok(true);
        }
    }

    fn seek(&mut self, frame: u64) -> Result<(), ClipError> {
        // Rounded up, so the reader's timestamp, rounded down, lands on the frame itself.
        let nanoseconds =
            (u128::from(frame) * 1_000_000_000).div_ceil(u128::from(self.sample_rate));
        let time = Time::try_from_nanos_u128(nanoseconds).ok_or(ClipError::Corrupt)?;
        let time_base = self
            .format
            .tracks()
            .iter()
            .find(|track| track.id == self.track_id)
            .and_then(|track| track.time_base);
        let sought = self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: Some(self.track_id),
            },
        );
        if let (Ok(sought), Some(time_base)) = (sought, time_base) {
            self.decoder.reset();
            // The reader lands at or before the target: the frames in between are dropped.
            let ticks = sought
                .required_ts
                .get()
                .saturating_sub(sought.actual_ts.get())
                .max(0)
                .unsigned_abs();
            self.frames_to_skip = u64::try_from(
                u128::from(ticks)
                    * u128::from(time_base.numer.get())
                    * u128::from(self.sample_rate)
                    / u128::from(time_base.denom.get()),
            )
            .unwrap_or(u64::MAX);
            return Ok(());
        }
        // A reader that cannot seek (ADTS): decode again from the start up to the target.
        let mut reopened = Self::open(Arc::clone(&self.bytes), &self.content_type)?;
        reopened.frames_to_skip = frame;
        *self = reopened;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A WAV of a 440 Hz tone, 16-bit PCM.
    pub(crate) fn tone_wav(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let mut data = Vec::new();
        for frame in 0..frames {
            #[allow(clippy::cast_possible_truncation)] // In i16 range by construction.
            let sample =
                ((f64::from(frame) * 440.0 * std::f64::consts::TAU / f64::from(sample_rate)).sin()
                    * 8_000.0) as i16;
            for _ in 0..channels {
                data.extend_from_slice(&sample.to_le_bytes());
            }
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + u32::try_from(data.len()).unwrap()).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16_u32.to_le_bytes());
        file.extend_from_slice(&1_u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&sample_rate.to_le_bytes());
        file.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16_u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        file.extend_from_slice(&data);
        file
    }

    /// An Ogg stream whose first packet is `head`, then a comment header, then `packets`; the
    /// last page's granule is `end_granule`.
    fn ogg_file(
        head: &[u8],
        packets: &[Vec<u8>],
        frames_per_packet: u64,
        end_granule: u64,
    ) -> Vec<u8> {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut file = Vec::new();
        let mut writer = PacketWriter::new(&mut file);
        writer
            .write_packet(head.to_vec(), 7, PacketWriteEndInfo::EndPage, 0)
            .unwrap();
        writer
            .write_packet(
                b"OpusTags\0\0\0\0\0\0\0\0".to_vec(),
                7,
                PacketWriteEndInfo::EndPage,
                0,
            )
            .unwrap();
        for (index, packet) in packets.iter().enumerate() {
            let is_last = index + 1 == packets.len();
            let granule = if is_last {
                end_granule
            } else {
                (index as u64 + 1) * frames_per_packet
            };
            writer
                .write_packet(
                    packet.clone(),
                    7,
                    if is_last {
                        PacketWriteEndInfo::EndStream
                    } else {
                        PacketWriteEndInfo::NormalPacket
                    },
                    granule,
                )
                .unwrap();
        }
        drop(writer);
        file
    }

    fn opus_head(channels: u8, pre_skip: u16) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.push(1);
        head.push(channels);
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&48_000_u32.to_le_bytes());
        head.extend_from_slice(&0_u16.to_le_bytes());
        head.push(0);
        head
    }

    /// An Ogg Opus file of `packets` 20 ms stereo packets, cut to `frames` after `pre_skip`.
    pub(crate) fn tone_ogg_opus(packets: usize, pre_skip: u16, frames: u64) -> Vec<u8> {
        let mut encoder =
            opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio).unwrap();
        let pcm = vec![0.2_f32; 960 * 2];
        let opus_packets: Vec<Vec<u8>> = (0..packets)
            .map(|_| encoder.encode_vec_float(&pcm, 4_000).unwrap())
            .collect();
        ogg_file(
            &opus_head(2, pre_skip),
            &opus_packets,
            960,
            u64::from(pre_skip) + frames,
        )
    }

    fn decode_all(clip: &mut dyn ClipDecoder) -> usize {
        let mut samples = Vec::new();
        while clip.decode_next(&mut samples).unwrap() {}
        samples.len() / clip.channels()
    }

    #[test]
    fn ogg_opus_plays_from_after_its_pre_skip_to_its_last_granule() {
        let file = tone_ogg_opus(50, 312, 47_000);
        let mut clip = open_clip(file.into(), "audio/ogg").unwrap();
        assert_eq!((clip.sample_rate(), clip.channels()), (48_000, 2));
        assert_eq!(clip.duration_frames(), Some(47_000));
        assert_eq!(decode_all(clip.as_mut()), 47_000);

        clip.seek(40_000).unwrap();
        assert_eq!(decode_all(clip.as_mut()), 7_000);
    }

    #[test]
    fn a_wav_decodes_to_its_length_and_seeks_to_the_frame() {
        let file = tone_wav(44_100, 2, 44_100);
        let mut clip = open_clip(file.into(), "audio/wav").unwrap();
        assert_eq!((clip.sample_rate(), clip.channels()), (44_100, 2));
        assert_eq!(clip.duration_frames(), Some(44_100));
        assert_eq!(decode_all(clip.as_mut()), 44_100);

        clip.seek(30_000).unwrap();
        assert_eq!(decode_all(clip.as_mut()), 14_100);
    }

    #[test]
    fn vorbis_surround_and_mislabelled_files_are_not_decoded() {
        let mut vorbis_head = b"\x01vorbis".to_vec();
        vorbis_head.resize(30, 0);
        let vorbis = ogg_file(&vorbis_head, &[vec![0; 10]], 960, 960);
        assert_eq!(
            open_clip(vorbis.into(), "audio/ogg").err(),
            Some(ClipError::Unsupported)
        );
        let mut surround_head = opus_head(6, 0);
        surround_head[18] = 1;
        let surround = ogg_file(&surround_head, &[vec![0; 10]], 960, 960);
        assert_eq!(
            open_clip(surround.into(), "audio/ogg").err(),
            Some(ClipError::Unsupported)
        );
        assert_eq!(
            open_clip(tone_wav(48_000, 6, 480).into(), "audio/wav").err(),
            Some(ClipError::Unsupported)
        );
        // A WAV labelled as FLAC, and a type the instance does not serve inline.
        assert_eq!(
            open_clip(tone_wav(48_000, 1, 480).into(), "audio/flac").err(),
            Some(ClipError::Unsupported)
        );
        assert_eq!(
            open_clip(tone_wav(48_000, 1, 480).into(), "audio/x-matroska").err(),
            Some(ClipError::Unsupported)
        );
    }

    #[test]
    fn a_truncated_header_is_damaged_not_a_crash() {
        assert!(open_clip(b"OggS".to_vec().into(), "audio/ogg").is_err());
        assert!(open_clip(b"RIFF\0\0\0\0WAVE".to_vec().into(), "audio/wav").is_err());
    }
}
