use super::package::Fixture;
use crate::*;
use mochi_package::LoadedPackage;
use mochi_platform::Renderer;
use mochi_renderer_software::{RenderOptions, SoftwareRenderer};
use std::{fs, time::Duration};

fn source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/animated-neko/neko.mochi")
}

#[test]
fn completion_transition_reserves_actions_without_changing_legacy_rule_budget() {
    let f = fixture();
    f.replace(
        "manifest.toml",
        "mochi.behavior.wander",
        "mochi.behavior.rules",
    );
    f.replace("manifest.toml", "behavior.toml", "rules.toml");
    fs::remove_file(f.0.join("behavior.toml")).unwrap();
    let animations = "schema_version=1\ndefault='idle'\n[clips.idle]\nmode='loop'\nframes=[{asset='sprites/frame-0.png',duration_ms=100}]\n[clips.wave]\nmode='once'\nframes=[{asset='sprites/frame-1.png',duration_ms=100}]\n";
    fs::write(f.0.join("animations.toml"), animations).unwrap();
    let rules = |moves: usize| {
        let mut s = String::from("schema_version=1\n[[rules]]\non={event='pointer_click',button=1}\nactions=[{type='play_animation',name='wave'}]\n");
        let commands = vec!["{type='move_by',x=1.0,y=0.0}"; moves];
        for chunk in commands.chunks(16) {
            s.push_str(&format!(
                "[[rules]]\non={{event='tick'}}\nactions=[{}]\n",
                chunk.join(",")
            ));
        }
        s
    };
    fs::write(f.0.join("rules.toml"), rules(126)).unwrap();
    LoadedPackage::load(&f.0).unwrap();
    fs::write(
        f.0.join("animations.toml"),
        animations.replace("mode='once'", "mode='once'\nnext='idle'"),
    )
    .unwrap();
    assert!(LoadedPackage::load(&f.0)
        .err()
        .unwrap()
        .to_string()
        .contains("action budget"));
    fs::write(f.0.join("rules.toml"), rules(125)).unwrap();
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("completion")
        .unwrap();
    mate.process(MateEvent::PointerClick {
        position: Vec2::default(),
        button: 1,
    })
    .unwrap();
    let output = mate
        .process(MateEvent::Tick {
            delta: Duration::from_millis(100),
        })
        .unwrap();
    assert_eq!(output.state.current_animation, "idle");
    assert_eq!(output.state.position.x, 125.0);
    assert_eq!(output.trace[0].actions.len(), 127);
}
fn fixture() -> Fixture {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("sprites/neko.png")).unwrap();
    for file in [
        "manifest.toml",
        "behavior.toml",
        "animations.toml",
        "sprites/frame-0.png",
        "sprites/frame-1.png",
        "sprites/frame-2.png",
    ] {
        fs::copy(source().join(file), fixture.0.join(file)).unwrap();
    }
    fixture
}
fn tick(mate: &mut Runtime) -> mochi_runtime::StepOutput {
    mate.process(MateEvent::Tick {
        delta: Duration::from_millis(100),
    })
    .unwrap()
}
fn asset(scene: &Scene) -> &str {
    match &scene.nodes[0].content {
        SceneContent::Sprite { asset, .. } => asset,
        _ => panic!("expected sprite"),
    }
}
#[test]
fn tick_wander_action_state_appearance_pixels_and_exact_replay() {
    let package = LoadedPackage::load(source()).unwrap();
    let mut a = package.instantiate("a").unwrap();
    let mut replay = package.instantiate("a").unwrap();
    let untouched = package.instantiate("b").unwrap();
    let mut renderer = SoftwareRenderer::new(
        "a",
        &package,
        RenderOptions {
            width: 64,
            height: 40,
            ..Default::default()
        },
    )
    .unwrap();
    let mut colors = vec![];
    for (index, expected) in [0, 1, 2, 0, 1, 2].into_iter().enumerate() {
        let step = tick(&mut a);
        assert_eq!(step, tick(&mut replay));
        assert!(matches!(step.trace[0].actions[0], MateAction::MoveTo(_)));
        assert_eq!(step.state.position.x, (index + 1) as f64);
        assert_eq!(
            step.scene.nodes[0].transform.translation,
            step.state.position
        );
        assert_eq!(asset(&step.scene), format!("sprites/frame-{expected}.png"));
        assert!(step.capability_requests.is_empty());
        renderer.render("a", &step.scene).unwrap();
        colors.push(renderer.frame().pixel(index as u32 + 5, 4).unwrap());
    }
    assert_ne!(colors[0], colors[1]);
    assert_ne!(colors[1], colors[2]);
    assert_eq!(colors[0], colors[3]);
    assert_eq!(untouched.state().position.x, 0.0);
    assert_eq!(untouched.elapsed(), Duration::ZERO);
    assert_eq!(untouched.state().current_animation, "idle");
}
#[test]
fn idle_animated_package_advances_without_movement_or_ai() {
    let f = fixture();
    f.replace(
        "manifest.toml",
        "mochi.behavior.wander",
        "mochi.behavior.idle",
    );
    f.replace("manifest.toml", "file = \"behavior.toml\"", "");
    fs::remove_file(f.0.join("behavior.toml")).unwrap();
    let p = LoadedPackage::load(&f.0).unwrap();
    let mut mate = p.instantiate("idle").unwrap();
    assert!(p.manifest().capabilities.is_empty());
    for _ in 0..3 {
        tick(&mut mate);
    }
    assert_eq!(asset(&mate.scene().unwrap()), "sprites/frame-1.png");
    assert_eq!(mate.state().position, Vec2::default());
}
#[test]
fn strict_animation_schema_and_duration_validation() {
    for (from, to) in [
        ("schema_version = 1", "schema_version = 2"),
        (
            "schema_version = 1",
            "schema_version = 1\nscript = 'run.sh'",
        ),
        ("default = \"idle\"", "default = \"missing\""),
        ("mode = \"loop\"", "mode = \"pingpong\""),
        ("duration_ms = 100", "duration_ms = 0"),
        ("duration_ms = 100", "duration_ms = -1"),
        ("duration_ms = 100", "duration_ms = 60001"),
        ("duration_ms = 100", "duration_ms = 1.5"),
        ("duration_ms = 100", "duration_ms = 9223372036854775807"),
        ("duration_ms = 100", "duration_ms = 100, script = 'run.sh'"),
        ("[clips.walk]", "[clips.' ' ]"),
    ] {
        let f = fixture();
        f.replace("animations.toml", from, to);
        assert!(LoadedPackage::load(&f.0).is_err(), "{to}");
    }
}
#[test]
fn animation_frame_and_clip_budgets_are_enforced() {
    let frame = "{asset='sprites/frame-0.png',duration_ms=1}";
    for (clips, frames) in [(0, 1), (1, 0), (65, 1), (1, 257), (5, 205)] {
        let f = fixture();
        let mut text = String::from("schema_version=1\ndefault='c0'\n");
        if clips == 0 {
            text.push_str("clips={}\n");
        }
        for n in 0..clips {
            text.push_str(&format!(
                "[clips.c{n}]\nmode='loop'\nframes=[{}]\n",
                vec![frame; frames].join(",")
            ));
        }
        assert!(text.len() < 64 * 1024);
        fs::write(f.0.join("animations.toml"), text).unwrap();
        assert!(
            LoadedPackage::load(&f.0).is_err(),
            "clips={clips} frames={frames}"
        );
    }
}
#[test]
fn missing_invalid_and_non_local_animation_assets_are_rejected() {
    for value in [
        "sprites/missing.png",
        "../outside.png",
        "/tmp/file.png",
        "https://example.com/a.png",
        "sounds/noise.wav",
    ] {
        let f = fixture();
        f.replace("animations.toml", "sprites/frame-1.png", value);
        assert!(LoadedPackage::load(&f.0).is_err(), "{value}");
    }
    let f = fixture();
    fs::write(f.0.join("sprites/frame-2.png"), b"invalid image").unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
}
#[test]
fn file_binding_unused_configuration_and_mixed_appearances_are_rejected() {
    for (from, to) in [
        ("animations.toml", "../animations.toml"),
        ("width = 32.0", "width = 0.0"),
        ("height = 32.0", "height = nan"),
        ("width = 32.0", "width = 32.0\nscript = 'bad'"),
    ] {
        let f = fixture();
        f.replace("manifest.toml", from, to);
        assert!(LoadedPackage::load(&f.0).is_err());
    }
    let f = fixture();
    fs::remove_file(f.0.join("animations.toml")).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    let f = Fixture::new();
    fs::copy(
        source().join("animations.toml"),
        f.0.join("animations.toml"),
    )
    .unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    let f = fixture();
    let mut manifest = fs::read_to_string(f.0.join("manifest.toml")).unwrap();
    manifest.push_str("\n[[components]]\nid='mochi.appearance.sprite'\n[components.config]\nasset='sprites/frame-0.png'\nwidth=32.0\nheight=32.0\n");
    fs::write(f.0.join("manifest.toml"), manifest).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
}
#[test]
fn animation_content_identity_and_snapshot_survive_source_deletion() {
    let f = fixture();
    let first = LoadedPackage::load(&f.0).unwrap();
    f.replace("animations.toml", "duration_ms = 100", "duration_ms = 200");
    let second = LoadedPackage::load(&f.0).unwrap();
    assert_ne!(first.content_hash(), second.content_hash());
    assert_eq!(first.assets().len(), 3);
    assert!(!first.assets().contains_key("animations.toml"));
    fs::remove_dir_all(&f.0).unwrap();
    let mut a = first.instantiate("original").unwrap();
    let mut b = second.instantiate("changed").unwrap();
    for _ in 0..2 {
        tick(&mut a);
        tick(&mut b);
    }
    assert_eq!(asset(&a.scene().unwrap()), "sprites/frame-1.png");
    assert_eq!(asset(&b.scene().unwrap()), "sprites/frame-0.png");
    first.image(asset(&a.scene().unwrap())).unwrap();
}
#[test]
fn animation_toml_size_limit_and_animated_image_codecs_remain_rejected() {
    let f = fixture();
    fs::write(f.0.join("animations.toml"), " ".repeat(65537)).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    let f = fixture();
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/animated.png"),
        f.0.join("sprites/frame-0.png"),
    )
    .unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
}
