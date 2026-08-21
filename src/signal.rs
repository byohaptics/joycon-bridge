use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::osc::{OscArgument, OscPacket};
use crate::sensation::{SensationPoint, SensationState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensation {
    Force,
    Vibration,
    Pain,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HapticEvent {
    Sensation {
        target: Target,
        sensation: Sensation,
        value: f32,
    },
    HeartbeatRestored,
}

#[derive(Debug)]
pub struct HapticRouter {
    heartbeat_address: String,
    channel_prefix: String,
    heartbeat_timeout: Duration,
    targets: HashMap<String, Target>,
    last_heartbeat: Option<Instant>,
    state: SensationState,
    timed_out: bool,
}

impl HapticRouter {
    pub fn new(
        namespace: &str,
        heartbeat_timeout: Duration,
        targets: impl IntoIterator<Item = (String, Target)>,
    ) -> Self {
        let base = format!("/avatar/parameters/{namespace}");
        Self {
            heartbeat_address: format!("{base}/heartbeat"),
            channel_prefix: format!("{base}/channel/"),
            heartbeat_timeout,
            targets: targets.into_iter().collect(),
            last_heartbeat: None,
            state: SensationState::default(),
            timed_out: true,
        }
    }

    pub fn route_packet(&mut self, packet: &OscPacket, now: Instant) -> Option<HapticEvent> {
        if packet.address == self.heartbeat_address {
            if !matches!(packet.arguments.as_slice(), [OscArgument::Int(_)]) {
                return None;
            }
            let restored = !self.heartbeat_valid(now);
            self.last_heartbeat = Some(now);
            self.timed_out = false;
            return restored.then_some(HapticEvent::HeartbeatRestored);
        }

        let relative = packet.address.strip_prefix(&self.channel_prefix)?;
        let mut segments = relative.split('/');
        let channel = segments.next()?;
        let sensation_name = segments.next();
        if segments.next().is_some() {
            return None;
        }
        let target = self.targets.get(channel).copied()?;
        let sensation = match sensation_name {
            Some("force") => Sensation::Force,
            Some("vibration") => Sensation::Vibration,
            Some("pain") => Sensation::Pain,
            _ => return None,
        };
        let [OscArgument::Float(raw_value)] = packet.arguments.as_slice() else {
            return None;
        };
        if !raw_value.is_finite() {
            return None;
        }

        let value = raw_value.clamp(0.0, 1.0);
        let point = self.point_mut(target);
        match sensation {
            Sensation::Force => point.force = value,
            Sensation::Vibration => point.vibration = value,
            Sensation::Pain => point.pain = value,
        }
        Some(HapticEvent::Sensation {
            target,
            sensation,
            value,
        })
    }

    pub fn heartbeat_valid(&self, now: Instant) -> bool {
        self.last_heartbeat.is_some_and(|received| {
            now.saturating_duration_since(received) < self.heartbeat_timeout
        })
    }

    pub fn take_timeout(&mut self, now: Instant) -> bool {
        if !self.timed_out && !self.heartbeat_valid(now) {
            self.timed_out = true;
            return true;
        }
        false
    }

    pub fn state(&self) -> SensationState {
        self.state
    }

    fn point_mut(&mut self, target: Target) -> &mut SensationPoint {
        match target {
            Target::Left => &mut self.state.left,
            Target::Right => &mut self.state.right,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(address: &str, argument: OscArgument) -> OscPacket {
        OscPacket {
            address: address.into(),
            arguments: vec![argument],
        }
    }

    fn router() -> HapticRouter {
        HapticRouter::new(
            "joycon",
            Duration::from_secs(2),
            [
                ("left".into(), Target::Left),
                ("right".into(), Target::Right),
            ],
        )
    }

    #[test]
    fn routes_independent_sensations() {
        let now = Instant::now();
        let mut router = router();
        router.route_packet(
            &packet(
                "/avatar/parameters/joycon/channel/left/vibration",
                OscArgument::Float(0.25),
            ),
            now,
        );
        router.route_packet(
            &packet(
                "/avatar/parameters/joycon/channel/left/pain",
                OscArgument::Float(0.75),
            ),
            now,
        );
        assert_eq!(router.state().left.vibration, 0.25);
        assert_eq!(router.state().left.pain, 0.75);
        assert_eq!(router.state().left.force, 0.0);
    }

    #[test]
    fn rejects_unknown_sensation() {
        let mut router = router();
        assert_eq!(
            router.route_packet(
                &packet(
                    "/avatar/parameters/joycon/channel/left/temperature",
                    OscArgument::Float(0.5),
                ),
                Instant::now(),
            ),
            None
        );
    }

    #[test]
    fn heartbeat_expires_once_after_two_seconds() {
        let start = Instant::now();
        let mut router = router();
        router.route_packet(
            &packet("/avatar/parameters/joycon/heartbeat", OscArgument::Int(0)),
            start,
        );
        assert!(!router.take_timeout(start + Duration::from_millis(1999)));
        assert!(router.take_timeout(start + Duration::from_secs(2)));
        assert!(!router.take_timeout(start + Duration::from_secs(3)));
    }

    #[test]
    fn routes_custom_target_name() {
        let mut router = HapticRouter::new(
            "joycon",
            Duration::from_secs(2),
            [("waist".into(), Target::Right)],
        );
        router.route_packet(
            &packet(
                "/avatar/parameters/joycon/channel/waist/force",
                OscArgument::Float(0.6),
            ),
            Instant::now(),
        );
        assert_eq!(router.state().right.force, 0.6);
    }
}
