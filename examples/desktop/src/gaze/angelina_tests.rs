use super::*;
fn config() -> SpineFollow {
    serde_json::from_str(include_str!(
        "../../../../characters/angelina-companion/follow-config.json"
    ))
    .unwrap()
}
fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../tests/fixtures/angelina-follow-reference.json"
    ))
    .unwrap()
}
#[test]
fn angelina_matches_original_separate_eye_head_targets_and_release() {
    let f = fixture();
    let events = f["events"].as_array().unwrap();
    let mut next = 0;
    let mut gaze = Gaze::new(config(), super::tests::camera());
    for sample in f["timeline"].as_array().unwrap() {
        let t = sample["time"].as_f64().unwrap();
        while next < events.len() && events[next]["time"].as_f64().unwrap() <= t {
            let e = &events[next];
            next += 1;
            gaze.time = e["time"].as_f64().unwrap();
            gaze.target(
                e["point"]
                    .as_array()
                    .map(|p| Vec2::new(p[0].as_f64().unwrap(), p[1].as_f64().unwrap())),
            );
        }
        for i in 0..2 {
            assert!(
                (gaze.eyes[i].state(t).0 - sample["rawGaze"][i].as_f64().unwrap()).abs() < 1e-10
            );
        }
        assert!((gaze.head.state(t).0 - sample["head"].as_f64().unwrap()).abs() < 1e-10);
    }
    let mut gaze = Gaze::new(config(), super::tests::camera());
    gaze.target(Some(Vec2::new(81.0, 319.0)));
    gaze.time = 1.1;
    assert!(gaze.eyes[0].state(1.1).0 / 5.5 > -gaze.head.state(1.1).0 / 3.8);
}
#[test]
fn native_gaze_rejects_outer_vertices_and_invalid_gates() {
    let good = config().gaze_pose(vec![], [5.5, 3.3], 1.0, 1.0).unwrap();
    assert!(good.validate().is_ok());
    let mut bad = good.clone();
    bad.eyes[0].closure.open = 0.0;
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.eyes[0].pupil.as_mut().unwrap().vertices[0] = 0;
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.eyes[0].pupil.as_mut().unwrap().vertices = [6; 4];
    assert!(bad.validate().is_err());
    let mut bad = good.clone();
    bad.eyes[0].translation.x = 7.0;
    assert!(bad.validate().is_err());
    let mut bad = good;
    bad.eyes[0].pupil.as_mut().unwrap().area_ratio[0] = 0.0;
    assert!(bad.validate().is_err());
}
#[test]
#[ignore = "requires supplied Angelina data and local GPU; independent source and offscreen rendering"]
fn supplied_angelina_gaze_render_and_reference() {
    use mochi_preview::Viewport;
    use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = root.join("target/angelina-follow-check");
    fs::create_dir_all(&output).unwrap();
    let package = mochi_package::LoadedPackage::load(
        root.join("characters/angelina-companion/angelina_wish.mochi"),
    )
    .unwrap();
    let resource = package.spine("spine.toml").unwrap();
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
    let mut reports = vec![];
    let f = fixture();
    for sample in f["samples"].as_array().unwrap() {
        let label = sample["label"].as_str().unwrap();
        let time = sample["time"].as_f64().unwrap();
        let mut gaze = Gaze::new(resource.follow.clone().unwrap(), resource.camera);
        gaze.active_since = Some(0.0);
        gaze.target(Some(Vec2::new(
            sample["point"][0].as_f64().unwrap(),
            sample["point"][1].as_f64().unwrap(),
        )));
        gaze.advance(time);
        let mut scene = package
            .instantiate("angelina-check")
            .unwrap()
            .scene()
            .unwrap();
        gaze.project(&mut scene, true);
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
        assert!(offsets.is_empty());
        let a = evaluator
            .pose_with_overlay(animation, *time_ns as f64 / 1e9, &[], overlay)
            .unwrap();
        let b = reference
            .pose_with_overlay(animation, *time_ns as f64 / 1e9, &[], overlay)
            .unwrap();
        let original = sample["bounds"].as_array().unwrap();
        assert_eq!(a.len(), b.len(), "{label}");
        assert_eq!(a.len(), original.len(), "{label}");
        let mut error = 0.0f64;
        let mut compared_vertices = 0;
        let mut vertex_error = 0.0f64;
        let mut js_vertex_error = 0.0f64;
        for (i, (a, b)) in a.iter().zip(&b).enumerate() {
            let source = sample["vertices"][i].as_array().unwrap();
            // Clipping can triangulate equivalent polygons differently in C/JS.
            // Equal-length native meshes retain vertex order, including the pupil centers.
            if source.len() == a.vertices.len() * 12 {
                for (actual, expected) in a.vertices.iter().flatten().zip(source) {
                    vertex_error =
                        vertex_error.max((*actual as f64 - expected.as_f64().unwrap()).abs());
                }
                if source.len() == b.vertices.len() * 12 {
                    for (actual, expected) in b.vertices.iter().flatten().zip(source) {
                        js_vertex_error = js_vertex_error
                            .max((*actual as f64 - expected.as_f64().unwrap()).abs());
                    }
                }
                compared_vertices += a.vertices.len();
            }
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
                    error = error
                        .max((ab[side] - bb[side]).abs())
                        .max((ab[side] - original[i][attr * 2 + side].as_f64().unwrap()).abs());
                }
            }
        }
        assert!(compared_vertices > 200);
        // The C runtime uses float32 through clipping near a closing eyelid;
        // 0.02 model units is under 0.013 pixels in this 512/790 viewport.
        assert!(
            vertex_error < 0.02 && js_vertex_error < 1e-6,
            "{label}: C vertex error {vertex_error}, original/adapter JS error {js_vertex_error}"
        );
        assert!(error < 0.003, "{label}: source/C/JS bounds error {error}");
        let projected = viewport.project_scene(&scene);
        let layers = [RenderLayer {
            package: &package,
            scene: &projected,
        }];
        cpu.render_layers(&layers).unwrap();
        gpu.render_layers(&layers).unwrap();
        let mut sum = 0.0;
        let mut visible = 0;
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
            visible += 1;
            let mut diff = (a[3] as f64 - b[3] as f64).abs();
            for c in 0..3 {
                diff +=
                    (a[c] as f64 * a[3] as f64 / 255.0 - b[c] as f64 * b[3] as f64 / 255.0).abs();
            }
            sum += diff;
            if diff > 32.0 {
                big += 1;
            }
        }
        assert!(visible > 2000, "empty {label}");
        let mean = sum / (visible * 4) as f64;
        let fraction = big as f64 / visible as f64;
        assert!(
            mean < 0.5 && fraction < 0.01,
            "{label}: CPU/GPU {mean} / {fraction}"
        );
        cpu.frame()
            .write_png(fs::File::create(output.join(format!("{label}.png"))).unwrap())
            .unwrap();
        gpu.frame()
            .write_png(fs::File::create(output.join(format!("{label}-gpu.png"))).unwrap())
            .unwrap();
        reports.push(serde_json::json!({"pose":label,"source_c_js_bounds_error":error,"c_source_vertex_error":vertex_error,"js_source_vertex_error":js_vertex_error,"cpu_gpu_mean_pma_error":mean,"large_difference_fraction":fraction,"source_eye_gates":sample["pose"]["eyeGates"]}));
        assert_eq!(
            evaluator.pose("Interact", 0.3).unwrap(),
            mochi_spine::Evaluator::new(&resource.model)
                .unwrap()
                .pose("Interact", 0.3)
                .unwrap(),
            "gaze leaked into action"
        );
        assert_eq!(
            reference.pose("Interact", 0.3).unwrap(),
            mochi_spine::ReferenceEvaluator::new(&resource.model)
                .unwrap()
                .pose("Interact", 0.3)
                .unwrap()
        );
        if label == "blink" {
            let g = overlay.gaze.as_ref().unwrap();
            let mut closed = overlay.clone();
            for e in &mut closed.gaze.as_mut().unwrap().eyes {
                e.translation = Vec2::default();
                e.pupil.as_mut().unwrap().translation = Vec2::default();
            }
            assert!(g.eyes.iter().any(|e| e.translation.x > 5.0));
            assert_eq!(
                a,
                evaluator
                    .pose_with_overlay(animation, *time_ns as f64 / 1e9, &[], &closed)
                    .unwrap(),
                "closed eyelids must cancel eye movement"
            );
        }
    }
    // Declared topology must match the actual original attachment, even if numeric bounds are valid.
    let mut bad = mochi_core::PoseOverlay {
        gaze: config().gaze_pose(vec![], [1.0, 0.0], 1.0, 1.0),
        ..Default::default()
    };
    bad.gaze.as_mut().unwrap().eyes[0]
        .pupil
        .as_mut()
        .unwrap()
        .vertex_count = 11;
    assert!(evaluator
        .pose_with_overlay("Relax", 1.0, &[], &bad)
        .is_err());
    assert!(reference
        .pose_with_overlay("Relax", 1.0, &[], &bad)
        .is_err());
    fs::write(
        output.join("results.json"),
        serde_json::to_string_pretty(&reports).unwrap(),
    )
    .unwrap();
    println!(
        "Angelina source and GPU poses verified: {}",
        output.display()
    );
}
