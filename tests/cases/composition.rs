use super::package::Fixture;
use crate::*;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mochi_host::{Host, NativeMate, Presentation};
use mochi_package::LoadedPackage;
use mochi_renderer_software::*;
use std::{fs, sync::Arc};

fn colored(pixel: [u8; 4]) -> LoadedPackage {
    let fixture = Fixture::new();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&pixel, 1, 1, ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(fixture.0.join("sprites/neko.png"), png).unwrap();
    LoadedPackage::load(&fixture.0).unwrap()
}
fn scene() -> Scene {
    SpriteAppearance {
        asset: "sprites/neko.png".into(),
        size: Vec2::new(1.0, 1.0),
        animations: BTreeMap::new(),
    }
    .scene(&MateState::default())
}
fn options() -> RenderOptions {
    RenderOptions {
        width: 2,
        height: 1,
        ..Default::default()
    }
}
fn layer<'a>(package: &'a LoadedPackage, scene: &'a Scene) -> RenderLayer<'a> {
    RenderLayer { package, scene }
}

#[test]
fn same_node_id_and_asset_key_resolve_to_distinct_package_pixels() {
    let red = colored([255, 0, 0, 255]);
    let blue = colored([0, 0, 255, 255]);
    assert_ne!(red.content_hash(), blue.content_hash());
    let a = scene();
    let mut b = scene();
    b.nodes[0].transform.translation.x = 1.0;
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer
        .render_layers(&[layer(&red, &a), layer(&blue, &b)])
        .unwrap();
    assert_eq!(renderer.frame().rgba8(), [255, 0, 0, 255, 0, 0, 255, 255]);
}
#[test]
fn cross_package_alpha_order_matches_hand_calculated_values() {
    let red = colored([255, 0, 0, 128]);
    let blue = colored([0, 0, 255, 128]);
    let s = scene();
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer
        .render_layers(&[layer(&red, &s), layer(&blue, &s)])
        .unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some([85, 0, 170, 192]));
    renderer
        .render_layers(&[layer(&blue, &s), layer(&red, &s)])
        .unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some([170, 0, 85, 192]));
}
#[test]
fn changing_host_z_changes_occlusion_without_changing_simulation() {
    let red = Arc::new(colored([255, 0, 0, 255]));
    let blue = Arc::new(colored([0, 0, 255, 255]));
    let mut host = Host::default();
    let a = host.spawn("a", red, Presentation::default()).unwrap();
    host.spawn("b", blue, Presentation::default()).unwrap();
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    for (z, expected) in [(0, [0, 0, 255, 255]), (1, [255, 0, 0, 255])] {
        host.set_presentation(
            &a,
            Presentation {
                z_index: z,
                ..Default::default()
            },
        )
        .unwrap();
        let scenes = host.scenes().unwrap();
        let layers: Vec<_> = scenes.iter().map(|s| layer(s.package, &s.scene)).collect();
        renderer.render_layers(&layers).unwrap();
        assert_eq!(renderer.frame().pixel(0, 0), Some(expected));
        assert_eq!(
            host.get(&a).unwrap().runtime().state().position,
            Vec2::default()
        );
    }
}
struct TwoNodes;
impl Appearance for TwoNodes {
    fn scene(&self, _: &MateState) -> Scene {
        let mut s = scene();
        let mut second = s.nodes[0].clone();
        second.id = "second".into();
        s.nodes.push(second);
        s
    }
}
#[test]
fn host_opacity_is_per_node_modulation_without_isolated_group_rounding() {
    let white = Arc::new(colored([255; 4]));
    let mut host = Host::default();
    host.spawn_native(
        "a",
        white,
        NativeMate {
            state: MateState::default(),
            behaviors: vec![],
            appearance: Box::new(TwoNodes),
        },
        Presentation {
            opacity: 0.5,
            ..Default::default()
        },
    )
    .unwrap();
    let scenes = host.scenes().unwrap();
    let layers: Vec<_> = scenes.iter().map(|s| layer(s.package, &s.scene)).collect();
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer.render_layers(&layers).unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some([255, 255, 255, 192]));
}
#[test]
fn layers_use_one_node_budget_and_one_sampling_budget_including_hidden_nodes() {
    let pkg = colored([255; 4]);
    let visible = scene();
    let mut hidden = scene();
    hidden.nodes[0].opacity = 0.0;
    for (limits, last, message) in [
        (
            RenderLimits {
                max_nodes: 1,
                ..Default::default()
            },
            &hidden,
            "scene nodes",
        ),
        (
            RenderLimits {
                max_sampled_pixels: 1,
                ..Default::default()
            },
            &visible,
            "sampled pixels",
        ),
    ] {
        let mut renderer = CanvasRenderer::new(RenderOptions {
            limits,
            ..options()
        })
        .unwrap();
        renderer.render_layers(&[layer(&pkg, &visible)]).unwrap();
        let before = renderer.frame().rgba8().to_vec();
        assert_eq!(
            renderer.render_layers(&[layer(&pkg, &visible), layer(&pkg, last)]),
            Err(RenderError::LimitExceeded(message))
        );
        assert_eq!(renderer.frame().rgba8(), before);
    }
}
#[test]
fn empty_layers_are_bounded_and_zero_layers_clear_previous_frame() {
    let pkg = colored([255; 4]);
    let s = scene();
    let empty = Scene::default();
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer.render_layers(&[layer(&pkg, &s)]).unwrap();
    assert_eq!(
        renderer.render_layers(&vec![layer(&pkg, &empty); MAX_LAYERS + 1]),
        Err(RenderError::LimitExceeded("layers"))
    );
    assert_eq!(renderer.frame().pixel(0, 0), Some([255; 4]));
    renderer.render_layers(&[]).unwrap();
    assert_eq!(renderer.frame().rgba8(), [0; 8]);
}
#[test]
fn missing_image_in_later_layer_cannot_fall_back_to_another_package() {
    let fixture = Fixture::new();
    fs::copy(
        fixture.0.join("sprites/neko.png"),
        fixture.0.join("sprites/extra.png"),
    )
    .unwrap();
    let full = LoadedPackage::load(&fixture.0).unwrap();
    let other = colored([255, 0, 0, 255]);
    let mut s = scene();
    if let SceneContent::Sprite { asset, .. } = &mut s.nodes[0].content {
        *asset = "sprites/extra.png".into();
    }
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer.render_layers(&[layer(&full, &s)]).unwrap();
    assert!(matches!(
        renderer.render_layers(&[layer(&full, &s), layer(&other, &s)]),
        Err(RenderError::MissingImage { .. })
    ));
    assert_eq!(renderer.frame().pixel(0, 0), Some([255; 4]));
}
#[test]
fn arithmetic_error_after_drawing_earlier_layer_preserves_whole_previous_frame() {
    let red = colored([255, 0, 0, 255]);
    let blue = colored([0, 0, 255, 255]);
    let normal = scene();
    let mut tiny = scene();
    tiny.nodes[0].transform.scale.x = 1e-320;
    let mut renderer = CanvasRenderer::new(options()).unwrap();
    renderer.render_layers(&[layer(&red, &normal)]).unwrap();
    assert!(matches!(
        renderer.render_layers(&[layer(&blue, &normal), layer(&red, &tiny)]),
        Err(RenderError::InvalidTransform { .. })
    ));
    assert_eq!(renderer.frame().pixel(0, 0), Some([255, 0, 0, 255]));
}
#[test]
fn single_instance_api_and_layered_api_share_exact_pixel_and_png_semantics() {
    let pkg = colored([12, 34, 56, 190]);
    let mut a = scene();
    let mut b = scene();
    a.nodes[0].opacity = 0.7;
    b.nodes[0].opacity = 0.3;
    let mut flattened = a.clone();
    let mut second = b.nodes[0].clone();
    second.id = "second".into();
    flattened.nodes.push(second);
    let mut single = SoftwareRenderer::new("a", &pkg, options()).unwrap();
    single.render_frame("a", &flattened).unwrap();
    let mut multi = CanvasRenderer::new(options()).unwrap();
    multi
        .render_layers(&[layer(&pkg, &a), layer(&pkg, &b)])
        .unwrap();
    assert_eq!(single.frame().rgba8(), multi.frame().rgba8());
    let mut p = Vec::new();
    let mut q = Vec::new();
    single.frame().write_png(&mut p).unwrap();
    multi.frame().write_png(&mut q).unwrap();
    assert_eq!(p, q);
}
