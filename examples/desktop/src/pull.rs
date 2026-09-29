//! v0.3 pull input, pickup expression and recovery, using absolute-time curves.
use crate::gaze::{smooth, world_point, Spring};
use mochi_core::{
    AnimationLayer, AppliedDelta, AppliedPose, BoneOffset, Scene, SceneContent, Vec2,
};
use mochi_package::{SpineCamera, SpineFollow, SpinePull};
use std::sync::Arc;
#[cfg(all(test, feature = "spine"))]
mod tests;

fn mix(a: f64, b: f64, w: f64) -> f64 {
    a + (b - a) * w
}
fn curve(from: f64, keys: &[[f64; 2]], time: f64) -> f64 {
    let (mut at, mut value) = (0.0, from);
    for &[next, target] in keys {
        if time <= next {
            return mix(value, target, smooth((time - at) / (next - at)));
        }
        at = next;
        value = target;
    }
    value
}
fn solve(position: f64, velocity: f64, target: f64, dt: f64, omega: f64) -> (f64, f64) {
    let error = position - target;
    let b = velocity + omega * error;
    let e = (-omega * dt).exp();
    (
        target + (error + b * dt) * e,
        (velocity - omega * b * dt) * e,
    )
}
#[derive(Clone)]
struct Lift {
    amount: f64,
    surprise: f64,
    smile: f64,
    gaze: [f64; 2],
    rotations: Vec<f64>,
    reaction: Vec<f64>,
}
struct Event {
    time: f64,
    held: bool,
    sign: f64,
    from: Lift,
    pull_from: f64,
}
#[derive(Default)]
struct Force {
    time: f64,
    position: [f64; 2],
    velocity: [f64; 2],
    target: [f64; 2],
    hold: f64,
}
pub struct Pull {
    config: Arc<SpinePull>,
    follow: SpineFollow,
    camera: SpineCamera,
    time: f64,
    event: Option<Event>,
    force: Force,
    previous: (f64, [f64; 2]),
    translation: [Spring; 2],
}
impl Pull {
    pub fn new(config: Arc<SpinePull>, follow: SpineFollow, camera: SpineCamera) -> Self {
        Self {
            translation: [
                Spring::new(config.follow_omega),
                Spring::new(config.follow_omega),
            ],
            config,
            follow,
            camera,
            time: 0.0,
            event: None,
            force: Force::default(),
            previous: (0.0, [0.0; 2]),
        }
    }
    pub fn advance(&mut self, dt: f64) {
        self.time += dt;
    }
    pub fn held(&self) -> bool {
        self.event.as_ref().is_some_and(|e| e.held)
    }
    pub fn active(&self) -> bool {
        self.event.as_ref().is_some_and(|e| {
            e.held
                || self.time - e.time
                    < self
                        .config
                        .lift
                        .timings
                        .recover
                        .max(self.config.release_seconds)
        })
    }
    pub fn clear(&mut self) {
        self.event = None;
        self.force = Force::default();
    }
    fn force_at(&self, time: f64) -> ([f64; 2], [f64; 2]) {
        let e = &self.force;
        let dt = (time - e.time).max(0.0);
        let mut pos = [0.0; 2];
        let mut vel = [0.0; 2];
        for i in 0..2 {
            let (p, v) = solve(
                e.position[i],
                e.velocity[i],
                e.target[i],
                dt.min(e.hold),
                self.config.force_omega,
            );
            let q = if dt > e.hold {
                solve(p, v, 0.0, dt - e.hold, self.config.relax_omega)
            } else {
                (p, v)
            };
            pos[i] = q.0;
            vel[i] = q.1;
        }
        (pos, vel)
    }
    fn amount(&self) -> f64 {
        self.event.as_ref().map_or(0.0, |e| {
            let dt = self.time - e.time;
            if e.held {
                mix(e.pull_from, 1.0, smooth(dt / self.config.pickup_seconds))
            } else {
                e.pull_from * (1.0 - smooth(dt / self.config.release_seconds))
            }
        })
    }
    fn force_event(&mut self, target: [f64; 2], hold: f64) {
        let (position, velocity) = self.force_at(self.time);
        self.force = Force {
            time: self.time,
            position,
            velocity,
            target,
            hold,
        };
    }
    fn neutral(&self) -> Lift {
        Lift {
            amount: 0.0,
            surprise: 0.0,
            smile: 0.0,
            gaze: [0.0; 2],
            rotations: vec![0.0; self.config.lift.bones.len()],
            reaction: vec![0.0; self.config.reaction.bones.len()],
        }
    }
    pub fn begin(&mut self, scene: &Scene, point: Vec2, _rendered: Option<&Scene>) {
        let Some(anchor) = world_point(scene, self.camera, point) else {
            return;
        };
        let from = self.sample();
        let pull_from = self.amount();
        self.event = Some(Event {
            time: self.time,
            held: true,
            sign: if anchor.x >= self.config.lift.gaze.focus[0] {
                1.0
            } else {
                -1.0
            },
            from,
            pull_from,
        });
        self.force_event([0.0; 2], 0.0);
        self.previous = (self.time, [0.0; 2]);
        self.translation = [
            Spring::new(self.config.follow_omega),
            Spring::new(self.config.follow_omega),
        ];
    }
    pub fn move_to(&mut self, delta: [i32; 2], size: u16, mirrored: bool) {
        if !self.held() {
            return;
        }
        let xy = [
            f64::from(delta[0]) * self.camera.width / f64::from(size)
                * if mirrored { -1.0 } else { 1.0 },
            -f64::from(delta[1]) * self.camera.height / f64::from(size),
        ];
        let dt = (self.time - self.previous.0).max(self.config.minimum_input_seconds);
        let target = std::array::from_fn(|i| {
            ((xy[i] - self.previous.1[i]) / dt / self.config.velocity_reference).clamp(-1.0, 1.0)
        });
        self.force_event(target, self.config.input_hold_seconds);
        self.previous = (self.time, xy);
        for (s, p) in self.translation.iter_mut().zip(xy) {
            s.target(self.time, p);
        }
    }
    pub fn release(&mut self) {
        if !self.held() {
            return;
        }
        let from = self.sample();
        let pull_from = self.amount();
        let sign = self.event.as_ref().unwrap().sign;
        self.event = Some(Event {
            time: self.time,
            held: false,
            sign,
            from,
            pull_from,
        });
        self.force_event([0.0; 2], 0.0);
    }
    fn sample(&self) -> Lift {
        let mut out = self.neutral();
        let Some(e) = &self.event else {
            return out;
        };
        let c = &self.config.lift;
        let t = &c.timings;
        let dt = self.time - e.time;
        let from = &e.from;
        if e.held {
            out.amount = mix(from.amount, 1.0, smooth(dt / t.pick_in));
            out.surprise = curve(from.surprise, &c.face.pick_surprise, dt);
            out.smile = curve(from.smile, &c.face.pick_smile, dt);
            for (i, b) in c.bones.iter().enumerate() {
                let age = dt - b.delay;
                let v = self.translation[0]
                    .state((self.time - b.delay).max(e.time))
                    .1;
                out.rotations[i] = (curve(from.rotations[i], &b.pick, age)
                    + b.sway_degrees * (v / c.velocity_reference).tanh() * smooth(age / t.pick_in))
                .clamp(-b.limit_degrees, b.limit_degrees);
            }
            let g = &c.gaze;
            let v = self.translation[0]
                .state((self.time - g.delay).max(e.time))
                .1;
            let target = [
                e.sign * g.held[0] + g.velocity_gain * (v / c.velocity_reference).tanh(),
                g.held[1],
            ];
            out.gaze = std::array::from_fn(|i| {
                mix(
                    from.gaze[i],
                    target[i].clamp(-g.limit[i], g.limit[i]),
                    smooth((dt - g.delay) / t.pick_in),
                )
            });
        } else if dt < t.recover {
            // Desktop placement is already final: landTime == releaseTime.
            out.amount = from.amount * (1.0 - smooth(dt / t.recover));
            out.surprise = from.surprise * (1.0 - smooth(dt / t.release_in));
            let scale_keys = |keys: &[[f64; 2]]| {
                keys.iter()
                    .map(|k| [k[0], k[1] * from.amount])
                    .collect::<Vec<_>>()
            };
            out.smile = curve(from.smile, &scale_keys(&c.face.recovery_smile), dt);
            for (i, b) in c.bones.iter().enumerate() {
                out.rotations[i] = curve(from.rotations[i], &scale_keys(&b.recovery), dt);
            }
            out.gaze = std::array::from_fn(|i| {
                let target = c.gaze.recovery[i] * if i == 0 { e.sign } else { 1.0 } * from.amount;
                curve(
                    from.gaze[i],
                    &[[0.28, target], [0.55, target], [t.recover, 0.0]],
                    dt,
                )
            });
        }
        let r = &self.config.reaction;
        for (i, b) in r.bones.iter().enumerate() {
            out.reaction[i] = if e.held {
                let u = smooth((dt - b.delay) / r.timings.pick_in);
                let v = self.translation[0]
                    .state((self.time - b.delay).max(e.time))
                    .1;
                (mix(from.reaction[i], b.hold_degrees, u)
                    + b.sway_degrees * (v / r.velocity_reference).tanh() * u)
                    .clamp(-b.limit_degrees, b.limit_degrees)
            } else {
                mix(
                    from.reaction[i],
                    b.fall_degrees,
                    smooth(dt / r.timings.release_in),
                ) * (1.0 - smooth((dt - b.delay * 0.4) / r.timings.land_recover))
            };
        }
        out
    }
    fn pull_pose(&self) -> (f64, f64, f64) {
        let force = self.force_at(self.time).0;
        let strength = force[0]
            .hypot(force[1] * self.config.vertical_weight)
            .clamp(0.0, 1.0);
        (
            strength,
            force[0].clamp(-1.0, 1.0),
            mix(
                self.config.source_range[0],
                self.config.source_range[1],
                smooth(strength),
            ),
        )
    }
    pub fn project(&self, scene: &mut Scene) {
        if !self.active() {
            return;
        }
        let p = self.sample();
        let amount = self.amount();
        let (_, direction, source_time) = self.pull_pose();
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
            // An original interaction retains priority; the pull keeps its clock.
            if *animation != self.follow.animation {
                continue;
            }
            *time_ns = ((self.follow.base_time_offset + self.time % self.follow.cycle_duration)
                * 1e9)
                .round() as u64;
            for (b, r) in self.config.lift.bones.iter().zip(&p.rotations) {
                offsets.push(BoneOffset {
                    bone: b.bone.clone(),
                    rotation_degrees: r * (1.0 - amount),
                    local_translation: Vec2::default(),
                    world_translation: Vec2::default(),
                });
            }
            for (b, r) in self.config.reaction.bones.iter().zip(&p.reaction) {
                offsets.push(BoneOffset {
                    bone: b.bone.clone(),
                    rotation_degrees: *r,
                    local_translation: Vec2::default(),
                    world_translation: Vec2::default(),
                });
            }
            let prior = overlay.gaze.as_ref();
            let raw = prior
                .and_then(|g| g.eyes.first())
                .map_or([0.0; 2], |e| [e.translation.x, e.translation.y]);
            let raw = std::array::from_fn(|i| mix(raw[i], p.gaze[i], p.amount));
            overlay.gaze = self.follow.gaze_pose(
                prior.map_or_else(Vec::new, |g| g.rotations.clone()),
                raw,
                1.0,
                1.0,
            );
            if p.amount > 0.25 {
                let face = &self.config.lift.face;
                let id = if p.surprise >= face.swap_threshold {
                    "surprised"
                } else if p.smile >= face.swap_threshold {
                    "happy"
                } else {
                    "neutral"
                };
                let s = &face.sources[id];
                overlay.layers.push(AnimationLayer {
                    animation: s.animation.clone(),
                    time: s.time,
                    weight: 1.0,
                    attachment_threshold: 0.5,
                    slots: vec!["F_S_Mouth".into()],
                    ..Default::default()
                });
            }
            if amount > 1e-9 {
                let table = &self.config.table;
                let hi = table
                    .times
                    .partition_point(|t| *t <= source_time)
                    .clamp(1, table.times.len() - 1);
                let lo = hi - 1;
                let w = ((source_time - table.times[lo]) / (table.times[hi] - table.times[lo]))
                    .clamp(0.0, 1.0);
                let bones = table
                    .bones
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let v = std::array::from_fn::<_, 3, _>(|k| {
                            mix(table.frames[lo][i][k], table.frames[hi][i][k], w) * amount
                        });
                        let d = self
                            .config
                            .direction
                            .iter()
                            .find(|d| d.bone == *b)
                            .map_or(0.0, |d| d.degrees * direction * amount);
                        AppliedDelta {
                            bone: b.clone(),
                            translation: Vec2::new(v[0], v[1]),
                            rotation_degrees: v[2] + d,
                        }
                    })
                    .collect();
                overlay.applied = Some(Box::new(AppliedPose {
                    bones,
                    grip: [
                        self.config.grip.weapon_bone.clone(),
                        self.config.grip.hand_bone.clone(),
                    ],
                }));
            }
        }
    }
    pub fn trace(&self) -> String {
        let (strength, direction, source) = self.pull_pose();
        let phase = if self.held() {
            if strength > 0.18 {
                "pulling"
            } else {
                "holding"
            }
        } else if self.active() {
            "recovering"
        } else {
            "none"
        };
        format!("phase={phase} held={} amount={:.4} strength={strength:.4} direction={direction:.4} source={source:.4}",self.held(),self.amount())
    }
}
