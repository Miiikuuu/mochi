//! Event-timed shake detection and finite reaction scheduling. No frame counters.
use crate::gaze::{curve, smooth};
use mochi_core::{BoneOffset, Scene, SceneContent, Vec2};
use mochi_package::{ShakeDetector, SpineDrag, SpineFollow, SpineShake};
use std::{f64::consts::TAU, time::Instant};
#[derive(Clone, Default)]
struct Axis {
    axis: usize,
    direction: f64,
    start: f64,
    start_time: f64,
    extreme: f64,
    extreme_time: f64,
    turns: Vec<f64>,
}
#[derive(Clone, Debug)]
struct Job {
    trigger: f64,
    ready: f64,
    expires: f64,
    start: Option<f64>,
}
#[derive(Clone, Debug)]
struct Stomp {
    amount: f64,
    source: f64,
    face: f64,
    phase: &'static str,
    pending: bool,
}
#[derive(Clone, Debug)]
struct Face {
    amount: f64,
    source: f64,
    mouth_only: bool,
}
#[derive(Clone)]
struct Cancel {
    time: f64,
    body: Stomp,
    face: Face,
}
#[derive(Clone, Debug)]
struct Dizzy {
    amount: f64,
    spin: f64,
    rotations: Vec<f64>,
    feet: Vec<[f64; 2]>,
}
pub struct Shake {
    config: SpineShake,
    detector: ShakeDetector,
    drag: SpineDrag,
    follow: SpineFollow,
    epoch: Instant,
    input_stamp: Option<(u32, f64)>,
    axes: Vec<Axis>,
    last_input: f64,
    held: bool,
    released: Option<f64>,
    trigger: Option<f64>,
    cooldown: f64,
    job: Option<Job>,
    canceled: Option<Cancel>,
    reset: Option<(f64, Dizzy)>,
    triggers: usize,
}
impl Shake {
    pub fn new(config: SpineShake, drag: SpineDrag, follow: SpineFollow) -> Self {
        // Desktop gestures use logical pixels, not the model's camera units.
        // A placement correction must not count as an intentional shake.
        let detector = ShakeDetector {
            reversals: config.detector.reversals.max(4),
            window_seconds: config.detector.window_seconds.min(1.0),
            min_excursion: config.detector.min_excursion.max(60.0),
            min_speed: config.detector.min_speed.max(360.0),
            reversal_distance: config.detector.reversal_distance.max(24.0),
            ..config.detector.clone()
        };
        Self {
            config,
            detector,
            drag,
            follow,
            epoch: Instant::now(),
            input_stamp: None,
            axes: vec![],
            last_input: f64::NEG_INFINITY,
            held: false,
            released: None,
            trigger: None,
            cooldown: 0.0,
            job: None,
            canceled: None,
            reset: None,
            triggers: 0,
        }
    }
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
    fn start_detector(&mut self, t: f64, xy: [f64; 2]) {
        self.axes = self
            .detector
            .axes
            .iter()
            .map(|&axis| Axis {
                axis,
                start: xy[axis],
                start_time: t,
                extreme: xy[axis],
                extreme_time: t,
                ..Default::default()
            })
            .collect();
        self.last_input = t;
    }
    pub fn begin(&mut self, stamp: u32) {
        let t = self.now();
        self.input_stamp = Some((stamp, t));
        self.begin_at(t);
    }
    fn begin_at(&mut self, t: f64) {
        let body = self.stomp(t);
        if body.amount > 0.0 {
            self.cancel_at(t, false);
        } else if body.pending {
            if let Some(j) = &mut self.job {
                j.start = None;
            }
        }
        self.held = true;
        self.released = None;
        self.start_detector(t, [0.0; 2]);
    }
    pub fn move_to(&mut self, delta: [i32; 2], desktop_scale: f64, stamp: u32) {
        self.move_at(delta, desktop_scale, stamp, self.now());
    }
    fn move_at(&mut self, delta: [i32; 2], desktop_scale: f64, stamp: u32, now: f64) {
        // X server event timestamps preserve spacing when several motions are
        // drained in one frame. Keep the grab's original time anchor: clamping
        // each event to now would turn queued, slow motion into a fast shake.
        // Reaction playback starts at now, independently of the input clock.
        let t = self
            .input_stamp
            .map_or(now, |(s, t)| t + f64::from(stamp.wrapping_sub(s)) / 1000.0);
        self.feed_at(t, now, delta.map(|v| f64::from(v) / desktop_scale));
    }
    #[cfg(all(test, feature = "spine"))]
    fn feed(&mut self, t: f64, xy: [f64; 2]) {
        self.feed_at(t, t, xy);
    }
    fn feed_at(&mut self, input_time: f64, now: f64, xy: [f64; 2]) {
        if !self.held || input_time <= self.last_input {
            return;
        }
        self.last_input = input_time;
        let stomp = self.stomp(now);
        if now < self.cooldown || stomp.pending || stomp.amount > 0.0 {
            self.start_detector(input_time, xy);
            return;
        }
        let t = input_time;
        let d = &self.detector;
        for a in &mut self.axes {
            let x = xy[a.axis];
            a.turns.retain(|q| *q >= t - d.window_seconds);
            if a.direction == 0.0 {
                if (x - a.start).abs() >= d.reversal_distance {
                    a.direction = (x - a.start).signum();
                    a.extreme = x;
                    a.extreme_time = t;
                }
                continue;
            }
            if (x - a.extreme) * a.direction > 0.0 {
                a.extreme = x;
                a.extreme_time = t;
                continue;
            }
            if (a.extreme - x) * a.direction < d.reversal_distance {
                continue;
            }
            let excursion = (a.extreme - a.start).abs();
            let speed = excursion / (t - a.start_time);
            if excursion >= d.min_excursion && speed >= d.min_speed {
                a.turns.push(t);
            } else {
                a.turns.clear();
            }
            a.direction = -a.direction;
            a.start = a.extreme;
            a.start_time = a.extreme_time;
            a.extreme = x;
            a.extreme_time = t;
            if a.turns.len() >= d.reversals {
                self.trigger = Some(now);
                self.reset = None;
                self.canceled = None;
                self.triggers += 1;
                self.cooldown = now + d.cooldown_seconds;
                self.job = Some(Job {
                    trigger: now,
                    ready: now + self.config.timings.duration + self.config.post_delay,
                    expires: now + self.config.stomp_timings.pending_lifetime,
                    start: None,
                });
                self.start_detector(t, xy);
                break;
            }
        }
    }
    pub fn release(&mut self) {
        self.release_at(self.now());
    }
    fn release_at(&mut self, t: f64) {
        if !self.held {
            return;
        }
        self.held = false;
        self.released = Some(t);
        self.axes.clear();
        self.input_stamp = None;
        if self.stomp(t).pending {
            if let Some(j) = &mut self.job {
                let s = &self.config.stomp_timings;
                let start = j
                    .ready
                    .max(t + self.drag.timings.land_recover + s.ground_pause);
                j.start = Some(start);
                if start < j.expires {
                    self.cooldown = self
                        .cooldown
                        .max(start + s.r#in + s.play_seconds + s.out + s.ground_pause);
                }
            }
        }
    }
    pub fn cancel(&mut self) {
        self.cancel_at(self.now(), true);
    }
    fn cancel_at(&mut self, t: f64, reset_dizzy: bool) {
        let body = self.stomp(t);
        let face = self.face(t, &body);
        if body.pending || body.amount > 0.0 {
            self.canceled = Some(Cancel {
                time: t,
                body,
                face,
            });
            self.job = None;
        }
        if reset_dizzy {
            self.reset = Some((t, self.dizzy(t)));
            self.axes.clear();
        }
    }
    fn stomp(&self, t: f64) -> Stomp {
        let c = &self.config;
        let s = &c.stomp_timings;
        let neutral = Stomp {
            amount: 0.0,
            source: c.stomp.start,
            face: c.face.start,
            phase: "none",
            pending: false,
        };
        if let Some(e) = &self.canceled {
            let amount = e.body.amount * (1.0 - smooth((t - e.time) / s.cancel_out));
            return if amount > 0.0 {
                Stomp {
                    amount,
                    phase: "canceling",
                    pending: false,
                    ..e.body.clone()
                }
            } else {
                neutral
            };
        }
        let Some(j) = &self.job else { return neutral };
        if j.start.is_none_or(|start| t < start) {
            return if t < j.expires {
                Stomp {
                    pending: true,
                    phase: "waiting",
                    ..neutral
                }
            } else {
                neutral
            };
        }
        let start = j.start.unwrap();
        let age = t - start;
        if start >= j.expires || age >= s.r#in + s.play_seconds + s.out {
            return neutral;
        }
        let amount = smooth(age / s.r#in) * (1.0 - smooth((age - s.r#in - s.play_seconds) / s.out));
        Stomp {
            amount,
            source: c.stomp.start
                + (c.stomp.end - c.stomp.start) * ((age - s.r#in) / s.play_seconds).clamp(0.0, 1.0),
            face: c.face.start
                + (c.face.end - c.face.start)
                    * ((age - s.r#in) / c.face_play_seconds).clamp(0.0, 1.0),
            phase: if age < s.r#in {
                "entering"
            } else if age < s.r#in + s.play_seconds {
                "stomping"
            } else {
                "exiting"
            },
            pending: false,
        }
    }
    fn dizzy(&self, t: f64) -> Dizzy {
        let c = &self.config;
        let timing = &c.timings;
        let mut out = Dizzy {
            amount: 0.0,
            spin: 0.0,
            rotations: vec![0.0; c.bones.len()],
            feet: vec![[0.0; 2]; c.feet.len()],
        };
        if let Some((time, from)) = &self.reset {
            let q = 1.0 - smooth((t - time) / timing.reset_out);
            if q <= 0.0 || from.amount <= 0.0 {
                return out;
            }
            out.amount = from.amount * q;
            out.spin = from.spin + TAU * c.spiral_turns_per_second * (t - time);
            out.rotations = from.rotations.iter().map(|v| v * q).collect();
            out.feet = from.feet.iter().map(|xy| xy.map(|v| v * q)).collect();
            return out;
        }
        let Some(trigger) = self.trigger else {
            return out;
        };
        let dt = t - trigger;
        if dt < 0.0 || dt >= timing.duration {
            return out;
        }
        let recover = 1.0 - smooth((dt - timing.duration + timing.out) / timing.out);
        out.amount = smooth(dt / timing.r#in) * recover;
        out.spin = TAU * c.spiral_turns_per_second * dt;
        for (v, b) in out.rotations.iter_mut().zip(&c.bones) {
            let age = (dt - b.delay).max(0.0);
            let w = smooth((dt - b.delay) / timing.r#in) * recover;
            let wave = (TAU * c.frequency * age + b.phase).sin() * (-c.decay * age).exp();
            *v = (w * (b.bias_degrees + b.amplitude_degrees * wave))
                .clamp(-b.limit_degrees, b.limit_degrees);
        }
        for (xy, b) in out.feet.iter_mut().zip(&c.feet) {
            let age = (dt - b.delay).max(0.0);
            let w = smooth((dt - b.delay) / timing.r#in) * recover * (-c.decay * age).exp();
            let wave = (TAU * b.frequency * age + b.phase).sin();
            *xy = [
                b.amplitude[0] * wave * w,
                b.amplitude[1] * (wave + 1.0) * 0.5 * w,
            ];
            for (i, v) in xy.iter_mut().enumerate() {
                *v = v.clamp(-b.limit[i], b.limit[i]);
            }
        }
        out
    }
    fn face(&self, t: f64, s: &Stomp) -> Face {
        let c = &self.config;
        if let Some(e) = &self.canceled {
            return Face {
                amount: e.face.amount * (1.0 - smooth((t - e.time) / c.stomp_timings.cancel_out)),
                ..e.face.clone()
            };
        }
        let mut out = Face {
            amount: s.amount,
            source: s.face,
            mouth_only: false,
        };
        let Some(j) = &self.job else { return out };
        let live = s.pending || ["entering", "stomping", "exiting"].contains(&s.phase);
        let d = self.dizzy(t);
        let age = t - j.trigger;
        let timing = &c.timings;
        if !live || age < timing.duration - timing.out || d.amount >= c.mouth_threshold {
            return out;
        }
        let recovery = ((age - timing.duration + timing.out) / timing.out).clamp(0.0, 1.0);
        out.source = c.face.start + (c.bridge_time - c.face.start) * smooth(recovery);
        if s.amount > 0.0 || s.phase == "entering" {
            out.source = c.bridge_time.max(s.face);
        }
        if s.phase == "exiting" {
            out.source = c.face.start + (c.face.end - c.face.start) * s.amount;
        }
        out.amount = 1.0;
        out.mouth_only = d.amount >= c.dizzy.swap_threshold;
        out
    }
    pub fn project(&self, scene: &mut Scene) {
        self.project_at(scene, self.now());
    }
    fn project_at(&self, scene: &mut Scene, t: f64) {
        let c = &self.config;
        let d = self.dizzy(t);
        let s = self.stomp(t);
        let face = self.face(t, &s);
        if d.amount <= 0.0 && s.amount <= 0.0 && face.amount <= 0.0 {
            return;
        }
        let air = if self.held {
            1.0
        } else {
            self.released.map_or(0.0, |r| {
                1.0 - smooth((t - r) / self.drag.timings.land_recover)
            })
        };
        for node in &mut scene.nodes {
            let SceneContent::Skeletal {
                animation,
                time_ns,
                offsets,
                overlay,
                ..
            } = &mut node.content
            else {
                continue;
            };
            if animation != &self.follow.animation {
                continue;
            }
            if d.amount > 0.0 {
                for (name, r, xy) in c
                    .bones
                    .iter()
                    .zip(&d.rotations)
                    .map(|(b, r)| (&b.bone, *r, [0.0; 2]))
                    .chain(
                        c.feet
                            .iter()
                            .zip(&d.feet)
                            .map(|(b, xy)| (&b.bone, 0.0, xy.map(|v| v * air))),
                    )
                {
                    let i = offsets
                        .iter()
                        .position(|o| &o.bone == name)
                        .unwrap_or_else(|| {
                            offsets.push(BoneOffset {
                                bone: name.clone(),
                                rotation_degrees: 0.0,
                                local_translation: Vec2::default(),
                                world_translation: Vec2::default(),
                            });
                            offsets.len() - 1
                        });
                    let o = &mut offsets[i];
                    o.rotation_degrees = (o.rotation_degrees + r).clamp(-30.0, 30.0);
                    o.local_translation.x = (o.local_translation.x + xy[0]).clamp(-32.0, 32.0);
                    o.local_translation.y = (o.local_translation.y + xy[1]).clamp(-32.0, 32.0);
                }
                if d.amount >= c.mouth_threshold {
                    overlay
                        .slots
                        .retain(|s| !self.drag.face.mouth_slots.contains(&s.slot));
                    overlay.slots.extend(
                        self.drag
                            .face
                            .overlay([0.0, 1.0])
                            .slots
                            .into_iter()
                            .filter(|s| self.drag.face.mouth_slots.contains(&s.slot)),
                    );
                }
                if d.amount >= c.dizzy.swap_threshold {
                    let phase = (*time_ns as f64 / 1e9 - self.follow.base_time_offset)
                        .rem_euclid(self.follow.cycle_duration);
                    let closed = curve(&self.follow.blink_weight, phase) <= 0.0;
                    overlay.spirals = c.dizzy.eyes(d.spin, closed);
                }
            }
            // Body first, face last: the original full expression retains ownership.
            if s.amount > 0.0 {
                overlay.layers.push(c.stomp.layer(s.source, s.amount));
            }
            if face.amount > 0.0 {
                let mut layer = c.face.layer(face.source, face.amount);
                if face.mouth_only {
                    layer.bones.clear();
                    layer.slots = self.drag.face.mouth_slots.clone();
                }
                overlay.layers.push(layer);
            }
        }
    }
    pub fn active(&self) -> bool {
        let t = self.now();
        let s = self.stomp(t);
        self.dizzy(t).amount > 0.0 || s.pending || s.amount > 0.0 || self.face(t, &s).amount > 0.0
    }
    pub fn trace(&self) -> String {
        let t = self.now();
        let d = self.dizzy(t);
        let s = self.stomp(t);
        let face = self.face(t, &s);
        format!("triggers={} held={} dizzy={:.3} phase={} body={:.3} source={:.3} face={:.3} face_source={:.3} mouth_only={}",self.triggers,self.held,d.amount,s.phase,s.amount,s.source,face.amount,face.source,face.mouth_only)
    }
}

#[cfg(all(test, feature = "spine"))]
mod tests {
    use super::*;
    use serde_json::Value;
    fn controller() -> Shake {
        let mut c = desktop_controller();
        // Original-source reaction/geometry fixtures use source coordinates and
        // their authored detector. Desktop input policy is tested separately.
        c.detector = c.config.detector.clone();
        c
    }
    fn desktop_controller() -> Shake {
        let old: Value =
            serde_json::from_str(include_str!("../tests/fixtures/drag-reference.json")).unwrap();
        Shake::new(
            serde_json::from_str(include_str!("../tests/fixtures/shake-config.json")).unwrap(),
            serde_json::from_value(old["drag"].clone()).unwrap(),
            serde_json::from_value(old["follow"].clone()).unwrap(),
        )
    }
    fn event(c: &mut Shake, e: &Value) {
        let t = e["time"].as_f64().unwrap();
        match e["kind"].as_str().unwrap() {
            "pick" => c.begin_at(t),
            "release" => c.release_at(t),
            "cancel" => c.cancel_at(t, true),
            "move" => c.feed(
                t,
                [e["xy"][0].as_f64().unwrap(), e["xy"][1].as_f64().unwrap()],
            ),
            _ => panic!(),
        }
    }
    // Physical desktop coordinates, server timestamps, and render-batched
    // delivery exercise the production input entry, including DPI conversion.
    fn desktop_path(points: &[(u32, [f64; 2])], dpi: f64, input_hz: u32, render_hz: u32) -> Shake {
        let mut c = desktop_controller();
        let stamp = u32::MAX - 150; // Also exercise X server timestamp rollover.
        c.begin_at(0.0);
        c.input_stamp = Some((stamp, 0.0));
        let mut prev = (0, [0.0; 2]);
        for &(end, xy) in points {
            let step = 1000 / input_hz;
            let mut ms = prev.0 + step;
            loop {
                ms = ms.min(end);
                let u = f64::from(ms - prev.0) / f64::from(end - prev.0);
                let delta = std::array::from_fn(|i| {
                    ((prev.1[i] + (xy[i] - prev.1[i]) * u) * dpi).round() as i32
                });
                let now =
                    (f64::from(ms) / 1000.0 * f64::from(render_hz)).ceil() / f64::from(render_hz);
                c.move_at(delta, dpi, stamp.wrapping_add(ms), now);
                if ms == end {
                    break;
                }
                ms += step;
            }
            prev = (end, xy);
        }
        c
    }
    const DELIBERATE: [(u32, [f64; 2]); 5] = [
        (100, [100.0, 0.0]),
        (200, [-100.0, 0.0]),
        (300, [100.0, 0.0]),
        (400, [-100.0, 0.0]),
        (500, [100.0, 0.0]),
    ];
    #[test]
    fn placement_corrections_reproduce_old_dizziness_and_are_now_rejected() {
        let points = DELIBERATE.map(|(t, xy)| (t, [xy[0] * 0.18, 0.0]));
        let mut old = controller();
        old.begin_at(0.0);
        for &(t, xy) in &points {
            // Previous mapping at the default 256 logical pixel window.
            old.feed(f64::from(t) / 1000.0, xy.map(|p| p * 790.0 / 256.0));
        }
        assert_eq!(
            old.triggers, 1,
            "regression must reproduce the reported sensitivity"
        );
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            for hz in [125, 500, 1000] {
                for render_hz in [30, 60, 144] {
                    assert_eq!(desktop_path(&points, dpi, hz, render_hz).triggers, 0);
                }
            }
        }
    }
    #[test]
    fn desktop_detector_rejects_placement_sweeps_curves_and_slow_motion() {
        let mut diagonal = vec![];
        let mut circle = vec![];
        for i in 1..=60 {
            diagonal.push((
                i * 10,
                [
                    f64::from(i) * 6.0,
                    f64::from(i) * -4.0 + (f64::from(i) * 0.9).sin() * 8.0,
                ],
            ));
            let angle = f64::from(i) / 60.0 * TAU;
            circle.push((i * 10, [110.0 * angle.sin(), 110.0 * (1.0 - angle.cos())]));
        }
        let corrections = vec![
            (100, [180.0, -70.0]),
            (170, [150.0, -50.0]),
            (240, [183.0, -76.0]),
            (310, [156.0, -52.0]),
            (380, [181.0, -70.0]),
            (450, [170.0, -62.0]),
        ];
        let slow: Vec<_> = DELIBERATE.iter().map(|&(t, xy)| (t * 10, xy)).collect();
        let sparse = vec![
            (100, [100.0, 0.0]),
            (200, [-100.0, 0.0]),
            (300, [100.0, 0.0]),
            (1800, [-100.0, 0.0]),
            (1900, [100.0, 0.0]),
            (2000, [-100.0, 0.0]),
        ];
        let mixed_axes = vec![
            (100, [100.0, 0.0]),
            (200, [-100.0, 0.0]),
            (300, [100.0, 0.0]),
            (400, [100.0, 100.0]),
            (500, [100.0, -100.0]),
            (600, [100.0, 100.0]),
        ];
        for path in [
            &diagonal[..],
            &circle,
            &corrections,
            &slow,
            &sparse,
            &mixed_axes,
            &DELIBERATE[..3],
            &DELIBERATE[..4],
        ] {
            for dpi in [1.0, 1.5, 2.0] {
                for hz in [125, 1000] {
                    assert_eq!(desktop_path(path, dpi, hz, 60).triggers, 0, "{path:?}");
                }
            }
        }
    }
    #[test]
    fn deliberate_desktop_shakes_trigger_once_across_axes_dpi_and_event_rates() {
        for axis in [0, 1] {
            for sign in [-1.0, 1.0] {
                let path = DELIBERATE.map(|(t, p)| {
                    let mut xy = [0.0; 2];
                    xy[axis] = p[0] * sign;
                    (t, xy)
                });
                for dpi in [1.0, 1.25, 1.5, 2.0] {
                    for hz in [125, 500, 1000] {
                        for render_hz in [30, 60, 144] {
                            let mut c = desktop_path(&path, dpi, hz, render_hz);
                            assert_eq!(
                                c.triggers, 1,
                                "axis={axis} sign={sign} dpi={dpi} input={hz} render={render_hz}"
                            );
                            let trigger = c.trigger.unwrap();
                            assert!(c.dizzy(trigger + 0.2).amount > 0.9);
                            for &(ms, xy) in &path {
                                c.feed(0.5 + f64::from(ms) / 1000.0, xy);
                            }
                            assert_eq!(c.triggers, 1, "cooldown must suppress repeats");
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn queued_motion_keeps_server_speed_and_starts_reaction_at_delivery_time() {
        for slow in [false, true] {
            let mut c = desktop_controller();
            c.begin_at(0.0);
            c.input_stamp = Some((123, 0.0));
            for (i, &(ms, xy)) in DELIBERATE.iter().enumerate() {
                // Five motions drained immediately; wall time is not input time.
                c.move_at(
                    xy.map(|v| v as i32),
                    1.0,
                    123 + ms * if slow { 10 } else { 1 },
                    (i + 1) as f64 * 0.001,
                );
            }
            assert_eq!(c.triggers, usize::from(!slow));
            if !slow {
                assert_eq!(c.trigger, Some(0.005));
            }
        }
        let mut c = desktop_controller();
        c.begin_at(0.0);
        c.input_stamp = Some((123, 0.0));
        for (i, &(_, xy)) in DELIBERATE.iter().enumerate() {
            c.move_at(xy.map(|v| v as i32), 1.0, 223, (i + 1) as f64 * 0.001);
        }
        assert_eq!(c.triggers, 0, "duplicate timestamps cannot imply velocity");
        for n in 0..4 {
            let t = f64::from(n);
            c.begin_at(t);
            for &(ms, xy) in &DELIBERATE[..3] {
                c.feed(t + f64::from(ms) / 1000.0, xy);
            }
            c.release_at(t + 0.4);
        }
        assert_eq!(c.triggers, 0, "separate drags cannot accumulate reversals");
    }
    #[test]
    fn supplied_controller_timing_detection_and_face_match_at_60_120_144_hz() {
        let f: Value =
            serde_json::from_str(include_str!("../tests/fixtures/shake-reference.json")).unwrap();
        for hz in [60, 120, 144] {
            for case in f["cases"].as_array().unwrap() {
                let mut c = controller();
                let mut next = 0;
                let mut rendered = 0.0;
                let events = case["events"].as_array().unwrap();
                for sample in case["samples"].as_array().unwrap() {
                    let t = sample["time"].as_f64().unwrap();
                    while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
                        event(&mut c, &events[next]);
                        next += 1;
                    }
                    while rendered < t {
                        let _ = c.dizzy(rendered);
                        let _ = c.stomp(rendered);
                        rendered += 1.0 / hz as f64;
                    }
                    let d = c.dizzy(t);
                    let s = c.stomp(t);
                    let face = c.face(t, &s);
                    let near = |a: f64, b: &Value, label: &str| {
                        assert!(
                            (a - b.as_f64().unwrap()).abs() < 1e-8,
                            "{} {hz}Hz t={t} {label}: {a} != {b}",
                            case["name"]
                        )
                    };
                    assert_eq!(
                        c.triggers,
                        sample["triggers"].as_u64().unwrap() as usize,
                        "{} t={t}",
                        case["name"]
                    );
                    near(d.amount, &sample["dizzy"]["amount"], "dizzy amount");
                    near(d.spin, &sample["dizzy"]["spin"], "spin");
                    for (b, r) in c.config.bones.iter().zip(d.rotations) {
                        near(r, &sample["dizzy"]["rotations"][&b.bone], &b.bone);
                    }
                    for (b, xy) in c.config.feet.iter().zip(d.feet) {
                        for i in 0..2 {
                            near(xy[i], &sample["dizzy"]["feet"][&b.bone][i], &b.bone);
                        }
                    }
                    near(s.amount, &sample["stomp"]["amount"], "body");
                    near(s.source, &sample["stomp"]["source"], "body source");
                    near(s.face, &sample["stomp"]["face"], "face source baseline");
                    assert_eq!(s.phase, sample["stomp"]["phase"].as_str().unwrap());
                    assert_eq!(s.pending, sample["stomp"]["pending"].as_bool().unwrap());
                    near(face.amount, &sample["face"]["amount"], "face");
                    near(face.source, &sample["face"]["source"], "face source");
                    assert_eq!(
                        face.mouth_only,
                        sample["face"]["mouthOnly"].as_bool().unwrap()
                    );
                }
            }
        }
    }
    #[test]
    fn overlay_rejects_excess_layers_invalid_masks_and_spiral_geometry() {
        use mochi_core::PoseOverlay;
        let c = controller();
        let valid = PoseOverlay {
            layers: vec![
                c.config.stomp.layer(0.8, 1.0),
                c.config.face.layer(0.7, 1.0),
            ],
            spirals: c.config.dizzy.eyes(0.0, false),
            ..Default::default()
        };
        valid.validate().unwrap();
        let mut bad = valid.clone();
        bad.layers
            .extend([bad.layers[0].clone(), bad.layers[0].clone()]);
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.layers[0].bones[0].channels = [false; 3];
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        let bone = bad.layers[0].bones[0].clone();
        bad.layers[0].bones.push(bone);
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.layers[0].time = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.spirals[0].segments = u16::MAX;
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.spirals[0].inner_radius = bad.spirals[0].radius;
        assert!(bad.validate().is_err());
        let mut bad = valid;
        bad.spirals[0].color[0] = f64::INFINITY;
        assert!(bad.validate().is_err());
    }

    #[test]
    #[ignore = "requires supplied Shanbi assets and local GPU; offscreen only"]
    fn supplied_shanbi_shake_render_and_reference() {
        use mochi_package::LoadedPackage;
        use mochi_preview::Viewport;
        use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
        use std::{fs, path::PathBuf};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("target/shake-check");
        fs::create_dir_all(&out).unwrap();
        let package =
            LoadedPackage::load(root.join("characters/shanbi-shake/shanbi.mochi")).unwrap();
        let resource = package.spine("spine.toml").unwrap();
        let mut c = controller();
        let mut next = 0;
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/shake-geometry.json")).unwrap();
        let events = fixture["events"].as_array().unwrap();
        let mut evaluator = mochi_spine::Evaluator::new(&resource.model).unwrap();
        let mut reference = mochi_spine::ReferenceEvaluator::new(&resource.model).unwrap();
        let base = evaluator.pose("Move", 0.3).unwrap();
        let options = RenderOptions {
            width: 512,
            height: 512,
            ..Default::default()
        };
        let mut cpu = CanvasRenderer::new(options).unwrap();
        let mut gpu = CanvasRenderer::new(options).unwrap();
        gpu.enable_spine_gpu().unwrap();
        let viewport = Viewport::new([112, 112], [512, 512]).unwrap().unwrap();
        let mut reports = vec![];
        for item in fixture["geometry"].as_array().unwrap() {
            let t = item["time"].as_f64().unwrap();
            while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
                event(&mut c, &events[next]);
                next += 1;
            }
            let mut scene = package.instantiate("shake-test").unwrap().scene().unwrap();
            if let SceneContent::Skeletal { time_ns, .. } = &mut scene.nodes[0].content {
                *time_ns = ((0.5 + t % 2.0) * 1e9).round() as u64;
            }
            c.project_at(&mut scene, t);
            scene.validate().unwrap();
            let SceneContent::Skeletal {
                animation,
                time_ns,
                offsets,
                overlay,
                ..
            } = &scene.nodes[0].content
            else {
                panic!()
            };
            let offsets: Vec<_> = offsets
                .iter()
                .map(|o| mochi_spine::BoneOffset {
                    bone: &o.bone,
                    rotation_degrees: o.rotation_degrees,
                    local_translation: [o.local_translation.x, o.local_translation.y],
                    world_translation: [o.world_translation.x, o.world_translation.y],
                })
                .collect();
            let a = evaluator
                .pose_with_overlay(animation, *time_ns as f64 / 1e9, &offsets, overlay)
                .unwrap();
            let b = reference
                .pose_with_overlay(animation, *time_ns as f64 / 1e9, &offsets, overlay)
                .unwrap();
            let orig = item["meshes"].as_array().unwrap();
            assert_eq!(a.len(), orig.len(), "t={t} original mesh count");
            assert_eq!(a.len(), b.len(), "t={t} bridge mesh count");
            let mut error = 0f32;
            let mut bridge = 0f32;
            for ((a, b), o) in a.iter().zip(&b).zip(orig) {
                assert_eq!(a.page, o["page"].as_u64().unwrap() as usize, "t={t}");
                for attr in 0..12 {
                    let bounds = |m: &mochi_spine::Mesh| {
                        m.vertices
                            .iter()
                            .fold([f32::INFINITY, f32::NEG_INFINITY], |[lo, hi], v| {
                                [lo.min(v[attr]), hi.max(v[attr])]
                            })
                    };
                    let av = bounds(a);
                    let bv = bounds(b);
                    for i in 0..2 {
                        error = error.max(
                            (av[i] - o["bounds"][attr * 2 + i].as_f64().unwrap() as f32).abs(),
                        );
                        bridge = bridge.max((av[i] - bv[i]).abs());
                    }
                }
            }
            assert!(error < 0.003, "t={t} original source bounds error={error}");
            assert!(bridge < 0.003, "t={t} C/JS error={bridge}");
            assert_eq!(
                evaluator.pose("Move", 0.3).unwrap(),
                base,
                "overlay leaked at {t}"
            );
            let scene = viewport.project_scene(&scene);
            let layers = [RenderLayer {
                package: &package,
                scene: &scene,
            }];
            cpu.render_layers(&layers).unwrap();
            gpu.render_layers(&layers).unwrap();
            let mut sum = 0.0;
            let mut count = 0;
            let mut big = 0;
            for (a, b) in cpu
                .frame()
                .rgba8()
                .chunks_exact(4)
                .zip(gpu.frame().rgba8().chunks_exact(4))
            {
                if a[3] == 0 && b[3] == 0 {
                    continue;
                }
                count += 1;
                let mut diff = (f64::from(a[3]) - f64::from(b[3])).abs();
                for k in 0..3 {
                    diff += (f64::from(a[k]) * f64::from(a[3]) / 255.0
                        - f64::from(b[k]) * f64::from(b[3]) / 255.0)
                        .abs();
                }
                sum += diff;
                if diff > 32.0 {
                    big += 1;
                }
            }
            let mean = sum / (count * 4) as f64;
            assert!(
                mean < 0.5 && big as f64 / (count as f64) < 0.01,
                "t={t} CPU/GPU {mean}"
            );
            cpu.frame()
                .write_png(fs::File::create(out.join(format!("shake-{t:.2}.png"))).unwrap())
                .unwrap();
            gpu.frame()
                .write_png(fs::File::create(out.join(format!("shake-{t:.2}-gpu.png"))).unwrap())
                .unwrap();
            // Actual desktop composition: drag acting, then shake/spirals/body/face.
            let mut combined = package.instantiate("combined").unwrap().scene().unwrap();
            let mut drag =
                crate::drag::Drag::new(c.drag.clone(), c.follow.clone(), resource.camera);
            drag.begin(&combined, Vec2::new(55.0, 35.0), None);
            drag.advance(t.min(0.5));
            if t >= 0.5 {
                drag.release();
                drag.advance(t - 0.5);
            }
            if let SceneContent::Skeletal { time_ns, .. } = &mut combined.nodes[0].content {
                *time_ns = ((0.5 + t % 2.0) * 1e9).round() as u64;
            }
            drag.project(&mut combined);
            c.project_at(&mut combined, t);
            combined.validate().unwrap();
            let combined = viewport.project_scene(&combined);
            let layers = [RenderLayer {
                package: &package,
                scene: &combined,
            }];
            cpu.render_layers(&layers).unwrap();
            gpu.render_layers(&layers).unwrap();
            cpu.frame()
                .write_png(fs::File::create(out.join(format!("combined-{t:.2}.png"))).unwrap())
                .unwrap();
            reports.push(serde_json::json!({"time":t,"original_source_bounds_error":error,"bridge_bounds_error":bridge,"cpu_gpu_mean_pma_error":mean}));
        }
        fs::write(
            out.join("results.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
}
