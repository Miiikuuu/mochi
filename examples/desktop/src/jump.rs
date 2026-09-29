//! A finite, stationary jump. Native key ownership is handled by the desktop.
use crate::gaze::{curve, smooth};
use mochi_core::{AnimationLayer, LocalRotation, Scene, SceneContent, SlotAlpha};
use mochi_package::{SpineCamera, SpineFollow, SpineJump};
use std::time::Instant;
#[derive(Clone, Debug)]
struct Pose {
    active: bool,
    phase: &'static str,
    age: f64,
    height: f64,
    amount: f64,
    source: f64,
    tuck: f64,
    crouch: f64,
    secondary: f64,
}
impl Default for Pose {
    fn default() -> Self {
        Self {
            active: false,
            phase: "none",
            age: 0.0,
            height: 0.0,
            amount: 0.0,
            source: 0.0,
            tuck: 0.0,
            crouch: 0.0,
            secondary: 0.0,
        }
    }
}
/// Monotone Hermite source clock, preserving the authored speed across landmarks.
fn source_curve(keys: &[[f64; 2]], t: f64) -> f64 {
    if t <= keys[0][0] {
        return keys[0][1];
    }
    if t >= keys[keys.len() - 1][0] {
        return keys[keys.len() - 1][1];
    }
    let slope = |i: usize| (keys[i + 1][1] - keys[i][1]) / (keys[i + 1][0] - keys[i][0]);
    let tangent = |i: usize| {
        if i == 0 {
            slope(0)
        } else if i == keys.len() - 1 {
            slope(i - 1)
        } else {
            let a = slope(i - 1);
            let b = slope(i);
            if a * b <= 0.0 {
                0.0
            } else {
                2.0 * a * b / (a + b)
            }
        }
    };
    let i = keys.windows(2).position(|k| t <= k[1][0]).unwrap();
    let [a, x] = keys[i];
    let [b, y] = keys[i + 1];
    let h = b - a;
    let u = (t - a) / h;
    (2.0 * u * u * u - 3.0 * u * u + 1.0) * x
        + (u * u * u - 2.0 * u * u + u) * h * tangent(i)
        + (-2.0 * u * u * u + 3.0 * u * u) * y
        + (u * u * u - u * u) * h * tangent(i + 1)
}
pub struct Jump {
    config: SpineJump,
    follow: SpineFollow,
    camera: SpineCamera,
    epoch: Instant,
    event: Option<(f64, f64)>,
    count: usize,
}
impl Jump {
    pub fn new(config: SpineJump, follow: SpineFollow, camera: SpineCamera) -> Self {
        Self {
            config,
            follow,
            camera,
            epoch: Instant::now(),
            event: None,
            count: 0,
        }
    }
    fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }
    pub fn active(&self) -> bool {
        self.sample(self.now()).active
    }
    pub fn start(&mut self, anchor_time: f64) -> bool {
        self.start_at(self.now(), anchor_time)
    }
    fn start_at(&mut self, t: f64, anchor_time: f64) -> bool {
        if self.sample(t).active {
            return false;
        }
        self.event = Some((t, anchor_time));
        self.count += 1;
        true
    }
    fn sample(&self, t: f64) -> Pose {
        let Some((start, _)) = self.event else {
            return Pose::default();
        };
        let age = t - start;
        let c = &self.config;
        if age < 0.0 || age >= c.duration {
            return Pose::default();
        }
        let f = &c.flight;
        let u = ((age - f.takeoff) / (f.landing - f.takeoff)).clamp(0.0, 1.0);
        let air = age > f.takeoff && age < f.landing;
        Pose {
            active: true,
            phase: if age < f.takeoff {
                "anticipation"
            } else if air {
                if u < 0.5 {
                    "rising"
                } else {
                    "falling"
                }
            } else if age < c.exit_start {
                "landing"
            } else {
                "settling"
            },
            age,
            height: if air {
                4.0 * f.height * u * (1.0 - u)
            } else {
                0.0
            },
            tuck: if air {
                (std::f64::consts::PI * u).sin().powi(2)
            } else {
                0.0
            },
            amount: smooth(age / c.entry_seconds)
                * (1.0 - smooth((age - c.exit_start) / (c.duration - c.exit_start))),
            source: source_curve(&c.source_curve, age),
            crouch: curve(&c.crouch_curve, age),
            secondary: curve(&c.secondary_curve, age),
        }
    }
    pub fn project(&self, scene: &mut Scene) {
        self.project_at(scene, self.now());
    }
    fn project_at(&self, scene: &mut Scene, t: f64) {
        let p = self.sample(t);
        if !p.active || p.amount <= 0.0 {
            return;
        }
        let c = &self.config;
        for node in &mut scene.nodes {
            let SceneContent::Skeletal {
                animation,
                size,
                overlay,
                ..
            } = &mut node.content
            else {
                continue;
            };
            if animation != &self.follow.animation {
                continue;
            }
            let mut body = c.body.layer(p.source, p.amount);
            let mut plant = c.plant(&self.follow, self.event.unwrap().1);
            plant.crouch = p.crouch;
            plant.tuck = p.tuck;
            body.foot_plant = Some(plant);
            body.rotations = c
                .secondary
                .iter()
                .map(|r| LocalRotation {
                    bone: r.bone.clone(),
                    degrees: r.degrees * p.secondary * p.amount,
                })
                .collect();
            overlay.layers.push(body);
            let timing = &c.face_timing;
            let amount = smooth((p.age - timing.start) / timing.r#in)
                * (1.0 - smooth((p.age - timing.hold_until) / timing.out))
                * p.amount;
            let amount = if amount <= 1e-12 {
                0.0
            } else if amount >= 1.0 - 1e-12 {
                1.0
            } else {
                amount
            };
            if amount > 0.0 {
                let source = curve(&c.face_curve, p.age).clamp(c.face.start, c.face.end);
                let mut face = c.face.layer(source, amount);
                face.slots.retain(|s| !c.mouth_slots.contains(s));
                overlay.layers.push(face);
                let h = &c.eye_highlights;
                let alpha = 1.0
                    - amount
                        * smooth(
                            (source - h.fade_source_start)
                                / (h.hidden_source_at - h.fade_source_start),
                        );
                overlay.layers.push(AnimationLayer {
                    animation: c.mouth_source.animation.clone(),
                    time: c.mouth_source.time,
                    weight: amount,
                    attachment_threshold: timing.swap_threshold,
                    slots: c.mouth_slots.clone(),
                    slot_alpha: h
                        .slots
                        .iter()
                        .map(|s| SlotAlpha {
                            slot: s.clone(),
                            alpha,
                        })
                        .collect(),
                    ..Default::default()
                });
            }
            node.transform.translation.y -= p.height / self.camera.height * size.y;
        }
    }
    pub fn trace(&self) -> String {
        let p = self.sample(self.now());
        format!(
            "count={} phase={} height={:.3} amount={:.3} source={:.4}",
            self.count, p.phase, p.height, p.amount, p.source
        )
    }
}

#[cfg(all(test, feature = "spine"))]
mod tests {
    use super::*;
    use serde_json::Value;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/jump-reference.json")).unwrap()
    }
    fn controller() -> Jump {
        let old: Value =
            serde_json::from_str(include_str!("../tests/fixtures/drag-reference.json")).unwrap();
        Jump::new(
            serde_json::from_str(include_str!("../tests/fixtures/jump-config.json")).unwrap(),
            serde_json::from_value(old["follow"].clone()).unwrap(),
            serde_json::from_value(old["camera"].clone()).unwrap(),
        )
    }
    #[test]
    fn original_jump_timing_matches_at_60_120_144_hz_and_rejects_retrigger() {
        let f = fixture();
        for hz in [60, 120, 144] {
            let mut c = controller();
            let mut next = 0;
            let mut render_time = 0.0;
            let events = f["events"].as_array().unwrap();
            for s in f["samples"].as_array().unwrap() {
                let t = s["time"].as_f64().unwrap();
                while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
                    let e = &events[next];
                    let time = e["time"].as_f64().unwrap();
                    assert_eq!(
                        c.start_at(time, 0.5 + time % 2.0),
                        e["accepted"].as_bool().unwrap()
                    );
                    next += 1;
                }
                while render_time < t {
                    let _ = c.sample(render_time);
                    render_time += 1.0 / hz as f64;
                }
                let p = c.sample(t);
                assert_eq!(p.active, s["active"].as_bool().unwrap());
                assert_eq!(p.phase, s["phase"].as_str().unwrap());
                for (name, value) in [
                    ("height", p.height),
                    ("amount", p.amount),
                    ("source", p.source),
                    ("tuck", p.tuck),
                    ("crouch", p.crouch),
                    ("secondary", p.secondary),
                ] {
                    assert!(
                        (value - s[name].as_f64().unwrap()).abs() < 1e-9,
                        "{hz}Hz t={t} {name}: {value} != {}",
                        s[name]
                    );
                }
            }
            assert_eq!(c.count, 2);
        }
    }
    #[test]
    fn contact_and_accent_validation_is_bounded() {
        let c = controller();
        let mut layer = c.config.body.layer(0.2, 1.0);
        layer.foot_plant = Some(c.config.plant(&c.follow, 0.5));
        let valid = mochi_core::PoseOverlay {
            layers: vec![layer],
            ..Default::default()
        };
        valid.validate().unwrap();
        let mut bad = valid.clone();
        bad.layers[0].foot_plant.as_mut().unwrap().tuck = f64::NAN;
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        let p = bad.layers[0].foot_plant.as_mut().unwrap();
        p.feet[1] = p.feet[0].clone();
        assert!(bad.validate().is_err());
        let mut bad = valid.clone();
        bad.layers[0].foot_plant.as_mut().unwrap().crouch = 33.0;
        assert!(bad.validate().is_err());
        let mut bad = valid;
        bad.layers[0].slot_alpha = vec![SlotAlpha {
            slot: "eye".into(),
            alpha: 1.1,
        }];
        assert!(bad.validate().is_err());
    }
    #[test]
    #[ignore = "requires supplied Shanbi assets and local GPU; offscreen only"]
    fn supplied_shanbi_jump_render_and_reference() {
        use mochi_package::LoadedPackage;
        use mochi_preview::Viewport;
        use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
        use std::{fs, path::PathBuf};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("target/jump-check");
        fs::create_dir_all(&out).unwrap();
        let package =
            LoadedPackage::load(root.join("characters/shanbi-jump/shanbi.mochi")).unwrap();
        let resource = package.spine("spine.toml").unwrap();
        let mut c = controller();
        let f = fixture();
        let events = f["events"].as_array().unwrap();
        let mut next = 0;
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
        for item in f["geometry"].as_array().unwrap() {
            let t = item["time"].as_f64().unwrap();
            while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
                let time = events[next]["time"].as_f64().unwrap();
                c.start_at(time, 0.5 + time % 2.0);
                next += 1;
            }
            let mut scene = package.instantiate("jump-test").unwrap().scene().unwrap();
            if let SceneContent::Skeletal { time_ns, .. } = &mut scene.nodes[0].content {
                *time_ns = ((0.5 + t % 2.0) * 1e9).round() as u64;
            }
            c.project_at(&mut scene, t);
            scene.validate().unwrap();
            let height = c.sample(t).height;
            let SceneContent::Skeletal {
                animation,
                time_ns,
                overlay,
                size,
                ..
            } = &scene.nodes[0].content
            else {
                panic!()
            };
            assert!(
                (scene.nodes[0].transform.translation.y + height / c.camera.height * size.y).abs()
                    < 1e-9
            );
            let a = evaluator
                .pose_with_overlay(animation, *time_ns as f64 / 1e9, &[], overlay)
                .unwrap();
            let b = reference
                .pose_with_overlay(animation, *time_ns as f64 / 1e9, &[], overlay)
                .unwrap();
            let original = item["bounds"].as_array().unwrap();
            assert_eq!(a.len(), original.len(), "t={t} original meshes");
            assert_eq!(a.len(), b.len());
            let mut error = 0f32;
            let mut bridge = 0f32;
            for ((a, b), orig) in a.iter().zip(&b).zip(original) {
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
                            (av[i] + if attr == 1 { height as f32 } else { 0.0 }
                                - orig[attr * 2 + i].as_f64().unwrap() as f32)
                                .abs(),
                        );
                        bridge = bridge.max((av[i] - bv[i]).abs());
                    }
                }
            }
            assert!(error < 0.003, "t={t} original source error={error}");
            assert!(bridge < 0.003, "t={t} C/JS error={bridge}");
            assert_eq!(
                evaluator.pose("Move", 0.3).unwrap(),
                base,
                "jump/slot alpha leaked at {t}"
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
            assert!(mean < 0.5 && big as f64 / (count as f64) < 0.01);
            cpu.frame()
                .write_png(fs::File::create(out.join(format!("jump-{t:.3}.png"))).unwrap())
                .unwrap();
            gpu.frame()
                .write_png(fs::File::create(out.join(format!("jump-{t:.3}-gpu.png"))).unwrap())
                .unwrap();
            reports.push(serde_json::json!({"time":t,"height":height,"original_source_bounds_error":error,"bridge_bounds_error":bridge,"cpu_gpu_mean_pma_error":mean}));
        }
        fs::write(
            out.join("results.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
}
