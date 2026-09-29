use super::package::Fixture;
use crate::*;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mochi_package::LoadedPackage;
use mochi_platform::{pump_once, MockClock, MockPlatform, Renderer};
use mochi_renderer_software::{Frame, RenderError, RenderLimits, RenderOptions, SoftwareRenderer};
use std::{
    fs,
    io::{self, Write},
    time::Duration,
};

const R: [u8; 4] = [255, 0, 0, 255];
const G: [u8; 4] = [0, 255, 0, 255];
const B: [u8; 4] = [0, 0, 255, 255];
const W: [u8; 4] = [255; 4];
const Z: [u8; 4] = [0; 4];

fn asset(fixture: &Fixture, name: &str, width: u32, height: u32, pixels: &[[u8; 4]]) {
    let mut encoded = Vec::new();
    let bytes: Vec<_> = pixels.iter().flatten().copied().collect();
    PngEncoder::new(&mut encoded)
        .write_image(&bytes, width, height, ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(fixture.0.join(format!("sprites/{name}.png")), encoded).unwrap();
}
fn package() -> (Fixture, LoadedPackage) {
    let fixture = Fixture::new();
    asset(&fixture, "neko", 2, 2, &[R, G, B, W]);
    asset(&fixture, "red", 1, 1, &[R]);
    asset(&fixture, "green", 1, 1, &[[0, 255, 0, 128]]);
    asset(&fixture, "red-half", 1, 1, &[[255, 0, 0, 128]]);
    asset(&fixture, "blue-half", 1, 1, &[[0, 0, 255, 128]]);
    asset(&fixture, "transparent", 1, 1, &[[255, 255, 255, 0]]);
    let loaded = LoadedPackage::load(&fixture.0).unwrap();
    (fixture, loaded)
}
fn node(name: &str, key: &str, w: f64, h: f64) -> SceneNode {
    SceneNode {
        id: name.into(),
        transform: Transform::default(),
        opacity: 1.0,
        content: SceneContent::Sprite {
            asset: format!("sprites/{key}.png"),
            size: Vec2::new(w, h),
            animation: "idle".into(),
        },
    }
}
fn options(w: u32, h: u32) -> RenderOptions {
    RenderOptions {
        width: w,
        height: h,
        ..Default::default()
    }
}
fn pixels(frame: &Frame) -> Vec<[u8; 4]> {
    frame
        .rgba8()
        .chunks_exact(4)
        .map(|p| p.try_into().unwrap())
        .collect()
}
fn draw(package: &LoadedPackage, width: u32, height: u32, nodes: Vec<SceneNode>) -> Vec<[u8; 4]> {
    let mut renderer = SoftwareRenderer::new("test", package, options(width, height)).unwrap();
    renderer.render_frame("test", &Scene { nodes }).unwrap();
    pixels(renderer.frame())
}

fn coverage(
    package: &LoadedPackage,
    width: u32,
    height: u32,
    nodes: Vec<SceneNode>,
) -> Vec<[u8; 4]> {
    let mut renderer = SoftwareRenderer::new(
        "test",
        package,
        RenderOptions {
            sampling: mochi_renderer_software::SamplingMode::Coverage,
            ..options(width, height)
        },
    )
    .unwrap();
    renderer.render_frame("test", &Scene { nodes }).unwrap();
    pixels(renderer.frame())
}

#[test]
fn native_projection_preserves_source_detail_and_logical_coordinates() {
    use mochi_preview::Viewport;
    use mochi_renderer_software::{CanvasRenderer, RenderLayer, SamplingMode};
    let (_fixture, pkg) = package();
    let mut n = node("detail", "neko", 0.5, 0.5);
    n.transform.translation = Vec2::new(0.25, 0.25);
    let logical = Scene { nodes: vec![n] };
    let original = logical.clone();
    let viewport = Viewport::new([1, 1], [4, 4]).unwrap().unwrap();
    let projected = viewport.project_scene(&logical);
    let mut native = CanvasRenderer::with_scaled_cache(
        RenderOptions {
            sampling: SamplingMode::Coverage,
            ..options(4, 4)
        },
        1024,
    )
    .unwrap();
    native
        .render_layers(&[RenderLayer {
            package: &pkg,
            scene: &projected,
        }])
        .unwrap();
    // All four source texels survive, though the logical sprite is subpixel.
    let px = pixels(native.frame());
    assert_eq!([px[5], px[6], px[9], px[10]], [R, G, B, W]);
    assert_eq!(logical, original);
    assert_eq!(
        viewport
            .to_canvas(projected.nodes[0].transform.translation)
            .unwrap(),
        original.nodes[0].transform.translation
    );
    let portrait = Viewport::new([1, 1], [4, 8]).unwrap().unwrap();
    assert_eq!(
        portrait.project_scene(&logical).nodes[0]
            .transform
            .translation,
        Vec2::new(1.0, 3.0)
    );
}

#[test]
fn cached_draws_preserve_mirrors_opacity_clipping_and_package_identity() {
    use mochi_renderer_software::{CanvasRenderer, RenderLayer, SamplingMode};
    let (fixture, pkg) = package();
    let mut cached = CanvasRenderer::with_scaled_cache(
        RenderOptions {
            sampling: SamplingMode::Coverage,
            ..options(9, 8)
        },
        1024,
    )
    .unwrap();
    let mut reference = CanvasRenderer::new(RenderOptions {
        sampling: SamplingMode::Coverage,
        ..options(9, 8)
    })
    .unwrap();
    for x in [-5.25, 0.0, 0.33, 3.75, 9.0] {
        for scale in [1.0, -1.0] {
            for opacity in [0.0, 0.55, 1.0] {
                let mut n = node("a", "neko", 4.0, 3.0);
                n.transform.translation = Vec2::new(x, 0.25);
                n.transform.scale.x = scale;
                n.opacity = opacity;
                let scene = Scene { nodes: vec![n] };
                let layers = [RenderLayer {
                    package: &pkg,
                    scene: &scene,
                }];
                cached.render_layers(&layers).unwrap();
                reference.render_layers(&layers).unwrap();
                assert_eq!(cached.frame().rgba8(), reference.frame().rgba8());
            }
        }
    }
    // A familiar asset name with different source content must never hit stale pixels.
    asset(&fixture, "neko", 2, 2, &[B, B, B, B]);
    let other = LoadedPackage::load(&fixture.0).unwrap();
    let scene = Scene {
        nodes: vec![node("a", "neko", 1.0, 1.0)],
    };
    cached
        .render_layers(&[RenderLayer {
            package: &pkg,
            scene: &scene,
        }])
        .unwrap();
    let previous = cached.frame().rgba8().to_vec();
    cached
        .render_layers(&[RenderLayer {
            package: &other,
            scene: &scene,
        }])
        .unwrap();
    assert_ne!(cached.frame().rgba8(), previous);
    assert_eq!(&cached.frame().rgba8()[..4], B);
    let before_error = cached.frame().rgba8().to_vec();
    let invalid = Scene {
        nodes: vec![node("missing", "absent", 1.0, 1.0)],
    };
    assert!(cached
        .render_layers(&[RenderLayer {
            package: &other,
            scene: &invalid
        }])
        .is_err());
    assert_eq!(cached.frame().rgba8(), before_error);
}

#[test]
fn coverage_preserves_fractional_edges_and_small_sprites() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "red", 2.0, 1.0);
    n.transform.translation = Vec2::new(0.25, 0.25);
    assert_eq!(
        coverage(&pkg, 3, 2, vec![n]),
        [
            [255, 0, 0, 143],
            [255, 0, 0, 191],
            [255, 0, 0, 48],
            [255, 0, 0, 48],
            [255, 0, 0, 64],
            [255, 0, 0, 16],
        ]
    );
    let mut small = node("a", "red", 0.5, 0.5);
    small.transform.translation = Vec2::new(0.25, 0.25);
    assert_eq!(
        coverage(&pkg, 2, 1, vec![small.clone()]),
        [[255, 0, 0, 64], Z]
    );
    small.transform.scale.x = -1.0;
    small.transform.translation.x = 0.75;
    assert_eq!(coverage(&pkg, 2, 1, vec![small]), [[255, 0, 0, 64], Z]);
}

#[test]
fn coverage_matches_integral_pixel_art_and_filters_transparent_colors_safely() {
    let (fixture, pkg) = package();
    for size in [2.0, 4.0, 6.0] {
        for scale in [1.0, -1.0] {
            for opacity in [0.0, 0.55, 1.0] {
                let mut n = node("a", "neko", size, size);
                n.transform.translation = Vec2::new(7.0, 2.0);
                n.transform.scale.x = scale;
                n.opacity = opacity;
                assert_eq!(
                    coverage(&pkg, 15, 10, vec![n.clone()]),
                    draw(&pkg, 15, 10, vec![n])
                );
            }
        }
    }
    asset(&fixture, "neko", 2, 1, &[R, [0, 255, 0, 0]]);
    let pkg = LoadedPackage::load(&fixture.0).unwrap();
    let mut n = node("a", "neko", 4.0, 1.0);
    n.transform.translation.x = 0.25;
    let output = coverage(&pkg, 6, 1, vec![n]);
    assert_eq!(output[2], [255, 0, 0, 64]); // no green fringe from invisible RGB
    assert_eq!(output[3], Z);
}

#[test]
fn coverage_fallback_handles_rotation_and_minification() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation.x = 2.0;
    n.transform.rotation_radians = std::f64::consts::FRAC_PI_2;
    assert_eq!(coverage(&pkg, 2, 2, vec![n]), [B, R, W, G]);
    // Every source texel covers one quarter of this destination pixel.
    assert_eq!(
        coverage(&pkg, 1, 1, vec![node("a", "neko", 1.0, 1.0)]),
        [[128, 128, 128, 255]]
    );
}

#[test]
fn coverage_budgets_filter_work_and_preserves_the_frame_on_failure() {
    use mochi_renderer_software::SamplingMode;
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            sampling: SamplingMode::Coverage,
            limits: RenderLimits {
                max_sampled_pixels: 4,
                ..Default::default()
            },
            ..options(4, 4)
        },
    )
    .unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    let before = renderer.frame().rgba8().to_vec();
    let mut rotated = node("a", "red", 1.0, 1.0);
    rotated.transform.rotation_radians = std::f64::consts::FRAC_PI_4;
    rotated.transform.translation.x = 2.0;
    assert_eq!(
        renderer.render_frame(
            "test",
            &Scene {
                nodes: vec![rotated]
            }
        ),
        Err(RenderError::LimitExceeded("sampled pixels"))
    );
    assert_eq!(renderer.frame().rgba8(), before);

    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            sampling: SamplingMode::Coverage,
            ..options(4, 4)
        },
    )
    .unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    let before = renderer.frame().rgba8().to_vec();
    let mut invalid = node("bad", "red", 1.0, 1.0);
    invalid.transform.scale.x = 1e-320;
    invalid.transform.rotation_radians = 0.1;
    assert!(matches!(
        renderer.render_frame(
            "test",
            &Scene {
                nodes: vec![node("first", "neko", 2.0, 2.0), invalid]
            }
        ),
        Err(RenderError::InvalidTransform { .. })
    ));
    assert_eq!(renderer.frame().rgba8(), before);
}

#[test]
fn coverage_retains_visible_area_at_large_valid_scales() {
    let (_fixture, pkg) = package();
    for key in ["red", "neko"] {
        let mut n = node("a", key, 1e300, 2.0);
        n.transform.translation.x = -5e299;
        assert_eq!(
            coverage(&pkg, 4, 2, vec![n.clone()]),
            draw(&pkg, 4, 2, vec![n])
        );
    }
}

#[test]
fn coverage_agrees_with_independent_texel_rectangle_integration() {
    let fixture = Fixture::new();
    let colors = [
        R,
        [0, 255, 0, 128],
        B,
        [255, 255, 0, 0],
        W,
        [19, 83, 127, 64],
    ];
    asset(&fixture, "neko", 3, 2, &colors);
    let pkg = LoadedPackage::load(&fixture.0).unwrap();
    for size in [(3.0, 2.0), (6.0, 4.0), (7.25, 3.5)] {
        for position in [(0.25, 0.75), (1.5, 2.125), (-1.25, 0.875), (7.75, 0.125)] {
            for flip in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let mut n = node("a", "neko", size.0, size.1);
                n.transform.translation = Vec2::new(position.0, position.1);
                n.transform.scale = Vec2::new(flip.0, flip.1);
                let actual = coverage(&pkg, 10, 8, vec![n]);
                for y in 0..8 {
                    for x in 0..10 {
                        let mut alpha = 0.0_f64;
                        let mut rgb = [0.0_f64; 3];
                        for sy in 0..2 {
                            for sx in 0..3 {
                                let x0 = position.0 + sx as f64 * size.0 * flip.0 / 3.0;
                                let x1 = position.0 + (sx + 1) as f64 * size.0 * flip.0 / 3.0;
                                let y0 = position.1 + sy as f64 * size.1 * flip.1 / 2.0;
                                let y1 = position.1 + (sy + 1) as f64 * size.1 * flip.1 / 2.0;
                                let area = ((x as f64 + 1.0).min(x0.max(x1))
                                    - (x as f64).max(x0.min(x1)))
                                .max(0.0)
                                    * ((y as f64 + 1.0).min(y0.max(y1))
                                        - (y as f64).max(y0.min(y1)))
                                    .max(0.0);
                                let source = colors[sy * 3 + sx];
                                alpha += area * f64::from(source[3]);
                                for c in 0..3 {
                                    rgb[c] += area * f64::from(source[3]) * f64::from(source[c]);
                                }
                            }
                        }
                        let mut expected = [0_u8; 4];
                        if alpha.round() > 0.0 {
                            expected[3] = alpha.round() as u8;
                            for c in 0..3 {
                                expected[c] = (rgb[c] / alpha).round() as u8;
                            }
                        }
                        for (a, e) in actual[y * 10 + x].iter().zip(expected) {
                            assert!(a.abs_diff(e) <= 1, "size={size:?} position={position:?} flip={flip:?} pixel={x},{y}: {:?} vs {expected:?}", actual[y * 10 + x]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn identity_matches_hand_authored_rgba_and_background() {
    let (_fixture, pkg) = package();
    assert_eq!(
        draw(&pkg, 3, 3, vec![node("a", "neko", 2.0, 2.0)]),
        [R, G, Z, B, W, Z, Z, Z, Z]
    );
}

#[test]
fn logical_size_resizes_with_nearest_neighbor() {
    let (_fixture, pkg) = package();
    assert_eq!(
        draw(&pkg, 4, 2, vec![node("a", "neko", 4.0, 2.0)]),
        [R, R, G, G, B, B, W, W]
    );
    assert_eq!(draw(&pkg, 1, 1, vec![node("a", "neko", 1.0, 1.0)]), [W]);
}

#[test]
fn translation_and_nonuniform_scale_place_exact_pixels() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation = Vec2::new(1.0, 1.0);
    n.transform.scale = Vec2::new(2.0, 1.0);
    assert_eq!(
        draw(&pkg, 5, 3, vec![n]),
        [Z, Z, Z, Z, Z, Z, R, R, G, G, Z, B, B, W, W]
    );
}

#[test]
fn negative_x_scale_mirrors_about_local_origin() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation.x = 2.0;
    n.transform.scale.x = -1.0;
    assert_eq!(draw(&pkg, 2, 2, vec![n]), [G, R, W, B]);
}

#[test]
fn negative_y_scale_mirrors_about_local_origin() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation.y = 2.0;
    n.transform.scale.y = -1.0;
    assert_eq!(draw(&pkg, 2, 2, vec![n]), [B, W, R, G]);
}

#[test]
fn positive_quarter_turn_is_clockwise_in_y_down_coordinates() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation.x = 2.0;
    n.transform.rotation_radians = std::f64::consts::FRAC_PI_2;
    assert_eq!(draw(&pkg, 2, 2, vec![n]), [B, R, W, G]);
}

#[test]
fn scale_happens_before_rotation_and_translation() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.scale.x = 2.0;
    n.transform.rotation_radians = std::f64::consts::FRAC_PI_2;
    n.transform.translation.x = 2.0;
    assert_eq!(draw(&pkg, 2, 4, vec![n]), [B, R, B, R, W, G, W, G]);
}

#[test]
fn arbitrary_rotation_uses_shape_coverage_not_bounding_box_fill() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "red", 2.0, 2.0);
    n.transform.translation = Vec2::new(2.25, 0.1);
    n.transform.rotation_radians = std::f64::consts::FRAC_PI_4;
    assert_eq!(
        draw(&pkg, 4, 3, vec![n]),
        [Z, Z, R, Z, Z, R, R, R, Z, Z, R, Z]
    );
}

#[test]
fn fractional_translation_samples_centers_with_half_open_edges() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "red", 1.0, 1.0);
    n.transform.translation.x = 0.5;
    assert_eq!(draw(&pkg, 3, 1, vec![n.clone()]), [R, Z, Z]);
    n.transform.translation.x = 0.5001;
    assert_eq!(draw(&pkg, 3, 1, vec![n]), [Z, R, Z]);
}

#[test]
fn clipping_preserves_texture_coordinates() {
    let (_fixture, pkg) = package();
    let mut n = node("a", "neko", 2.0, 2.0);
    n.transform.translation = Vec2::new(-1.0, -1.0);
    assert_eq!(draw(&pkg, 2, 2, vec![n]), [W, Z, Z, Z]);
}

#[test]
fn huge_offscreen_extents_are_clipped_before_work_accounting() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            limits: RenderLimits {
                max_sampled_pixels: 4,
                ..Default::default()
            },
            ..options(2, 2)
        },
    )
    .unwrap();
    let mut n = node("a", "red", 1e12, 1e12);
    n.transform.translation = Vec2::new(-1.0, -1.0);
    renderer
        .render_frame("test", &Scene { nodes: vec![n] })
        .unwrap();
    assert_eq!(pixels(renderer.frame()), [R; 4]);
}

#[test]
fn zero_scale_opacity_and_offscreen_geometry_produce_no_pixels() {
    let (_fixture, pkg) = package();
    for (scale, opacity, translation) in [
        (Vec2::new(0.0, 1.0), 1.0, 0.0),
        (Vec2::new(1.0, 0.0), 1.0, 0.0),
        (Vec2::new(1.0, 1.0), 0.0, 0.0),
        (Vec2::new(1.0, 1.0), 1.0, 100.0),
    ] {
        let mut n = node("a", "red", 1.0, 1.0);
        n.transform.scale = scale;
        n.transform.translation.x = translation;
        n.opacity = opacity;
        let mut renderer = SoftwareRenderer::new(
            "test",
            &pkg,
            RenderOptions {
                limits: RenderLimits {
                    max_sampled_pixels: 0,
                    ..Default::default()
                },
                ..options(2, 2)
            },
        )
        .unwrap();
        renderer
            .render_frame("test", &Scene { nodes: vec![n] })
            .unwrap();
        assert_eq!(pixels(renderer.frame()), [Z; 4]);
    }
}

#[test]
fn source_alpha_multiplies_node_opacity() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            background: B,
            ..options(1, 1)
        },
    )
    .unwrap();
    let mut n = node("a", "green", 1.0, 1.0);
    n.opacity = 0.5;
    renderer
        .render_frame("test", &Scene { nodes: vec![n] })
        .unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some([0, 64, 191, 255]));
}

#[test]
fn source_over_partial_alpha_and_painter_order_match_reference_values() {
    let (_fixture, pkg) = package();
    let red = node("a", "red-half", 1.0, 1.0);
    let blue = node("b", "blue-half", 1.0, 1.0);
    assert_eq!(draw(&pkg, 1, 1, vec![red.clone()]), [[255, 0, 0, 128]]);
    assert_eq!(
        draw(&pkg, 1, 1, vec![red.clone(), blue.clone()]),
        [[85, 0, 170, 192]]
    );
    assert_eq!(draw(&pkg, 1, 1, vec![blue, red]), [[170, 0, 85, 192]]);
}

#[test]
fn transparent_pixels_preserve_destination_and_opaque_pixels_replace_it() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            background: [12, 34, 56, 78],
            ..options(1, 1)
        },
    )
    .unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "transparent", 1.0, 1.0)],
            },
        )
        .unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some([12, 34, 56, 78]));
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    assert_eq!(renderer.frame().pixel(0, 0), Some(R));
}

#[test]
fn each_frame_clears_previous_pixels_and_empty_scene_clears_all() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(2, 1)).unwrap();
    let mut n = node("a", "red", 1.0, 1.0);
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![n.clone()],
            },
        )
        .unwrap();
    n.transform.translation.x = 1.0;
    renderer
        .render_frame("test", &Scene { nodes: vec![n] })
        .unwrap();
    assert_eq!(pixels(renderer.frame()), [Z, R]);
    renderer.render_frame("test", &Scene::default()).unwrap();
    assert_eq!(pixels(renderer.frame()), [Z, Z]);
    assert_eq!(renderer.frame().pixel(2, 0), None);
    assert_eq!(renderer.frame().pixel(0, u32::MAX), None);
}

#[test]
fn png_round_trip_preserves_dimensions_and_pixels() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(2, 2)).unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "neko", 2.0, 2.0)],
            },
        )
        .unwrap();
    let mut png = Vec::new();
    renderer.frame().write_png(&mut png).unwrap();
    let decoded = image::load_from_memory(&png).unwrap().into_rgba8();
    assert_eq!(decoded.dimensions(), (2, 2));
    assert_eq!(decoded.as_raw(), renderer.frame().rgba8());
}

#[test]
fn png_writer_failure_is_reported_without_mutating_frame() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture writer failure"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (_fixture, pkg) = package();
    let renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    assert!(matches!(
        renderer.frame().write_png(Broken),
        Err(RenderError::PngEncoding(_))
    ));
    assert_eq!(renderer.frame().pixel(0, 0), Some(Z));
}

#[test]
fn unsupported_nodes_fail_even_when_invisible_and_preserve_last_frame() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("old", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    for content in [
        SceneContent::Text {
            text: "hello".into(),
            font_size: 12.0,
        },
        SceneContent::Extension {
            kind: ns("future.shape"),
            data: Value::Bool(true),
        },
    ] {
        let mut n = node("bad", "neko", 1.0, 1.0);
        n.content = content;
        n.opacity = 0.0;
        assert!(matches!(
            renderer.render_frame(
                "test",
                &Scene {
                    nodes: vec![node("a", "neko", 1.0, 1.0), n]
                }
            ),
            Err(RenderError::UnsupportedContent { .. })
        ));
        assert_eq!(renderer.frame().pixel(0, 0), Some(R));
    }
}

#[test]
fn missing_assets_fail_before_visibility_culling() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    for scale in [0.0, 1.0] {
        let mut n = node("bad", "missing", 1.0, 1.0);
        n.opacity = 0.0;
        n.transform.scale.x = scale;
        n.transform.translation.x = 1e6;
        assert!(matches!(
            renderer.render_frame("test", &Scene { nodes: vec![n] }),
            Err(RenderError::MissingImage { .. })
        ));
    }
}

#[test]
fn invalid_scene_rejects_without_publishing_partial_pixels() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("old", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    let mut bad_opacity = node("bad", "neko", 1.0, 1.0);
    bad_opacity.opacity = 1.1;
    let mut bad_number = node("bad", "neko", 1.0, 1.0);
    bad_number.transform.translation.x = f64::NAN;
    for nodes in [
        vec![bad_opacity],
        vec![bad_number],
        vec![node("a", "red", 1.0, 1.0), node("a", "neko", 1.0, 1.0)],
    ] {
        assert!(matches!(
            renderer.render_frame("test", &Scene { nodes }),
            Err(RenderError::InvalidScene(_))
        ));
        assert_eq!(renderer.frame().pixel(0, 0), Some(R));
    }
}

#[test]
fn unrepresentable_transform_products_and_corners_fail() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    for (size, scale, translation) in [
        (2.0, f64::MAX, 0.0),
        (f64::MIN_POSITIVE, 1e-300, 0.0),
        (1.0, f64::MAX, f64::MAX),
    ] {
        let mut n = node("bad", "red", size, 1.0);
        n.transform.scale.x = scale;
        n.transform.translation.x = translation;
        assert!(matches!(
            renderer.render_frame("test", &Scene { nodes: vec![n] }),
            Err(RenderError::InvalidTransform { .. })
        ));
        assert_eq!(renderer.frame().pixel(0, 0), Some(Z));
    }
}

#[test]
fn canvas_limits_and_overflow_are_rejected_before_allocation() {
    let (_fixture, pkg) = package();
    for options in [
        options(0, 1),
        options(1, 0),
        options(4097, 1),
        RenderOptions {
            limits: RenderLimits {
                max_frame_bytes: 15,
                ..Default::default()
            },
            ..options(2, 2)
        },
        RenderOptions {
            limits: RenderLimits {
                max_width: u32::MAX,
                max_height: u32::MAX,
                max_frame_bytes: u64::MAX,
                ..Default::default()
            },
            ..options(u32::MAX, u32::MAX)
        },
    ] {
        assert!(SoftwareRenderer::new("test", &pkg, options).is_err());
    }
    let renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            limits: RenderLimits {
                max_frame_bytes: 16,
                ..Default::default()
            },
            ..options(2, 2)
        },
    )
    .unwrap();
    assert_eq!(renderer.frame().rgba8().len(), 16);
    assert!(SoftwareRenderer::new(" ", &pkg, options(1, 1)).is_err());
}

#[test]
fn arithmetic_failure_during_drawing_discards_working_frame() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(1, 1)).unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("old", "red", 1.0, 1.0)],
            },
        )
        .unwrap();
    let mut tiny = node("tiny", "red", 1.0, 1.0);
    tiny.transform.scale.x = 1e-320; // finite corners, but pixel-center inverse overflows
    assert!(matches!(
        renderer.render_frame(
            "test",
            &Scene {
                nodes: vec![node("first", "neko", 1.0, 1.0), tiny]
            }
        ),
        Err(RenderError::InvalidTransform { .. })
    ));
    assert_eq!(renderer.frame().pixel(0, 0), Some(R));
}

#[test]
fn work_budget_accumulates_across_layers_and_failure_preserves_frame() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            limits: RenderLimits {
                max_sampled_pixels: 7,
                ..Default::default()
            },
            ..options(2, 2)
        },
    )
    .unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "red", 2.0, 2.0)],
            },
        )
        .unwrap();
    assert!(matches!(
        renderer.render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "neko", 2.0, 2.0), node("b", "neko", 2.0, 2.0)]
            }
        ),
        Err(RenderError::LimitExceeded("sampled pixels"))
    ));
    assert_eq!(pixels(renderer.frame()), [R; 4]);
}

#[test]
fn node_budget_includes_invisible_nodes() {
    let (_fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new(
        "test",
        &pkg,
        RenderOptions {
            limits: RenderLimits {
                max_nodes: 1,
                ..Default::default()
            },
            ..options(1, 1)
        },
    )
    .unwrap();
    let mut nodes = vec![node("a", "red", 1.0, 1.0), node("b", "red", 1.0, 1.0)];
    for n in &mut nodes {
        n.opacity = 0.0;
    }
    assert!(matches!(
        renderer.render_frame("test", &Scene { nodes }),
        Err(RenderError::LimitExceeded("scene nodes"))
    ));
}

#[test]
fn renderer_binds_instance_and_package_without_global_key_collision() {
    let (_fixture, pkg) = package();
    let other = LoadedPackage::load(neko_path()).unwrap();
    let mut a = SoftwareRenderer::new("a", &pkg, options(2, 2)).unwrap();
    let mut b = SoftwareRenderer::new("b", &other, options(2, 2)).unwrap();
    let scene = Scene {
        nodes: vec![node("a", "neko", 2.0, 2.0)],
    };
    a.render_frame("a", &scene).unwrap();
    b.render_frame("b", &scene).unwrap();
    assert_eq!(pixels(a.frame()), [R, G, B, W]);
    assert_eq!(pixels(b.frame()), [W; 4]);
    assert!(matches!(
        a.render_frame("b", &scene),
        Err(RenderError::WrongInstance { .. })
    ));
    assert_eq!(pixels(a.frame()), [R, G, B, W]);
}

#[test]
fn renderer_does_not_reopen_deleted_source_files() {
    let (fixture, pkg) = package();
    let mut renderer = SoftwareRenderer::new("test", &pkg, options(2, 2)).unwrap();
    fs::remove_dir_all(&fixture.0).unwrap();
    renderer
        .render_frame(
            "test",
            &Scene {
                nodes: vec![node("a", "neko", 2.0, 2.0)],
            },
        )
        .unwrap();
    assert_eq!(pixels(renderer.frame()), [R, G, B, W]);
}

#[test]
fn tick_wander_scene_renderer_trait_and_png_complete_the_pipeline() {
    let pkg = LoadedPackage::load(neko_path()).unwrap();
    let mut mate = pkg.instantiate("neko").unwrap();
    let mut renderer = SoftwareRenderer::new(mate.instance_id(), &pkg, options(64, 40)).unwrap();
    let mut platform = MockPlatform::default();
    let mut clock = MockClock::default();
    for expected_x in [10, 20, 30] {
        platform.push_event(clock.advance(Duration::from_secs(1)).unwrap());
        let output = pump_once(&mut mate, &mut platform).unwrap().unwrap();
        assert_eq!(output.state.position.x, f64::from(expected_x));
        assert!(output.capability_requests.is_empty());
        let backend: &mut dyn Renderer = &mut renderer;
        backend.render(mate.instance_id(), &output.scene).unwrap();
        assert_eq!(renderer.frame().pixel(expected_x - 1, 0), Some(Z));
        assert_eq!(renderer.frame().pixel(expected_x, 0), Some(W));
        assert_eq!(renderer.frame().pixel(expected_x + 31, 31), Some(W));
        assert_eq!(renderer.frame().pixel(expected_x + 32, 0), Some(Z));
        assert_eq!(renderer.frame().pixel(expected_x, 32), Some(Z));
        let mut png = Vec::new();
        renderer.frame().write_png(&mut png).unwrap();
        assert_eq!(
            image::load_from_memory(&png).unwrap().into_rgba8().as_raw(),
            renderer.frame().rgba8()
        );
    }
}

#[test]
fn same_scene_sequence_replays_to_identical_pixels_and_png_bytes() {
    let (_fixture, pkg) = package();
    let mut a = SoftwareRenderer::new("a", &pkg, options(8, 8)).unwrap();
    let mut b = SoftwareRenderer::new("b", &pkg, options(8, 8)).unwrap();
    for angle in [0.0, 0.3, -0.7, 1.0] {
        let mut n = node("a", "neko", 4.0, 4.0);
        n.transform.translation = Vec2::new(3.0, 2.0);
        n.transform.rotation_radians = angle;
        n.opacity = 0.7;
        let scene = Scene { nodes: vec![n] };
        a.render_frame("a", &scene).unwrap();
        b.render_frame("b", &scene).unwrap();
        assert_eq!(a.frame().rgba8(), b.frame().rgba8());
        let mut pa = Vec::new();
        let mut pb = Vec::new();
        a.frame().write_png(&mut pa).unwrap();
        b.frame().write_png(&mut pb).unwrap();
        assert_eq!(pa, pb);
    }
}
