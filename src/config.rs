use std::env;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_CONFIG_PATH: &str = "joycon-rumble.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceSide {
    Left,
    Right,
}

impl DeviceSide {
    pub fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub side: DeviceSide,
    pub bluetooth_address: String,
    pub osc_address: String,
    pub id: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub listen: SocketAddr,
    pub namespace: String,
    pub heartbeat_timeout_ms: u64,
    pub low_freq_hz: f32,
    pub high_freq_hz: f32,
    pub imu_profile_path: PathBuf,
    pub devices: Vec<DeviceConfig>,
    #[serde(skip)]
    pub dry_run: bool,
    #[serde(skip)]
    pub trace_csv: Option<PathBuf>,
    #[serde(skip)]
    pub config_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:9010".parse().expect("valid default address"),
            namespace: "joyconrumble".into(),
            heartbeat_timeout_ms: 2000,
            low_freq_hz: 160.0,
            high_freq_hz: 320.0,
            imu_profile_path: PathBuf::from("joycon-rumble-profiles.toml"),
            devices: vec![
                DeviceConfig {
                    side: DeviceSide::Left,
                    bluetooth_address: "auto".into(),
                    osc_address: "left".into(),
                    id: 1,
                },
                DeviceConfig {
                    side: DeviceSide::Right,
                    bluetooth_address: "auto".into(),
                    osc_address: "right".into(),
                    id: 2,
                },
            ],
            dry_run: false,
            trace_csv: None,
            config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
        }
    }
}

impl Config {
    pub fn load() -> io::Result<Self> {
        let args: Vec<String> = env::args().skip(1).collect();
        Self::load_from_args(args)
    }

    fn load_from_args(args: Vec<String>) -> io::Result<Self> {
        let config_path = find_config_path(&args)?;
        let mut config = if config_path.exists() {
            let source = fs::read_to_string(&config_path)?;
            toml::from_str::<Config>(&source).map_err(|error| {
                invalid(&format!(
                    "failed to parse config {}: {error}",
                    config_path.display()
                ))
            })?
        } else {
            Config::default()
        };
        config.config_path = config_path;

        let mut save_config = false;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--config" => index += 2,
                "--listen" => {
                    config.listen = value(&args, index, "--listen")?
                        .parse()
                        .map_err(|_| invalid("--listen must be host:port"))?;
                    index += 2;
                }
                "--listen-port" => {
                    let port = parse_u16(value(&args, index, "--listen-port")?, "--listen-port")?;
                    config.listen = SocketAddr::new(config.listen.ip(), port);
                    index += 2;
                }
                "--namespace" => {
                    config.namespace = value(&args, index, "--namespace")?.to_owned();
                    index += 2;
                }
                "--heartbeat-timeout-ms" => {
                    config.heartbeat_timeout_ms = parse_u64(
                        value(&args, index, "--heartbeat-timeout-ms")?,
                        "--heartbeat-timeout-ms",
                    )?;
                    index += 2;
                }
                "--low-freq" => {
                    config.low_freq_hz =
                        parse_f32(value(&args, index, "--low-freq")?, "--low-freq")?;
                    index += 2;
                }
                "--high-freq" => {
                    config.high_freq_hz =
                        parse_f32(value(&args, index, "--high-freq")?, "--high-freq")?;
                    index += 2;
                }
                "--imu-profile" => {
                    config.imu_profile_path = PathBuf::from(value(&args, index, "--imu-profile")?);
                    index += 2;
                }
                "--device" => {
                    let device = parse_device(value(&args, index, "--device")?)?;
                    if let Some(existing) = config
                        .devices
                        .iter_mut()
                        .find(|existing| existing.side == device.side)
                    {
                        *existing = device;
                    } else {
                        config.devices.push(device);
                    }
                    index += 2;
                }
                "--dry-run" => {
                    config.dry_run = true;
                    index += 1;
                }
                "--trace-csv" => {
                    config.trace_csv = Some(PathBuf::from(value(&args, index, "--trace-csv")?));
                    index += 2;
                }
                "--save-config" => {
                    save_config = true;
                    index += 1;
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                argument => return Err(invalid(&format!("unknown argument: {argument}"))),
            }
        }

        config.validate()?;
        if save_config {
            config.save()?;
        }
        Ok(config)
    }

    fn validate(&mut self) -> io::Result<()> {
        validate_path_segment(&self.namespace, "namespace")?;
        if self.listen.port() == 0 {
            return Err(invalid("listen port must be in 1..65535"));
        }
        if self.heartbeat_timeout_ms == 0 {
            return Err(invalid("heartbeat_timeout_ms must be greater than zero"));
        }
        if self.devices.len() != 2
            || !self
                .devices
                .iter()
                .any(|device| device.side == DeviceSide::Left)
            || !self
                .devices
                .iter()
                .any(|device| device.side == DeviceSide::Right)
        {
            return Err(invalid("config must contain one left and one right device"));
        }
        for device in &mut self.devices {
            device.bluetooth_address = normalize_binding_address(&device.bluetooth_address)?;
            validate_path_segment(&device.osc_address, "device osc_address")?;
            if !(1..=4).contains(&device.id) {
                return Err(invalid("device id must be in 1..4"));
            }
        }
        if self.devices[0].osc_address == self.devices[1].osc_address {
            return Err(invalid(
                "left and right device osc_address values must be unique",
            ));
        }
        Ok(())
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.config_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let output = toml::to_string_pretty(self)
            .map_err(|error| io::Error::other(format!("config serialization failed: {error}")))?;
        fs::write(&self.config_path, output)?;
        eprintln!("saved config: {}", self.config_path.display());
        Ok(())
    }
}

fn find_config_path(args: &[String]) -> io::Result<PathBuf> {
    let mut path = PathBuf::from(DEFAULT_CONFIG_PATH);
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--config" {
            path = PathBuf::from(value(args, index, "--config")?);
            index += 2;
        } else {
            index += 1;
        }
    }
    Ok(path)
}

fn parse_device(value: &str) -> io::Result<DeviceConfig> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(invalid(
            "--device must be SIDE,AUTO_OR_BLUETOOTH_ADDRESS,OSC_ADDRESS,ID",
        ));
    }
    let side = match parts[0].to_ascii_lowercase().as_str() {
        "left" | "l" => DeviceSide::Left,
        "right" | "r" => DeviceSide::Right,
        _ => return Err(invalid("device side must be left or right")),
    };
    Ok(DeviceConfig {
        side,
        bluetooth_address: normalize_binding_address(parts[1])?,
        osc_address: parts[2].to_owned(),
        id: parts[3]
            .parse()
            .map_err(|_| invalid("device id must be an integer"))?,
    })
}

fn normalize_binding_address(value: &str) -> io::Result<String> {
    if value.eq_ignore_ascii_case("auto") {
        Ok("auto".into())
    } else {
        normalize_bluetooth_address(value)
    }
}

pub fn normalize_bluetooth_address(value: &str) -> io::Result<String> {
    let normalized: String = value
        .chars()
        .filter(|character| !matches!(character, ':' | '-' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    if normalized.len() != 12
        || !normalized
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(invalid(
            "Bluetooth address must contain exactly 12 hexadecimal digits",
        ));
    }
    Ok(normalized)
}

fn validate_path_segment(value: &str, name: &str) -> io::Result<()> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(invalid(&format!(
            "{name} must use only ASCII letters, digits, '.', '-' or '_'"
        )));
    }
    Ok(())
}

fn value<'a>(args: &'a [String], index: usize, name: &str) -> io::Result<&'a str> {
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| invalid(&format!("{name} requires a value")))
}

fn parse_f32(value: &str, name: &str) -> io::Result<f32> {
    value
        .parse()
        .map_err(|_| invalid(&format!("{name} must be a number")))
}

fn parse_u64(value: &str, name: &str) -> io::Result<u64> {
    value
        .parse()
        .map_err(|_| invalid(&format!("{name} must be an integer")))
}

fn parse_u16(value: &str, name: &str) -> io::Result<u16> {
    value
        .parse()
        .map_err(|_| invalid(&format!("{name} must be an integer in 1..65535")))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn print_help() {
    println!(
        "joycon-rumble-bridge\n\
         --config joycon-rumble.toml\n\
         --listen 127.0.0.1:9010\n\
         --listen-port 9010\n\
         --namespace joyconrumble\n\
         --heartbeat-timeout-ms 2000\n\
         --low-freq 160\n\
         --high-freq 320\n\
         --imu-profile joycon-rumble-profiles.toml\n\
         --device left,auto,left,1\n\
         --device right,auto,right,2\n\
         --save-config\n\
         --dry-run
         --trace-csv bridge-haptics.csv"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bluetooth_address() {
        assert_eq!(
            normalize_bluetooth_address("00:11:22:33:44:55").unwrap(),
            "001122334455"
        );
    }

    #[test]
    fn parses_device_override() {
        assert_eq!(
            parse_device("left,00-11-22-33-44-55,hand-left,3").unwrap(),
            DeviceConfig {
                side: DeviceSide::Left,
                bluetooth_address: "001122334455".into(),
                osc_address: "hand-left".into(),
                id: 3,
            }
        );
    }

    #[test]
    fn parses_automatic_device_binding() {
        assert_eq!(
            parse_device("right,auto,right,2")
                .unwrap()
                .bluetooth_address,
            "auto"
        );
    }

    #[test]
    fn cli_overrides_file_and_save_persists_effective_values() {
        let path = std::env::temp_dir().join(format!(
            "joycon-rumble-config-test-{}.toml",
            std::process::id()
        ));
        fs::write(
            &path,
            r#"listen = "127.0.0.1:7000"
namespace = "from-file"
heartbeat_timeout_ms = 2000
low_freq_hz = 160.0
high_freq_hz = 320.0

[[devices]]
side = "left"
bluetooth_address = "001122334455"
osc_address = "file-left"
id = 1

[[devices]]
side = "right"
bluetooth_address = "66778899aabb"
osc_address = "file-right"
id = 2
"#,
        )
        .unwrap();

        let config = Config::load_from_args(vec![
            "--config".into(),
            path.to_string_lossy().into_owned(),
            "--listen-port".into(),
            "9010".into(),
            "--device".into(),
            "left,001122334455,left,3".into(),
            "--save-config".into(),
        ])
        .unwrap();

        assert_eq!(config.listen.to_string(), "127.0.0.1:9010");
        assert_eq!(config.namespace, "from-file");
        let left = config
            .devices
            .iter()
            .find(|device| device.side == DeviceSide::Left)
            .unwrap();
        assert_eq!(left.bluetooth_address, "001122334455");
        assert_eq!(left.osc_address, "left");
        assert_eq!(left.id, 3);

        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("127.0.0.1:9010"));
        assert!(saved.contains("001122334455"));
        fs::remove_file(path).unwrap();
    }
}
