//! Spike load generator: N headless participants that talk all the time through the SFU.
//!
//! Each bot sends one 20 ms frame of `--frame-bytes` (160 bytes = 64 kbit/s, Discord's default
//! voice bitrate) carrying its id and the send time, and measures, for every frame it hears,
//! the time from send to receipt. Bots run on one machine, so they share one clock: the figure
//! is the true one-way latency through the network and the SFU, without codec or jitter buffer.
//!
//! Usage: `load --server http://192.0.2.10:3000 [--bots 10] [--speakers 10] [--slots 9] [--seconds 30] [--frame-bytes 160]`

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use str0m::change::SdpAnswer;
use str0m::format::Codec;
use str0m::media::{Direction, Frequency, MediaKind, MediaTime};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, Input, Output, Rtc};

const FRAME_INTERVAL: Duration = Duration::from_millis(20);
const OPUS_SAMPLES_PER_FRAME: u64 = 960;
const WARM_UP: Duration = Duration::from_secs(3);
const HEADER_BYTES: usize = 16;

#[derive(Debug, Default)]
struct BotResult {
    connected: bool,
    frames_sent: u64,
    frames_heard: u64,
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
    let seconds: u64 = value_of("--seconds").map_or(30, |v| v.parse().expect("--seconds"));
    // Silent participants send nothing, as browsers do with Opus DTX.
    let speakers: usize = value_of("--speakers").map_or(bots, |v| v.parse().expect("--speakers"));
    // How many voices each participant can hear at once (receive m-lines in its offer).
    let slots: usize = value_of("--slots").map_or(bots - 1, |v| v.parse().expect("--slots"));
    let frame_bytes: usize =
        value_of("--frame-bytes").map_or(160, |v| v.parse().expect("--frame-bytes"));
    assert!(
        frame_bytes >= HEADER_BYTES,
        "--frame-bytes must be at least {HEADER_BYTES}"
    );

    str0m::crypto::from_feature_flags().install_process_default();

    let server_host: SocketAddr = server
        .trim_start_matches("http://")
        .parse()
        .expect("--server must be http://IP:port");
    let local_ip = local_ip_towards(server_host);
    let all_connected = Arc::new(Barrier::new(bots));
    let measure_until = Duration::from_secs(seconds);

    let threads: Vec<_> = (0..bots)
        .map(|bot_id| {
            let server = server.clone();
            let all_connected = Arc::clone(&all_connected);
            std::thread::spawn(move || {
                run_bot(
                    u32::try_from(bot_id).expect("bot id"),
                    slots,
                    &server,
                    local_ip,
                    frame_bytes,
                    bot_id < speakers,
                    &all_connected,
                    measure_until,
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
    let heard: u64 = results.iter().map(|r| r.frames_heard).sum();
    #[allow(clippy::cast_possible_truncation)]
    let expected = sent * (connected.saturating_sub(1)) as u64;
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
        100.0 * (1.0 - heard as f64 / expected as f64)
    };
    println!(
        "load: {connected}/{bots} connected, {seconds} s measured | frames sent {sent}, heard {heard} of {expected} expected (loss {loss:.2}%) | one-way latency ms p50 {:.2}, p95 {:.2}, p99 {:.2}, max {:.2}",
        percentile(0.50),
        percentile(0.95),
        percentile(0.99),
        latencies.last().map_or(0.0, |l| f64::from(*l) / 1000.0),
    );
}

fn local_ip_towards(server: SocketAddr) -> IpAddr {
    let probe = UdpSocket::bind("0.0.0.0:0").expect("probe socket");
    probe.connect(server).expect("probe connect");
    probe.local_addr().expect("probe address").ip()
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

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn run_bot(
    bot_id: u32,
    slots: usize,
    server: &str,
    local_ip: IpAddr,
    frame_bytes: usize,
    is_speaker: bool,
    all_connected: &Barrier,
    measure_for: Duration,
) -> BotResult {
    let mut result = BotResult::default();
    let socket = UdpSocket::bind((local_ip, 0)).expect("bot socket");
    let local_address = socket.local_addr().expect("bot address");
    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local_address, "udp").expect("candidate"));

    let mut change = rtc.sdp_api();
    let voice_mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    for _ in 0..slots {
        change.add_media(MediaKind::Audio, Direction::RecvOnly, None, None, None);
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
    let mut sequence: u32 = 0;
    let mut rtp_time: u64 = 0;

    loop {
        let timeout = loop {
            match rtc.poll_output() {
                Ok(Output::Timeout(at)) => break at,
                Ok(Output::Transmit(transmit)) => {
                    let _ = socket.send_to(&transmit.contents, transmit.destination);
                }
                // DTLS is done: the writer can send media.
                Ok(Output::Event(Event::Connected)) => {
                    is_connected = true;
                }
                Ok(Output::Event(Event::MediaData(frame))) => {
                    if let Some(from) = measuring_from
                        && is_inside_window(from, measure_for)
                        && frame.data.len() >= HEADER_BYTES
                    {
                        let sent_at =
                            u64::from_be_bytes(frame.data[8..16].try_into().expect("8 bytes"));
                        let latency = unix_microseconds().saturating_sub(sent_at);
                        result.frames_heard += 1;
                        result
                            .latency_microseconds
                            .push(u32::try_from(latency).unwrap_or(u32::MAX));
                    }
                }
                Ok(Output::Event(_)) => {}
                Err(error) => panic!("bot {bot_id}: {error}"),
            }
        };

        // Wait until everyone is connected and has a codec, then measure together.
        if is_connected && !waited_at_barrier && rtc.writer(voice_mid).is_some() {
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
        if waited_at_barrier && is_speaker && now >= next_frame_at {
            let mut payload = vec![0u8; frame_bytes];
            payload[0..4].copy_from_slice(&bot_id.to_be_bytes());
            payload[4..8].copy_from_slice(&sequence.to_be_bytes());
            payload[8..16].copy_from_slice(&unix_microseconds().to_be_bytes());
            if let Some(writer) = rtc.writer(voice_mid) {
                let pt = writer
                    .payload_params()
                    .find(|p| p.spec().codec == Codec::Opus)
                    .map(|p| p.pt())
                    .expect("opus negotiated");
                writer
                    .write(
                        pt,
                        now,
                        MediaTime::new(rtp_time, Frequency::FORTY_EIGHT_KHZ),
                        payload,
                    )
                    .expect("write frame");
            }
            if measuring_from.is_some_and(|from| is_inside_window(from, measure_for)) {
                result.frames_sent += 1;
            }
            sequence = sequence.wrapping_add(1);
            rtp_time += OPUS_SAMPLES_PER_FRAME;
            next_frame_at += FRAME_INTERVAL;
            continue;
        }

        let wake_at = if waited_at_barrier {
            timeout.min(next_frame_at)
        } else {
            timeout
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
