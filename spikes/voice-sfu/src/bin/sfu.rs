//! Spike SFU: one room, audio only, one UDP port, ICE-lite, signalling by a single HTTP POST.
//!
//! Each client offers one `sendonly` audio m-line (its voice) and K `recvonly` ones (slots for
//! the others), so no renegotiation is needed while people come and go. The SFU forwards Opus
//! frames as they are, never decodes them. Every 5 seconds it prints its CPU use and how long a
//! frame spends inside it.
//!
//! Usage: `sfu [--ip 192.0.2.10] [--udp-port 50000] [--http-port 3000]`

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use str0m::change::SdpOffer;
use str0m::media::{Direction, MediaData, MediaKind, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

const REPORT_INTERVAL: Duration = Duration::from_secs(5);
const MAXIMUM_UDP_DATAGRAM_BYTES: usize = 2000;

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

    str0m::crypto::from_feature_flags().install_process_default();

    let socket = UdpSocket::bind((ip, udp_port)).expect("binding the UDP port");
    let udp_address = socket.local_addr().expect("local UDP address");
    eprintln!("sfu: media on udp://{udp_address}, signalling on http://{ip}:{http_port}");

    let (new_rtc_sender, new_rtc_receiver) = mpsc::sync_channel::<Rtc>(16);
    std::thread::Builder::new()
        .name("sfu-media".into())
        .spawn(move || run_media_loop(&socket, &new_rtc_receiver))
        .expect("media thread");

    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async move {
        let app = Router::new()
            .route(
                "/",
                get(|| async { Html(include_str!("../../web/index.html")) }),
            )
            .route("/offer", post(accept_offer))
            .with_state((udp_address, new_rtc_sender));
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

async fn accept_offer(
    State((udp_address, new_rtc_sender)): State<(SocketAddr, SyncSender<Rtc>)>,
    body: String,
) -> impl IntoResponse {
    let Ok(offer) = SdpOffer::from_sdp_string(&body) else {
        return (axum::http::StatusCode::BAD_REQUEST, "bad offer".to_owned());
    };
    let mut rtc = Rtc::builder()
        .set_ice_lite(true)
        .clear_codecs()
        .enable_opus(true, false)
        .build(Instant::now());
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

#[derive(Debug)]
struct Participant {
    id: ParticipantId,
    rtc: Rtc,
    /// Our receive m-line: the participant's voice.
    voice_mid: Option<Mid>,
    /// Our send m-lines not given to anyone yet.
    free_slots: Vec<Mid>,
    /// Which slot carries which other participant's voice.
    slot_by_speaker: HashMap<ParticipantId, Mid>,
    /// When str0m next needs `Input::Timeout`.
    timeout_at: Instant,
}

#[derive(Debug, Default)]
struct Measurements {
    frames_received: u64,
    frames_forwarded: u64,
    frames_dropped_no_slot: u64,
    /// From the UDP read of a frame to the end of writing it to every listener.
    forward_microseconds: Vec<u32>,
}

fn run_media_loop(socket: &UdpSocket, new_rtc_receiver: &Receiver<Rtc>) {
    let local_address = socket.local_addr().expect("local UDP address");
    let mut participants: HashMap<ParticipantId, Participant> = HashMap::new();
    // Demultiplexing by source address: `Rtc::accepts` on everyone is O(n) per packet.
    let mut participant_by_source: HashMap<SocketAddr, ParticipantId> = HashMap::new();
    let mut next_participant_id: ParticipantId = 0;
    let mut buffer = vec![0u8; MAXIMUM_UDP_DATAGRAM_BYTES];
    let mut measurements = Measurements::default();
    let mut last_report = Instant::now();
    let mut cpu_at_last_report = process_cpu_time();
    let mut frames_to_forward: Vec<(ParticipantId, MediaData)> = Vec::new();

    loop {
        while let Ok(rtc) = new_rtc_receiver.try_recv() {
            let mut participant = Participant {
                id: next_participant_id,
                rtc,
                voice_mid: None,
                free_slots: Vec::new(),
                slot_by_speaker: HashMap::new(),
                timeout_at: Instant::now(),
            };
            drain_output(&mut participant, socket, &mut frames_to_forward);
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
                    drain_output(participant, socket, &mut frames_to_forward);
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
            drain_output(participant, socket, &mut frames_to_forward);
        }

        for (speaker, frame) in std::mem::take(&mut frames_to_forward) {
            forward_frame(
                speaker,
                &frame,
                &mut participants,
                socket,
                &mut measurements,
            );
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
                    if let Some(slot) = participant.slot_by_speaker.remove(id) {
                        participant.free_slots.push(slot);
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
                participants.len(),
                cpu_now - cpu_at_last_report,
                last_report.elapsed(),
            );
            measurements = Measurements::default();
            cpu_at_last_report = cpu_now;
            last_report = Instant::now();
        }
    }
}

/// Sends what str0m wants sent, collects incoming voice frames and records the next timeout.
fn drain_output(
    participant: &mut Participant,
    socket: &UdpSocket,
    frames_to_forward: &mut Vec<(ParticipantId, MediaData)>,
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
                let _ = socket.send_to(&transmit.contents, transmit.destination);
            }
            Ok(Output::Event(Event::MediaAdded(added))) if added.kind == MediaKind::Audio => {
                match added.direction {
                    Direction::RecvOnly => participant.voice_mid = Some(added.mid),
                    Direction::SendOnly => participant.free_slots.push(added.mid),
                    _ => {}
                }
            }
            Ok(Output::Event(Event::MediaData(frame)))
                if Some(frame.mid) == participant.voice_mid =>
            {
                frames_to_forward.push((participant.id, frame));
            }
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

/// Writes the frame to every other participant and sends it at once, so the measured time
/// covers encryption and the UDP send.
fn forward_frame(
    speaker: ParticipantId,
    frame: &MediaData,
    participants: &mut HashMap<ParticipantId, Participant>,
    socket: &UdpSocket,
    measurements: &mut Measurements,
) {
    measurements.frames_received += 1;
    let mut ignored_frames = Vec::new();
    for listener in participants
        .values_mut()
        .filter(|p| p.id != speaker && p.rtc.is_alive())
    {
        let slot = if let Some(slot) = listener.slot_by_speaker.get(&speaker) {
            *slot
        } else {
            let Some(slot) = listener.free_slots.pop() else {
                measurements.frames_dropped_no_slot += 1;
                continue;
            };
            listener.slot_by_speaker.insert(speaker, slot);
            slot
        };
        let Some(writer) = listener.rtc.writer(slot) else {
            continue;
        };
        let Some(pt) = writer.match_params(frame.params) else {
            continue;
        };
        if writer
            .write(pt, frame.network_time, frame.time, frame.data.clone())
            .is_err()
        {
            listener.rtc.disconnect();
            continue;
        }
        measurements.frames_forwarded += 1;
        // A listener only sends media here; frames it produces arrive through its own input.
        drain_output(listener, socket, &mut ignored_frames);
    }
    let elapsed = frame.network_time.elapsed().as_micros();
    measurements
        .forward_microseconds
        .push(u32::try_from(elapsed).unwrap_or(u32::MAX));
}

fn report(measurements: &Measurements, participant_count: usize, cpu: Duration, wall: Duration) {
    let mut forward = measurements.forward_microseconds.clone();
    forward.sort_unstable();
    let percentile = |p: f64| {
        if forward.is_empty() {
            return 0;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let index = ((forward.len() - 1) as f64 * p).round() as usize;
        forward[index]
    };
    let seconds = wall.as_secs_f64();
    #[allow(clippy::cast_precision_loss)]
    let per_second = |count: u64| count as f64 / seconds;
    eprintln!(
        "sfu: {participant_count} in room | cpu {:.1}% of one core | in {:.0} frames/s, out {:.0} frames/s, no slot {} | inside the SFU p50 {} µs, p99 {} µs, max {} µs",
        100.0 * cpu.as_secs_f64() / seconds,
        per_second(measurements.frames_received),
        per_second(measurements.frames_forwarded),
        measurements.frames_dropped_no_slot,
        percentile(0.50),
        percentile(0.99),
        forward.last().copied().unwrap_or(0),
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
