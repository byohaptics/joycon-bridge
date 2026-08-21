use std::env;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use hidapi::{DeviceInfo, HidApi, HidDevice};
use serde::{Deserialize, Serialize};

use crate::backend::{ImuSample, RumbleBackend};
use crate::config::{DeviceConfig, DeviceSide, normalize_bluetooth_address};
use crate::sensation::HapticDriveFrame;
use crate::signal::Target;

const NINTENDO_VENDOR_ID: u16 = 0x057e;
const JOYCON_LEFT_PRODUCT_ID: u16 = 0x2006;
const JOYCON_RIGHT_PRODUCT_ID: u16 = 0x2007;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CarrierBand {
    pub max_amplitude: f32,
    pub low_hz: f32,
    pub high_hz: f32,
    pub measured_rms_lsb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImuRumbleProfile {
    pub side: DeviceSide,
    pub bands: Vec<CarrierBand>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ImuProfileStore {
    pub profiles: Vec<ImuRumbleProfile>,
}

impl ImuProfileStore {
    pub fn load(path: &PathBuf) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source = fs::read_to_string(path)?;
        let store: Self = toml::from_str(&source).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to parse IMU profile {}: {error}", path.display()),
            )
        })?;
        for profile in &store.profiles {
            let mut previous_max = 0.0;
            for band in &profile.bands {
                if !band.max_amplitude.is_finite()
                    || !band.low_hz.is_finite()
                    || !band.high_hz.is_finite()
                    || band.max_amplitude <= previous_max
                    || band.max_amplitude > 1.0
                    || band.low_hz <= 0.0
                    || band.high_hz <= 0.0
                {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid IMU carrier band in {}", path.display()),
                    ));
                }
                previous_max = band.max_amplitude;
            }
        }
        Ok(store)
    }

    fn save(&self, path: &PathBuf) -> io::Result<()> {
        let source = toml::to_string_pretty(self).map_err(|error| {
            io::Error::other(format!("IMU profile serialization failed: {error}"))
        })?;
        fs::write(path, source)
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    fn carrier(&self, side: DeviceSide, amplitude: f32) -> Option<(f32, f32)> {
        self.profiles
            .iter()
            .find(|profile| profile.side == side)
            .and_then(|profile| {
                profile
                    .bands
                    .iter()
                    .find(|band| amplitude <= band.max_amplitude)
                    .or_else(|| profile.bands.last())
            })
            .map(|band| (band.low_hz, band.high_hz))
    }
}

pub fn list_joycons() -> io::Result<()> {
    let api = HidApi::new().map_err(to_io_error)?;
    let devices: Vec<&DeviceInfo> = api
        .device_list()
        .filter(|device| {
            device.vendor_id() == NINTENDO_VENDOR_ID
                && matches!(
                    device.product_id(),
                    JOYCON_LEFT_PRODUCT_ID | JOYCON_RIGHT_PRODUCT_ID
                )
        })
        .collect();

    if devices.is_empty() {
        println!("No Joy-Con HID devices found via hidapi.");
        println!("Expected Nintendo VID 057e with PID 2006 (L) or 2007 (R).");
        return Ok(());
    }

    println!("Found {} Joy-Con HID device(s):", devices.len());
    for (index, device) in devices.iter().enumerate() {
        let side = match device.product_id() {
            JOYCON_LEFT_PRODUCT_ID => "Left",
            JOYCON_RIGHT_PRODUCT_ID => "Right",
            _ => "Unknown",
        };
        let product = device.product_string().unwrap_or("(no product string)");
        let serial = device.serial_number().unwrap_or("(no serial)");
        let open_status = match device.open_device(&api) {
            Ok(_) => "open: ok",
            Err(error) => {
                println!(
                    "{}. {} Joy-Con {:04x}:{:04x} product=\"{}\" serial=\"{}\" path=\"{}\"",
                    index + 1,
                    side,
                    device.vendor_id(),
                    device.product_id(),
                    product,
                    serial,
                    device.path().to_string_lossy()
                );
                println!("   open: failed ({error})");
                continue;
            }
        };

        println!(
            "{}. {} Joy-Con {:04x}:{:04x} product=\"{}\" serial=\"{}\" path=\"{}\"",
            index + 1,
            side,
            device.vendor_id(),
            device.product_id(),
            product,
            serial,
            device.path().to_string_lossy()
        );
        println!("   {open_status}");
    }

    Ok(())
}

pub fn list_hid_devices() -> io::Result<()> {
    let api = HidApi::new().map_err(to_io_error)?;
    let devices: Vec<&DeviceInfo> = api.device_list().collect();

    println!("Found {} HID device(s):", devices.len());
    for (index, device) in devices.iter().enumerate() {
        let product = device.product_string().unwrap_or("(no product string)");
        let manufacturer = device
            .manufacturer_string()
            .unwrap_or("(no manufacturer string)");
        let serial = device.serial_number().unwrap_or("(no serial)");
        println!(
            "{}. {:04x}:{:04x} manufacturer=\"{}\" product=\"{}\" serial=\"{}\" usage_page={:#06x} usage={:#06x} interface={} path=\"{}\"",
            index + 1,
            device.vendor_id(),
            device.product_id(),
            manufacturer,
            product,
            serial,
            device.usage_page(),
            device.usage(),
            device.interface_number(),
            device.path().to_string_lossy()
        );
    }

    Ok(())
}

pub fn rumble_test_from_env() -> io::Result<()> {
    let mut side = JoyConSide::Right;
    let mut duration = Duration::from_millis(180);
    let mut intensity = 0.35_f32;
    let mut low_freq = DEFAULT_LOW_FREQ_HZ;
    let mut high_freq = DEFAULT_HIGH_FREQ_HZ;
    let mut args = env::args().skip(2);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--side" => {
                side = match next_arg(&mut args, "--side")?.as_str() {
                    "left" | "l" => JoyConSide::Left,
                    "right" | "r" => JoyConSide::Right,
                    value => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("--side must be left or right, got {value}"),
                        ));
                    }
                };
            }
            "--duration-ms" => {
                let millis = parse_u64(&next_arg(&mut args, "--duration-ms")?, "--duration-ms")?;
                duration = Duration::from_millis(millis.clamp(20, 1000));
            }
            "--intensity" => {
                intensity = parse_f32(&next_arg(&mut args, "--intensity")?, "--intensity")?
                    .clamp(0.05, 1.0);
            }
            "--low-freq" => {
                low_freq = parse_f32(&next_arg(&mut args, "--low-freq")?, "--low-freq")?;
            }
            "--high-freq" => {
                high_freq = parse_f32(&next_arg(&mut args, "--high-freq")?, "--high-freq")?;
            }
            "--help" | "-h" => {
                println!(
                    "joycon-rumble-test --side right|left --duration-ms 180 --intensity 0.35 --low-freq 160 --high-freq 320"
                );
                return Ok(());
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown joycon-rumble-test argument: {arg}"),
                ));
            }
        }
    }

    rumble_test(side, duration, intensity, low_freq, high_freq)
}

pub fn rumble_test(
    side: JoyConSide,
    duration: Duration,
    intensity: f32,
    low_freq: f32,
    high_freq: f32,
) -> io::Result<()> {
    let api = HidApi::new().map_err(to_io_error)?;
    let device_info = api
        .device_list()
        .find(|device| {
            device.vendor_id() == NINTENDO_VENDOR_ID && device.product_id() == side.product_id()
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} Joy-Con was not found via hidapi", side.label()),
            )
        })?;

    println!(
        "Opening {} Joy-Con {:04x}:{:04x} product=\"{}\" serial=\"{}\"",
        side.label(),
        device_info.vendor_id(),
        device_info.product_id(),
        device_info
            .product_string()
            .unwrap_or("(no product string)"),
        device_info.serial_number().unwrap_or("(no serial)")
    );

    let device = device_info.open_device(&api).map_err(to_io_error)?;
    let mut output = JoyConOutput::new(device);
    output.enable_vibration()?;

    println!(
        "Sending rumble: side={} intensity={:.2} low_freq={:.1} high_freq={:.1} duration_ms={}",
        side.label(),
        intensity,
        low_freq,
        high_freq,
        duration.as_millis()
    );
    output.rumble_for(duration, intensity, low_freq, high_freq)?;
    output.stop()?;
    println!("Rumble test finished.");
    Ok(())
}

pub fn imu_sweep_from_env() -> io::Result<()> {
    let mut side = JoyConSide::Right;
    let mut low_start = 80.0_f32;
    let mut low_end = 240.0_f32;
    let mut low_step = 10.0_f32;
    let mut amplitudes = vec![0.25_f32, 0.5, 0.75, 1.0];
    let mut measure_ms = 500_u64;
    let mut settle_ms = 150_u64;
    let mut output_path = PathBuf::from("joycon-imu-sweep.csv");
    let mut profile_path = PathBuf::from("joycon-rumble-profiles.toml");
    let mut args = env::args().skip(2);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--side" => {
                side = match next_arg(&mut args, "--side")?.as_str() {
                    "left" | "l" => JoyConSide::Left,
                    "right" | "r" => JoyConSide::Right,
                    value => {
                        return Err(invalid_arg(format!(
                            "--side must be left or right, got {value}"
                        )));
                    }
                };
            }
            "--low-start" => {
                low_start = parse_f32(&next_arg(&mut args, "--low-start")?, "--low-start")?
            }
            "--low-end" => low_end = parse_f32(&next_arg(&mut args, "--low-end")?, "--low-end")?,
            "--low-step" => {
                low_step = parse_f32(&next_arg(&mut args, "--low-step")?, "--low-step")?
            }
            "--amplitudes" => {
                amplitudes = next_arg(&mut args, "--amplitudes")?
                    .split(',')
                    .map(|value| parse_f32(value.trim(), "--amplitudes"))
                    .collect::<io::Result<Vec<_>>>()?;
            }
            "--measure-ms" => {
                measure_ms = parse_u64(&next_arg(&mut args, "--measure-ms")?, "--measure-ms")?
            }
            "--settle-ms" => {
                settle_ms = parse_u64(&next_arg(&mut args, "--settle-ms")?, "--settle-ms")?
            }
            "--output" => output_path = PathBuf::from(next_arg(&mut args, "--output")?),
            "--profile" => profile_path = PathBuf::from(next_arg(&mut args, "--profile")?),
            "--help" | "-h" => {
                println!(
                    "joycon-imu-sweep --side right|left --low-start 80 --low-end 240 \
                     --low-step 10 --amplitudes 0.25,0.5,0.75,1.0 \
                     --settle-ms 150 --measure-ms 500 --output joycon-imu-sweep.csv \
                     --profile joycon-rumble-profiles.toml"
                );
                return Ok(());
            }
            _ => {
                return Err(invalid_arg(format!(
                    "unknown joycon-imu-sweep argument: {arg}"
                )));
            }
        }
    }

    if !low_start.is_finite()
        || !low_end.is_finite()
        || !low_step.is_finite()
        || low_start <= 0.0
        || low_end < low_start
        || low_step <= 0.0
    {
        return Err(invalid_arg(
            "frequency range must be finite, positive, and ascending",
        ));
    }
    if amplitudes.is_empty()
        || amplitudes
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(invalid_arg(
            "--amplitudes must contain comma-separated values in 0..1",
        ));
    }
    if measure_ms < 100 || settle_ms > 10_000 {
        return Err(invalid_arg(
            "--measure-ms must be >= 100 and --settle-ms must be <= 10000",
        ));
    }

    imu_sweep(ImuSweepOptions {
        side,
        low_start,
        low_end,
        low_step,
        amplitudes,
        settle: Duration::from_millis(settle_ms),
        measure: Duration::from_millis(measure_ms),
        output_path,
        profile_path,
    })
}

struct ImuSweepOptions {
    side: JoyConSide,
    low_start: f32,
    low_end: f32,
    low_step: f32,
    amplitudes: Vec<f32>,
    settle: Duration,
    measure: Duration,
    output_path: PathBuf,
    profile_path: PathBuf,
}

fn imu_sweep(options: ImuSweepOptions) -> io::Result<()> {
    let ImuSweepOptions {
        side,
        low_start,
        low_end,
        low_step,
        amplitudes,
        settle,
        measure,
        output_path,
        profile_path,
    } = options;
    let api = HidApi::new().map_err(to_io_error)?;
    let info = api
        .device_list()
        .find(|device| {
            device.vendor_id() == NINTENDO_VENDOR_ID && device.product_id() == side.product_id()
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} Joy-Con was not found via hidapi", side.label()),
            )
        })?;
    let mut output = JoyConOutput::new(info.open_device(&api).map_err(to_io_error)?);
    output.enable_vibration()?;
    output.enable_imu()?;
    output.drain_input();

    println!(
        "Keep the {} Joy-Con stationary. Stabilizing IMU...",
        side.label()
    );
    output.collect_imu_samples(Duration::from_millis(1500), NEUTRAL_RUMBLE)?;
    output.drain_input();
    println!("Measuring three baseline windows...");
    let mut best_baseline: Option<(Vec<[i16; 3]>, [f64; 3], f64)> = None;
    for _ in 0..3 {
        let samples = output.collect_imu_samples(Duration::from_millis(500), NEUTRAL_RUMBLE)?;
        let mean = AccelStats::from_samples(&samples, [0.0; 3]).mean;
        let noise = AccelStats::from_samples(&samples, mean).rms;
        if best_baseline
            .as_ref()
            .is_none_or(|(_, _, best_noise)| noise < *best_noise)
        {
            best_baseline = Some((samples, mean, noise));
        }
    }
    let Some((baseline_samples, baseline_mean, baseline_noise)) = best_baseline else {
        output.stop()?;
        output.disable_imu()?;
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "IMU produced no standard 0x30 input reports",
        ));
    };
    if baseline_samples.is_empty() {
        output.stop()?;
        output.disable_imu()?;
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "IMU produced no standard 0x30 input reports",
        ));
    }
    println!(
        "baseline samples={} noise_rms_lsb={baseline_noise:.2}",
        baseline_samples.len()
    );

    let file = File::create(&output_path)?;
    let mut csv = BufWriter::new(file);
    writeln!(
        csv,
        "side,low_hz,high_hz,amplitude,samples,accel_rms_lsb,noise_corrected_rms_lsb,accel_peak_lsb"
    )?;
    let mut measurements = Vec::new();

    for amplitude in amplitudes {
        let mut low = low_start;
        while low <= low_end + low_step * 0.001 {
            let high = low * 2.0;
            let rumble = rumble_bytes(low, high, amplitude);
            output.collect_imu_samples(settle, rumble)?;
            output.drain_input();
            let samples = output.collect_imu_samples(measure, rumble)?;
            if samples.is_empty() {
                output.stop()?;
                output.disable_imu()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("IMU produced no samples at low={low:.1} high={high:.1}"),
                ));
            }
            let stats = AccelStats::from_samples(&samples, baseline_mean);
            let corrected = (stats.rms * stats.rms - baseline_noise * baseline_noise)
                .max(0.0)
                .sqrt();
            measurements.push((amplitude, low, high, corrected));
            writeln!(
                csv,
                "{},{low:.2},{high:.2},{amplitude:.3},{},{:.3},{corrected:.3},{:.3}",
                side.label().to_ascii_lowercase(),
                samples.len(),
                stats.rms,
                stats.peak
            )?;
            csv.flush()?;
            println!(
                "amplitude={amplitude:.2} low={low:.1} high={high:.1} samples={} rms={corrected:.1}",
                samples.len()
            );
            output.stop()?;
            thread::sleep(Duration::from_millis(80));
            low += low_step;
        }
    }
    output.stop()?;
    output.disable_imu()?;
    save_optimized_profile(side, &measurements, &profile_path)?;
    println!("IMU sweep finished: {}", output_path.display());
    println!("optimized profile saved: {}", profile_path.display());
    Ok(())
}

fn save_optimized_profile(
    side: JoyConSide,
    measurements: &[(f32, f32, f32, f64)],
    path: &PathBuf,
) -> io::Result<()> {
    let device_side = match side {
        JoyConSide::Left => DeviceSide::Left,
        JoyConSide::Right => DeviceSide::Right,
    };
    let ranges = [(0.0, 0.375), (0.375, 0.625), (0.625, 1.0)];
    let mut bands = Vec::new();
    for (minimum, maximum) in ranges {
        let best = measurements
            .iter()
            .filter(|(amplitude, _, _, _)| *amplitude > minimum && *amplitude <= maximum)
            .max_by(|left, right| left.3.total_cmp(&right.3));
        if let Some((_, low_hz, high_hz, rms)) = best {
            bands.push(CarrierBand {
                max_amplitude: maximum,
                low_hz: *low_hz,
                high_hz: *high_hz,
                measured_rms_lsb: *rms,
            });
        }
    }
    if bands.is_empty() {
        return Err(invalid_arg(
            "measurement did not cover any amplitude profile band",
        ));
    }

    let mut store = ImuProfileStore::load(path)?;
    if let Some(profile) = store
        .profiles
        .iter_mut()
        .find(|profile| profile.side == device_side)
    {
        profile.bands = bands;
    } else {
        store.profiles.push(ImuRumbleProfile {
            side: device_side,
            bands,
        });
    }
    store.save(path)
}

#[derive(Debug)]
struct AccelStats {
    mean: [f64; 3],
    rms: f64,
    peak: f64,
}

impl AccelStats {
    fn from_samples(samples: &[[i16; 3]], center: [f64; 3]) -> Self {
        if samples.is_empty() {
            return Self {
                mean: [0.0; 3],
                rms: 0.0,
                peak: 0.0,
            };
        }
        let count = samples.len() as f64;
        let mut mean = [0.0; 3];
        let mut energy = 0.0;
        let mut peak = 0.0_f64;
        for sample in samples {
            for axis in 0..3 {
                mean[axis] += f64::from(sample[axis]) / count;
            }
            let magnitude_sq = (0..3)
                .map(|axis| {
                    let value = f64::from(sample[axis]) - center[axis];
                    value * value
                })
                .sum::<f64>();
            energy += magnitude_sq;
            peak = peak.max(magnitude_sq.sqrt());
        }
        Self {
            mean,
            rms: (energy / count).sqrt(),
            peak,
        }
    }
}

fn invalid_arg(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

pub struct HidJoyConBackend {
    left: Option<JoyConOutput>,
    right: Option<JoyConOutput>,
    low_freq: f32,
    high_freq: f32,
    frames: [HapticDriveFrame; 2],
    next_keepalive: Instant,
    next_player_light_refresh: Instant,
    devices: Vec<DeviceConfig>,
    profiles: ImuProfileStore,
    capture_imu: bool,
}

impl HidJoyConBackend {
    pub fn new(
        low_freq: f32,
        high_freq: f32,
        devices: Vec<DeviceConfig>,
        profiles: ImuProfileStore,
        capture_imu: bool,
    ) -> Self {
        let now = Instant::now();
        Self {
            left: None,
            right: None,
            low_freq,
            high_freq,
            frames: [HapticDriveFrame::default(); 2],
            next_keepalive: now,
            next_player_light_refresh: now,
            devices,
            profiles,
            capture_imu,
        }
    }
}

impl RumbleBackend for HidJoyConBackend {
    fn connect(&mut self) -> io::Result<()> {
        self.discover_missing()?;
        if self.left.is_none() || self.right.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "both configured Joy-Cons must be connected",
            ));
        }
        eprintln!("bridge-status joycon-left=connected joycon-right=connected");
        Ok(())
    }

    fn poll(&mut self) -> io::Result<()> {
        let now = Instant::now();
        if now >= self.next_keepalive {
            self.keepalive()?;
            self.next_keepalive = now + Duration::from_secs(1);
        }
        if now >= self.next_player_light_refresh {
            self.refresh_player_lights()?;
            self.next_player_light_refresh = now + Duration::from_secs(5);
        }
        Ok(())
    }

    fn drive(&mut self, target: Target, frame: HapticDriveFrame) -> io::Result<()> {
        let (low_freq, high_freq) = self.map_frequencies(target, frame);
        let amplitude = frame.amplitude;
        self.frames[target_index(target)] = frame;
        let result = match self.output_for(target) {
            Some(output) if amplitude > 0.0 => {
                output.write_rumble_report(rumble_bytes(low_freq, high_freq, amplitude))
            }
            Some(output) => output.stop(),
            None => return Err(disconnected_error(target)),
        };
        self.handle_write_result(target, result)
    }

    fn stop(&mut self, target: Target) -> io::Result<()> {
        self.frames[target_index(target)] = HapticDriveFrame::default();
        let result = match self.output_for(target) {
            Some(output) => output.stop(),
            None => return Err(disconnected_error(target)),
        };
        self.handle_write_result(target, result)
    }

    fn take_imu_samples(&mut self) -> io::Result<Vec<ImuSample>> {
        if !self.capture_imu {
            return Ok(Vec::new());
        }
        let mut samples = Vec::new();
        for target in [Target::Left, Target::Right] {
            let Some(output) = self.output_for(target) else {
                continue;
            };
            samples.extend(
                output
                    .read_imu_samples()?
                    .into_iter()
                    .map(|acceleration| ImuSample {
                        target,
                        acceleration,
                    }),
            );
        }
        Ok(samples)
    }
}

impl HidJoyConBackend {
    fn map_frequencies(&self, target: Target, frame: HapticDriveFrame) -> (f32, f32) {
        let low_scale = self.low_freq / DEFAULT_LOW_FREQ_HZ;
        let high_scale = self.high_freq / DEFAULT_HIGH_FREQ_HZ;
        let low = frame.requested_frequency_hz * low_scale;
        let high = frame.requested_frequency_hz * 2.0 * high_scale;
        let vibration_mix = frame.vibration_mix.clamp(0.0, 1.0);
        // Select the strongest carrier measured by the Joy-Con's accelerometer
        // for each device-amplitude band. At full amplitude 90/180 Hz measured
        // about 1.52x the RMS acceleration of 160/320 Hz on the right Joy-Con.
        let side = match target {
            Target::Left => DeviceSide::Left,
            Target::Right => DeviceSide::Right,
        };
        let (measured_low, measured_high) =
            self.profiles
                .carrier(side, frame.amplitude)
                .unwrap_or(match frame.amplitude {
                    amplitude if amplitude <= 0.375 => (80.0, 160.0),
                    amplitude if amplitude <= 0.625 => (160.0, 320.0),
                    _ => (90.0, 180.0),
                });
        let vibration_low = measured_low * low_scale;
        let vibration_high = measured_high * high_scale;
        (
            low + (vibration_low - low) * vibration_mix,
            high + (vibration_high - high) * vibration_mix,
        )
    }

    fn discover_missing(&mut self) -> io::Result<()> {
        let api = HidApi::new().map_err(to_io_error)?;
        for device in api.device_list() {
            if device.vendor_id() != NINTENDO_VENDOR_ID {
                continue;
            }

            let side = match device.product_id() {
                JOYCON_LEFT_PRODUCT_ID => JoyConSide::Left,
                JOYCON_RIGHT_PRODUCT_ID => JoyConSide::Right,
                _ => continue,
            };
            if self.output_for_side(side).is_some() {
                continue;
            }
            let Some(binding) = self.binding(side).cloned() else {
                continue;
            };
            if binding.bluetooth_address != "auto" {
                let serial = device.serial_number().unwrap_or("");
                let Ok(serial_address) = normalize_bluetooth_address(serial) else {
                    continue;
                };
                if serial_address != binding.bluetooth_address {
                    continue;
                }
            }

            let hid_device = match device.open_device(&api) {
                Ok(device) => device,
                Err(error) => {
                    eprintln!("failed to open {} Joy-Con: {error}", side.label());
                    continue;
                }
            };

            let mut output = JoyConOutput::new(hid_device);
            if let Err(error) = output.enable_vibration() {
                eprintln!("failed to initialize {} Joy-Con: {error}", side.label());
                continue;
            }
            if self.capture_imu
                && let Err(error) = output.enable_imu()
            {
                eprintln!("failed to enable {} Joy-Con IMU: {error}", side.label());
                continue;
            }
            if let Err(error) =
                output.set_player_lights(player_light_mask(binding.id), NEUTRAL_RUMBLE)
            {
                eprintln!(
                    "failed to set {} Joy-Con player lights: {error}",
                    side.label()
                );
                continue;
            }
            eprintln!(
                "connected {} Joy-Con serial={} target={} id={}",
                side.label(),
                device.serial_number().unwrap_or("(no serial)"),
                binding.osc_address,
                binding.id
            );

            match side {
                JoyConSide::Left => self.left = Some(output),
                JoyConSide::Right => self.right = Some(output),
            }
        }
        Ok(())
    }

    fn keepalive(&mut self) -> io::Result<()> {
        for target in [Target::Left, Target::Right] {
            let frame = self.frames[target_index(target)];
            let (low_freq, high_freq) = self.map_frequencies(target, frame);
            let rumble = rumble_bytes(low_freq, high_freq, frame.amplitude);
            let result = match self.output_for(target) {
                Some(output) => output.write_rumble_report(rumble),
                None => return Err(disconnected_error(target)),
            };
            self.handle_write_result(target, result)?;
        }
        Ok(())
    }

    fn refresh_player_lights(&mut self) -> io::Result<()> {
        for (target, side) in [
            (Target::Left, JoyConSide::Left),
            (Target::Right, JoyConSide::Right),
        ] {
            let Some(id) = self.binding(side).map(|binding| binding.id) else {
                continue;
            };
            let frame = self.frames[target_index(target)];
            let (low_freq, high_freq) = self.map_frequencies(target, frame);
            let rumble = rumble_bytes(low_freq, high_freq, frame.amplitude);
            let result = match self.output_for(target) {
                Some(output) => output.set_player_lights(player_light_mask(id), rumble),
                None => return Err(disconnected_error(target)),
            };
            self.handle_write_result(target, result)?;
        }
        Ok(())
    }

    fn handle_write_result(&mut self, target: Target, result: io::Result<()>) -> io::Result<()> {
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.disconnect(target, &error);
                Err(error)
            }
        }
    }

    fn disconnect(&mut self, target: Target, error: &io::Error) {
        eprintln!("{target:?} Joy-Con disconnected: {error}; stopping bridge service");
        match target {
            Target::Left => self.left = None,
            Target::Right => self.right = None,
        }
        eprintln!(
            "bridge-status joycon-left={} joycon-right={}",
            if self.left.is_some() {
                "connected"
            } else {
                "disconnected"
            },
            if self.right.is_some() {
                "connected"
            } else {
                "disconnected"
            }
        );
    }

    fn output_for(&mut self, target: Target) -> Option<&mut JoyConOutput> {
        match target {
            Target::Left => self.left.as_mut(),
            Target::Right => self.right.as_mut(),
        }
    }

    fn output_for_side(&self, side: JoyConSide) -> Option<&JoyConOutput> {
        match side {
            JoyConSide::Left => self.left.as_ref(),
            JoyConSide::Right => self.right.as_ref(),
        }
    }

    fn binding(&self, side: JoyConSide) -> Option<&DeviceConfig> {
        let configured_side = match side {
            JoyConSide::Left => DeviceSide::Left,
            JoyConSide::Right => DeviceSide::Right,
        };
        self.devices
            .iter()
            .find(|device| device.side == configured_side)
    }
}

fn disconnected_error(target: Target) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        format!("{target:?} Joy-Con is disconnected"),
    )
}

const fn target_index(target: Target) -> usize {
    match target {
        Target::Left => 0,
        Target::Right => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoyConSide {
    Left,
    Right,
}

impl JoyConSide {
    fn product_id(self) -> u16 {
        match self {
            Self::Left => JOYCON_LEFT_PRODUCT_ID,
            Self::Right => JOYCON_RIGHT_PRODUCT_ID,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }
}

fn player_light_mask(id: u8) -> u8 {
    (1_u8 << id) - 1
}

struct JoyConOutput {
    device: HidDevice,
    packet_counter: u8,
}

impl JoyConOutput {
    fn new(device: HidDevice) -> Self {
        Self {
            device,
            packet_counter: 0,
        }
    }

    fn enable_vibration(&mut self) -> io::Result<()> {
        self.write_subcommand(0x48, &[0x01])?;
        thread::sleep(Duration::from_millis(20));
        Ok(())
    }

    fn enable_imu(&mut self) -> io::Result<()> {
        // Standard full input report, followed by IMU enable.
        self.write_subcommand(0x03, &[0x30])?;
        thread::sleep(Duration::from_millis(50));
        self.write_subcommand(0x40, &[0x01])?;
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    fn disable_imu(&mut self) -> io::Result<()> {
        self.write_subcommand(0x40, &[0x00])?;
        thread::sleep(Duration::from_millis(20));
        Ok(())
    }

    fn drain_input(&self) {
        let mut report = [0_u8; 64];
        for _ in 0..64 {
            match self.device.read_timeout(&mut report, 0) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }

    fn collect_imu_samples(
        &mut self,
        duration: Duration,
        rumble: [u8; 8],
    ) -> io::Result<Vec<[i16; 3]>> {
        let started = Instant::now();
        let mut samples = Vec::new();
        let mut report = [0_u8; 64];
        while started.elapsed() < duration {
            self.write_rumble_report(rumble)?;
            match self.device.read_timeout(&mut report, 15) {
                Ok(length) if length > 0 => {
                    samples.extend(parse_accelerometer_samples(&report[..length]))
                }
                Ok(_) => {}
                Err(error) => return Err(to_io_error(error)),
            }
        }
        Ok(samples)
    }

    fn read_imu_samples(&self) -> io::Result<Vec<[i16; 3]>> {
        let mut samples = Vec::new();
        let mut report = [0_u8; 64];
        for _ in 0..64 {
            match self.device.read_timeout(&mut report, 0) {
                Ok(0) => break,
                Ok(length) => samples.extend(parse_accelerometer_samples(&report[..length])),
                Err(error) => return Err(to_io_error(error)),
            }
        }
        Ok(samples)
    }

    fn set_player_lights(&mut self, mask: u8, rumble: [u8; 8]) -> io::Result<()> {
        self.write_subcommand_with_rumble(0x30, &[mask], rumble)?;
        thread::sleep(Duration::from_millis(20));
        Ok(())
    }

    fn rumble_for(
        &mut self,
        duration: Duration,
        intensity: f32,
        low_freq: f32,
        high_freq: f32,
    ) -> io::Result<()> {
        let started = Instant::now();
        let rumble = rumble_bytes(low_freq, high_freq, intensity);
        while started.elapsed() < duration {
            self.write_rumble_report(rumble)?;
            thread::sleep(Duration::from_millis(15));
        }
        Ok(())
    }

    fn stop(&mut self) -> io::Result<()> {
        for _ in 0..3 {
            self.write_rumble_report(NEUTRAL_RUMBLE)?;
            thread::sleep(Duration::from_millis(15));
        }
        Ok(())
    }

    fn write_subcommand(&mut self, subcommand: u8, args: &[u8]) -> io::Result<()> {
        self.write_subcommand_with_rumble(subcommand, args, NEUTRAL_RUMBLE)
    }

    fn write_subcommand_with_rumble(
        &mut self,
        subcommand: u8,
        args: &[u8],
        rumble: [u8; 8],
    ) -> io::Result<()> {
        let mut packet = Vec::with_capacity(11 + args.len());
        packet.push(0x01);
        packet.push(self.next_packet_number());
        packet.extend_from_slice(&rumble);
        packet.push(subcommand);
        packet.extend_from_slice(args);
        self.write_output(&packet)
    }

    fn write_rumble_report(&mut self, rumble: [u8; 8]) -> io::Result<()> {
        let mut packet = Vec::with_capacity(10);
        packet.push(0x10);
        packet.push(self.next_packet_number());
        packet.extend_from_slice(&rumble);
        self.write_output(&packet)
    }

    fn write_output(&self, packet: &[u8]) -> io::Result<()> {
        match self.device.write(packet) {
            Ok(_) => Ok(()),
            Err(first_error) => {
                let mut padded = [0_u8; 49];
                let len = packet.len().min(padded.len());
                padded[..len].copy_from_slice(&packet[..len]);
                self.device
                    .write(&padded)
                    .map(|_| ())
                    .map_err(|second_error| {
                        io::Error::other(format!(
                            "HID write failed: {first_error}; padded write failed: {second_error}"
                        ))
                    })
            }
        }
    }

    fn next_packet_number(&mut self) -> u8 {
        let value = self.packet_counter & 0x0f;
        self.packet_counter = self.packet_counter.wrapping_add(1) & 0x0f;
        value
    }
}

const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const DEFAULT_LOW_FREQ_HZ: f32 = 160.0;
const DEFAULT_HIGH_FREQ_HZ: f32 = 320.0;

fn parse_accelerometer_samples(report: &[u8]) -> Vec<[i16; 3]> {
    if report.first() != Some(&0x30) || report.len() < 49 {
        return Vec::new();
    }
    (0..3)
        .map(|sample| {
            let offset = 13 + sample * 12;
            [
                i16::from_le_bytes([report[offset], report[offset + 1]]),
                i16::from_le_bytes([report[offset + 2], report[offset + 3]]),
                i16::from_le_bytes([report[offset + 4], report[offset + 5]]),
            ]
        })
        .collect()
}

fn rumble_bytes(low_freq: f32, high_freq: f32, amplitude: f32) -> [u8; 8] {
    let base = encode_rumble(low_freq, high_freq, amplitude);
    [
        base[0], base[1], base[2], base[3], base[0], base[1], base[2], base[3],
    ]
}

fn encode_rumble(low_freq: f32, high_freq: f32, amplitude: f32) -> [u8; 4] {
    if amplitude <= 0.0 {
        return [0x00, 0x01, 0x40, 0x40];
    }

    let low_freq = low_freq.clamp(40.875_885, 626.286_13);
    let high_freq = high_freq.clamp(81.751_77, 1_252.572_3);
    let amplitude = amplitude.clamp(0.0, 1.0);

    let high_encoded = (((32.0 * (high_freq * 0.1).log2()).round() as i32 - 0x60) * 4) as u16;
    let low_encoded = ((32.0 * (low_freq * 0.1).log2()).round() as i32 - 0x40) as u8;

    let mut high_amp = encode_amplitude(amplitude);
    let mut low_amp = (high_amp as f32 * 0.5).round() as u16;
    let parity = low_amp & 0x01;
    if parity != 0 {
        low_amp -= 1;
    }
    low_amp = (low_amp >> 1) + 0x40;
    if parity != 0 {
        low_amp |= 0x8000;
    }

    high_amp -= high_amp % 2;

    [
        (high_encoded & 0xff) as u8,
        (((high_encoded >> 8) & 0xff) as u8).wrapping_add(high_amp),
        (((low_amp >> 8) & 0xff) as u8).wrapping_add(low_encoded),
        (low_amp & 0xff) as u8,
    ]
}

fn encode_amplitude(amplitude: f32) -> u8 {
    if amplitude <= 0.0 {
        0
    } else if amplitude < 0.117 {
        (((amplitude * 1000.0).log2() * 32.0 - 0x60 as f32) / (5.0 - amplitude.powi(2)) - 1.0)
            .round()
            .clamp(0.0, 255.0) as u8
    } else if amplitude < 0.23 {
        (((amplitude * 1000.0).log2() * 32.0 - 0x60 as f32) - 0x5c as f32)
            .round()
            .clamp(0.0, 255.0) as u8
    } else {
        ((((amplitude * 1000.0).log2() * 32.0 - 0x60 as f32) * 2.0) - 0xf6 as f32)
            .round()
            .clamp(0.0, 255.0) as u8
    }
}

fn next_arg(args: &mut impl Iterator<Item = String>, name: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{name} needs a value")))
}

fn parse_u64(value: &str, name: &str) -> io::Result<u64> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be integer"),
        )
    })
}

fn parse_f32(value: &str, name: &str) -> io::Result<f32> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must be number"),
        )
    })
}

fn to_io_error(error: hidapi::HidError) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_player_led_patterns() {
        assert_eq!(player_light_mask(1), 0x01);
        assert_eq!(player_light_mask(2), 0x03);
        assert_eq!(player_light_mask(4), 0x0f);
    }

    #[test]
    fn zero_amplitude_uses_neutral_rumble() {
        assert_eq!(
            rumble_bytes(DEFAULT_LOW_FREQ_HZ, DEFAULT_HIGH_FREQ_HZ, 0.0),
            NEUTRAL_RUMBLE
        );
    }

    #[test]
    fn encodes_frequency_and_amplitude_data_for_both_motors() {
        let data = rumble_bytes(160.0, 320.0, 1.0);
        assert_eq!(&data[..4], &[0x00, 0xc9, 0x40, 0x72]);
        assert_eq!(&data[..4], &data[4..]);
    }

    #[test]
    fn amplitude_changes_encoded_bytes() {
        let weak = rumble_bytes(160.0, 320.0, 0.25);
        let strong = rumble_bytes(160.0, 320.0, 1.0);
        assert_ne!(weak, strong);
    }

    #[test]
    fn full_amplitude_vibration_uses_measured_strongest_carrier() {
        let backend =
            HidJoyConBackend::new(160.0, 320.0, Vec::new(), ImuProfileStore::default(), false);
        let frequencies = backend.map_frequencies(
            Target::Right,
            HapticDriveFrame {
                amplitude: 1.0,
                requested_frequency_hz: 20.75,
                vibration_mix: 1.0,
                vibration_level: 0.05,
                duration: Duration::from_millis(50),
            },
        );
        assert_eq!(frequencies, (90.0, 180.0));
    }

    #[test]
    fn force_only_keeps_requested_carrier() {
        let backend =
            HidJoyConBackend::new(160.0, 320.0, Vec::new(), ImuProfileStore::default(), false);
        let frequencies = backend.map_frequencies(
            Target::Right,
            HapticDriveFrame {
                amplitude: 0.5,
                requested_frequency_hz: 90.0,
                vibration_mix: 0.0,
                vibration_level: 0.0,
                duration: Duration::from_millis(50),
            },
        );
        assert_eq!(frequencies, (90.0, 180.0));
    }

    #[test]
    fn vibration_uses_measured_carrier_for_each_amplitude_band() {
        let backend =
            HidJoyConBackend::new(160.0, 320.0, Vec::new(), ImuProfileStore::default(), false);
        let low = backend.map_frequencies(
            Target::Right,
            HapticDriveFrame {
                amplitude: 0.25,
                requested_frequency_hz: 20.75,
                vibration_mix: 1.0,
                vibration_level: 0.05,
                duration: Duration::from_millis(50),
            },
        );
        let medium = backend.map_frequencies(
            Target::Right,
            HapticDriveFrame {
                amplitude: 0.5,
                requested_frequency_hz: 320.0,
                vibration_mix: 1.0,
                vibration_level: 1.0,
                duration: Duration::from_millis(50),
            },
        );
        let high = backend.map_frequencies(
            Target::Right,
            HapticDriveFrame {
                amplitude: 1.0,
                requested_frequency_hz: 320.0,
                vibration_mix: 1.0,
                vibration_level: 1.0,
                duration: Duration::from_millis(50),
            },
        );
        assert_eq!(low, (80.0, 160.0));
        assert_eq!(medium, (160.0, 320.0));
        assert_eq!(high, (90.0, 180.0));
    }

    #[test]
    fn parses_three_accelerometer_samples_from_standard_report() {
        let mut report = [0_u8; 49];
        report[0] = 0x30;
        for (sample_index, values) in [[1_i16, -2, 3], [4, -5, 6], [7, -8, 9]]
            .into_iter()
            .enumerate()
        {
            let offset = 13 + sample_index * 12;
            for (axis, value) in values.into_iter().enumerate() {
                report[offset + axis * 2..offset + axis * 2 + 2]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        assert_eq!(
            parse_accelerometer_samples(&report),
            vec![[1, -2, 3], [4, -5, 6], [7, -8, 9]]
        );
    }

    #[test]
    fn ignores_non_imu_reports() {
        assert!(parse_accelerometer_samples(&[0x21; 49]).is_empty());
        assert!(parse_accelerometer_samples(&[0x30; 48]).is_empty());
    }

    #[test]
    fn optimized_profile_selects_best_carrier_in_each_band() {
        let path = std::env::temp_dir().join(format!(
            "joycon-rumble-profile-test-{}.toml",
            std::process::id()
        ));
        let measurements = vec![
            (0.25, 80.0, 160.0, 100.0),
            (0.25, 90.0, 180.0, 50.0),
            (0.5, 150.0, 300.0, 200.0),
            (0.5, 160.0, 320.0, 300.0),
            (1.0, 90.0, 180.0, 500.0),
            (1.0, 180.0, 360.0, 400.0),
        ];
        save_optimized_profile(JoyConSide::Right, &measurements, &path).unwrap();
        let store = ImuProfileStore::load(&path).unwrap();
        assert_eq!(store.carrier(DeviceSide::Right, 0.25), Some((80.0, 160.0)));
        assert_eq!(store.carrier(DeviceSide::Right, 0.5), Some((160.0, 320.0)));
        assert_eq!(store.carrier(DeviceSide::Right, 1.0), Some((90.0, 180.0)));
        fs::remove_file(path).unwrap();
    }
}
