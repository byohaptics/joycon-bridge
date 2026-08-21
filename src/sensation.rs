use std::f32::consts::PI;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SensationPoint {
    pub force: f32,
    pub vibration: f32,
    pub pain: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SensationState {
    pub left: SensationPoint,
    pub right: SensationPoint,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HapticDriveFrame {
    pub amplitude: f32,
    pub requested_frequency_hz: f32,
    pub vibration_mix: f32,
    pub vibration_level: f32,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
struct XorShift32(u32);

impl Default for XorShift32 {
    fn default() -> Self {
        Self(0x6d2b_79f5)
    }
}

impl XorShift32 {
    fn next_f32(&mut self) -> f32 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.0 = value;
        (value as f64 / u32::MAX as f64) as f32
    }
}

pub struct SensationDriveEngine {
    pain_phase: f32,
    random: XorShift32,
}

impl SensationDriveEngine {
    pub fn reset(&mut self) {
        self.pain_phase = 0.0;
    }

    pub fn update(&mut self, state: SensationState, delta_time: Duration) -> [HapticDriveFrame; 2] {
        let left = sanitize_point(state.left);
        let right = sanitize_point(state.right);
        let maximum_pain = left.pain.max(right.pain);
        let phase_rate = 2.0 * PI * (4.0 / 3.0 + maximum_pain);
        self.pain_phase =
            (self.pain_phase + phase_rate * delta_time.as_secs_f32()).rem_euclid(4.0 * PI);
        let envelope = pain_envelope(self.pain_phase);

        [
            render_point(left, envelope, self.random.next_f32()),
            render_point(right, envelope, self.random.next_f32()),
        ]
    }
}

impl Default for SensationDriveEngine {
    fn default() -> Self {
        Self {
            pain_phase: 0.0,
            random: XorShift32::default(),
        }
    }
}

fn sanitize(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn sanitize_point(point: SensationPoint) -> SensationPoint {
    SensationPoint {
        force: sanitize(point.force),
        vibration: sanitize(point.vibration),
        pain: sanitize(point.pain),
    }
}

fn pain_envelope(phase: f32) -> f32 {
    if (phase * 0.5).sin() > 0.0 {
        phase.sin().abs().powf(0.25)
    } else {
        0.0
    }
}

fn render_point(point: SensationPoint, pain_envelope: f32, random_01: f32) -> HapticDriveFrame {
    let force_amplitude = point.force;
    let force_frequency = 20.0 + 140.0 * point.force;
    let vibration_amplitude = (20.0 * point.vibration).clamp(0.0, 1.0);
    let vibration_frequency = 5.0 + 315.0 * point.vibration;
    let pain_amplitude = point.pain.powf(0.25) * (pain_envelope + 0.2 * random_01);
    let pain_frequency = 60.0 + 80.0 * pain_envelope + point.pain * (20.0 + 40.0 * pain_envelope);
    let weight = point.force + point.vibration + point.pain;

    if weight <= 0.0 {
        return HapticDriveFrame {
            duration: Duration::from_millis(50),
            ..HapticDriveFrame::default()
        };
    }

    HapticDriveFrame {
        amplitude: ((force_amplitude * point.force
            + vibration_amplitude * point.vibration
            + pain_amplitude * point.pain)
            / weight)
            .clamp(0.0, 1.0),
        requested_frequency_hz: (force_frequency * point.force
            + vibration_frequency * point.vibration
            + pain_frequency * point.pain)
            / weight,
        vibration_mix: point.vibration / weight,
        vibration_level: point.vibration,
        duration: Duration::from_millis(50),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_engine() -> SensationDriveEngine {
        SensationDriveEngine {
            pain_phase: 0.0,
            random: XorShift32(0),
        }
    }

    fn frame(point: SensationPoint) -> HapticDriveFrame {
        render_point(point, 0.0, 0.0)
    }

    #[test]
    fn force_only_is_linear() {
        let output = frame(SensationPoint {
            force: 0.5,
            ..SensationPoint::default()
        });
        assert!((output.amplitude - 0.5).abs() < 0.0001);
        assert!((output.requested_frequency_hz - 90.0).abs() < 0.0001);
        assert_eq!(output.vibration_mix, 0.0);
        assert_eq!(output.vibration_level, 0.0);
    }

    #[test]
    fn vibration_reaches_full_amplitude_at_five_percent() {
        let output = frame(SensationPoint {
            vibration: 0.05,
            ..SensationPoint::default()
        });
        assert!((output.amplitude - 1.0).abs() < 0.0001);
        assert!((output.requested_frequency_hz - 20.75).abs() < 0.0001);
        assert_eq!(output.vibration_mix, 1.0);
        assert_eq!(output.vibration_level, 0.05);
    }

    #[test]
    fn zero_input_stops() {
        assert_eq!(frame(SensationPoint::default()).amplitude, 0.0);
    }

    #[test]
    fn pain_uses_shared_phase_for_both_sides() {
        let mut engine = fixed_engine();
        let outputs = engine.update(
            SensationState {
                left: SensationPoint {
                    pain: 0.5,
                    ..SensationPoint::default()
                },
                right: SensationPoint {
                    pain: 0.5,
                    ..SensationPoint::default()
                },
            },
            Duration::from_millis(50),
        );
        assert_eq!(outputs[0], outputs[1]);
    }

    #[test]
    fn non_finite_values_are_sanitized() {
        let mut engine = fixed_engine();
        let outputs = engine.update(
            SensationState {
                left: SensationPoint {
                    force: f32::NAN,
                    vibration: f32::INFINITY,
                    pain: f32::NEG_INFINITY,
                },
                ..SensationState::default()
            },
            Duration::from_millis(50),
        );
        assert_eq!(outputs[0].amplitude, 0.0);
    }
}
