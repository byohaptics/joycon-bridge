use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::sensation::{HapticDriveFrame, SensationState};
use crate::signal::{Sensation, Target};

pub struct TraceRecorder {
    writer: BufWriter<File>,
    started: Instant,
    next_flush: Instant,
}

impl TraceRecorder {
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut writer = BufWriter::new(File::create(path)?);
        writeln!(
            writer,
            "unix_seconds,elapsed_seconds,stage,target,sensation,value,force,vibration,pain,amplitude,frequency_hz,vibration_mix,vibration_level,imu_x,imu_y,imu_z,result"
        )?;
        writer.flush()?;
        let started = Instant::now();
        Ok(Self {
            writer,
            started,
            next_flush: started + Duration::from_millis(100),
        })
    }

    pub fn osc(&mut self, target: Target, sensation: Sensation, value: f32, state: SensationState) {
        let point = match target {
            Target::Left => state.left,
            Target::Right => state.right,
        };
        self.write(
            "osc_input",
            target,
            Some(sensation),
            value,
            point.force,
            point.vibration,
            point.pain,
            HapticDriveFrame::default(),
            "ok",
            None,
        );
    }

    pub fn frame(&mut self, target: Target, state: SensationState, frame: HapticDriveFrame) {
        let point = match target {
            Target::Left => state.left,
            Target::Right => state.right,
        };
        self.write(
            "drive_frame",
            target,
            None,
            0.0,
            point.force,
            point.vibration,
            point.pain,
            frame,
            "ok",
            None,
        );
    }

    pub fn hid(&mut self, target: Target, frame: HapticDriveFrame, result: &io::Result<()>) {
        self.write(
            if frame.amplitude > 0.0 {
                "hid_drive"
            } else {
                "hid_stop"
            },
            target,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
            frame,
            if result.is_ok() { "ok" } else { "error" },
            None,
        );
    }

    pub fn imu(&mut self, target: Target, acceleration: [i16; 3]) {
        self.write(
            "imu",
            target,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
            HapticDriveFrame::default(),
            "ok",
            Some(acceleration),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        stage: &str,
        target: Target,
        sensation: Option<Sensation>,
        value: f32,
        force: f32,
        vibration: f32,
        pain: f32,
        frame: HapticDriveFrame,
        result: &str,
        acceleration: Option<[i16; 3]>,
    ) {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |duration| duration.as_secs_f64());
        let elapsed = self.started.elapsed().as_secs_f64();
        let sensation = sensation.map_or("", |value| match value {
            Sensation::Force => "force",
            Sensation::Vibration => "vibration",
            Sensation::Pain => "pain",
        });
        let _ = writeln!(
            self.writer,
            "{unix:.9},{elapsed:.9},{stage},{},{sensation},{value:.9},{force:.9},{vibration:.9},{pain:.9},{:.9},{:.9},{:.9},{:.9},{},{},{},{result}",
            match target {
                Target::Left => "left",
                Target::Right => "right",
            },
            frame.amplitude,
            frame.requested_frequency_hz,
            frame.vibration_mix,
            frame.vibration_level,
            acceleration.map_or(String::new(), |value| value[0].to_string()),
            acceleration.map_or(String::new(), |value| value[1].to_string()),
            acceleration.map_or(String::new(), |value| value[2].to_string()),
        );
        if Instant::now() >= self.next_flush {
            let _ = self.writer.flush();
            self.next_flush = Instant::now() + Duration::from_millis(100);
        }
    }
}

impl Drop for TraceRecorder {
    fn drop(&mut self) {
        let _ = self.writer.flush();
    }
}
