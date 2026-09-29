#![cfg(feature = "spine")]
use mochi_core::{MateEvent, Namespace, SceneContent, Value};
use mochi_package::LoadedPackage;
use mochi_preview::Viewport;
use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};

#[test]
fn imported_sit_keeps_the_rig_and_plays_one_complete_native_cycle() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let package = LoadedPackage::load(root.join("characters/shanbi-sit/shanbi.mochi")).unwrap();
    let resource = package.spine("spine.toml").unwrap();
    assert_eq!(resource.idle_action.as_deref(), Some("idle_sit"));
    assert!(resource.follow.is_some() && resource.drag.is_some());
    assert!(resource.shake.is_some() && resource.jump.is_some());
    assert_eq!(resource.camera.width, 790.0);
    let sit = resource
        .model
        .metadata
        .animations
        .iter()
        .find(|a| a.name == "Sit")
        .unwrap();
    assert!((sit.duration - 8.0).abs() < 0.001);
    let mut mate = package.instantiate("sit-test").unwrap();
    let action = |name: &str| MateEvent::Custom {
        name: Namespace::new(format!("mochi.desktop.{name}")).unwrap(),
        payload: Value::Object(BTreeMap::new()),
    };
    mate.process(action("idle_sit")).unwrap();
    let out = root.join("target/shanbi-sit-render");
    fs::create_dir_all(&out).unwrap();
    let viewport = Viewport::new([112, 112], [512, 512]).unwrap().unwrap();
    let mut renderer = CanvasRenderer::new(RenderOptions {
        width: 512,
        height: 512,
        background: [0; 4],
        ..Default::default()
    })
    .unwrap();
    let mut previous = 0;
    for ms in [0, 4000, 5600, 7999] {
        mate.process(MateEvent::Tick {
            delta: Duration::from_millis(ms - previous),
        })
        .unwrap();
        previous = ms;
        assert_eq!(mate.state().current_animation, "idle_sit");
        let scene = viewport.project_scene(&mate.scene().unwrap());
        assert!(
            matches!(&scene.nodes[0].content, SceneContent::Skeletal {animation, ..} if animation == "Sit")
        );
        renderer
            .render_layers(&[RenderLayer {
                package: &package,
                scene: &scene,
            }])
            .unwrap();
        let frame = renderer.frame();
        assert!(frame.rgba8().chunks_exact(4).any(|p| p[3] > 240));
        assert_eq!(frame.pixel(0, 0), Some([0; 4]));
        frame
            .write_png(fs::File::create(out.join(format!("sit-{ms}.png"))).unwrap())
            .unwrap();
    }
    mate.process(MateEvent::Tick {
        delta: Duration::from_millis(2),
    })
    .unwrap();
    assert_eq!(mate.state().current_animation, "idle");
    mate.process(action("sit")).unwrap();
    mate.process(MateEvent::Tick {
        delta: Duration::from_secs(16),
    })
    .unwrap();
    assert_eq!(mate.state().current_animation, "sit");
    mate.process(action("idle_sit")).unwrap();
    mate.process(action("walk")).unwrap();
    assert_eq!(mate.state().current_animation, "walk");
}
