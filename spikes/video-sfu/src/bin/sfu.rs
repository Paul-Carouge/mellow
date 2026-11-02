//! Spike SFU for ADR-0035: one room, VP8 video only, one UDP port, ICE-lite, signalling by a
//! single HTTP POST.
//!
//! Each client offers at most one `sendonly` video m-line (its camera, simulcast or not) and K
//! `recvonly` ones (slots for the cameras it watches). The SFU forwards VP8 frames as they are,
//! never decodes them, and picks one simulcast layer per watcher and camera: the "focus" camera
//! (the active speaker of a speaker view) gets `--focus-layer`, the others `--other-layer`. The
//! focus moves to the next camera every `--focus-seconds`, which makes watchers switch layers. A
//! watcher only gets a layer from one of its keyframes: the SFU asks the camera for one (PLI) and
//! keeps forwarding the old layer until it arrives.
//!
//! Every 5 seconds it prints its CPU use, its UDP traffic, the time a frame spends inside it and
//! how long layer switches waited for their keyframe.
//!
//! Usage: `sfu [--ip 192.0.2.10] [--udp-port 50000] [--http-port 3000] [--focus-layer h]
//! [--other-layer q] [--focus-seconds 4] [--bwe-desired-kbps 4000]`

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use str0m::bwe::{Bitrate, BweKind};
use str0m::change::SdpOffer;
use str0m::media::{Direction, KeyframeRequestKind, MediaData, MediaKind, Mid, Rid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

const REPORT_INTERVAL: Duration = Duration::from_secs(5);
const MAXIMUM_UDP_DATAGRAM_BYTES: usize = 2000;
/// A 720p keyframe is a burst of dozens of packets per camera: default socket buffers drop some.
const SOCKET_BUFFER_BYTES: libc::c_int = 4 * 1024 * 1024;
/// At most one keyframe request per camera layer in this time, as browsers do.
const KEYFRAME_REQUEST_INTERVAL: Duration = Duration::from_millis(300);
const INITIAL_BANDWIDTH_ESTIMATE: Bitrate = Bitrate::kbps(1_000);

#[derive(Debug, Clone)]
struct Config {
    /// `None`: cameras send one layer, without simulcast.
    focus_layer: Option<Rid>,
    other_layer: Option<Rid>,
    focus_period: Duration,
    /// `None`: no bandwidth estimation towards watchers.
    bwe_desired: Option<Bitrate>,
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
    let ip: IpAddr = value_of("--ip")
        .map(|v| v.parse().expect("--ip must be an IP address"))
        .unwrap_or_else(local_ip_address_towards_the_internet);
    let udp_port: u16 = value_of("--udp-port").map_or(50000, |v| v.parse().expect("--udp-port"));
    let http_port: u16 = value_of("--http-port").map_or(3000, |v| v.parse().expect("--http-port"));
    let config = Config {
        focus_layer: value_of("--focus-layer").map(|v| Rid::from(v.as_str())),
        other_layer: value_of("--other-layer").map(|v| Rid::from(v.as_str())),
        focus_period: Duration::from_secs(
            value_of("--focus-seconds").map_or(4, |v| v.parse().expect("--focus-seconds")),
        ),
        bwe_desired: value_of("--bwe-desired-kbps")
            .map(|v| Bitrate::kbps(v.parse().expect("--bwe-desired-kbps"))),
    };

    str0m::crypto::from_feature_flags().install_process_default();

    let socket = UdpSocket::bind((ip, udp_port)).expect("binding the UDP port");
    enlarge_socket_buffers(&socket);
    let udp_address = socket.local_addr().expect("local UDP address");
    eprintln!("sfu: media on udp://{udp_address}, signalling on http://{ip}:{http_port}");

    let (new_rtc_sender, new_rtc_receiver) = mpsc::sync_channel::<Rtc>(16);
    let media_config = config.clone();
    std::thread::Builder::new()
        .name("sfu-media".into())
        .spawn(move || run_media_loop(&socket, &new_rtc_receiver, &media_config))
        .expect("media thread");

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let app = Router::new()
            .route(
                "/",
                get(|| async { Html(include_str!("../../web/index.html")) }),
            )
            .route("/offer", post(accept_offer))
            .with_state((udp_address, new_rtc_sender, config.bwe_desired));
        let listener = tokio::net::TcpListener::bind((ip, http_port))
            .await
            .expect("binding the HTTP port");
        axum::serve(listener, app).await.expect("HTTP server");
    });
}

/// The address the OS would use to reach the internet; no packet is sent.
fn local_ip_address_towards_the_internet() -> IpAddr {
    let probe = UdpSocket::bind("0.0.0.0:0").expect("probe socket");
    probe.connect("1.1.1.1:80").expect("probe connect");
    probe.local_addr().expect("probe address").ip()
}

/// Raises the kernel's send and receive buffers of `socket`; the OS may cap the value.
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

type SignallingState = (SocketAddr, SyncSender<Rtc>, Option<Bitrate>);

async fn accept_offer(
    State((udp_address, new_rtc_sender, bwe_desired)): State<SignallingState>,
    body: String,
) -> impl IntoResponse {
    let Ok(offer) = SdpOffer::from_sdp_string(&body) else {
        return (axum::http::StatusCode::BAD_REQUEST, "bad offer".to_owned());
    };
    let mut builder = Rtc::builder()
        .set_ice_lite(true)
        .clear_codecs()
        .enable_vp8(true);
    if bwe_desired.is_some() {
        builder = builder.enable_bwe(Some(INITIAL_BANDWIDTH_ESTIMATE));
    }
    let mut rtc = builder.build(Instant::now());
    if let Some(desired) = bwe_desired {
        rtc.bwe().set_desired_bitrate(desired);
    }
    rtc.add_local_candidate(Candidate::host(udp_address, "udp").expect("host candidate"));
    let Ok(answer) = rtc.sdp_api().accept_offer(offer) else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            "offer refused".to_owned(),
        );
    };
    if new_rtc_sender.send(rtc).is_err() {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "media loop gone".to_owned(),
        );
    }
    (axum::http::StatusCode::OK, answer.to_sdp_string())
}

type ParticipantId = u64;

/// One camera as one watcher receives it.
#[derive(Debug)]
struct Subscription {
    /// The watcher's send m-line carrying this camera.
    mid: Mid,
    /// The layer being forwarded; `None` until a keyframe of the wanted layer arrived.
    forwarded_layer: Option<Option<Rid>>,
    /// When the SFU first wanted a layer it does not forward yet.
    waiting_since: Option<Instant>,
}

#[derive(Debug)]
struct Participant {
    id: ParticipantId,
    rtc: Rtc,
    /// Our receive m-line: the participant's camera.
    camera_mid: Option<Mid>,
    /// Our send m-lines not given to any camera yet.
    free_slots: Vec<Mid>,
    subscription_by_camera: HashMap<ParticipantId, Subscription>,
    /// When str0m next needs `Input::Timeout`.
    timeout_at: Instant,
    /// Latest TWCC estimate of the link towards this participant.
    bandwidth_estimate: Option<Bitrate>,
}

/// Work found while draining one participant's output, done once every participant was drained.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum Propagated {
    Frame(ParticipantId, MediaData),
    /// A watcher lost packets of the camera on its send m-line `Mid`.
    WatcherKeyframeRequest(ParticipantId, Mid),
}

#[derive(Debug, Default)]
struct Measurements {
    frames_received: u64,
    frames_forwarded: u64,
    frames_dropped_no_slot: u64,
    datagrams_received: u64,
    bytes_received: u64,
    datagrams_sent: u64,
    bytes_sent: u64,
    send_errors: u64,
    keyframe_requests_sent: u64,
    /// From the UDP read of a frame to the end of writing it to every watcher.
    forward_microseconds: Vec<u32>,
    /// From wanting a new layer for a watcher to forwarding that layer's keyframe.
    switch_milliseconds: Vec<u32>,
}

#[allow(clippy::too_many_lines)]
fn run_media_loop(socket: &UdpSocket, new_rtc_receiver: &Receiver<Rtc>, config: &Config) {
    let local_address = socket.local_addr().expect("local UDP address");
    let mut participants: HashMap<ParticipantId, Participant> = HashMap::new();
    // Demultiplexing by source address: `Rtc::accepts` on everyone is O(n) per packet.
    let mut participant_by_source: HashMap<SocketAddr, ParticipantId> = HashMap::new();
    let mut next_participant_id: ParticipantId = 0;
    let mut buffer = vec![0u8; MAXIMUM_UDP_DATAGRAM_BYTES];
    let mut measurements = Measurements::default();
    let mut last_report = Instant::now();
    let mut cpu_at_last_report = process_cpu_time();
    let mut propagated: Vec<Propagated> = Vec::new();
    let mut last_keyframe_request: HashMap<(ParticipantId, Option<Rid>), Instant> = HashMap::new();
    let started = Instant::now();

    loop {
        while let Ok(rtc) = new_rtc_receiver.try_recv() {
            let mut participant = Participant {
                id: next_participant_id,
                rtc,
                camera_mid: None,
                free_slots: Vec::new(),
                subscription_by_camera: HashMap::new(),
                timeout_at: Instant::now(),
                bandwidth_estimate: None,
            };
            drain_output(&mut participant, socket, &mut propagated, &mut measurements);
            participants.insert(next_participant_id, participant);
            next_participant_id += 1;
        }

        let next_timeout = participants
            .values()
            .map(|p| p.timeout_at)
            .min()
            .unwrap_or_else(|| Instant::now() + Duration::from_millis(100));
        let wait = next_timeout
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        socket.set_read_timeout(Some(wait)).expect("read timeout");
        match socket.recv_from(&mut buffer) {
            Ok((length, source)) => {
                measurements.datagrams_received += 1;
                measurements.bytes_received += length as u64;
                let Ok(contents) = buffer[..length].try_into() else {
                    continue;
                };
                let input = Input::Receive(
                    Instant::now(),
                    Receive {
                        proto: Protocol::Udp,
                        source,
                        destination: local_address,
                        contents,
                    },
                );
                let known = participant_by_source
                    .get(&source)
                    .filter(|id| participants.get(id).is_some_and(|p| p.rtc.accepts(&input)))
                    .copied();
                let target = known.or_else(|| {
                    let id = participants
                        .values()
                        .find(|p| p.rtc.accepts(&input))
                        .map(|p| p.id)?;
                    participant_by_source.insert(source, id);
                    Some(id)
                });
                if let Some(participant) = target.and_then(|id| participants.get_mut(&id)) {
                    if participant.rtc.handle_input(input).is_err() {
                        participant.rtc.disconnect();
                    }
                    drain_output(participant, socket, &mut propagated, &mut measurements);
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => panic!("UDP read failed: {error}"),
        }

        let now = Instant::now();
        for participant in participants.values_mut().filter(|p| p.timeout_at <= now) {
            if participant.rtc.handle_input(Input::Timeout(now)).is_err() {
                participant.rtc.disconnect();
            }
            drain_output(participant, socket, &mut propagated, &mut measurements);
        }

        let mut cameras: Vec<ParticipantId> = participants
            .values()
            .filter(|p| p.camera_mid.is_some())
            .map(|p| p.id)
            .collect();
        cameras.sort_unstable();
        #[allow(clippy::cast_possible_truncation)]
        let focus_turn =
            (started.elapsed().as_millis() / config.focus_period.as_millis().max(1)) as usize;

        let mut keyframe_requests: Vec<(ParticipantId, Option<Rid>)> = Vec::new();
        for work in std::mem::take(&mut propagated) {
            match work {
                Propagated::Frame(camera, frame) => forward_frame(
                    camera,
                    &frame,
                    &mut participants,
                    socket,
                    config,
                    &cameras,
                    focus_turn,
                    &mut keyframe_requests,
                    &mut measurements,
                ),
                Propagated::WatcherKeyframeRequest(watcher, mid) => {
                    if let Some((camera, subscription)) = participants
                        .get(&watcher)
                        .and_then(|w| w.subscription_by_camera.iter().find(|(_, s)| s.mid == mid))
                        && let Some(layer) = subscription.forwarded_layer
                    {
                        keyframe_requests.push((*camera, layer));
                    }
                }
            }
        }
        for (camera, layer) in keyframe_requests {
            let now = Instant::now();
            let last = last_keyframe_request.entry((camera, layer));
            let is_throttled = matches!(&last, std::collections::hash_map::Entry::Occupied(at)
                if now.duration_since(*at.get()) < KEYFRAME_REQUEST_INTERVAL);
            if is_throttled {
                continue;
            }
            last.insert_entry(now);
            if let Some(publisher) = participants.get_mut(&camera)
                && let Some(mid) = publisher.camera_mid
                && let Some(mut writer) = publisher.rtc.writer(mid)
                && writer
                    .request_keyframe(layer, KeyframeRequestKind::Pli)
                    .is_ok()
            {
                measurements.keyframe_requests_sent += 1;
                drain_output(publisher, socket, &mut propagated, &mut measurements);
            }
        }

        let gone: Vec<ParticipantId> = participants
            .values()
            .filter(|p| !p.rtc.is_alive())
            .map(|p| p.id)
            .collect();
        if !gone.is_empty() {
            participants.retain(|_, p| p.rtc.is_alive());
            participant_by_source.retain(|_, id| participants.contains_key(id));
            for participant in participants.values_mut() {
                for id in &gone {
                    if let Some(subscription) = participant.subscription_by_camera.remove(id) {
                        participant.free_slots.push(subscription.mid);
                    }
                }
            }
            eprintln!(
                "sfu: {} left, {} in the room",
                gone.len(),
                participants.len()
            );
        }

        if last_report.elapsed() >= REPORT_INTERVAL {
            let cpu_now = process_cpu_time();
            report(
                &measurements,
                &participants,
                cpu_now - cpu_at_last_report,
                last_report.elapsed(),
            );
            measurements = Measurements::default();
            cpu_at_last_report = cpu_now;
            last_report = Instant::now();
        }
    }
}

/// Sends what str0m wants sent, collects incoming frames and requests, records the next timeout.
fn drain_output(
    participant: &mut Participant,
    socket: &UdpSocket,
    propagated: &mut Vec<Propagated>,
    measurements: &mut Measurements,
) {
    loop {
        if !participant.rtc.is_alive() {
            participant.timeout_at = Instant::now();
            return;
        }
        match participant.rtc.poll_output() {
            Ok(Output::Timeout(at)) => {
                participant.timeout_at = at;
                return;
            }
            Ok(Output::Transmit(transmit)) => {
                match socket.send_to(&transmit.contents, transmit.destination) {
                    Ok(sent) => {
                        measurements.datagrams_sent += 1;
                        measurements.bytes_sent += sent as u64;
                    }
                    Err(_) => measurements.send_errors += 1,
                }
            }
            Ok(Output::Event(Event::MediaAdded(added))) if added.kind == MediaKind::Video => {
                match added.direction {
                    Direction::RecvOnly => participant.camera_mid = Some(added.mid),
                    Direction::SendOnly => participant.free_slots.push(added.mid),
                    _ => {}
                }
            }
            Ok(Output::Event(Event::MediaData(frame)))
                if Some(frame.mid) == participant.camera_mid =>
            {
                propagated.push(Propagated::Frame(participant.id, frame));
            }
            Ok(Output::Event(Event::KeyframeRequest(request))) => {
                propagated.push(Propagated::WatcherKeyframeRequest(
                    participant.id,
                    request.mid,
                ));
            }
            Ok(Output::Event(Event::EgressBitrateEstimate(
                BweKind::Twcc { estimate, .. } | BweKind::Remb { estimate, .. },
            ))) => participant.bandwidth_estimate = Some(estimate),
            Ok(Output::Event(Event::IceConnectionStateChange(
                IceConnectionState::Disconnected,
            ))) => {
                participant.rtc.disconnect();
            }
            Ok(Output::Event(_)) => {}
            Err(_) => participant.rtc.disconnect(),
        }
    }
}

/// The camera a watcher sees large in a speaker view: the cameras take turns, and a watcher's own
/// camera is skipped.
fn focus_of(
    watcher: ParticipantId,
    cameras: &[ParticipantId],
    focus_turn: usize,
) -> Option<ParticipantId> {
    let others: Vec<ParticipantId> = cameras.iter().copied().filter(|c| *c != watcher).collect();
    others.get(focus_turn % others.len().max(1)).copied()
}

/// Writes the frame to every watcher that wants this layer of this camera and sends it at once,
/// so the measured time covers encryption and the UDP send.
#[allow(clippy::too_many_arguments)]
fn forward_frame(
    camera: ParticipantId,
    frame: &MediaData,
    participants: &mut HashMap<ParticipantId, Participant>,
    socket: &UdpSocket,
    config: &Config,
    cameras: &[ParticipantId],
    focus_turn: usize,
    keyframe_requests: &mut Vec<(ParticipantId, Option<Rid>)>,
    measurements: &mut Measurements,
) {
    measurements.frames_received += 1;
    let mut ignored = Vec::new();
    for watcher in participants
        .values_mut()
        .filter(|p| p.id != camera && p.rtc.is_alive())
    {
        let wanted_layer = if focus_of(watcher.id, cameras, focus_turn) == Some(camera) {
            config.focus_layer
        } else {
            config.other_layer
        };
        if !watcher.subscription_by_camera.contains_key(&camera) {
            let Some(mid) = watcher.free_slots.pop() else {
                measurements.frames_dropped_no_slot += 1;
                continue;
            };
            watcher.subscription_by_camera.insert(
                camera,
                Subscription {
                    mid,
                    forwarded_layer: None,
                    waiting_since: None,
                },
            );
        }
        let subscription = watcher
            .subscription_by_camera
            .get_mut(&camera)
            .expect("inserted above");
        let is_wanted_layer = frame.rid == wanted_layer;
        let is_forwarded_layer = subscription.forwarded_layer.as_ref() == Some(&frame.rid);
        let should_forward = if is_wanted_layer && is_forwarded_layer {
            subscription.waiting_since = None;
            true
        } else if is_wanted_layer && frame.is_keyframe() {
            if let Some(since) = subscription.waiting_since.take() {
                measurements
                    .switch_milliseconds
                    .push(u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX));
            }
            subscription.forwarded_layer = Some(frame.rid);
            true
        } else if is_wanted_layer {
            subscription.waiting_since.get_or_insert_with(Instant::now);
            keyframe_requests.push((camera, wanted_layer));
            false
        } else {
            // Another layer: keep the old one flowing until the wanted layer's keyframe arrives.
            is_forwarded_layer
        };
        if !should_forward {
            continue;
        }
        let mid = subscription.mid;
        let Some(writer) = watcher.rtc.writer(mid) else {
            continue;
        };
        let Some(pt) = writer.match_params(frame.params) else {
            continue;
        };
        if writer
            .write(pt, frame.network_time, frame.time, frame.data.clone())
            .is_err()
        {
            watcher.rtc.disconnect();
            continue;
        }
        measurements.frames_forwarded += 1;
        // A watcher only sends media here; frames it produces arrive through its own input.
        drain_output(watcher, socket, &mut ignored, measurements);
    }
    let elapsed = frame.network_time.elapsed().as_micros();
    measurements
        .forward_microseconds
        .push(u32::try_from(elapsed).unwrap_or(u32::MAX));
}

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

fn report(
    measurements: &Measurements,
    participants: &HashMap<ParticipantId, Participant>,
    cpu: Duration,
    wall: Duration,
) {
    let mut forward = measurements.forward_microseconds.clone();
    forward.sort_unstable();
    let mut switches = measurements.switch_milliseconds.clone();
    switches.sort_unstable();
    let mut estimates: Vec<u32> = participants
        .values()
        .filter_map(|p| p.bandwidth_estimate)
        .map(|b| u32::try_from(b.as_u64() / 1000).unwrap_or(u32::MAX))
        .collect();
    estimates.sort_unstable();
    let seconds = wall.as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let per_second = |count: u64| count as f64 / seconds;
    #[allow(clippy::cast_precision_loss)]
    let megabits = |bytes: u64| bytes as f64 * 8.0 / seconds / 1_000_000.0;
    eprintln!(
        "sfu: {} in room | cpu {:.1}% of one core | in {:.0} pkt/s {:.1} Mbit/s, out {:.0} pkt/s {:.1} Mbit/s, send errors {} | frames in {:.0}/s, out {:.0}/s, no slot {} | inside the SFU p50 {} µs, p99 {} µs | keyframe requests {}, switches {} waited p50 {} ms, p99 {} ms | estimate kbit/s min {} p50 {}",
        participants.len(),
        100.0 * cpu.as_secs_f64() / seconds,
        per_second(measurements.datagrams_received),
        megabits(measurements.bytes_received),
        per_second(measurements.datagrams_sent),
        megabits(measurements.bytes_sent),
        measurements.send_errors,
        per_second(measurements.frames_received),
        per_second(measurements.frames_forwarded),
        measurements.frames_dropped_no_slot,
        percentile(&forward, 0.50),
        percentile(&forward, 0.99),
        measurements.keyframe_requests_sent,
        switches.len(),
        percentile(&switches, 0.50),
        percentile(&switches, 0.99),
        estimates.first().copied().unwrap_or(0),
        percentile(&estimates, 0.50),
    );
}

/// User plus system CPU time of this process.
fn process_cpu_time() -> Duration {
    // SAFETY: getrusage only writes into the zeroed struct we own.
    #[allow(unsafe_code)]
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &raw mut usage);
        usage
    };
    let to_duration = |t: libc::timeval| {
        Duration::from_secs(u64::try_from(t.tv_sec).unwrap_or(0))
            + Duration::from_micros(u64::try_from(t.tv_usec).unwrap_or(0))
    };
    to_duration(usage.ru_utime) + to_duration(usage.ru_stime)
}
