//! Audio attachments transcoded to Ogg Opus after upload (ADR-0045): lossless uploads (WAV, FLAC)
//! and lossy ones above `LOSSY_SOURCE_MAXIMUM_KEPT_BITS_PER_SECOND` are decoded by symphonia,
//! resampled to 48 kHz and encoded by libopus, one packet at a time, so memory stays bounded by the
//! output, never by the decoded length. The caller keeps the upload unless the result is smaller.

use std::borrow::Cow;
use std::io::Cursor;
use std::time::Instant;

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters};
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_PCM_ALAW, CODEC_ID_PCM_F32LE,
    CODEC_ID_PCM_F64LE, CODEC_ID_PCM_MULAW, CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24LE,
    CODEC_ID_PCM_S32LE, CODEC_ID_PCM_U8,
};
use symphonia::core::codecs::audio::{AudioCodecId, AudioCodecParameters, AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

/// Upload types the transcode may take (ADR-0038's sniffed types). Ogg is left alone: it already
/// holds Opus or Vorbis.
pub(crate) const TRANSCODABLE_AUDIO_TYPES: [&str; 5] = [
    "audio/wav",
    "audio/flac",
    "audio/mpeg",
    "audio/aac",
    "audio/mp4",
];
/// What a transcoded file is stored and served as: Ogg Opus (RFC 7845).
pub(crate) const OPUS_CONTENT_TYPE: &str = "audio/ogg";
/// The extension a transcoded file's name takes.
pub(crate) const OPUS_FILE_EXTENSION: &str = "opus";
/// Types decoded whatever their bitrate: nothing is lost by encoding them once.
const LOSSLESS_AUDIO_TYPES: [&str; 2] = ["audio/wav", "audio/flac"];
/// A lossy upload at or under this average bitrate is left as it is: a second lossy generation
/// would cost quality for less than about a third of its size.
const LOSSY_SOURCE_MAXIMUM_KEPT_BITS_PER_SECOND: u64 = 160_000;
const STEREO_OPUS_BITS_PER_SECOND: i32 = 96_000;
const MONO_OPUS_BITS_PER_SECOND: i32 = 64_000;
/// The only rate Opus encodes music at without its own band limit.
const OPUS_SAMPLE_RATE: u32 = 48_000;
/// 20 ms, the frame size libopus is tuned for.
const OPUS_FRAME_SAMPLES_PER_CHANNEL: usize = 960;
/// The largest packet one 20 ms frame can make (RFC 6716, section 3.4).
const MAXIMUM_OPUS_PACKET_BYTES: usize = 1275;
/// Pages of about one second, so a player seeking by bisection reads little.
const OPUS_PACKETS_PER_OGG_PAGE: u64 = 50;
/// Any value does for a file holding one logical stream.
const OGG_STREAM_SERIAL: u32 = 0x4D45_4C4F;
/// libopus's own complexity ceiling: the encode is a background job, quality first.
const OPUS_ENCODER_COMPLEXITY: i32 = 10;
/// Input frames handed to the resampler at a time.
const RESAMPLER_CHUNK_FRAMES: usize = 1024;
/// Sample rates outside this range are not audio anyone shares; they stay as they are. The upper
/// bound also caps the resampling work one second of audio costs.
const SUPPORTED_SOURCE_SAMPLE_RATES: std::ops::RangeInclusive<u32> = 8_000..=192_000;
/// The most frames one packet may declare or decode to: FLAC's ceiling. symphonia sizes its decode
/// buffer from the container's declared frames per packet (MP4 `lpcm` takes any `u32`), so a larger
/// value would let a tiny upload allocate gigabytes and abort the process.
const MAXIMUM_FRAMES_PER_PACKET: u64 = 65_536;
/// The largest packet read: an uncompressed FLAC frame of 65 535 stereo 32-bit samples is about
/// 512 KiB, every other codec taken is far smaller. One packet is decoded and resampled before any
/// budget check, so its size bounds the work and memory between two checks.
const MAXIMUM_PACKET_BYTES: usize = 1024 * 1024;
/// Longest audio transcoded: a few kilobytes of FLAC can hold hours of digital silence.
pub(crate) const MAXIMUM_TRANSCODED_AUDIO_SECONDS: u64 = 2 * 60 * 60;
/// Decoded audio after which the Opus written so far must stay smaller than the audio bytes read so
/// far: input smaller per second than its Opus form (near-silent FLAC) would otherwise cost up to
/// the whole duration budget in CPU for a few kilobytes of upload.
const MINIMUM_SECONDS_DECODED_BEFORE_COMPARING_SIZES: u64 = 30;

/// How much one transcode may take: the decoded length, and the time it may run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AudioTranscodingBudget {
    pub(crate) maximum_seconds: u64,
    pub(crate) deadline: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AudioTranscodingOutcome {
    /// Ogg Opus bytes, strictly smaller than the upload.
    Transcoded(Vec<u8>),
    /// The Opus form was not smaller: the upload stays.
    NotSmaller,
    /// Not worth it or not handled: low-bitrate lossy audio, more than two channels, an unusual
    /// sample rate.
    LeftAsItIs,
    /// Longer than the budget's duration, or past its deadline: stopped where it was.
    OverBudget,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum AudioTranscodingError {
    #[error("not audio this server decodes")]
    Undecodable,
    #[error("the audio could not be resampled or encoded")]
    Encoding,
}

impl From<symphonia::core::errors::Error> for AudioTranscodingError {
    fn from(_: symphonia::core::errors::Error) -> Self {
        Self::Undecodable
    }
}

impl From<opus::Error> for AudioTranscodingError {
    fn from(_: opus::Error) -> Self {
        Self::Encoding
    }
}

/// The upload as Ogg Opus at 96 kbit/s (stereo) or 64 kbit/s (mono), or why it stays as it is.
/// `content_type` is the sniffed one; anything outside `TRANSCODABLE_AUDIO_TYPES` is left alone.
///
/// # Errors
/// `Undecodable` for bytes symphonia cannot read to their end (a corrupt packet included, and a
/// lossless stream shorter than it declares: dropping audio would change the sound), `Encoding`
/// when the resampler or libopus fails.
pub(crate) fn transcode_to_opus(
    upload: &[u8],
    content_type: &str,
    budget: AudioTranscodingBudget,
) -> Result<AudioTranscodingOutcome, AudioTranscodingError> {
    if !TRANSCODABLE_AUDIO_TYPES.contains(&content_type) {
        return Ok(AudioTranscodingOutcome::LeftAsItIs);
    }
    let is_lossless = LOSSLESS_AUDIO_TYPES.contains(&content_type);
    let upload_bytes = u64::try_from(upload.len()).unwrap_or(u64::MAX);
    let OpenedAudioTrack {
        mut format,
        track_id,
        declared_frames,
        parameters,
    } = open_audio_track(upload)?;
    let (Some(source_rate), Some(channels)) = (
        parameters.sample_rate,
        parameters
            .channels
            .as_ref()
            .map(symphonia::core::audio::Channels::count),
    ) else {
        return Err(AudioTranscodingError::Undecodable);
    };
    let opus_channels = match channels {
        1 => opus::Channels::Mono,
        2 => opus::Channels::Stereo,
        // shortcut: more than two channels stay as uploaded; downmix (or Opus mapping family 1)
        // once surround uploads show up.
        _ => return Ok(AudioTranscodingOutcome::LeftAsItIs),
    };
    if !is_track_taken(&parameters, source_rate, content_type) {
        return Ok(AudioTranscodingOutcome::LeftAsItIs);
    }
    let maximum_frames = budget
        .maximum_seconds
        .saturating_mul(u64::from(source_rate));
    let frames_before_comparing_sizes =
        MINIMUM_SECONDS_DECODED_BEFORE_COMPARING_SIZES.saturating_mul(u64::from(source_rate));
    if let Some(frames) = declared_frames {
        if frames > maximum_frames {
            return Ok(AudioTranscodingOutcome::OverBudget);
        }
        // The whole upload (tags and cover art included) bounds its audio from above: under the
        // kept bitrate, so is the audio, and nothing needs decoding.
        if !is_lossless && !is_above_kept_bitrate(upload_bytes, frames, source_rate) {
            return Ok(AudioTranscodingOutcome::LeftAsItIs);
        }
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&parameters, &AudioDecoderOptions::default())?;
    let mut resampler = ResamplerTo48Kilohertz::new(source_rate, channels)?;
    let mut encoder = OggOpusEncoder::new(opus_channels, source_rate)?;
    let mut decoded_samples = Vec::new();
    let mut samples_at_48_kilohertz = Vec::new();
    let mut decoded_frames: u64 = 0;
    let mut audio_bytes_read: u64 = 0;
    while let Some(packet) = next_packet_or_end(format.as_mut())? {
        if Instant::now() >= budget.deadline {
            return Ok(AudioTranscodingOutcome::OverBudget);
        }
        if packet.track_id != track_id {
            continue;
        }
        if packet.data.len() > MAXIMUM_PACKET_BYTES {
            return Err(AudioTranscodingError::Undecodable);
        }
        audio_bytes_read =
            audio_bytes_read.saturating_add(u64::try_from(packet.data.len()).unwrap_or(u64::MAX));
        let buffer = decoder.decode(&packet)?;
        // A stream whose layout changes midway (MP3 frames switching to mono, HE-AAC decoded as
        // its core) would be encoded as noise: it stays as uploaded.
        if buffer.spec().rate() != source_rate || buffer.spec().channels().count() != channels {
            return Err(AudioTranscodingError::Undecodable);
        }
        let packet_frames = u64::try_from(buffer.frames()).unwrap_or(u64::MAX);
        if packet_frames > MAXIMUM_FRAMES_PER_PACKET {
            return Err(AudioTranscodingError::Undecodable);
        }
        decoded_frames = decoded_frames.saturating_add(packet_frames);
        if decoded_frames > maximum_frames {
            return Ok(AudioTranscodingOutcome::OverBudget);
        }
        buffer.copy_to_vec_interleaved(&mut decoded_samples);
        resampler.push(&decoded_samples, &mut samples_at_48_kilohertz)?;
        encoder.encode_whole_frames(&mut samples_at_48_kilohertz)?;
        let written_bytes = u64::try_from(encoder.written_bytes()).unwrap_or(u64::MAX);
        if written_bytes >= upload_bytes
            || (decoded_frames >= frames_before_comparing_sizes
                && written_bytes >= audio_bytes_read)
        {
            return Ok(AudioTranscodingOutcome::NotSmaller);
        }
    }
    // symphonia's FLAC reader skips a frame whose checksum fails instead of failing: a lossless
    // stream shorter than it declares lost audio, and is kept as uploaded rather than with a gap.
    if is_lossless && declared_frames.is_some_and(|frames| frames != decoded_frames) {
        return Err(AudioTranscodingError::Undecodable);
    }
    // The bitrate of the audio alone: cover art must not push a low-bitrate file over the bar.
    if !is_lossless && !is_above_kept_bitrate(audio_bytes_read, decoded_frames, source_rate) {
        return Ok(AudioTranscodingOutcome::LeftAsItIs);
    }
    resampler.finish(&mut samples_at_48_kilohertz)?;
    let opus = encoder.finish(samples_at_48_kilohertz)?;
    Ok(if opus.len() < upload.len() {
        AudioTranscodingOutcome::Transcoded(opus)
    } else {
        AudioTranscodingOutcome::NotSmaller
    })
}

/// The upload's default audio track, ready to read.
struct OpenedAudioTrack<'upload> {
    format: Box<dyn FormatReader + 'upload>,
    track_id: u32,
    declared_frames: Option<u64>,
    parameters: AudioCodecParameters,
}

fn open_audio_track(upload: &[u8]) -> Result<OpenedAudioTrack<'_>, AudioTranscodingError> {
    let source = MediaSourceStream::new(
        Box::new(Cursor::new(upload)),
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
        .ok_or(AudioTranscodingError::Undecodable)?;
    let track_id = track.id;
    let declared_frames = track.num_frames;
    let parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or(AudioTranscodingError::Undecodable)?
        .clone();
    Ok(OpenedAudioTrack {
        format,
        track_id,
        declared_frames,
        parameters,
    })
}

/// The next packet; `None` at the end of the stream, which a file of undeclared length (a WAV
/// written while recording) reaches by running out of bytes.
fn next_packet_or_end(
    format: &mut dyn FormatReader,
) -> Result<Option<symphonia::core::packet::Packet>, AudioTranscodingError> {
    match format.next_packet() {
        Ok(packet) => Ok(packet),
        Err(symphonia::core::errors::Error::IoError(error))
            if error.kind() == std::io::ErrorKind::UnexpectedEof =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Whether the track's rate, codec and declared packet size are ones the transcode takes.
fn is_track_taken(parameters: &AudioCodecParameters, source_rate: u32, content_type: &str) -> bool {
    SUPPORTED_SOURCE_SAMPLE_RATES.contains(&source_rate)
        && is_codec_expected_for_sniffed_type(parameters.codec, content_type)
        && parameters
            .max_frames_per_packet
            .is_none_or(|frames| frames <= MAXIMUM_FRAMES_PER_PACKET)
}

/// Whether the track's codec is one the sniffed container type normally holds. The probe ignores the
/// sniffed type and skips leading ID3 tags, so an `audio/mpeg` upload may hold another container, and
/// an M4A may declare raw PCM (`lpcm`) with sizes no recorder writes: only the usual codec is taken.
fn is_codec_expected_for_sniffed_type(codec: AudioCodecId, content_type: &str) -> bool {
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

/// Whether `bytes` holding `frames` at `rate` average more than the bitrate kept as it is. An empty
/// stream has no bitrate worth a transcode.
fn is_above_kept_bitrate(bytes: u64, frames: u64, rate: u32) -> bool {
    frames > 0
        && u128::from(bytes) * 8 * u128::from(rate)
            > u128::from(LOSSY_SOURCE_MAXIMUM_KEPT_BITS_PER_SECOND) * u128::from(frames)
}

/// Interleaved samples at any rate in, interleaved 48 kHz samples out, with the sinc resampler's
/// start delay dropped and its tail flushed, so the output lasts exactly as long as the input.
struct ResamplerTo48Kilohertz {
    /// `None` when the source is already at 48 kHz.
    resampler: Option<Async<f32>>,
    channels: usize,
    source_rate: u32,
    pending_input: Vec<f32>,
    chunk_output: Vec<f32>,
    delay_frames_to_drop: usize,
    input_frames: u64,
    output_frames: u64,
}

impl ResamplerTo48Kilohertz {
    fn new(source_rate: u32, channels: usize) -> Result<Self, AudioTranscodingError> {
        let resampler = if source_rate == OPUS_SAMPLE_RATE {
            None
        } else {
            Some(
                Async::new_sinc(
                    f64::from(OPUS_SAMPLE_RATE) / f64::from(source_rate),
                    1.0,
                    &SincInterpolationParameters::default(),
                    RESAMPLER_CHUNK_FRAMES,
                    channels,
                    FixedAsync::Input,
                )
                .map_err(|_| AudioTranscodingError::Encoding)?,
            )
        };
        let (chunk_output, delay_frames_to_drop) = match &resampler {
            Some(resampler) => (
                vec![0.0; resampler.output_frames_max() * channels],
                resampler.output_delay(),
            ),
            None => (Vec::new(), 0),
        };
        Ok(Self {
            resampler,
            channels,
            source_rate,
            pending_input: Vec::new(),
            chunk_output,
            delay_frames_to_drop,
            input_frames: 0,
            output_frames: 0,
        })
    }

    /// Resamples every whole chunk `input` completes and appends it to `output`.
    fn push(&mut self, input: &[f32], output: &mut Vec<f32>) -> Result<(), AudioTranscodingError> {
        self.input_frames = self
            .input_frames
            .saturating_add(u64::try_from(input.len() / self.channels).unwrap_or(u64::MAX));
        if self.resampler.is_none() {
            output.extend_from_slice(input);
            return Ok(());
        }
        self.pending_input.extend_from_slice(input);
        // Consumed input is dropped once per push, not per chunk: one move of what is left.
        let mut consumed_samples = 0;
        while self.pending_input.len() - consumed_samples >= RESAMPLER_CHUNK_FRAMES * self.channels
        {
            consumed_samples += self.process_chunk(consumed_samples, None, output)? * self.channels;
        }
        self.pending_input.drain(..consumed_samples);
        Ok(())
    }

    /// Resamples what is left, then runs silence through until the output is as long as the
    /// input, and cuts it there.
    fn finish(&mut self, output: &mut Vec<f32>) -> Result<(), AudioTranscodingError> {
        if self.resampler.is_none() {
            return Ok(());
        }
        let expected_frames = (u128::from(self.input_frames) * u128::from(OPUS_SAMPLE_RATE))
            .div_ceil(u128::from(self.source_rate));
        let expected_frames = u64::try_from(expected_frames).unwrap_or(u64::MAX);
        let remaining = self.pending_input.len() / self.channels;
        self.pending_input
            .resize(RESAMPLER_CHUNK_FRAMES * self.channels, 0.0);
        let mut partial = Some(remaining);
        while self.output_frames < expected_frames {
            let before = self.output_frames;
            self.process_chunk(0, partial, output)?;
            partial = Some(0);
            if self.output_frames == before && self.delay_frames_to_drop == 0 {
                return Err(AudioTranscodingError::Encoding);
            }
        }
        let excess = usize::try_from(self.output_frames - expected_frames).unwrap_or(usize::MAX);
        output.truncate(output.len().saturating_sub(excess * self.channels));
        self.output_frames = expected_frames;
        Ok(())
    }

    /// One call of the resampler on `pending_input` from `start_sample`; the input frames it used.
    fn process_chunk(
        &mut self,
        start_sample: usize,
        partial_frames: Option<usize>,
        output: &mut Vec<f32>,
    ) -> Result<usize, AudioTranscodingError> {
        let Some(resampler) = self.resampler.as_mut() else {
            return Ok(0);
        };
        let pending_input = &self.pending_input[start_sample..];
        let input_frames = pending_input.len() / self.channels;
        let output_capacity = self.chunk_output.len() / self.channels;
        let input = InterleavedSlice::new(pending_input, self.channels, input_frames)
            .map_err(|_| AudioTranscodingError::Encoding)?;
        let mut chunk_output =
            InterleavedSlice::new_mut(&mut self.chunk_output, self.channels, output_capacity)
                .map_err(|_| AudioTranscodingError::Encoding)?;
        let indexing = Indexing {
            input_offset: 0,
            output_offset: 0,
            partial_len: partial_frames,
            active_channels_mask: None,
        };
        let (consumed, produced) = resampler
            .process_into_buffer(&input, &mut chunk_output, Some(&indexing))
            .map_err(|_| AudioTranscodingError::Encoding)?;
        let dropped = produced.min(self.delay_frames_to_drop);
        self.delay_frames_to_drop -= dropped;
        output.extend_from_slice(
            &self.chunk_output[dropped * self.channels..produced * self.channels],
        );
        self.output_frames = self
            .output_frames
            .saturating_add(u64::try_from(produced - dropped).unwrap_or(u64::MAX));
        Ok(consumed)
    }
}

/// libopus behind an Ogg writer (RFC 7845): identification and comment headers on pages of their
/// own, then one 20 ms packet after another; the granule position of each page counts the 48 kHz
/// samples decoded so far, the encoder's look-ahead (pre-skip) included.
struct OggOpusEncoder {
    encoder: opus::Encoder,
    writer: PacketWriter<'static, Vec<u8>>,
    channels: usize,
    pre_skip: u16,
    packet: Vec<u8>,
    /// The packet before the last one encoded, held back: only the last may end the stream.
    held_packet: Option<Vec<u8>>,
    packets_written: u64,
    /// Samples per channel of the upload encoded so far, padding excluded.
    samples_encoded: u64,
}

impl OggOpusEncoder {
    fn new(channels: opus::Channels, source_rate: u32) -> Result<Self, AudioTranscodingError> {
        let mut encoder = opus::Encoder::new(OPUS_SAMPLE_RATE, channels, opus::Application::Audio)?;
        encoder.set_bitrate(opus::Bitrate::Bits(match channels {
            opus::Channels::Mono => MONO_OPUS_BITS_PER_SECOND,
            opus::Channels::Stereo => STEREO_OPUS_BITS_PER_SECOND,
        }))?;
        encoder.set_vbr(true)?;
        encoder.set_complexity(OPUS_ENCODER_COMPLEXITY)?;
        let pre_skip =
            u16::try_from(encoder.get_lookahead()?).map_err(|_| AudioTranscodingError::Encoding)?;
        let channel_count = match channels {
            opus::Channels::Mono => 1_u8,
            opus::Channels::Stereo => 2,
        };
        let mut writer = PacketWriter::new(Vec::new());
        let mut identification = Vec::with_capacity(19);
        identification.extend_from_slice(b"OpusHead");
        identification.push(1);
        identification.push(channel_count);
        identification.extend_from_slice(&pre_skip.to_le_bytes());
        identification.extend_from_slice(&source_rate.to_le_bytes());
        // Output gain 0, channel mapping family 0 (mono or stereo).
        identification.extend_from_slice(&[0, 0, 0]);
        let mut comments = Vec::new();
        comments.extend_from_slice(b"OpusTags");
        let vendor = b"Mellow";
        comments.extend_from_slice(&u32::try_from(vendor.len()).unwrap_or(0).to_le_bytes());
        comments.extend_from_slice(vendor);
        // No user comments: the upload's tags are not carried over (ADR-0045).
        comments.extend_from_slice(&0_u32.to_le_bytes());
        for header in [identification, comments] {
            writer
                .write_packet(header, OGG_STREAM_SERIAL, PacketWriteEndInfo::EndPage, 0)
                .map_err(|_| AudioTranscodingError::Encoding)?;
        }
        Ok(Self {
            encoder,
            writer,
            channels: usize::from(channel_count),
            pre_skip,
            packet: vec![0; MAXIMUM_OPUS_PACKET_BYTES],
            held_packet: None,
            packets_written: 0,
            samples_encoded: 0,
        })
    }

    fn written_bytes(&self) -> usize {
        self.writer.inner().len()
    }

    /// Encodes every whole 20 ms frame at the front of `samples` and removes them.
    fn encode_whole_frames(&mut self, samples: &mut Vec<f32>) -> Result<(), AudioTranscodingError> {
        let frame_samples = OPUS_FRAME_SAMPLES_PER_CHANNEL * self.channels;
        let whole = samples.len() / frame_samples * frame_samples;
        for frame in samples[..whole].chunks_exact(frame_samples) {
            self.encode_frame(frame)?;
        }
        self.samples_encoded += u64::try_from(whole / self.channels).unwrap_or(u64::MAX);
        samples.drain(..whole);
        Ok(())
    }

    fn encode_frame(&mut self, frame: &[f32]) -> Result<(), AudioTranscodingError> {
        let length = self.encoder.encode_float(frame, &mut self.packet)?;
        let packet = self.packet[..length].to_vec();
        if let Some(previous) = self.held_packet.replace(packet) {
            self.packets_written += 1;
            let granule = self.packets_written * OPUS_FRAME_SAMPLES_PER_CHANNEL as u64;
            let end = if self
                .packets_written
                .is_multiple_of(OPUS_PACKETS_PER_OGG_PAGE)
            {
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            self.writer
                .write_packet(previous, OGG_STREAM_SERIAL, end, granule)
                .map_err(|_| AudioTranscodingError::Encoding)?;
        }
        Ok(())
    }

    /// Encodes `remaining` with silence after it for the look-ahead, and ends the stream at the
    /// last real sample. Every sample handed to `encode_whole_frames` counts.
    fn finish(mut self, mut remaining: Vec<f32>) -> Result<Vec<u8>, AudioTranscodingError> {
        let frame_samples = OPUS_FRAME_SAMPLES_PER_CHANNEL * self.channels;
        let real_samples_per_channel = self.samples_encoded
            + u64::try_from(remaining.len() / self.channels).unwrap_or(u64::MAX);
        let padded = (remaining.len() + usize::from(self.pre_skip) * self.channels)
            .div_ceil(frame_samples)
            * frame_samples;
        remaining.resize(padded.max(frame_samples), 0.0);
        for frame in remaining.chunks_exact(frame_samples) {
            self.encode_frame(frame)?;
        }
        let last = self
            .held_packet
            .take()
            .ok_or(AudioTranscodingError::Encoding)?;
        let end_granule = u64::from(self.pre_skip) + real_samples_per_channel;
        self.writer
            .write_packet(
                Cow::Owned(last),
                OGG_STREAM_SERIAL,
                PacketWriteEndInfo::EndStream,
                end_granule,
            )
            .map_err(|_| AudioTranscodingError::Encoding)?;
        Ok(self.writer.into_inner())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::time::Duration;

    const LOW_BITRATE_MP3: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/tone-64kbps-mono.mp3"
    ));
    const HIGH_BITRATE_MP3: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/tones-320kbps-stereo.mp3"
    ));
    const ONE_SECOND_FLAC: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/tone-1s-mono.flac"
    ));
    /// The same AAC stream as `HIGH_BITRATE_M4A`, remuxed as raw ADTS frames.
    const HIGH_BITRATE_ADTS: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/tones-1s-256kbps-stereo.aac"
    ));
    const HIGH_BITRATE_M4A: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/tones-1s-256kbps-stereo.m4a"
    ));
    /// Eight minutes of digital silence in about 54 kB, written to a pipe so its STREAMINFO
    /// declares no length: smaller per second than its own Opus form.
    const SILENCE_FLAC_OF_UNDECLARED_LENGTH: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/audio/silence-8min-mono-undeclared-length.flac"
    ));

    pub(crate) fn generous_budget() -> AudioTranscodingBudget {
        AudioTranscodingBudget {
            maximum_seconds: MAXIMUM_TRANSCODED_AUDIO_SECONDS,
            deadline: Instant::now() + Duration::from_secs(3600),
        }
    }

    /// A 16-bit PCM WAV of `seconds` of a chord, one pair of partials per channel.
    pub(crate) fn chord_wav(sample_rate: u32, channels: u16, seconds: u32) -> Vec<u8> {
        let frames = sample_rate * seconds;
        let mut samples = Vec::new();
        for frame in 0..frames {
            let time = f64::from(frame) / f64::from(sample_rate);
            for channel in 0..channels {
                let frequency = 220.0 * f64::from(channel + 2);
                let value = (time * frequency * std::f64::consts::TAU).sin() * 0.4
                    + (time * 3.0 * frequency * std::f64::consts::TAU).sin() * 0.1;
                #[allow(clippy::cast_possible_truncation)]
                samples.extend_from_slice(&((value * f64::from(i16::MAX)) as i16).to_le_bytes());
            }
        }
        wav(sample_rate, channels, &samples)
    }

    fn wav(sample_rate: u32, channels: u16, data: &[u8]) -> Vec<u8> {
        let data_bytes = u32::try_from(data.len()).unwrap();
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16_u32.to_le_bytes());
        file.extend_from_slice(&1_u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&sample_rate.to_le_bytes());
        file.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16_u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&data_bytes.to_le_bytes());
        file.extend_from_slice(data);
        file
    }

    struct DecodedOggOpus {
        channels: u8,
        pre_skip: u64,
        end_granule: u64,
        decoded_samples_per_channel: u64,
        tags: Vec<u8>,
    }

    /// Reads the file back as a player would: headers, then every packet through libopus.
    fn decode_ogg_opus(file: &[u8]) -> DecodedOggOpus {
        let mut reader = ogg::reading::PacketReader::new(Cursor::new(file));
        let head = reader.read_packet().unwrap().unwrap();
        assert_eq!(&head.data[..8], b"OpusHead");
        let channels = head.data[9];
        let pre_skip = u64::from(u16::from_le_bytes([head.data[10], head.data[11]]));
        let tags = reader.read_packet().unwrap().unwrap().data;
        let mut decoder = opus::Decoder::new(
            OPUS_SAMPLE_RATE,
            if channels == 1 {
                opus::Channels::Mono
            } else {
                opus::Channels::Stereo
            },
        )
        .unwrap();
        let mut output = vec![0.0_f32; 5760 * usize::from(channels)];
        let mut decoded_samples_per_channel = 0;
        let mut end_granule = 0;
        while let Some(packet) = reader.read_packet().unwrap() {
            let samples = decoder
                .decode_float(&packet.data, &mut output, false)
                .unwrap();
            decoded_samples_per_channel += u64::try_from(samples).unwrap();
            end_granule = packet.absgp_page();
            assert_eq!(packet.stream_serial(), OGG_STREAM_SERIAL);
        }
        DecodedOggOpus {
            channels,
            pre_skip,
            end_granule,
            decoded_samples_per_channel,
            tags,
        }
    }

    #[test]
    fn a_wav_becomes_a_smaller_ogg_opus_file_of_the_same_length() {
        let upload = chord_wav(44_100, 2, 5);

        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(&upload, "audio/wav", generous_budget()).unwrap()
        else {
            panic!("a WAV is transcoded");
        };

        assert!(opus.len() * 5 < upload.len(), "{} bytes", opus.len());
        assert_eq!(
            crate::domain::message_rules::sniff_content_type(&opus),
            OPUS_CONTENT_TYPE
        );
        let decoded = decode_ogg_opus(&opus);
        assert_eq!(decoded.channels, 2);
        assert_eq!(
            decoded.end_granule - decoded.pre_skip,
            5 * u64::from(OPUS_SAMPLE_RATE),
            "plays exactly five seconds"
        );
        assert!(
            decoded.decoded_samples_per_channel >= decoded.end_granule,
            "every sample up to the end is in a packet"
        );
        assert_eq!(&decoded.tags[..8], b"OpusTags");
    }

    #[test]
    fn a_codec_other_than_the_sniffed_container_usually_holds_is_left_as_it_is() {
        // An M4A declaring raw PCM could size symphonia's decode buffer from any `u32`; a file the
        // probe reads as another container than sniffed is not taken either.
        let wav = chord_wav(44_100, 2, 1);

        for content_type in ["audio/mp4", "audio/mpeg", "audio/flac"] {
            assert_eq!(
                transcode_to_opus(&wav, content_type, generous_budget()),
                Ok(AudioTranscodingOutcome::LeftAsItIs),
                "{content_type}"
            );
        }
    }

    #[test]
    fn a_mono_wav_at_48_kilohertz_is_encoded_without_resampling_and_stays_mono() {
        let upload = chord_wav(48_000, 1, 3);

        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(&upload, "audio/wav", generous_budget()).unwrap()
        else {
            panic!("a WAV is transcoded");
        };

        let decoded = decode_ogg_opus(&opus);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.end_granule - decoded.pre_skip, 3 * 48_000);
    }

    #[test]
    fn a_clip_shorter_than_one_resampler_chunk_keeps_its_exact_length() {
        let frames = 900_u32;
        let mut upload = chord_wav(22_050, 2, 1);
        let data_bytes = frames * 4;
        upload.truncate(44 + usize::try_from(data_bytes).unwrap());
        upload[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        upload[40..44].copy_from_slice(&data_bytes.to_le_bytes());

        let outcome = transcode_to_opus(&upload, "audio/wav", generous_budget()).unwrap();

        // 3.6 kB of PCM, still under one resampler chunk: only the flush of `finish` runs.
        let AudioTranscodingOutcome::Transcoded(opus) = outcome else {
            panic!("{outcome:?}: the clip is smaller as Opus");
        };
        let decoded = decode_ogg_opus(&opus);
        assert_eq!(
            decoded.end_granule - decoded.pre_skip,
            u64::from(frames * 48_000).div_ceil(22_050)
        );
    }

    #[test]
    fn a_low_bitrate_mp3_is_left_as_it_is_and_a_high_bitrate_one_is_transcoded() {
        assert_eq!(
            transcode_to_opus(LOW_BITRATE_MP3, "audio/mpeg", generous_budget()),
            Ok(AudioTranscodingOutcome::LeftAsItIs)
        );

        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(HIGH_BITRATE_MP3, "audio/mpeg", generous_budget()).unwrap()
        else {
            panic!("a 320 kbit/s MP3 is transcoded");
        };
        assert!(
            opus.len() * 2 < HIGH_BITRATE_MP3.len(),
            "{} bytes",
            opus.len()
        );
        assert_eq!(decode_ogg_opus(&opus).channels, 2);
    }

    #[test]
    fn a_flac_becomes_ogg_opus_of_the_same_length() {
        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(ONE_SECOND_FLAC, "audio/flac", generous_budget()).unwrap()
        else {
            panic!("a FLAC is transcoded");
        };

        assert!(opus.len() < ONE_SECOND_FLAC.len(), "{} bytes", opus.len());
        let decoded = decode_ogg_opus(&opus);
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.end_granule - decoded.pre_skip, 48_000);
    }

    #[test]
    fn a_high_bitrate_adts_aac_is_transcoded() {
        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(HIGH_BITRATE_ADTS, "audio/aac", generous_budget()).unwrap()
        else {
            panic!("a 256 kbit/s ADTS stream is transcoded");
        };

        assert_eq!(decode_ogg_opus(&opus).channels, 2);
    }

    #[test]
    fn a_high_bitrate_m4a_is_transcoded() {
        let AudioTranscodingOutcome::Transcoded(opus) =
            transcode_to_opus(HIGH_BITRATE_M4A, "audio/mp4", generous_budget()).unwrap()
        else {
            panic!("a 256 kbit/s M4A is transcoded");
        };

        assert!(
            opus.len() * 2 < HIGH_BITRATE_M4A.len(),
            "{} bytes",
            opus.len()
        );
        let decoded = decode_ogg_opus(&opus);
        assert_eq!(decoded.channels, 2);
        // symphonia keeps the AAC encoder's priming: a second, give or take one AAC frame or two.
        let played = decoded.end_granule - decoded.pre_skip;
        assert!(
            (48_000..48_000 + 2 * 1024 * 48 / 44).contains(&played),
            "{played}"
        );
    }

    #[test]
    fn a_flac_with_a_corrupt_frame_is_refused_rather_than_played_with_a_gap() {
        let mut upload = ONE_SECOND_FLAC.to_vec();
        let middle = upload.len() / 2;
        for byte in &mut upload[middle..middle + 16] {
            *byte ^= 0xA5;
        }

        assert_eq!(
            transcode_to_opus(&upload, "audio/flac", generous_budget()),
            Err(AudioTranscodingError::Undecodable)
        );
    }

    #[test]
    fn a_stream_switching_from_mono_to_stereo_midway_is_refused() {
        let upload = [LOW_BITRATE_MP3, HIGH_BITRATE_MP3].concat();

        assert_eq!(
            transcode_to_opus(&upload, "audio/mpeg", generous_budget()),
            Err(AudioTranscodingError::Undecodable)
        );
    }

    #[test]
    fn silence_of_undeclared_length_stops_once_its_opus_form_outgrows_the_input_read() {
        // Bounding the work by the whole upload's size would run this to the duration budget:
        // the input read so far is what bounds it.
        let budget = AudioTranscodingBudget {
            maximum_seconds: 2 * MINIMUM_SECONDS_DECODED_BEFORE_COMPARING_SIZES,
            ..generous_budget()
        };

        assert_eq!(
            transcode_to_opus(SILENCE_FLAC_OF_UNDECLARED_LENGTH, "audio/flac", budget),
            Ok(AudioTranscodingOutcome::NotSmaller)
        );
    }

    #[test]
    fn a_low_bitrate_mp3_behind_large_cover_art_is_left_as_it_is() {
        // An ID3v2.3 tag holding a 64 kB picture: the file averages far above the kept bitrate,
        // its audio does not.
        let picture = vec![0x5A_u8; 64 * 1024];
        let mut frame_body = b"\0image/png\0\x03\0".to_vec();
        frame_body.extend_from_slice(&picture);
        let mut tag_body = b"APIC".to_vec();
        tag_body.extend_from_slice(&u32::try_from(frame_body.len()).unwrap().to_be_bytes());
        tag_body.extend_from_slice(&[0, 0]);
        tag_body.extend_from_slice(&frame_body);
        let tag_size = u32::try_from(tag_body.len()).unwrap();
        let syncsafe_size: Vec<u8> = (0..4)
            .rev()
            .map(|shift| u8::try_from((tag_size >> (7 * shift)) & 0x7F).unwrap())
            .collect();
        let mut upload = b"ID3\x03\0\0".to_vec();
        upload.extend_from_slice(&syncsafe_size);
        upload.extend_from_slice(&tag_body);
        upload.extend_from_slice(LOW_BITRATE_MP3);

        assert_eq!(
            transcode_to_opus(&upload, "audio/mpeg", generous_budget()),
            Ok(AudioTranscodingOutcome::LeftAsItIs)
        );
    }

    #[test]
    fn audio_declared_longer_than_the_budget_is_not_decoded() {
        let upload = chord_wav(8_000, 1, 3);
        let budget = AudioTranscodingBudget {
            maximum_seconds: 2,
            ..generous_budget()
        };

        assert_eq!(
            transcode_to_opus(&upload, "audio/wav", budget),
            Ok(AudioTranscodingOutcome::OverBudget)
        );
    }

    #[test]
    fn audio_of_undeclared_length_stops_once_its_decoded_length_passes_the_budget() {
        // A data chunk claiming to run to the end of the file, as streaming recorders write it
        // (and as a file crafted to decode to hours would): the length is learnt by decoding.
        let mut upload = chord_wav(8_000, 1, 3);
        upload[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        upload[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        let budget = AudioTranscodingBudget {
            maximum_seconds: 2,
            ..generous_budget()
        };

        assert_eq!(
            transcode_to_opus(&upload, "audio/wav", budget),
            Ok(AudioTranscodingOutcome::OverBudget)
        );
        assert!(
            matches!(
                transcode_to_opus(&upload, "audio/wav", generous_budget()),
                Ok(AudioTranscodingOutcome::Transcoded(_))
            ),
            "within the budget, the end of the bytes ends the stream"
        );
    }

    #[test]
    fn a_transcode_past_its_deadline_stops() {
        let upload = chord_wav(44_100, 2, 2);
        let budget = AudioTranscodingBudget {
            deadline: Instant::now(),
            ..generous_budget()
        };

        assert_eq!(
            transcode_to_opus(&upload, "audio/wav", budget),
            Ok(AudioTranscodingOutcome::OverBudget)
        );
    }

    #[test]
    fn surround_audio_and_types_outside_the_list_are_left_as_they_are() {
        assert_eq!(
            transcode_to_opus(&chord_wav(48_000, 3, 1), "audio/wav", generous_budget()),
            Ok(AudioTranscodingOutcome::LeftAsItIs)
        );
        assert_eq!(
            transcode_to_opus(&chord_wav(48_000, 2, 1), "audio/ogg", generous_budget()),
            Ok(AudioTranscodingOutcome::LeftAsItIs)
        );
    }

    #[test]
    fn bytes_that_are_not_audio_are_refused() {
        let mut upload = b"RIFF\x24\x00\x00\x00WAVE".to_vec();
        upload.extend_from_slice(&[0x55; 64]);

        assert_eq!(
            transcode_to_opus(&upload, "audio/wav", generous_budget()),
            Err(AudioTranscodingError::Undecodable)
        );
    }
}
