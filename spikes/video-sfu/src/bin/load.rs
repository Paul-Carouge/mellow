//! Spike load generator for ADR-0035: N headless participants, of which `--cameras` send a
//! synthetic VP8 camera through the SFU, and every one watches up to `--slots` cameras.
//!
//! A camera sends 30 frames per second on each of its `--layers` (`rid:kbit/s`, e.g.
//! `h:1500,l:500,q:150` for three simulcast layers; one layer without a rid means no simulcast).
//! Frame sizes give the layer's bitrate; a keyframe is three times larger, sent first and on each
//! keyframe request (PLI). Every frame carries the send time, so the bots, which share one clock,
//! measure the true one-way latency of every frame they receive, without decoder or jitter buffer.
//!
//! Usage: `load --server http://192.0.2.10:3000 [--bots 10] [--cameras 5] [--slots 9]
//! [--layers h:1500,l:500,q:150] [--seconds 20]`

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use str0m::change::SdpAnswer;
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Rid, Simulcast, SimulcastLayer};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

const FRAMES_PER_SECOND: u32 = 30;
const VIDEO_CLOCK_TICKS_PER_FRAME: u64 = 90_000 / FRAMES_PER_SECOND as u64;
const KEYFRAME_SIZE_FACTOR: usize = 3;
const WARM_UP: Duration = Duration::from_secs(3);
/// VP8 frame tag (3 bytes), then for keyframes the start code and the size (7 bytes).
const VP8_HEADER_BYTES: usize = 10;
const SENT_AT_OFFSET: usize = VP8_HEADER_BYTES;
const MINIMUM_FRAME_BYTES: usize = SENT_AT_OFFSET + 8;
const SOCKET_BUFFER_BYTES: libc::c_int = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
struct Layer {
    /// `None`: the only layer of a camera without simulcast.
    rid: Option<Rid>,
    frame_bytes: usize,
    width: u16,
    height: u16,
}

#[derive(Debug, Default)]
struct BotResult {
    connected: bool,
    frames_sent: u64,
    keyframes_sent: u64,
    frames_received: u64,
    latency_microseconds: Vec<u32>,
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let value_of = |name: &str| {
        arguments
            .iter()
            .position(|a| a == name)
            .and_then(|i| arguments.get(i + 1))
            .cloned()
    };
    let server = value_of("--server").expect("--server http://host:port is required");
    let bots: usize = value_of("--bots").map_or(10, |v| v.parse().expect("--bots"));
    let cameras: usize = value_of("--cameras").map_or(bots, |v| v.parse().expect("--cameras"));
    let seconds: u64 = value_of("--seconds").map_or(20, |v| v.parse().expect("--seconds"));
    // How many cameras each participant can watch at once (receive m-lines in its offer).
    let slots: usize = value_of("--slots").map_or(cameras, |v| v.parse().expect("--slots"));
    let layers = parse_layers(&value_of("--layers").unwrap_or_else(|| "h:1500".to_owned()));

    str0m::crypto::from_feature_flags().install_process_default();

    let server_host: SocketAddr = server
        .trim_start_matches("http://")
        .parse()
        .expect("--server must be http://IP:port");
    let local_ip = local_ip_towards(server_host);
    let all_connected = Arc::new(Barrier::new(bots));
    let measure_for = Duration::from_secs(seconds);

    let threads: Vec<_> = (0..bots)
        .map(|bot_id| {
            let server = server.clone();
            let all_connected = Arc::clone(&all_connected);
            let layers = if bot_id < cameras {
                layers.clone()
            } else {
                Vec::new()
            };
            std::thread::spawn(move || {
                run_bot(
                    u32::try_from(bot_id).expect("bot id"),
                    slots,
                    &server,
                    local_ip,
                    &layers,
                    &all_connected,
                    measure_for,
                )
            })
        })
        .collect();
    let results: Vec<BotResult> = threads
        .into_iter()
        .map(|t| t.join().expect("bot thread"))
        .collect();

    let connected = results.iter().filter(|r| r.connected).count();
    let sent: u64 = results.iter().map(|r| r.frames_sent).sum();
    let keyframes: u64 = results.iter().map(|r| r.keyframes_sent).sum();
    let received: u64 = results.iter().map(|r| r.frames_received).sum();
    // Each bot watches every other camera, up to its slots, at 30 frames/s on one layer.
    let watched_streams: u64 = (0..bots)
        .map(|bot| {
            let others = if bot < cameras { cameras - 1 } else { cameras };
            others.min(slots) as u64
        })
        .sum();
    let expected = watched_streams * u64::from(FRAMES_PER_SECOND) * seconds;
    let mut latencies: Vec<u32> = results
        .into_iter()
        .flat_map(|r| r.latency_microseconds)
        .collect();
    latencies.sort_unstable();
    let percentile = |p: f64| {
        if latencies.is_empty() {
            return 0.0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let index = ((latencies.len() - 1) as f64 * p).round() as usize;
        f64::from(latencies[index]) / 1000.0
    };
    #[allow(clippy::cast_precision_loss)]
    let loss = if expected == 0 {
        100.0
    } else {
        100.0 * (1.0 - received as f64 / expected as f64)
    };
    println!(
        "load: {connected}/{bots} connected, {cameras} cameras, {seconds} s measured | frames sent {sent} (keyframes {keyframes}), received {received} of {expected} expected (missing {loss:.2}%) | one-way latency ms p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2}",
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        latencies.last().map_or(0.0, |l| f64::from(*l) / 1000.0),
    );
}

/// `h:1500,l:500,q:150` gives three simulcast layers; a single `h:1500` gives one layer without
/// simulcast. Sizes follow the layer's position: 720p, 360p, 180p.
fn parse_layers(text: &str) -> Vec<Layer> {
    const SIZES: [(u16, u16); 3] = [(1280, 720), (640, 360), (320, 180)];
    let parts: Vec<&str> = text.split(',').collect();
    let is_simulcast = parts.len() > 1;
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let (rid, kbps) = part.split_once(':').expect("--layers takes rid:kbps,…");
            let kbps: usize = kbps.parse().expect("layer bitrate in kbit/s");
            let (width, height) = SIZES[index.min(SIZES.len() - 1)];
            Layer {
                rid: is_simulcast.then(|| Rid::from(rid)),
                frame_bytes: (kbps * 1000 / 8 / FRAMES_PER_SECOND as usize)
                    .max(MINIMUM_FRAME_BYTES),
                width,
                height,
            }
        })
        .collect()
}

fn local_ip_towards(server: SocketAddr) -> IpAddr {
    let probe = UdpSocket::bind("0.0.0.0:0").expect("probe socket");
    probe.connect(server).expect("probe connect");
    probe.local_addr().expect("probe address").ip()
}

fn enlarge_socket_buffers(socket: &UdpSocket) {
    use std::os::fd::AsRawFd;
    let size = SOCKET_BUFFER_BYTES;
    for option in [libc::SO_RCVBUF, libc::SO_SNDBUF] {
        // SAFETY: setsockopt reads one c_int from a live local for the duration of the call, on a
        // descriptor owned by `socket`.
        #[allow(unsafe_code)]
        unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                option,
                (&raw const size).cast(),
                #[allow(clippy::cast_possible_truncation)]
                {
                    size_of::<libc::c_int>() as libc::socklen_t
                },
            );
        }
    }
}

/// `Instant::elapsed` is zero for a future instant, so the warm-up must be excluded explicitly.
fn is_inside_window(from: Instant, length: Duration) -> bool {
    let now = Instant::now();
    now >= from && now - from <= length
}

fn unix_microseconds() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_micros(),
    )
    .expect("time")
}

/// A frame the VP8 depacketizer accepts: a frame tag (bit 0 clear on keyframes, "show frame"
/// set), then for keyframes the start code and the picture size, then the send time.
fn synthetic_vp8_frame(layer: &Layer, is_keyframe: bool) -> Vec<u8> {
    let length = if is_keyframe {
        layer.frame_bytes * KEYFRAME_SIZE_FACTOR
    } else {
        layer.frame_bytes
    };
    let mut frame = vec![0u8; length];
    frame[0] = if is_keyframe { 0x10 } else { 0x11 };
    if is_keyframe {
        frame[3..6].copy_from_slice(&[0x9d, 0x01, 0x2a]);
        frame[6..8].copy_from_slice(&layer.width.to_le_bytes());
        frame[8..10].copy_from_slice(&layer.height.to_le_bytes());
    }
    frame[SENT_AT_OFFSET..SENT_AT_OFFSET + 8].copy_from_slice(&unix_microseconds().to_be_bytes());
    frame
}

#[allow(clippy::too_many_lines)]
fn run_bot(
    bot_id: u32,
    slots: usize,
    server: &str,
    local_ip: IpAddr,
    layers: &[Layer],
    all_connected: &Barrier,
    measure_for: Duration,
) -> BotResult {
    let mut result = BotResult::default();
    let socket = UdpSocket::bind((local_ip, 0)).expect("bot socket");
    enlarge_socket_buffers(&socket);
    let local_address = socket.local_addr().expect("bot address");
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_vp8(true)
        .build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local_address, "udp").expect("candidate"));

    let mut change = rtc.sdp_api();
    let camera_mid = (!layers.is_empty()).then(|| {
        let simulcast = layers.iter().all(|l| l.rid.is_some()).then(|| {
            let mut simulcast = Simulcast::new();
            for layer in layers {
                simulcast.add_send_layer(SimulcastLayer::new(
                    layer.rid.as_deref().expect("simulcast layer rid"),
                ));
            }
            simulcast
        });
        change.add_media(MediaKind::Video, Direction::SendOnly, None, None, simulcast)
    });
    for _ in 0..slots {
        change.add_media(MediaKind::Video, Direction::RecvOnly, None, None, None);
    }
    let (offer, pending) = change.apply().expect("offer");
    let answer_text = ureq::post(format!("{server}/offer"))
        .send(offer.to_sdp_string())
        .expect("signalling request")
        .body_mut()
        .read_to_string()
        .expect("answer body");
    let answer = SdpAnswer::from_sdp_string(&answer_text).expect("answer");
    rtc.sdp_api()
        .accept_answer(pending, answer)
        .expect("answer accepted");

    let mut buffer = vec![0u8; 2000];
    let mut is_connected = false;
    let mut waited_at_barrier = false;
    let started = Instant::now();
    let mut measuring_from: Option<Instant> = None;
    let mut next_frame_at = Instant::now();
    let mut rtp_time: u64 = 0;
    // One flag per layer: the next frame of that layer is a keyframe.
    let mut is_keyframe_due = vec![true; layers.len()];

    loop {
        let timeout = loop {
            match rtc.poll_output() {
                Ok(Output::Timeout(at)) => break at,
                Ok(Output::Transmit(transmit)) => {
                    let _ = socket.send_to(&transmit.contents, transmit.destination);
                }
                // DTLS is done: the writer can send media.
                Ok(Output::Event(Event::Connected)) => is_connected = true,
                Ok(Output::Event(Event::KeyframeRequest(request))) => {
                    for (due, layer) in is_keyframe_due.iter_mut().zip(layers) {
                        if request.rid.is_none() || request.rid == layer.rid {
                            *due = true;
                        }
                    }
                }
                Ok(Output::Event(Event::MediaData(frame))) => {
                    if let Some(from) = measuring_from
                        && is_inside_window(from, measure_for)
                        && frame.data.len() >= MINIMUM_FRAME_BYTES
                    {
                        let sent_at = u64::from_be_bytes(
                            frame.data[SENT_AT_OFFSET..SENT_AT_OFFSET + 8]
                                .try_into()
                                .expect("8 bytes"),
                        );
                        result.frames_received += 1;
                        result.latency_microseconds.push(
                            u32::try_from(unix_microseconds().saturating_sub(sent_at))
                                .unwrap_or(u32::MAX),
                        );
                    }
                }
                Ok(Output::Event(_)) => {}
                Err(error) => panic!("bot {bot_id}: {error}"),
            }
        };

        // Wait until everyone is connected (and cameras have a codec), then measure together.
        let can_send = camera_mid.is_none_or(|mid| rtc.writer(mid).is_some());
        if is_connected && !waited_at_barrier && can_send {
            waited_at_barrier = true;
            result.connected = true;
            all_connected.wait();
            measuring_from = Some(Instant::now() + WARM_UP);
            next_frame_at = Instant::now();
        }
        if !waited_at_barrier && started.elapsed() > Duration::from_secs(15) {
            eprintln!("bot {bot_id}: never connected");
            all_connected.wait();
            return result;
        }
        if let Some(from) = measuring_from
            && from.elapsed() > measure_for + Duration::from_secs(1)
            && Instant::now() > from
        {
            return result;
        }

        let now = Instant::now();
        if waited_at_barrier
            && let Some(mid) = camera_mid
            && now >= next_frame_at
        {
            let is_measuring =
                measuring_from.is_some_and(|from| is_inside_window(from, measure_for));
            for (layer, is_due) in layers.iter().zip(is_keyframe_due.iter_mut()) {
                let frame = synthetic_vp8_frame(layer, *is_due);
                let Some(writer) = rtc.writer(mid) else {
                    break;
                };
                let pt = writer
                    .payload_params()
                    .find(|p| p.spec().codec == Codec::Vp8)
                    .map(|p| p.pt())
                    .expect("VP8 negotiated");
                let writer = match &layer.rid {
                    Some(rid) => writer.rid(*rid),
                    None => writer,
                };
                writer
                    .write(
                        pt,
                        now,
                        MediaTime::new(rtp_time, Frequency::NINETY_KHZ),
                        frame,
                    )
                    .expect("write frame");
                if is_measuring {
                    result.frames_sent += 1;
                    result.keyframes_sent += u64::from(*is_due);
                }
                *is_due = false;
            }
            rtp_time += VIDEO_CLOCK_TICKS_PER_FRAME;
            next_frame_at += Duration::from_secs(1) / FRAMES_PER_SECOND;
            continue;
        }

        let wake_at = match (waited_at_barrier, camera_mid) {
            (true, Some(_)) => timeout.min(next_frame_at),
            _ => timeout,
        };
        let wait = wake_at
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        socket.set_read_timeout(Some(wait)).expect("read timeout");
        let input = match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                let Ok(contents) = buffer[..length].try_into() else {
                    continue;
                };
                Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: local_address,
                        contents,
                    },
                )
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                Input::Timeout(Instant::now())
            }
            Err(error) => panic!("bot {bot_id}: UDP read failed: {error}"),
        };
        if let Err(error) = rtc.handle_input(input) {
            panic!("bot {bot_id}: {error}");
        }
    }
}
