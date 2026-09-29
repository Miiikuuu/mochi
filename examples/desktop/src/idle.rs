//! Per-character autonomous idle actions, independent of rendering and input.
//! Bounded strolls alternate with an optional package action; input cancels both.
use crate::motion::{Motion, SPEED};

struct Trip {
    direction: i8,
    remaining: f64,
}
pub struct Step {
    pub direction: [i8; 2],
    pub distance: f64,
    pub finished: bool,
}
pub struct Idle {
    after: f64,
    delay: f64,
    quiet: f64,
    random: u64,
    trip: Option<Trip>,
    action: Option<String>,
    playing: bool,
    action_next: bool,
    walk_enabled: bool,
}
impl Idle {
    pub fn new(after: f64, seed: u64) -> Self {
        let mut idle = Self {
            after,
            delay: after,
            quiet: 0.0,
            random: seed,
            trip: None,
            action: None,
            playing: false,
            action_next: true,
            walk_enabled: true,
        };
        idle.schedule();
        idle
    }
    pub fn with_action(mut self, action: Option<String>) -> Self {
        self.action = action;
        self
    }
    pub fn with_walk(mut self, enabled: bool) -> Self {
        self.walk_enabled = enabled;
        self
    }
    pub fn owns_action(&self, animation: &str) -> bool {
        self.playing && self.action.as_deref() == Some(animation)
    }
    pub fn finish_action(&mut self, animation: &str) -> bool {
        if self.playing && !self.owns_action(animation) {
            self.playing = false;
            self.schedule();
            return true;
        }
        false
    }
    pub fn begin_action(&mut self) -> Option<String> {
        if (self.action_next || !self.walk_enabled) && self.action.is_some() {
            self.playing = true;
            self.action_next = false;
            self.schedule();
            self.action.clone()
        } else {
            self.action_next = true;
            None
        }
    }
    fn random(&mut self) -> f64 {
        // SplitMix64 decorrelates nearby creation timestamps/window IDs.
        self.random = self.random.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.random;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        ((value ^ (value >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    fn schedule(&mut self) {
        self.delay = self.after * (1.0 + self.random() * 0.5);
        self.quiet = 0.0;
    }
    pub fn active(&self) -> bool {
        self.trip.is_some() || self.playing
    }
    pub fn interrupt(&mut self) -> bool {
        let active = self.active();
        self.trip = None;
        self.playing = false;
        if active {
            self.schedule();
        }
        self.quiet = 0.0;
        active
    }
    pub fn ready(&mut self, dt: f64) -> bool {
        if self.active() {
            return false;
        }
        self.quiet += dt;
        self.quiet >= self.delay
    }
    /// Called after the host synchronizes actual window/monitor placement.
    pub fn begin(&mut self, motion: &Motion, scale: f64) -> Option<(i8, f64)> {
        if !self.walk_enabled {
            self.schedule();
            return None;
        }
        let room = motion.horizontal_room();
        let preferred = usize::from(self.random() >= 0.5);
        let desired = (48.0 + 48.0 * self.random()) * scale;
        let side = if room[preferred] < desired && room[1 - preferred] > room[preferred] {
            1 - preferred
        } else {
            preferred
        };
        let distance = desired.min(room[side]);
        self.schedule();
        if distance < 8.0 * scale {
            return None;
        }
        let direction = if side == 0 { -1 } else { 1 };
        self.trip = Some(Trip {
            direction,
            remaining: distance,
        });
        Some((direction, distance / scale))
    }
    pub fn step(&mut self, dt: f64, scale: f64) -> Option<Step> {
        let trip = self.trip.as_mut()?;
        let distance = (SPEED * scale * dt).min(trip.remaining);
        trip.remaining -= distance;
        let step = Step {
            direction: [trip.direction, 0],
            distance,
            finished: trip.remaining <= f64::EPSILON,
        };
        if step.finished {
            self.trip = None;
            self.schedule();
        }
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::Bounds;
    fn motion(scale: f64, x: f64) -> Motion {
        Motion::new(
            [(x * scale) as i32, (100.0 * scale) as i32],
            Bounds {
                origin: [0, 0],
                size: [(1000.0 * scale) as u32, (800.0 * scale) as u32],
            },
            (256.0 * scale) as u16,
        )
    }
    #[test]
    fn disabled_stroll_repeats_only_the_enabled_idle_clip() {
        let mut idle = Idle::new(30.0, 42)
            .with_action(Some("idle_sit".into()))
            .with_walk(false);
        for _ in 0..3 {
            assert!(idle.ready(45.0));
            assert_eq!(idle.begin_action().as_deref(), Some("idle_sit"));
            assert!(idle.finish_action("idle"));
            assert!(!idle.ready(29.0));
        }
        assert!(idle.begin(&motion(1.0, 300.0), 1.0).is_none());
    }
    #[test]
    fn waits_again_after_interaction_and_does_not_sync_all_characters() {
        let mut a = Idle::new(30.0, 42);
        let b = Idle::new(30.0, 123456);
        assert!((30.0..45.0).contains(&a.delay));
        assert_ne!(a.delay, b.delay);
        assert!(!a.ready(29.0));
        a.interrupt();
        assert!(!a.ready(29.0));
        assert!(a.ready(16.0));
        a.begin(&motion(1.0, 300.0), 1.0).unwrap();
        assert!(a.interrupt());
        assert!(a.step(1.0, 1.0).is_none());
        assert!(!a.ready(29.0));
    }
    #[test]
    fn distance_is_bounded_and_independent_of_frame_rate_and_dpi() {
        for scale in [1.0, 1.5, 2.0] {
            for hz in [30.0, 60.0, 144.0] {
                let mut idle = Idle::new(30.0, 4321);
                let mut m = motion(scale, 300.0);
                let (direction, distance) = idle.begin(&m, scale).unwrap();
                assert!((48.0..96.0).contains(&distance));
                let start = m.origin();
                let mut ended = 0;
                for _ in 0..(hz as usize * 2) {
                    if let Some(step) = idle.step(1.0 / hz, scale) {
                        assert!(m.advance(step.direction, step.distance));
                        ended += usize::from(step.finished);
                    }
                }
                assert_eq!(ended, 1);
                assert!(!idle.active());
                assert!(
                    (f64::from((m.origin()[0] - start[0]) * i32::from(direction)) / scale
                        - distance)
                        .abs()
                        <= 0.5 / scale + 1e-9
                );
                assert_eq!(m.origin()[1], start[1]);
                assert!(!idle.ready(29.0));
            }
        }
    }
    #[test]
    fn chooses_inward_at_edges_and_leaves_offscreen_placement_untouched() {
        for (x, expected) in [(0.0, 1), (744.0, -1)] {
            let mut idle = Idle::new(30.0, 4);
            let mut m = motion(2.0, x);
            assert_eq!(idle.begin(&m, 2.0).unwrap().0, expected);
            while let Some(step) = idle.step(0.1, 2.0) {
                m.advance(step.direction, step.distance);
            }
            assert!((0..=1488).contains(&m.origin()[0]));
        }
        for position in [[-10, 100], [800, 100], [300, 700]] {
            let mut m = motion(1.0, 300.0);
            m.place(position);
            let mut idle = Idle::new(30.0, 7);
            assert!(idle.begin(&m, 1.0).is_none());
            assert_eq!(m.origin(), position);
            assert!(!idle.ready(29.0));
        }
    }

    #[test]
    fn clip_and_walk_alternate_and_completion_restarts_the_full_wait() {
        let mut idle = Idle::new(30.0, 4).with_action(Some("idle_sit".into()));
        assert!(idle.ready(45.0));
        assert_eq!(idle.begin_action().as_deref(), Some("idle_sit"));
        assert!(idle.owns_action("idle_sit"));
        assert!(!idle.ready(90.0));
        assert!(idle.step(8.0, 2.0).is_none());
        assert!(!idle.finish_action("idle_sit"));
        assert!(idle.finish_action("idle"));
        assert!(!idle.finish_action("idle"));
        assert!(!idle.ready(29.0));
        assert!(idle.ready(16.0));
        assert!(idle.begin_action().is_none());
        idle.begin(&motion(1.0, 300.0), 1.0).unwrap();
        assert!(idle.step(2.0, 1.0).unwrap().finished);
        assert!(!idle.ready(29.0));
        assert!(idle.ready(16.0));
        assert_eq!(idle.begin_action().as_deref(), Some("idle_sit"));
    }

    #[test]
    fn user_input_cancels_owned_clip_without_owning_manual_sit() {
        let mut idle = Idle::new(30.0, 5).with_action(Some("idle_sit".into()));
        idle.begin_action();
        assert!(!idle.owns_action("sit"));
        assert!(idle.interrupt());
        assert!(!idle.active());
        assert!(!idle.finish_action("idle"));
        assert!(!idle.ready(29.0));
        assert!(!idle.interrupt());
    }
}
