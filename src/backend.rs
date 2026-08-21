use crate::sensation::HapticDriveFrame;
use crate::signal::Target;
use std::io;

pub trait RumbleBackend {
    fn connect(&mut self) -> io::Result<()>;
    fn poll(&mut self) -> io::Result<()>;
    fn drive(&mut self, target: Target, frame: HapticDriveFrame) -> io::Result<()>;
    fn stop(&mut self, target: Target) -> io::Result<()>;
    fn take_imu_samples(&mut self) -> io::Result<Vec<ImuSample>>;
}

#[derive(Debug, Clone, Copy)]
pub struct ImuSample {
    pub target: Target,
    pub acceleration: [i16; 3],
}

pub struct ConsoleBackend;

impl RumbleBackend for ConsoleBackend {
    fn connect(&mut self) -> io::Result<()> {
        eprintln!("dry-run backend active");
        Ok(())
    }

    fn poll(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn drive(&mut self, target: Target, frame: HapticDriveFrame) -> io::Result<()> {
        eprintln!(
            "drive target={target:?} amplitude={:.3} frequency_hz={:.1} vibration_mix={:.3} vibration_level={:.3}",
            frame.amplitude,
            frame.requested_frequency_hz,
            frame.vibration_mix,
            frame.vibration_level
        );
        Ok(())
    }

    fn stop(&mut self, target: Target) -> io::Result<()> {
        eprintln!("stop target={target:?}");
        Ok(())
    }

    fn take_imu_samples(&mut self) -> io::Result<Vec<ImuSample>> {
        Ok(Vec::new())
    }
}
