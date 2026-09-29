use super::*;
use serde_json::Value;
fn config() -> Arc<SpinePull> {
    Arc::new(
        serde_json::from_str(include_str!(
            "../../../../characters/angelina-drag/angelina_wish.mochi/spine/pull.json"
        ))
        .unwrap(),
    )
}
fn follow() -> SpineFollow {
    serde_json::from_str(include_str!(
        "../../../../characters/angelina-companion/follow-config.json"
    ))
    .unwrap()
}
fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/angelina-drag-reference.json"
    ))
    .unwrap()
}
fn camera() -> SpineCamera {
    SpineCamera {
        x: -395.0,
        y: -180.0,
        width: 790.0,
        height: 790.0,
    }
}
fn scene() -> Scene {
    Scene {
        nodes: vec![mochi_core::SceneNode {
            id: "body".into(),
            transform: Default::default(),
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
fn event(p: &mut Pull, e: &Value) {
    p.time = e["time"].as_f64().unwrap();
    match e["kind"].as_str().unwrap() {
        "pick" => {
            let a = &e["anchor"];
            let c = p.camera;
            p.begin(
                &scene(),
                Vec2::new(
                    (a[0].as_f64().unwrap() - c.x) * 112.0 / c.width,
                    (1.0 - (a[1].as_f64().unwrap() - c.y) / c.height) * 112.0,
                ),
                None,
            );
        }
        "move" => p.move_to(
            [
                e["delta"][0].as_i64().unwrap() as i32,
                e["delta"][1].as_i64().unwrap() as i32,
            ],
            512,
            false,
        ),
        "release" => p.release(),
        _ => panic!(),
    }
}
#[test]
fn supplied_pull_lift_and_sway_match_across_rates_and_regrab() {
    let f = fixture();
    let events = f["events"].as_array().unwrap();
    for hz in [60, 120, 144] {
        let mut p = Pull::new(config(), follow(), camera());
        let mut next = 0;
        for s in f["samples"].as_array().unwrap() {
            let t = s["time"].as_f64().unwrap();
            while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
                event(&mut p, &events[next]);
                next += 1;
            }
            while p.time + 1.0 / (hz as f64) < t {
                p.advance(1.0 / hz as f64);
            }
            p.advance(t - p.time);
            let close = |a: f64, b: &Value| {
                assert!(
                    (a - b.as_f64().unwrap()).abs() < 1e-8,
                    "{hz}Hz t={t}: {a} != {b}"
                )
            };
            let l = p.sample();
            let (strength, direction, source) = p.pull_pose();
            close(p.amount(), &s["pull"]["amount"]);
            close(strength, &s["pull"]["strength"]);
            close(direction, &s["pull"]["direction"]);
            close(source, &s["pull"]["sourceTime"]);
            close(l.amount, &s["lift"]["amount"]);
            close(l.surprise, &s["lift"]["surprise"]);
            close(l.smile, &s["lift"]["smile"]);
            for i in 0..2 {
                close(l.gaze[i], &s["lift"]["gaze"][i]);
            }
            for (b, v) in p.config.lift.bones.iter().zip(&l.rotations) {
                close(*v, &s["lift"]["rotations"][&b.bone]);
            }
            for (b, v) in p.config.reaction.bones.iter().zip(&l.reaction) {
                close(*v, &s["reaction"]["rotations"][&b.bone]);
            }
        }
        assert!(!p.active());
    }
}
#[test]
fn interaction_priority_and_applied_limits() {
    let mut p = Pull::new(config(), follow(), camera());
    p.begin(&scene(), Vec2::new(70.0, 40.0), None);
    p.advance(0.5);
    let mut s = scene();
    p.project(&mut s);
    s.validate().unwrap();
    let SceneContent::Skeletal {
        animation,
        offsets,
        overlay,
        ..
    } = &mut s.nodes[0].content
    else {
        panic!()
    };
    let applied = overlay.applied.as_mut().unwrap();
    applied.bones[0].translation.x = f64::NAN;
    assert!(overlay.validate().is_err());
    *animation = "Interact".into();
    offsets.clear();
    *overlay = Default::default();
    let original = s.clone();
    p.project(&mut s);
    assert_eq!(s, original);
    p.clear();
    assert!(!p.active());
}
#[test]
#[ignore = "requires supplied v0.3.0 data and local GPU; original-controller geometry and CPU/GPU comparison"]
fn supplied_angelina_pull_render_and_reference() {
    use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("target/angelina-drag-check");
    fs::create_dir_all(&out).unwrap();
    let package = mochi_package::LoadedPackage::load(
        root.join("characters/angelina-drag/angelina_wish.mochi"),
    )
    .unwrap();
    let resource = package.spine("spine.toml").unwrap();
    let mut eval = mochi_spine::Evaluator::new(&resource.model).unwrap();
    let mut reference = mochi_spine::ReferenceEvaluator::new(&resource.model).unwrap();
    let opts = RenderOptions {
        width: 512,
        height: 512,
        ..Default::default()
    };
    let mut cpu = CanvasRenderer::new(opts).unwrap();
    let mut gpu = CanvasRenderer::new(opts).unwrap();
    gpu.enable_spine_gpu().unwrap();
    let viewport = mochi_preview::Viewport::new([112, 112], [512, 512])
        .unwrap()
        .unwrap();
    let mut p = Pull::new(
        resource.pull.clone().unwrap(),
        resource.follow.clone().unwrap(),
        resource.camera,
    );
    let f = fixture();
    let events = f["events"].as_array().unwrap();
    let mut next = 0;
    let mut reports = vec![];
    for sample in f["geometry"].as_array().unwrap() {
        let t = sample["time"].as_f64().unwrap();
        while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
            event(&mut p, &events[next]);
            next += 1;
        }
        p.time = t;
        let mut s = scene();
        let mut gaze = crate::gaze::Gaze::new(follow(), camera());
        gaze.advance(t);
        gaze.project(&mut s, true);
        p.project(&mut s);
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
        let offsets: Vec<_> = offsets
            .iter()
            .map(|o| mochi_spine::BoneOffset {
                bone: &o.bone,
                rotation_degrees: o.rotation_degrees,
                local_translation: [o.local_translation.x, o.local_translation.y],
                world_translation: [o.world_translation.x, o.world_translation.y],
            })
            .collect();
        let a = eval
            .pose_with_overlay(animation, *time_ns as f64 / 1e9, &offsets, overlay)
            .unwrap();
        let b = reference
            .pose_with_overlay(animation, *time_ns as f64 / 1e9, &offsets, overlay)
            .unwrap();
        let source = sample["bounds"].as_array().unwrap();
        assert_eq!(a.len(), source.len(), "t={t}");
        assert_eq!(a.len(), b.len());
        let mut error = 0.0f64;
        let mut js_error = 0.0f64;
        for (i, (a, b)) in a.iter().zip(&b).enumerate() {
            for attr in 0..12 {
                let bounds = |m: &mochi_spine::Mesh| {
                    m.vertices
                        .iter()
                        .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], v| {
                            [lo.min(v[attr] as f64), hi.max(v[attr] as f64)]
                        })
                };
                let ab = bounds(a);
                let bb = bounds(b);
                for side in 0..2 {
                    let orig = source[i][attr * 2 + side].as_f64().unwrap();
                    error = error.max((ab[side] - orig).abs());
                    js_error = js_error.max((bb[side] - orig).abs());
                }
            }
        }
        assert!(
            error < 0.1 && js_error < 0.1,
            "t={t} C/source={error} JS/source={js_error}"
        );
        let projected = viewport.project_scene(&s);
        let layers = [RenderLayer {
            package: &package,
            scene: &projected,
        }];
        cpu.render_layers(&layers).unwrap();
        gpu.render_layers(&layers).unwrap();
        let mut sum = 0.0;
        let mut visible = 0;
        for (a, b) in cpu
            .frame()
            .rgba8()
            .chunks_exact(4)
            .zip(gpu.frame().rgba8().chunks_exact(4))
        {
            if a[3] == 0 && b[3] == 0 {
                continue;
            }
            visible += 1;
            sum += (a[3] as f64 - b[3] as f64).abs();
            for c in 0..3 {
                sum += (a[c] as f64 * a[3] as f64 - b[c] as f64 * b[3] as f64).abs() / 255.0;
            }
        }
        let mean = sum / (visible * 4) as f64;
        assert!(visible > 2000 && mean < 0.5, "t={t}: CPU/GPU={mean}");
        cpu.frame()
            .write_png(fs::File::create(out.join(format!("{t:.2}.png"))).unwrap())
            .unwrap();
        reports.push(serde_json::json!({"time":t,"c_source_bounds_error":error,"js_source_bounds_error":js_error,"cpu_gpu_mean_pma_error":mean}));
        // No applied pose or constraint state may leak into the next action.
        assert_eq!(
            eval.pose("Interact", 0.3).unwrap(),
            mochi_spine::Evaluator::new(&resource.model)
                .unwrap()
                .pose("Interact", 0.3)
                .unwrap()
        );
        assert_eq!(
            reference.pose("Interact", 0.3).unwrap(),
            mochi_spine::ReferenceEvaluator::new(&resource.model)
                .unwrap()
                .pose("Interact", 0.3)
                .unwrap()
        );
    }
    fs::write(
        out.join("results.json"),
        serde_json::to_string_pretty(&reports).unwrap(),
    )
    .unwrap();
}
