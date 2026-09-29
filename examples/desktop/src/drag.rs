//! Authored drag acting on an absolute clock. Window placement is deliberately
//! separate: release starts recovery at the user's chosen desktop position.
use crate::gaze::{curve, smooth, world_point, Spring};
use mochi_core::{BoneOffset, Scene, SceneContent, Vec2};
use mochi_package::{DragPose, SpineCamera, SpineDrag, SpineFollow};

#[derive(Clone, Debug)]
struct Pose {
    rotations: Vec<f64>,
    feet: Vec<[f64; 2]>,
    face: [f64; 2],
    gaze: [f64; 2],
    phase: String,
}
#[derive(Clone)]
enum Kind {
    Pick([f64; 2]),
    Release,
}
struct Event {
    time: f64,
    kind: Kind,
    from: Pose,
}

pub struct StoryDrag {
    config: SpineDrag,
    follow: SpineFollow,
    camera: SpineCamera,
    time: f64,
    event: Option<Event>,
    translation: [Spring; 2],
}
fn lerp(a: f64, b: f64, w: f64) -> f64 {
    a + (b - a) * w
}
fn bounded_gaze(mut xy: [f64; 2], limit: [f64; 2]) -> [f64; 2] {
    let r = (xy[0] / limit[0]).hypot(xy[1] / limit[1]);
    if r > 1.0 {
        xy = xy.map(|v| v / r);
    }
    xy
}
impl StoryDrag {
    pub fn new(config: SpineDrag, follow: SpineFollow, camera: SpineCamera) -> Self {
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
        }
    }
    pub fn advance(&mut self, dt: f64) {
        self.time += dt;
    }
    pub fn active(&self) -> bool {
        self.sample(self.time).phase != "none"
    }
    pub fn held(&self) -> bool {
        self.event
            .as_ref()
            .is_some_and(|e| matches!(e.kind, Kind::Pick(_)))
    }
    pub fn clear(&mut self) {
        self.event = None;
    }
    pub fn begin(&mut self, scene: &Scene, point: Vec2, rendered: Option<&Scene>) {
        let Some(anchor) = world_point(scene, self.camera, point) else {
            return;
        };
        let mut from = self.sample(self.time);
        // Carry the visible gaze into pickup; otherwise disabling live follow
        // would snap a tilted head/pupils back to neutral on the first frame.
        if let Some(scene) = rendered {
            if let Some(SceneContent::Skeletal { offsets, .. }) =
                scene.nodes.first().map(|n| &n.content)
            {
                for (i, b) in self.config.bones.iter().enumerate() {
                    if let Some(o) = offsets.iter().find(|o| o.bone == b.bone) {
                        from.rotations[i] = o.rotation_degrees;
                    }
                }
                if let Some(o) = offsets.iter().find(|o| o.bone == self.config.gaze.bones[0]) {
                    from.gaze = [o.world_translation.x, o.world_translation.y];
                }
            }
        }
        self.translation = [
            Spring::new(self.config.follow_omega),
            Spring::new(self.config.follow_omega),
        ];
        self.event = Some(Event {
            time: self.time,
            kind: Kind::Pick([anchor.x, anchor.y]),
            from,
        });
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
        for (spring, target) in self.translation.iter_mut().zip(xy) {
            spring.target(self.time, target);
        }
    }
    pub fn release(&mut self) {
        if self.held() {
            let from = self.sample(self.time);
            self.event = Some(Event {
                time: self.time,
                kind: Kind::Release,
                from,
            });
        }
    }
    fn neutral(&self) -> Pose {
        Pose {
            rotations: vec![0.0; self.config.bones.len()],
            feet: vec![[0.0; 2]; self.config.feet.len()],
            face: [0.0; 2],
            gaze: [0.0; 2],
            phase: "none".into(),
        }
    }
    fn story(&self, t: f64, initial: f64, pick: impl Fn(&DragPose) -> f64) -> f64 {
        let mut start = 0.0;
        let mut value = initial;
        for k in &self.config.story.keyframes {
            let target = pick(&self.config.story.poses[&k.pose]);
            if t <= k.time {
                return lerp(value, target, smooth((t - start) / (k.time - start)));
            }
            start = k.time;
            value = target;
        }
        value
    }
    fn sample(&self, time: f64) -> Pose {
        let mut out = self.neutral();
        let Some(e) = &self.event else {
            return out;
        };
        let c = &self.config;
        let t = &c.timings;
        let dt = time - e.time;
        let from = &e.from;
        match e.kind {
            Kind::Pick(anchor) => {
                let w = smooth(dt / t.pick_in);
                out.phase = c
                    .story
                    .keyframes
                    .iter()
                    .find(|k| dt <= k.time)
                    .map_or("question", |k| k.phase.as_str())
                    .into();
                for (i, b) in c.bones.iter().enumerate() {
                    let velocity = self.translation[0].state((time - b.delay).max(e.time)).1;
                    let sway = b.sway_degrees
                        * (velocity / c.velocity_reference).tanh()
                        * smooth((dt - b.delay) / t.pick_in);
                    out.rotations[i] = (self
                        .story(dt - b.delay, from.rotations[i], |p| p.rotations[&b.bone])
                        + sway)
                        .clamp(-b.limit_degrees, b.limit_degrees);
                }
                for (i, b) in c.feet.iter().enumerate() {
                    let velocity = self.translation[0].state((time - b.delay).max(e.time)).1;
                    let sway = b.sway_x
                        * (velocity / c.velocity_reference).tanh()
                        * smooth((dt - b.delay) / t.pick_in);
                    for axis in 0..2 {
                        out.feet[i][axis] = (self
                            .story(dt - b.delay, from.feet[i][axis], |p| p.feet[&b.bone][axis])
                            + if axis == 0 { sway } else { 0.0 })
                        .clamp(-b.limit[axis], b.limit[axis]);
                    }
                }
                for i in 0..2 {
                    out.face[i] = self.story(dt, from.face[i], |p| p.face[i]);
                }
                let g = &c.gaze;
                let target = bounded_gaze(
                    std::array::from_fn(|i| {
                        let lag = (self.translation[i].state(time).1 / c.follow_omega)
                            .clamp(-g.lag_limit[i], g.lag_limit[i]);
                        (anchor[i] + lag - g.focus[i]) / g.radius[i] * g.limit[i]
                    }),
                    g.limit,
                );
                let weight = self.story(dt, 0.0, |p| p.gaze_weight);
                out.gaze = bounded_gaze(
                    std::array::from_fn(|i| from.gaze[i] * (1.0 - w) + target[i] * weight),
                    g.limit,
                );
            }
            Kind::Release => {
                let l = &c.landing_glance;
                let end = (t.land_smile_in + t.land_smile_hold + t.expression_out)
                    .max(l.hold_until + l.out)
                    .max(
                        t.land_recover + 0.4 * c.bones.iter().map(|b| b.delay).fold(0.0, f64::max),
                    );
                if dt >= end {
                    return out;
                }
                let w = smooth(dt / t.release_in);
                let glance =
                    smooth((dt - l.start) / l.rise) * (1.0 - smooth((dt - l.hold_until) / l.out));
                let smile = smooth(dt / t.land_smile_in);
                let fade =
                    1.0 - smooth((dt - t.land_smile_in - t.land_smile_hold) / t.expression_out);
                out.phase = if glance > 0.0 {
                    "landing-glance"
                } else {
                    "landing-rest"
                }
                .into();
                for (i, b) in c.bones.iter().enumerate() {
                    let recover = 1.0 - smooth((dt - b.delay * 0.4) / t.land_recover);
                    out.rotations[i] = (lerp(from.rotations[i], b.fall_degrees, w) * recover
                        + l.rotations.get(&b.bone).copied().unwrap_or(0.0) * glance)
                        .clamp(-b.limit_degrees, b.limit_degrees);
                }
                // Native placement has no virtual floor/ballistic fall. Keep
                // feet continuous at release and return them over landRecover.
                for (i, b) in c.feet.iter().enumerate() {
                    out.feet[i] = std::array::from_fn(|axis| {
                        lerp(from.feet[i][axis], b.fall_offset[axis], w)
                            * (1.0 - smooth(dt / t.land_recover))
                    });
                }
                out.face = [
                    lerp(from.face[0], 0.6, w) * (1.0 - smile) * fade,
                    lerp(from.face[1], 0.0, w) * (1.0 - smile) * fade + smile * fade,
                ];
                out.gaze = bounded_gaze(
                    std::array::from_fn(|i| from.gaze[i] * (1.0 - w) + c.gaze.landing[i] * glance),
                    c.gaze.limit,
                );
            }
        }
        out
    }
    pub fn project(&self, scene: &mut Scene) {
        let p = self.sample(self.time);
        if p.phase == "none" {
            return;
        }
        let phase = self.time % self.follow.cycle_duration;
        let blink = curve(&self.follow.blink_weight, phase);
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
            *overlay = self.config.face.overlay(p.face);
            if animation != &self.follow.animation {
                continue;
            }
            *time_ns = ((self.follow.base_time_offset + phase) * 1e9).round() as u64;
            let mut add = |bone: &str, rotation: f64, local: [f64; 2]| {
                if let Some(o) = offsets.iter_mut().find(|o| o.bone == bone) {
                    o.rotation_degrees = (o.rotation_degrees + rotation).clamp(-30.0, 30.0);
                    o.local_translation = Vec2::new(local[0], local[1]);
                } else {
                    offsets.push(BoneOffset {
                        bone: bone.into(),
                        rotation_degrees: rotation,
                        local_translation: Vec2::new(local[0], local[1]),
                        world_translation: Vec2::default(),
                    });
                }
            };
            for (b, r) in self.config.bones.iter().zip(&p.rotations) {
                add(&b.bone, *r, [0.0; 2]);
            }
            for (b, xy) in self.config.feet.iter().zip(&p.feet) {
                add(&b.bone, 0.0, *xy);
            }
            for bone in &self.config.gaze.bones {
                let index = if let Some(i) = offsets.iter().position(|o| &o.bone == bone) {
                    i
                } else {
                    offsets.push(BoneOffset {
                        bone: bone.clone(),
                        rotation_degrees: 0.0,
                        local_translation: Vec2::default(),
                        world_translation: Vec2::default(),
                    });
                    offsets.len() - 1
                };
                let o = &mut offsets[index];
                let xy = bounded_gaze(
                    [
                        o.world_translation.x + p.gaze[0] * blink,
                        o.world_translation.y + p.gaze[1] * blink,
                    ],
                    self.config.gaze.limit,
                );
                o.world_translation = Vec2::new(xy[0], xy[1]);
            }
        }
    }
    pub fn trace(&self) -> String {
        let p = self.sample(self.time);
        format!(
            "phase={} held={} left_y={:.3} right_y={:.3} face={:.3},{:.3}",
            p.phase,
            self.held(),
            p.feet.first().map_or(0.0, |p| p[1]),
            p.feet.get(1).map_or(0.0, |p| p[1]),
            p.face[0],
            p.face[1]
        )
    }
}

/// The desktop gesture has one interface; each package chooses its authored rig.
pub enum Drag {
    Story(Box<StoryDrag>),
    Pull(Box<crate::pull::Pull>),
}
impl Drag {
    pub fn new(c: SpineDrag, f: SpineFollow, camera: SpineCamera) -> Self {
        Self::Story(Box::new(StoryDrag::new(c, f, camera)))
    }
    pub fn new_pull(
        c: std::sync::Arc<mochi_package::SpinePull>,
        f: SpineFollow,
        camera: SpineCamera,
    ) -> Self {
        Self::Pull(Box::new(crate::pull::Pull::new(c, f, camera)))
    }
    pub fn active(&self) -> bool {
        match self {
            Self::Story(d) => d.active(),
            Self::Pull(d) => d.active(),
        }
    }
    pub fn clear(&mut self) {
        match self {
            Self::Story(d) => d.clear(),
            Self::Pull(d) => d.clear(),
        }
    }
    pub fn advance(&mut self, dt: f64) {
        match self {
            Self::Story(d) => d.advance(dt),
            Self::Pull(d) => d.advance(dt),
        }
    }
    pub fn begin(&mut self, s: &Scene, p: Vec2, r: Option<&Scene>) {
        match self {
            Self::Story(d) => d.begin(s, p, r),
            Self::Pull(d) => d.begin(s, p, r),
        }
    }
    pub fn move_to(&mut self, p: [i32; 2], size: u16, mirrored: bool) {
        match self {
            Self::Story(d) => d.move_to(p, size, mirrored),
            Self::Pull(d) => d.move_to(p, size, mirrored),
        }
    }
    pub fn release(&mut self) {
        match self {
            Self::Story(d) => d.release(),
            Self::Pull(d) => d.release(),
        }
    }
    pub fn project(&self, s: &mut Scene) {
        match self {
            Self::Story(d) => d.project(s),
            Self::Pull(d) => d.project(s),
        }
    }
    pub fn trace(&self) -> String {
        match self {
            Self::Story(d) => d.trace(),
            Self::Pull(d) => d.trace(),
        }
    }
}

#[cfg(all(test, feature = "spine"))]
mod tests {
    use super::*;
    use mochi_core::{SceneNode, Transform};
    use serde_json::Value;
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/drag-reference.json")).unwrap()
    }
    fn controller(v: &Value) -> StoryDrag {
        StoryDrag::new(
            serde_json::from_value(v["drag"].clone()).unwrap(),
            serde_json::from_value(v["follow"].clone()).unwrap(),
            serde_json::from_value(v["camera"].clone()).unwrap(),
        )
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
    fn event(d: &mut StoryDrag, e: &Value) {
        d.time = e["time"].as_f64().unwrap();
        match e["kind"].as_str().unwrap() {
            "pick" => {
                let a = &e["anchor"];
                let p = Vec2::new(
                    (a[0].as_f64().unwrap() - d.camera.x) / d.camera.width * 112.0,
                    (1.0 - (a[1].as_f64().unwrap() - d.camera.y) / d.camera.height) * 112.0,
                );
                d.begin(&scene(), p, None);
            }
            "release" => d.release(),
            "move" => d.move_to(
                [
                    e["delta"][0].as_i64().unwrap() as i32,
                    e["delta"][1].as_i64().unwrap() as i32,
                ],
                512,
                false,
            ),
            _ => panic!(),
        }
    }
    #[test]
    fn authored_controller_matches_reference_at_60_120_144_hz() {
        let f = fixture();
        for hz in [60, 120, 144] {
            let mut d = controller(&f);
            let mut next = 0;
            let events = f["events"].as_array().unwrap();
            for s in f["samples"].as_array().unwrap() {
                let time = s["time"].as_f64().unwrap();
                while next < events.len() && events[next]["time"].as_f64().unwrap() <= time {
                    event(&mut d, &events[next]);
                    next += 1;
                }
                while d.time + 1.0 / (hz as f64) < time {
                    d.advance(1.0 / (hz as f64));
                }
                d.advance(time - d.time);
                let p = d.sample(d.time);
                let near =
                    |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{hz}Hz t={time}: {a} != {b}");
                for (b, v) in d.config.bones.iter().zip(p.rotations) {
                    near(v, s["rotations"][&b.bone].as_f64().unwrap());
                }
                for (b, xy) in d.config.feet.iter().zip(p.feet) {
                    for (i, v) in xy.iter().enumerate() {
                        near(*v, s["feet"][&b.bone][i].as_f64().unwrap());
                    }
                }
                for i in 0..2 {
                    near(p.face[i], s["face"][i].as_f64().unwrap());
                    near(p.gaze[i], s["gaze"][i].as_f64().unwrap());
                }
            }
        }
    }
    #[test]
    fn release_regrab_blink_and_original_action_priority() {
        let f = fixture();
        let mut d = controller(&f);
        event(&mut d, &f["events"][0]);
        d.time = 2.28;
        let before = d.sample(d.time);
        d.release();
        let after = d.sample(d.time);
        assert_eq!(before.rotations, after.rotations);
        assert_eq!(before.feet, after.feet);
        assert_eq!(before.face, after.face);
        assert_eq!(before.gaze, after.gaze);
        d.advance(0.01);
        let before = d.sample(d.time);
        d.begin(&scene(), Vec2::new(55.0, 35.0), None);
        let after = d.sample(d.time);
        assert_eq!(before.rotations, after.rotations);
        assert_eq!(before.feet, after.feet);
        assert_eq!(before.gaze, after.gaze);
        d.time = 20.0;
        let p = d.sample(d.time);
        assert_eq!(p.phase, "question");
        assert_eq!(p.feet[0], [-2.0, 5.0]);
        d.time = 20.78;
        let mut s = scene();
        d.project(&mut s);
        if let SceneContent::Skeletal { offsets, .. } = &s.nodes[0].content {
            assert!(offsets
                .iter()
                .filter(|o| o.bone.contains("Eyeball"))
                .all(|o| o.world_translation == Vec2::default()));
        }
        if let SceneContent::Skeletal {
            animation,
            time_ns,
            offsets,
            overlay,
            ..
        } = &mut s.nodes[0].content
        {
            *animation = "Interact".into();
            *time_ns = 900000000;
            offsets.clear();
            *overlay = Default::default();
        }
        d.project(&mut s);
        if let SceneContent::Skeletal {
            time_ns,
            offsets,
            overlay,
            ..
        } = &s.nodes[0].content
        {
            assert_eq!(*time_ns, 900000000);
            assert!(offsets.is_empty());
            assert!(!overlay.bones.is_empty());
        }
        d.release();
        d.advance(2.0);
        let mut s = scene();
        let base = s.clone();
        d.project(&mut s);
        assert_eq!(s, base);
    }
    #[test]
    fn gaze_handoff_and_combined_gaze_are_bounded() {
        let f = fixture();
        let mut d = controller(&f);
        let mut s = scene();
        if let SceneContent::Skeletal { offsets, .. } = &mut s.nodes[0].content {
            offsets.push(BoneOffset {
                bone: "F_Face".into(),
                rotation_degrees: 5.0,
                local_translation: Vec2::default(),
                world_translation: Vec2::default(),
            });
        }
        d.begin(&scene(), Vec2::new(55.0, 35.0), Some(&s));
        d.project(&mut s);
        if let SceneContent::Skeletal { offsets, .. } = &s.nodes[0].content {
            assert_eq!(
                offsets
                    .iter()
                    .find(|o| o.bone == "F_Face")
                    .unwrap()
                    .rotation_degrees,
                10.0
            );
        }
        // Native prepare starts from a fresh scene: carry only once.
        let mut fresh = scene();
        d.project(&mut fresh);
        if let SceneContent::Skeletal { offsets, .. } = &fresh.nodes[0].content {
            assert_eq!(
                offsets
                    .iter()
                    .find(|o| o.bone == "F_Face")
                    .unwrap()
                    .rotation_degrees,
                5.0
            );
        }
        d.time = 4.0;
        d.release();
        for i in 0..120 {
            d.advance(1.0 / 120.0);
            let mut s = scene();
            if let SceneContent::Skeletal { offsets, .. } = &mut s.nodes[0].content {
                for bone in &d.config.gaze.bones {
                    offsets.push(BoneOffset {
                        bone: bone.clone(),
                        rotation_degrees: 0.0,
                        local_translation: Vec2::default(),
                        world_translation: Vec2::new(if i % 2 == 0 { 2.6 } else { -2.6 }, 0.0),
                    });
                }
            }
            d.project(&mut s);
            s.validate().unwrap();
            if let SceneContent::Skeletal { offsets, .. } = &s.nodes[0].content {
                for o in offsets.iter().filter(|o| o.bone.contains("Eyeball")) {
                    assert!(
                        (o.world_translation.x / 2.6).hypot(o.world_translation.y / 2.2)
                            <= 1.0 + 1e-10
                    );
                }
            }
        }
        d.clear();
        assert_eq!(d.sample(d.time).phase, "none");
    }
    #[test]
    #[ignore = "requires supplied Shanbi assets and local GPU; offscreen only"]
    fn supplied_shanbi_drag_render_and_reference() {
        use mochi_package::LoadedPackage;
        use mochi_preview::Viewport;
        use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
        use std::{fs, path::PathBuf};
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let out = root.join("target/drag-check");
        fs::create_dir_all(&out).unwrap();
        let package =
            LoadedPackage::load(root.join("characters/shanbi-drag/shanbi.mochi")).unwrap();
        let resource = package.spine("spine.toml").unwrap();
        let f = fixture();
        let mut d = controller(&f);
        let mut next = 0;
        let events = f["events"].as_array().unwrap();
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
            let time = item["time"].as_f64().unwrap();
            while next < events.len() && events[next]["time"].as_f64().unwrap() <= time {
                event(&mut d, &events[next]);
                next += 1;
            }
            d.time = time;
            let mut s = package.instantiate("drag-test").unwrap().scene().unwrap();
            d.project(&mut s);
            s.validate().unwrap();
            let SceneContent::Skeletal {
                animation,
                time_ns,
                offsets,
                overlay,
                ..
            } = &s.nodes[0].content
            else {
                panic!()
            };
            // Neutral drag returns to the regular gaze base after its recovery.
            let pose_time = if d.sample(time).phase == "none" {
                0.5 + time % 2.0
            } else {
                *time_ns as f64 / 1e9
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
                .pose_with_overlay(animation, pose_time, &offsets, overlay)
                .unwrap();
            let b = reference
                .pose_with_overlay(animation, pose_time, &offsets, overlay)
                .unwrap();
            let original = item["bounds"].as_array().unwrap();
            assert_eq!(a.len(), original.len(), "t={time}");
            assert_eq!(a.len(), b.len());
            let mut error = 0f32;
            let mut bridge_error = 0f32;
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
                        error =
                            error.max((av[i] - orig[attr * 2 + i].as_f64().unwrap() as f32).abs());
                        bridge_error = bridge_error.max((av[i] - bv[i]).abs());
                    }
                }
            }
            assert!(error < 0.003, "t={time} original source error {error}");
            assert!(bridge_error < 0.003, "t={time} C/JS error {bridge_error}");
            assert_eq!(
                evaluator.pose("Move", 0.3).unwrap(),
                base,
                "overlays must not leak into the next pose"
            );
            if let SceneContent::Skeletal { time_ns, .. } = &mut s.nodes[0].content {
                *time_ns = (pose_time * 1e9).round() as u64;
            }
            let s = viewport.project_scene(&s);
            let layers = [RenderLayer {
                package: &package,
                scene: &s,
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
            let mean = sum / (count * 4) as f64;
            assert!(mean < 0.5 && (big as f64 / count as f64) < 0.01);
            cpu.frame()
                .write_png(fs::File::create(out.join(format!("drag-{time:.2}.png"))).unwrap())
                .unwrap();
            gpu.frame()
                .write_png(fs::File::create(out.join(format!("drag-{time:.2}-gpu.png"))).unwrap())
                .unwrap();
            reports.push(serde_json::json!({"time":time,"original_source_bounds_error":error,"bridge_bounds_error":bridge_error,"cpu_gpu_mean_pma_error":mean}));
        }
        fs::write(
            out.join("results.json"),
            serde_json::to_vec_pretty(&reports).unwrap(),
        )
        .unwrap();
    }
}
