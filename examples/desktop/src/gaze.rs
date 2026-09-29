//! Analytic springs from the supplied follow controller, sampled independently
//! of rendering. Inputs are model coordinates; X11 stays in desktop.rs.
use mochi_core::{BoneOffset, LocalRotation, Scene, SceneContent, Vec2};
use mochi_package::{SpineCamera, SpineFollow};
use std::collections::VecDeque;

#[derive(Clone, Copy)]
struct Segment {
    time: f64,
    position: f64,
    velocity: f64,
    target: f64,
}
pub(super) struct Spring {
    omega: f64,
    segments: VecDeque<Segment>,
}
impl Spring {
    pub(super) fn new(omega: f64) -> Self {
        Self {
            omega,
            segments: VecDeque::from([Segment {
                time: 0.0,
                position: 0.0,
                velocity: 0.0,
                target: 0.0,
            }]),
        }
    }
    pub(super) fn state(&self, time: f64) -> (f64, f64) {
        if time <= 0.0 {
            return (0.0, 0.0);
        }
        let s = self
            .segments
            .iter()
            .rev()
            .find(|s| s.time <= time)
            .unwrap_or(&self.segments[0]);
        let dt = (time - s.time).max(0.0);
        let error = s.position - s.target;
        let b = s.velocity + self.omega * error;
        let e = (-self.omega * dt).exp();
        (
            s.target + (error + b * dt) * e,
            (s.velocity - self.omega * b * dt) * e,
        )
    }
    pub(super) fn target(&mut self, time: f64, target: f64) {
        if self.segments.back().is_some_and(|s| s.target == target) {
            return;
        }
        let (position, velocity) = self.state(time);
        if self.segments.back().is_some_and(|s| s.time == time) {
            self.segments.pop_back();
        }
        self.segments.push_back(Segment {
            time,
            position,
            velocity,
            target,
        });
        while self.segments.len() > 2
            && (self.segments[1].time < time - 2.0 || self.segments.len() > 256)
        {
            self.segments.pop_front();
        }
    }
}
pub(super) fn smooth(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * u * (u * (u * 6.0 - 15.0) + 10.0)
}
pub(super) fn curve(keys: &[[f64; 2]], time: f64) -> f64 {
    for pair in keys.windows(2) {
        if time <= pair[1][0] {
            return pair[0][1]
                + (pair[1][1] - pair[0][1])
                    * smooth((time - pair[0][0]) / (pair[1][0] - pair[0][0]));
        }
    }
    keys.last().map_or(1.0, |k| k[1])
}

pub struct Gaze {
    config: SpineFollow,
    camera: SpineCamera,
    eyes: [Spring; 2],
    head: Spring,
    time: f64,
    active_since: Option<f64>,
}
impl Gaze {
    pub fn new(config: SpineFollow, camera: SpineCamera) -> Self {
        Self {
            eyes: [Spring::new(config.eye_omega), Spring::new(config.eye_omega)],
            head: Spring::new(config.head_omega),
            config,
            camera,
            time: 0.0,
            active_since: None,
        }
    }
    pub fn pose_time(&self) -> f64 {
        self.config.base_time_offset + self.time % self.config.cycle_duration
    }
    pub fn advance(&mut self, delta: f64) {
        self.time += delta;
    }
    pub fn input(&mut self, scene: &Scene, point: Option<Vec2>) {
        let world = point
            .and_then(|p| world_point(scene, self.camera, p))
            .filter(|p| p.x.is_finite() && p.y.is_finite());
        // Desktop attention is local to the character. Preserve the authored
        // close-range response, then fade out between 12 and 18 input radii.
        // Model coordinates keep this range consistent across DPI and mirroring.
        let attention = world.map_or(0.0, |p| {
            let distance = ((p.x - self.config.focus[0]) / self.config.radius[0])
                .hypot((p.y - self.config.focus[1]) / self.config.radius[1]);
            1.0 - smooth((distance - 12.0) / 6.0)
        });
        self.target_with_attention(world, attention);
    }
    #[cfg(all(test, feature = "spine"))]
    fn target(&mut self, world: Option<Vec2>) {
        self.target_with_attention(world, 1.0);
    }
    fn target_with_attention(&mut self, world: Option<Vec2>, attention: f64) {
        let normalized = |radius: [f64; 2], dead: f64| {
            let [x, y] = world.map_or([0.0, 0.0], |p| {
                [
                    (p.x - self.config.focus[0]) / radius[0],
                    (p.y - self.config.focus[1]) / radius[1],
                ]
            });
            let length = x.hypot(y);
            let gain = if length > 0.0 {
                ((length - dead) / (1.0 - dead)).clamp(0.0, 1.0) / length
            } else {
                0.0
            };
            [x * gain * attention, y * gain * attention]
        };
        let [x, _] = normalized(self.config.radius, self.config.dead_zone);
        let eyes = normalized(
            self.config.gaze_radius.unwrap_or(self.config.radius),
            self.config.gaze_dead_zone.unwrap_or(self.config.dead_zone),
        );
        self.eyes[0].target(self.time, eyes[0] * self.config.gaze_limit[0]);
        self.eyes[1].target(self.time, eyes[1] * self.config.gaze_limit[1]);
        self.head.target(
            self.time + self.config.head_delay,
            -self.config.head_max_degrees * x,
        );
    }
    pub fn project(&mut self, scene: &mut Scene, enabled: bool) {
        if !enabled {
            self.active_since = None;
            return;
        }
        let since = *self.active_since.get_or_insert(self.time);
        let weight = smooth((self.time - since) / 0.22);
        let phase = self.time % self.config.cycle_duration;
        let blink = curve(&self.config.blink_weight, phase);
        let head = self
            .head
            .state(self.time)
            .0
            .clamp(-self.config.head_max_degrees, self.config.head_max_degrees);
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
            if animation != &self.config.animation {
                continue;
            }
            *time_ns = ((self.config.base_time_offset + phase) * 1e9).round() as u64;
            offsets.clear();
            let mut rotations = Vec::new();
            let mut rotate = |bone: &str, angle: f64| {
                rotations.push(LocalRotation {
                    bone: bone.into(),
                    degrees: angle * weight,
                });
            };
            rotate(&self.config.head_bone, head);
            rotate(
                &self.config.torso_bone,
                -self.config.torso_counter_ratio * head,
            );
            for s in &self.config.secondary {
                let delta = s.gain * (self.head.state((self.time - s.delay).max(0.0)).0 - head);
                rotate(&s.bone, s.limit_degrees * (delta / s.limit_degrees).tanh());
            }
            let raw = std::array::from_fn(|i| {
                self.eyes[i]
                    .state(self.time)
                    .0
                    .clamp(-self.config.gaze_limit[i], self.config.gaze_limit[i])
            });
            overlay.gaze = self.config.gaze_pose(rotations.clone(), raw, blink, weight);
            if overlay.gaze.is_none() {
                offsets.extend(rotations.into_iter().map(|r| BoneOffset {
                    bone: r.bone,
                    rotation_degrees: r.degrees,
                    local_translation: Vec2::default(),
                    world_translation: Vec2::default(),
                }));
                for bone in &self.config.gaze_bones {
                    offsets.push(BoneOffset {
                        bone: bone.clone(),
                        rotation_degrees: 0.0,
                        local_translation: Vec2::default(),
                        world_translation: Vec2::new(
                            raw[0] * blink * weight,
                            raw[1] * blink * weight,
                        ),
                    });
                }
            }
        }
    }
}
pub(super) fn world_point(scene: &Scene, camera: SpineCamera, p: Vec2) -> Option<Vec2> {
    let node = scene
        .nodes
        .iter()
        .find(|n| matches!(n.content, SceneContent::Skeletal { .. }))?;
    let SceneContent::Skeletal { size, .. } = &node.content else {
        return None;
    };
    let t = &node.transform;
    if t.scale.x == 0.0 || t.scale.y == 0.0 {
        return None;
    }
    let (sin, cos) = t.rotation_radians.sin_cos();
    let (dx, dy) = (p.x - t.translation.x, p.y - t.translation.y);
    Some(Vec2::new(
        camera.x + (cos * dx + sin * dy) / t.scale.x / size.x * camera.width,
        camera.y + (1.0 - (-sin * dx + cos * dy) / t.scale.y / size.y) * camera.height,
    ))
}

#[cfg(all(test, feature = "spine"))]
mod tests {
    use super::*;
    use mochi_core::{SceneNode, Transform};
    use mochi_package::FollowSecondary;
    fn config() -> SpineFollow {
        SpineFollow {
            animation: "Relax".into(),
            base_time_offset: 0.5,
            cycle_duration: 2.0,
            focus: [16.0, 270.0],
            radius: [175.0, 150.0],
            gaze_radius: None,
            gaze_dead_zone: None,
            native_eyes: None,
            dead_zone: 0.025,
            gaze_bones: ["F_L_Eyeball".into(), "F_R_Eyeball".into()],
            gaze_limit: [2.6, 2.2],
            eye_omega: 32.0,
            head_bone: "F_Face".into(),
            head_max_degrees: 5.8,
            head_omega: 13.0,
            head_delay: 0.11,
            torso_bone: "F_Chest2".into(),
            torso_counter_ratio: 0.095,
            blink_weight: vec![
                [0.0, 1.0],
                [0.65, 1.0],
                [0.725, 0.0],
                [0.825, 0.0],
                [0.95, 1.0],
                [2.0, 1.0],
            ],
            secondary: [
                ("F_L_Ear_B2", 0.12, 0.55, 2.4),
                ("F_R_Ear_B4", 0.15, 0.45, 2.2),
                ("F_L_Hair_B4", 0.15, 0.4, 2.0),
                ("F_R_Hair_B3", 0.15, 0.35, 1.8),
                ("F_R_Hair_A4", 0.14, 0.3, 1.5),
            ]
            .into_iter()
            .map(|(bone, delay, gain, limit_degrees)| FollowSecondary {
                bone: bone.into(),
                delay,
                gain,
                limit_degrees,
            })
            .collect(),
        }
    }
    pub(super) fn camera() -> SpineCamera {
        SpineCamera {
            x: -395.0,
            y: -180.0,
            width: 790.0,
            height: 790.0,
        }
    }
    fn scene() -> Scene {
        Scene {
            nodes: vec![SceneNode {
                id: "body".into(),
                transform: Transform::default(),
                opacity: 1.0,
                content: SceneContent::Skeletal {
                    asset: "spine.toml".into(),
                    size: Vec2::new(112.0, 112.0),
                    animation: "Relax".into(),
                    time_ns: 0,
                    offsets: vec![],
                    overlay: Default::default(),
                },
            }],
        }
    }
    #[test]
    fn matches_supplied_controller_including_head_delay_blink_and_release() {
        let reference: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/gaze-reference.json")).unwrap();
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-10, "{a} != {b}");
        for hz in [60, 120, 144] {
            let mut gaze = Gaze::new(config(), camera());
            gaze.active_since = Some(0.0);
            gaze.target(Some(Vec2::new(200.0, 420.0)));
            let mut released = false;
            for sample in reference.as_array().unwrap() {
                let t = sample["time"].as_f64().unwrap();
                if t >= 1.2 && !released {
                    gaze.time = 1.2;
                    gaze.target(None);
                    released = true;
                }
                // Extra render samples must not integrate or mutate springs.
                for frame in 0..(t * f64::from(hz)) as usize {
                    gaze.head.state(frame as f64 / f64::from(hz));
                }
                gaze.time = t;
                let mut scene = scene();
                gaze.project(&mut scene, true);
                let SceneContent::Skeletal {
                    time_ns, offsets, ..
                } = &scene.nodes[0].content
                else {
                    panic!()
                };
                assert!(
                    (*time_ns as f64 / 1e9 - sample["sourceTime"].as_f64().unwrap()).abs() < 1e-8
                );
                let weight = smooth(t / 0.22);
                for o in offsets {
                    if let Some(v) = sample["rotations"].get(&o.bone) {
                        close(o.rotation_degrees, v.as_f64().unwrap() * weight);
                    } else {
                        close(
                            o.world_translation.x,
                            sample["gaze"][0].as_f64().unwrap() * weight,
                        );
                        close(
                            o.world_translation.y,
                            sample["gaze"][1].as_f64().unwrap() * weight,
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn disabled_follow_preserves_original_pose_and_resumes_from_latest_target() {
        let mut gaze = Gaze::new(config(), camera());
        gaze.target(Some(Vec2::new(-10000.0, 0.0)));
        gaze.advance(1.0);
        let base = scene();
        let mut actual = base.clone();
        gaze.project(&mut actual, false);
        assert_eq!(actual, base);
        gaze.target(Some(Vec2::new(10000.0, 270.0)));
        gaze.advance(1.0);
        gaze.project(&mut actual, true); // Fade back in at zero weight.
        let SceneContent::Skeletal { offsets, .. } = &actual.nodes[0].content else {
            panic!()
        };
        assert!(offsets
            .iter()
            .all(|o| o.rotation_degrees == 0.0 && o.world_translation == Vec2::default()));
        gaze.advance(0.3);
        actual = base;
        gaze.project(&mut actual, true);
        let SceneContent::Skeletal { offsets, .. } = &actual.nodes[0].content else {
            panic!()
        };
        assert!(offsets[0].rotation_degrees < -5.0);
    }
    #[test]
    fn dead_zone_bounds_history_and_world_coordinate_mapping() {
        let mut gaze = Gaze::new(config(), camera());
        gaze.target(Some(Vec2::new(16.01, 270.01)));
        gaze.advance(1.0);
        assert_eq!(gaze.head.state(1.0), (0.0, 0.0));
        for i in 0..10000 {
            gaze.time = i as f64 / 60.0;
            gaze.target(Some(Vec2::new(if i % 2 == 0 { -1e6 } else { 1e6 }, 0.0)));
        }
        assert!(gaze.head.segments.len() <= 123);
        let mut s = scene();
        s.nodes[0].transform = Transform {
            translation: Vec2::new(200.0, 20.0),
            scale: Vec2::new(-2.0, 2.0),
            rotation_radians: 0.0,
        };
        let p = world_point(&s, camera(), Vec2::new(88.0, 132.0)).unwrap();
        assert_eq!(p, Vec2::new(0.0, 215.0));
    }

    #[test]
    fn desktop_distance_fades_and_returns_both_rigs_to_neutral() {
        let angelina: SpineFollow = serde_json::from_str(include_str!(
            "../../../characters/angelina-companion/follow-config.json"
        ))
        .unwrap();
        for config in [config(), angelina] {
            let local = |distance: f64| {
                Vec2::new(
                    (config.focus[0] + config.radius[0] * distance - camera().x) / camera().width
                        * 112.0,
                    (1.0 - (config.focus[1] - camera().y) / camera().height) * 112.0,
                )
            };
            let base = scene();
            let mut gaze = Gaze::new(config.clone(), camera());
            // This point was outside the previous attention range.
            gaze.input(&base, Some(local(10.0)));
            gaze.advance(1.0);
            assert!(gaze.eyes[0].state(gaze.time).0 > config.gaze_limit[0] * 0.99);
            assert!(gaze.head.state(gaze.time).0 < -config.head_max_degrees * 0.99);
            let eye_before = gaze.eyes[0].state(gaze.time);
            let head_before = gaze.head.state(gaze.time);
            gaze.input(&base, Some(local(21.0)));
            assert_eq!(
                gaze.eyes[0].state(gaze.time),
                eye_before,
                "No snap at the range boundary"
            );
            assert_eq!(gaze.head.state(gaze.time), head_before);
            gaze.advance(1.0);
            assert!(gaze.eyes[0].state(gaze.time).0.abs() < 0.001);
            assert!(gaze.head.state(gaze.time).0.abs() < 0.01);
            // Staying far away must not re-arm a saturated target.
            for _ in 0..30 {
                gaze.input(&base, Some(local(-24.0)));
                gaze.advance(1.0 / 30.0);
            }
            assert!(gaze.eyes[0].state(gaze.time).0.abs() < 0.001);
            gaze.input(&base, Some(local(-10.0)));
            gaze.advance(1.0);
            assert!(gaze.eyes[0].state(gaze.time).0 < -config.gaze_limit[0] * 0.99);
            gaze.input(&base, None);
            gaze.advance(1.0);
            assert!(gaze.eyes[0].state(gaze.time).0.abs() < 0.001);
            // Halfway through the fade, both eye and head targets are halved.
            gaze.input(&base, Some(local(15.0)));
            gaze.advance(1.0);
            assert!((gaze.eyes[0].state(gaze.time).0 / config.gaze_limit[0] - 0.5).abs() < 0.001);
            assert!((gaze.head.state(gaze.time).0 / config.head_max_degrees + 0.5).abs() < 0.001);
        }
    }

    #[test]
    fn distance_uses_character_coordinates_through_mirror_scale_and_rotation() {
        let config = config();
        let camera = camera();
        for scale in [0.5, 1.0, 2.0] {
            for mirrored in [false, true] {
                for rotation in [0.0, 0.4] {
                    let mut s = scene();
                    let t = &mut s.nodes[0].transform;
                    t.translation = Vec2::new(160.0, 120.0);
                    t.scale = Vec2::new(if mirrored { -scale } else { scale }, scale);
                    t.rotation_radians = rotation;
                    let (sin, cos) = rotation.sin_cos();
                    let pointer = |distance: f64| {
                        let x = (config.focus[0] + config.radius[0] * distance - camera.x)
                            / camera.width
                            * 112.0
                            * t.scale.x;
                        let y = (1.0 - (config.focus[1] - camera.y) / camera.height)
                            * 112.0
                            * t.scale.y;
                        Vec2::new(
                            t.translation.x + cos * x - sin * y,
                            t.translation.y + sin * x + cos * y,
                        )
                    };
                    let near = pointer(10.0);
                    let far = pointer(21.0);
                    let mut gaze = Gaze::new(config.clone(), camera);
                    gaze.input(&s, Some(near));
                    gaze.advance(1.0);
                    assert!(gaze.eyes[0].state(gaze.time).0 > 2.5);
                    gaze.input(&s, Some(far));
                    gaze.advance(1.0);
                    assert!(gaze.eyes[0].state(gaze.time).0.abs() < 0.001);
                }
            }
        }
    }

    #[test]
    #[ignore = "requires supplied Shanbi data and local GPU; renders offscreen without desktop input"]
    fn supplied_shanbi_gaze_render_and_reference() {
        use mochi_preview::Viewport;
        use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
        use std::{fs, path::PathBuf};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let output = root.join("target/gaze-check");
        fs::create_dir_all(&output).unwrap();
        let package = mochi_package::LoadedPackage::load(
            root.join("characters/shanbi-companion/shanbi.mochi"),
        )
        .unwrap();
        let resource = package.spine("spine.toml").unwrap();
        let cfg = resource.follow.clone().unwrap();
        let mut evaluator = mochi_spine::Evaluator::new(&resource.model).unwrap();
        let mut reference = mochi_spine::ReferenceEvaluator::new(&resource.model).unwrap();
        let options = RenderOptions {
            width: 512,
            height: 512,
            ..Default::default()
        };
        let mut cpu = CanvasRenderer::new(options).unwrap();
        let mut gpu = CanvasRenderer::new(options).unwrap();
        gpu.enable_spine_gpu().unwrap();
        let viewport = Viewport::new([112, 112], [512, 512]).unwrap().unwrap();
        let mut reports = Vec::new();
        let mut images = Vec::new();
        for (label, point, time) in [
            ("neutral", [16.0, 270.0], 1.1),
            ("left", [-400.0, 270.0], 1.1),
            ("right", [400.0, 270.0], 1.1),
            ("up", [16.0, 600.0], 1.1),
            ("down", [16.0, -100.0], 1.1),
            ("blink", [400.0, 270.0], 0.78),
        ] {
            let mut gaze = Gaze::new(cfg.clone(), resource.camera);
            gaze.active_since = Some(0.0);
            gaze.target(Some(Vec2::new(point[0], point[1])));
            gaze.advance(time);
            let mut scene = package.instantiate("gaze-check").unwrap().scene().unwrap();
            gaze.project(&mut scene, true);
            let SceneContent::Skeletal {
                animation,
                time_ns,
                offsets,
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
                    local_translation: [0.0; 2],
                    world_translation: [o.world_translation.x, o.world_translation.y],
                })
                .collect();
            if label == "blink" {
                assert!(offsets
                    .iter()
                    .filter(|o| o.bone.contains("Eyeball"))
                    .all(|o| o.world_translation == [0.0, 0.0]));
            }
            let a = evaluator
                .pose_with_offsets(animation, *time_ns as f64 / 1e9, &offsets)
                .unwrap();
            let b = reference
                .pose_with_offsets(animation, *time_ns as f64 / 1e9, &offsets)
                .unwrap();
            assert_eq!(a.len(), b.len());
            let mut bound_error = 0.0f32;
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.page, b.page);
                for attr in 0..12 {
                    let bounds = |m: &mochi_spine::Mesh| {
                        m.vertices
                            .iter()
                            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                                (lo.min(v[attr]), hi.max(v[attr]))
                            })
                    };
                    let (alo, ahi) = bounds(a);
                    let (blo, bhi) = bounds(b);
                    bound_error = bound_error.max((alo - blo).abs()).max((ahi - bhi).abs());
                }
            }
            assert!(
                bound_error < 0.002,
                "{label}: C/JS geometry bounds differ by {bound_error}"
            );
            let scene = viewport.project_scene(&scene);
            let layers = [RenderLayer {
                package: &package,
                scene: &scene,
            }];
            cpu.render_layers(&layers).unwrap();
            gpu.render_layers(&layers).unwrap();
            let a = cpu.frame().rgba8();
            let b = gpu.frame().rgba8();
            let mut sum = 0.0;
            let mut visible = 0;
            let mut big = 0;
            for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
                if a[3] != 0 || b[3] != 0 {
                    visible += 1;
                    let mut diff = (f64::from(a[3]) - f64::from(b[3])).abs();
                    for c in 0..3 {
                        diff += (f64::from(a[c]) * f64::from(a[3]) / 255.0
                            - f64::from(b[c]) * f64::from(b[3]) / 255.0)
                            .abs();
                    }
                    sum += diff;
                    if diff > 32.0 {
                        big += 1;
                    }
                }
            }
            let mean = sum / (visible * 4) as f64;
            let big_fraction = big as f64 / visible as f64;
            assert!(
                mean < 0.5 && big_fraction < 0.01,
                "{label}: CPU/GPU mean {mean}, large differences {big_fraction}"
            );
            assert_eq!(cpu.frame().pixel(0, 0), Some([0; 4]));
            cpu.frame()
                .write_png(fs::File::create(output.join(format!("{label}.png"))).unwrap())
                .unwrap();
            gpu.frame()
                .write_png(fs::File::create(output.join(format!("{label}-gpu.png"))).unwrap())
                .unwrap();
            reports.push(serde_json::json!({"pose":label,"bounds_error":bound_error,"cpu_gpu_mean_pma_error":mean,"large_difference_fraction":big_fraction}));
            images.push(a.to_vec());
            // Absolute evaluation must not carry controls into original actions.
            assert_eq!(
                evaluator.pose("Move", 0.3).unwrap(),
                mochi_spine::Evaluator::new(&resource.model)
                    .unwrap()
                    .pose("Move", 0.3)
                    .unwrap()
            );
        }
        assert_ne!(images[0], images[1]);
        assert_ne!(images[1], images[2]);
        assert_ne!(images[3], images[4]);
        fs::write(
            output.join("results.json"),
            serde_json::to_string_pretty(&reports).unwrap(),
        )
        .unwrap();
        println!("Gaze poses verified: {}", output.display());
    }
}

#[cfg(all(test, feature = "spine"))]
mod angelina_tests;
