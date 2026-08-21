mod backend;
mod config;
mod joycon;
mod osc;
mod sensation;
mod signal;
mod trace;
mod transport;

use std::collections::{HashMap, HashSet};
use std::env;
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use backend::{ConsoleBackend, RumbleBackend};
use config::{Config, DeviceSide};
use joycon::{HidJoyConBackend, ImuProfileStore};
use sensation::{HapticDriveFrame, SensationDriveEngine};
use signal::{HapticEvent, HapticRouter, Target};
use trace::TraceRecorder;
use transport::ReceivedPacket;

const OUTPUT_REFRESH: Duration = Duration::from_millis(15);
const SENSATION_FRAME: Duration = Duration::from_millis(50);
const IDLE_SLEEP: Duration = Duration::from_millis(1);

#[derive(Debug)]
struct PeriodicDeadline {
    interval: Duration,
    next: Instant,
}

impl PeriodicDeadline {
    fn new(now: Instant, interval: Duration) -> Self {
        Self {
            interval,
            next: now,
        }
    }

    fn take_due(&mut self, now: Instant) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + self.interval;
        true
    }

    fn trigger_now(&mut self, now: Instant) {
        self.next = now;
    }
}

struct OscRuntime {
    heartbeat_address: String,
    status_port_address: String,
    status_heartbeat_address: String,
    router: HapticRouter,
    status_targets: HashMap<IpAddr, HashSet<SocketAddr>>,
    unregistered_heartbeat_peers: HashSet<SocketAddr>,
}

impl OscRuntime {
    fn process_batch(
        &mut self,
        socket: &UdpSocket,
        packets: &[ReceivedPacket],
        drive_engine: &mut SensationDriveEngine,
        sensation_schedule: &mut PeriodicDeadline,
        trace: &mut Option<TraceRecorder>,
    ) {
        // Registration must be visible before an accompanying heartbeat is acknowledged.
        for received in packets {
            if received.packet.address == self.status_port_address {
                self.register_status_target(received);
            }
        }

        for received in packets {
            let packet = &received.packet;
            let peer = received.peer;
            let now = Instant::now();
            let heartbeat_sequence = if packet.address == self.heartbeat_address {
                match packet.arguments.as_slice() {
                    [osc::OscArgument::Int(sequence)] => Some(*sequence),
                    _ => None,
                }
            } else {
                None
            };

            if let Some(event) = self.router.route_packet(packet, now) {
                match event {
                    HapticEvent::HeartbeatRestored => {
                        drive_engine.reset();
                        sensation_schedule.trigger_now(now);
                    }
                    HapticEvent::Sensation {
                        target,
                        sensation,
                        value,
                    } => {
                        if let Some(trace) = trace.as_mut() {
                            trace.osc(target, sensation, value, self.router.state());
                        }
                    }
                }
            }

            if let Some(sequence) = heartbeat_sequence {
                self.acknowledge_heartbeat(socket, peer, sequence);
            }
        }
    }

    fn register_status_target(&mut self, received: &ReceivedPacket) {
        if let [osc::OscArgument::Int(port)] = received.packet.arguments.as_slice()
            && let Ok(port) = u16::try_from(*port)
            && port != 0
        {
            let target = SocketAddr::new(received.peer.ip(), port);
            if self
                .status_targets
                .entry(received.peer.ip())
                .or_default()
                .insert(target)
            {
                self.unregistered_heartbeat_peers
                    .retain(|entry| entry.ip() != received.peer.ip());
                eprintln!(
                    "status acknowledgement target registered: {} -> {target}",
                    received.peer
                );
            }
        }
    }

    fn acknowledge_heartbeat(&mut self, socket: &UdpSocket, peer: SocketAddr, sequence: i32) {
        let Some(targets) = self
            .status_targets
            .get(&peer.ip())
            .map(|targets| targets.iter().copied().collect::<Vec<_>>())
        else {
            if self.unregistered_heartbeat_peers.insert(peer) {
                eprintln!("heartbeat received without status acknowledgement target: {peer}");
            }
            return;
        };

        let reply = match osc::encode_int_message(&self.status_heartbeat_address, sequence) {
            Ok(reply) => reply,
            Err(error) => {
                eprintln!("status acknowledgement encode error: {error}");
                return;
            }
        };
        let mut failed_targets = Vec::new();
        for target in targets {
            if let Err(error) = socket.send_to(&reply, target) {
                failed_targets.push(target);
                eprintln!("status acknowledgement disabled after send error for {target}: {error}");
            }
        }
        if !failed_targets.is_empty()
            && let Some(targets) = self.status_targets.get_mut(&peer.ip())
        {
            for target in failed_targets {
                targets.remove(&target);
            }
            if targets.is_empty() {
                self.status_targets.remove(&peer.ip());
            }
        }
    }

    fn reset_status_targets(&mut self, error: &io::Error) {
        let removed = self
            .status_targets
            .values()
            .map(HashSet::len)
            .sum::<usize>();
        self.status_targets.clear();
        eprintln!(
            "status acknowledgement registrations reset after UDP port closure \
             ({removed} removed); bridge continues: {error}"
        );
    }
}

pub fn run() -> io::Result<()> {
    if env::args().nth(1).as_deref() == Some("joycon-list") {
        return joycon::list_joycons();
    }
    if env::args().nth(1).as_deref() == Some("hid-list") {
        return joycon::list_hid_devices();
    }
    if env::args().nth(1).as_deref() == Some("joycon-rumble-test") {
        return joycon::rumble_test_from_env();
    }
    if env::args().nth(1).as_deref() == Some("joycon-imu-sweep") {
        return joycon::imu_sweep_from_env();
    }

    let config = Config::load()?;
    let mut trace = config
        .trace_csv
        .as_deref()
        .map(TraceRecorder::create)
        .transpose()?;
    if let Some(path) = &config.trace_csv {
        eprintln!("haptics trace={}", path.display());
    }
    let target_bindings = config.devices.iter().map(|device| {
        let target = match device.side {
            DeviceSide::Left => Target::Left,
            DeviceSide::Right => Target::Right,
        };
        (device.osc_address.clone(), target)
    });
    let mut backend: Box<dyn RumbleBackend> = if config.dry_run {
        Box::new(ConsoleBackend)
    } else {
        let profiles = ImuProfileStore::load(&config.imu_profile_path)?;
        if !profiles.is_empty() {
            eprintln!(
                "loaded {} IMU-derived rumble profile(s) from {}",
                profiles.len(),
                config.imu_profile_path.display()
            );
        }
        Box::new(HidJoyConBackend::new(
            config.low_freq_hz,
            config.high_freq_hz,
            config.devices.clone(),
            profiles,
            config.trace_csv.is_some(),
        ))
    };
    backend.connect()?;

    let socket = UdpSocket::bind(config.listen)?;
    socket.set_nonblocking(true)?;
    eprintln!(
        "osc listening on {} namespace={} dry_run={}",
        config.listen, config.namespace, config.dry_run
    );
    eprintln!(
        "heartbeat=/avatar/parameters/{}/heartbeat targets={} timeout_ms={}",
        config.namespace,
        config
            .devices
            .iter()
            .map(|device| format!("{}:{}", device.osc_address, device.side.label()))
            .collect::<Vec<_>>()
            .join(","),
        config.heartbeat_timeout_ms
    );
    let heartbeat_address = format!("/avatar/parameters/{}/heartbeat", config.namespace);
    let status_port_address = format!("/avatar/parameters/{}/status/port", config.namespace);
    let status_heartbeat_address =
        format!("/avatar/parameters/{}/status/heartbeat", config.namespace);
    eprintln!(
        "status registration={} acknowledgement={}",
        status_port_address, status_heartbeat_address
    );

    let router = HapticRouter::new(
        &config.namespace,
        Duration::from_millis(config.heartbeat_timeout_ms),
        target_bindings,
    );
    let mut runtime = OscRuntime {
        heartbeat_address,
        status_port_address,
        status_heartbeat_address,
        router,
        status_targets: HashMap::new(),
        unregistered_heartbeat_peers: HashSet::new(),
    };
    let mut drive_engine = SensationDriveEngine::default();
    let mut drive_frames = [HapticDriveFrame::default(); 2];
    let now = Instant::now();
    let mut sensation_schedule = PeriodicDeadline::new(now, SENSATION_FRAME);
    let mut output_schedule = PeriodicDeadline::new(now, OUTPUT_REFRESH);
    let mut last_saturation_warning = None;
    let mut plugin_connected = false;
    let mut buffer = [0_u8; 2048];

    loop {
        let batch = transport::drain_latest(&socket, &mut buffer)?;
        for (peer, error) in batch.malformed {
            eprintln!("ignored malformed OSC packet from {peer}: {error}");
        }
        if batch.connection_reset {
            runtime.reset_status_targets(&io::Error::new(
                io::ErrorKind::ConnectionReset,
                "UDP port closure",
            ));
        }
        if batch.saturated {
            let now = Instant::now();
            if last_saturation_warning
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(1))
            {
                eprintln!(
                    "OSC receive batch reached {} packets; continuing on next loop",
                    transport::MAX_PACKET_BATCH
                );
                last_saturation_warning = Some(now);
            }
        }
        runtime.process_batch(
            &socket,
            &batch.packets,
            &mut drive_engine,
            &mut sensation_schedule,
            &mut trace,
        );

        let now = Instant::now();
        backend.poll()?;
        let samples = backend.take_imu_samples()?;
        if let Some(trace) = trace.as_mut() {
            for sample in samples {
                trace.imu(sample.target, sample.acceleration);
            }
        }
        let heartbeat_valid = runtime.router.heartbeat_valid(now);
        if heartbeat_valid != plugin_connected {
            plugin_connected = heartbeat_valid;
            eprintln!(
                "bridge-status plugin={}",
                if plugin_connected {
                    "connected"
                } else {
                    "disconnected"
                }
            );
        }
        if runtime.router.take_timeout(now) {
            drive_engine.reset();
            drive_frames = [HapticDriveFrame::default(); 2];
            stop_all(backend.as_mut(), &mut trace)?;
        } else if heartbeat_valid {
            if sensation_schedule.take_due(now) {
                let state = runtime.router.state();
                let next_frames = drive_engine.update(state, SENSATION_FRAME);
                if let Some(trace) = trace.as_mut() {
                    for (target, frame) in
                        [Target::Left, Target::Right].into_iter().zip(next_frames)
                    {
                        trace.frame(target, state, frame);
                    }
                }
                stop_released_outputs(backend.as_mut(), drive_frames, next_frames, &mut trace)?;
                drive_frames = next_frames;
            }
            if output_schedule.take_due(now) {
                refresh_outputs(backend.as_mut(), drive_frames, &mut trace)?;
            }
        }
        thread::sleep(IDLE_SLEEP);
    }
}

fn refresh_outputs(
    backend: &mut dyn RumbleBackend,
    frames: [HapticDriveFrame; 2],
    trace: &mut Option<TraceRecorder>,
) -> io::Result<()> {
    for (target, frame) in [Target::Left, Target::Right].into_iter().zip(frames) {
        if frame.amplitude > 0.0 {
            apply_frame(backend, target, frame, trace)?;
        }
    }
    Ok(())
}

fn stop_released_outputs(
    backend: &mut dyn RumbleBackend,
    previous: [HapticDriveFrame; 2],
    next: [HapticDriveFrame; 2],
    trace: &mut Option<TraceRecorder>,
) -> io::Result<()> {
    for ((target, previous), next) in [Target::Left, Target::Right]
        .into_iter()
        .zip(previous)
        .zip(next)
    {
        if previous.amplitude > 0.0 && next.amplitude <= 0.0 {
            let result = backend.stop(target);
            if let Some(trace) = trace.as_mut() {
                trace.hid(target, HapticDriveFrame::default(), &result);
            }
            result?;
        }
    }
    Ok(())
}

fn apply_frame(
    backend: &mut dyn RumbleBackend,
    target: Target,
    frame: HapticDriveFrame,
    trace: &mut Option<TraceRecorder>,
) -> io::Result<()> {
    let result = if frame.amplitude > 0.0 {
        backend.drive(target, frame)
    } else {
        backend.stop(target)
    };
    if let Some(trace) = trace.as_mut() {
        trace.hid(target, frame, &result);
    }
    result
}

fn stop_all(backend: &mut dyn RumbleBackend, trace: &mut Option<TraceRecorder>) -> io::Result<()> {
    for target in [Target::Left, Target::Right] {
        let result = backend.stop(target);
        if let Some(trace) = trace.as_mut() {
            trace.hid(target, HapticDriveFrame::default(), &result);
        }
        result?;
    }
    Ok(())
}

#[cfg(test)]
mod scheduler_tests {
    use super::*;

    #[test]
    fn deadline_runs_once_per_interval() {
        let start = Instant::now();
        let mut deadline = PeriodicDeadline::new(start, Duration::from_millis(15));
        assert!(deadline.take_due(start));
        assert!(!deadline.take_due(start + Duration::from_millis(14)));
        assert!(deadline.take_due(start + Duration::from_millis(15)));
        assert!(!deadline.take_due(start + Duration::from_millis(16)));
    }

    #[test]
    fn heartbeat_restore_can_trigger_sensation_frame_immediately() {
        let start = Instant::now();
        let mut deadline = PeriodicDeadline::new(start, Duration::from_millis(50));
        assert!(deadline.take_due(start));
        let restored = start + Duration::from_millis(10);
        deadline.trigger_now(restored);
        assert!(deadline.take_due(restored));
    }
}
