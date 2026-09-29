use super::package::Fixture;
use crate::*;
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mochi_host::{Host, InstanceHandle, NativeMate, Presentation};
use mochi_package::LoadedPackage;
use mochi_preview::*;
use mochi_renderer_software::{CanvasRenderer, RenderOptions};
use std::{fs, sync::Arc, time::Duration};

fn package() -> Arc<LoadedPackage> {
    Arc::new(LoadedPackage::load(neko_path()).unwrap())
}
fn host() -> (Host, InstanceHandle, InstanceHandle) {
    let mut host = Host::default();
    let pkg = package();
    let a = host
        .spawn(
            "a",
            pkg.clone(),
            Presentation {
                offset: Vec2::new(10.0, 10.0),
                ..Default::default()
            },
        )
        .unwrap();
    let b = host
        .spawn(
            "b",
            pkg,
            Presentation {
                offset: Vec2::new(20.0, 20.0),
                ..Default::default()
            },
        )
        .unwrap();
    (host, a, b)
}
fn router() -> PointerRouter {
    PointerRouter::new(100, 100).unwrap()
}
fn move_to(router: &mut PointerRouter, host: &Host, x: f64, y: f64) -> Vec<RoutedEvent> {
    router
        .route(host, PointerInput::Move(Vec2::new(x, y)))
        .unwrap()
}
fn expected(target: &InstanceHandle, event: MateEvent) -> RoutedEvent {
    RoutedEvent {
        target: target.clone(),
        event,
    }
}

#[test]
fn configurable_drag_threshold_is_atomic_and_keeps_the_pressed_gesture_stable() {
    let (host, _, _) = host();
    let mut router = router();
    router.set_drag_threshold(2.0).unwrap();
    for invalid in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(router.set_drag_threshold(invalid).is_err());
    }
    move_to(&mut router, &host, 15.0, 15.0);
    router.route(&host, PointerInput::Press(1)).unwrap();
    assert!(move_to(&mut router, &host, 16.5, 15.0).is_empty());
    router.set_drag_threshold(1.0).unwrap();
    assert!(move_to(&mut router, &host, 16.75, 15.0).is_empty());
    assert!(move_to(&mut router, &host, 17.0, 15.0)
        .iter()
        .any(|e| matches!(e.event, MateEvent::DragStart { .. })));
    router.route(&host, PointerInput::Release(1)).unwrap();
    router.route(&host, PointerInput::Press(1)).unwrap();
    assert!(move_to(&mut router, &host, 18.0, 15.0)
        .iter()
        .any(|e| matches!(e.event, MateEvent::DragStart { .. })));
}

#[test]
fn hover_uses_frontmost_mate_and_leaves_before_entering_another() {
    let (host, a, b) = host();
    let mut router = router();
    assert_eq!(
        move_to(&mut router, &host, 25.0, 25.0),
        [expected(&b, MateEvent::PointerEnter)]
    );
    assert!(move_to(&mut router, &host, 26.0, 26.0).is_empty());
    assert_eq!(
        move_to(&mut router, &host, 15.0, 15.0),
        [
            expected(&b, MateEvent::PointerLeave),
            expected(&a, MateEvent::PointerEnter)
        ]
    );
    assert_eq!(
        move_to(&mut router, &host, 99.0, 99.0),
        [expected(&a, MateEvent::PointerLeave)]
    );
}
#[test]
fn hit_testing_is_clipped_to_canvas_and_uses_half_open_sprite_bounds() {
    let (host, a, _) = host();
    let mut router = router();
    assert_eq!(
        move_to(&mut router, &host, 10.0, 10.0),
        [expected(&a, MateEvent::PointerEnter)]
    );
    assert_eq!(
        move_to(&mut router, &host, 42.0, 10.0),
        [expected(&a, MateEvent::PointerLeave)]
    );
    for (x, y) in [(-1.0, 12.0), (12.0, -1.0), (100.0, 10.0), (10.0, 100.0)] {
        assert!(move_to(&mut router, &host, x, y).is_empty());
    }
}
#[test]
fn refresh_observes_z_order_and_mates_moving_under_stationary_pointer() {
    let (mut host, a, b) = host();
    let mut router = router();
    move_to(&mut router, &host, 25.0, 25.0);
    host.set_presentation(
        &a,
        Presentation {
            offset: Vec2::new(10.0, 10.0),
            z_index: 5,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        router.route(&host, PointerInput::Refresh).unwrap(),
        [
            expected(&b, MateEvent::PointerLeave),
            expected(&a, MateEvent::PointerEnter)
        ]
    );
    let _step = host
        .dispatch(
            &a,
            MateEvent::Tick {
                delta: Duration::from_secs(3),
            },
        )
        .unwrap();
    assert_eq!(
        router.route(&host, PointerInput::Refresh).unwrap(),
        [
            expected(&a, MateEvent::PointerLeave),
            expected(&b, MateEvent::PointerEnter)
        ]
    );
}
#[test]
fn transparent_texels_and_effectively_zero_opacity_fall_through_to_lower_mate() {
    let (mut host, a, b) = host();
    host.remove(&b).unwrap();
    let fixture = Fixture::new();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &[255, 0, 0, 0, 255, 0, 0, 255],
            2,
            1,
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    fs::write(fixture.0.join("sprites/neko.png"), png).unwrap();
    let b = host
        .spawn(
            "b",
            Arc::new(LoadedPackage::load(&fixture.0).unwrap()),
            Presentation {
                offset: Vec2::new(10.0, 10.0),
                ..Default::default()
            },
        )
        .unwrap();
    let mut r = router();
    move_to(&mut r, &host, 11.0, 11.0);
    assert_eq!(r.hovered(), Some(&a));
    move_to(&mut r, &host, 30.0, 11.0);
    assert_eq!(r.hovered(), Some(&b));
    host.set_presentation(
        &b,
        Presentation {
            offset: Vec2::new(10.0, 10.0),
            opacity: 0.001,
            ..Default::default()
        },
    )
    .unwrap();
    r.route(&host, PointerInput::Refresh).unwrap();
    assert_eq!(r.hovered(), Some(&a));
}
struct FixedScene(Scene);
impl Appearance for FixedScene {
    fn scene(&self, _: &MateState) -> Scene {
        self.0.clone()
    }
}
fn transformed(transform: Transform) -> (Host, InstanceHandle) {
    let mut host = Host::default();
    let scene = Scene {
        nodes: vec![SceneNode {
            id: "sprite".into(),
            transform,
            opacity: 1.0,
            content: SceneContent::Sprite {
                asset: "sprites/neko.png".into(),
                size: Vec2::new(4.0, 2.0),
                animation: "idle".into(),
            },
        }],
    };
    let h = host
        .spawn_native(
            "transformed",
            package(),
            NativeMate {
                state: MateState::default(),
                behaviors: vec![],
                appearance: Box::new(FixedScene(scene)),
            },
            Presentation::default(),
        )
        .unwrap();
    (host, h)
}
#[test]
fn hit_testing_inverts_rotation_scale_mirror_and_translation() {
    let (host, h) = transformed(Transform {
        translation: Vec2::new(10.0, 10.0),
        scale: Vec2::new(-2.0, 1.0),
        rotation_radians: std::f64::consts::FRAC_PI_2,
    });
    let mut r = router();
    move_to(&mut r, &host, 9.0, 5.0);
    assert_eq!(r.hovered(), Some(&h));
    move_to(&mut r, &host, 11.0, 5.0);
    assert_eq!(r.hovered(), None);
}
#[test]
fn rotation_empty_bounding_box_corners_and_zero_scale_are_not_hits() {
    let (host, h) = transformed(Transform {
        translation: Vec2::new(10.0, 10.0),
        rotation_radians: std::f64::consts::FRAC_PI_4,
        ..Default::default()
    });
    let mut r = router();
    move_to(&mut r, &host, 10.0, 11.0);
    assert_eq!(r.hovered(), Some(&h));
    move_to(&mut r, &host, 8.7, 10.1);
    assert_eq!(r.hovered(), None);
    let (host, _) = transformed(Transform {
        translation: Vec2::new(10.0, 10.0),
        scale: Vec2::new(0.0, 1.0),
        ..Default::default()
    });
    assert!(move_to(&mut router(), &host, 10.0, 10.0).is_empty());
}
#[test]
fn click_on_release_uses_simulation_coordinates_without_changing_state() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 13.0);
    assert!(r.route(&host, PointerInput::Press(1)).unwrap().is_empty());
    assert_eq!(
        r.route(&host, PointerInput::Release(1)).unwrap(),
        [expected(
            &a,
            MateEvent::PointerClick {
                position: Vec2::new(2.0, 3.0),
                button: 1
            }
        )]
    );
    assert_eq!(
        host.get(&a).unwrap().runtime().state().position,
        Vec2::default()
    );
    assert!(r.route(&host, PointerInput::Release(1)).unwrap().is_empty());
}
#[test]
fn primary_drag_starts_at_threshold_stays_captured_and_has_no_click() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    assert!(move_to(&mut r, &host, 15.0, 12.0).is_empty());
    assert_eq!(
        move_to(&mut r, &host, 16.0, 12.0),
        [
            expected(
                &a,
                MateEvent::DragStart {
                    position: Vec2::new(2.0, 2.0)
                }
            ),
            expected(
                &a,
                MateEvent::DragMove {
                    position: Vec2::new(6.0, 2.0)
                }
            )
        ]
    );
    assert_eq!(
        move_to(&mut r, &host, 26.0, 26.0),
        [expected(
            &a,
            MateEvent::DragMove {
                position: Vec2::new(16.0, 16.0)
            }
        )]
    );
    assert_eq!(r.dragging(), Some(&a));
    let events = r.route(&host, PointerInput::Release(1)).unwrap();
    assert!(matches!(events[0].event, MateEvent::DragEnd { .. }));
    assert!(!events
        .iter()
        .any(|e| matches!(e.event, MateEvent::PointerClick { .. })));
    assert_eq!(r.dragging(), None);
}
#[test]
fn right_button_click_does_not_drag_and_mismatched_release_cannot_end_capture() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(3)).unwrap();
    assert!(move_to(&mut r, &host, 18.0, 12.0).is_empty());
    assert!(r.route(&host, PointerInput::Release(1)).unwrap().is_empty());
    assert_eq!(
        r.route(&host, PointerInput::Release(3)).unwrap(),
        [expected(
            &a,
            MateEvent::PointerClick {
                position: Vec2::new(8.0, 2.0),
                button: 3
            }
        )]
    );
}
#[test]
fn leave_preserves_drag_capture_and_release_outside_does_not_click() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    move_to(&mut r, &host, 18.0, 12.0);
    assert_eq!(
        r.route(&host, PointerInput::Leave).unwrap(),
        [expected(&a, MateEvent::PointerLeave)]
    );
    assert_eq!(
        move_to(&mut r, &host, -5.0, -5.0),
        [expected(
            &a,
            MateEvent::DragMove {
                position: Vec2::new(-15.0, -15.0)
            }
        )]
    );
    assert_eq!(
        r.route(&host, PointerInput::Release(1)).unwrap(),
        [expected(
            &a,
            MateEvent::DragEnd {
                position: Vec2::new(-15.0, -15.0)
            }
        )]
    );
}
#[test]
fn focus_cancel_ends_drag_and_clears_hover_exactly_once() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    move_to(&mut r, &host, 18.0, 12.0);
    assert_eq!(
        r.route(&host, PointerInput::Cancel).unwrap(),
        [
            expected(
                &a,
                MateEvent::DragEnd {
                    position: Vec2::new(8.0, 2.0)
                }
            ),
            expected(&a, MateEvent::PointerLeave)
        ]
    );
    assert!(r.route(&host, PointerInput::Cancel).unwrap().is_empty());
    assert!(r.route(&host, PointerInput::Release(1)).unwrap().is_empty());
    assert_eq!(r.dragging(), None);
    assert_eq!(r.hovered(), None);
}
#[test]
fn capture_removal_and_name_reuse_cannot_redirect_drag_or_release() {
    let (mut host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    move_to(&mut r, &host, 18.0, 12.0);
    host.remove(&a).unwrap();
    let new = host.spawn("a", package(), Presentation::default()).unwrap();
    assert!(r.route(&host, PointerInput::Release(1)).unwrap().is_empty());
    assert_eq!(r.dragging(), None);
    assert_ne!(a, new);
    assert_eq!(
        move_to(&mut r, &host, 12.0, 12.0),
        [expected(&new, MateEvent::PointerEnter)]
    );
}
#[test]
fn gesture_coordinates_keep_press_offset_when_host_layout_moves() {
    let (mut host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    move_to(&mut r, &host, 18.0, 12.0);
    host.set_presentation(
        &a,
        Presentation {
            offset: Vec2::new(60.0, 60.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        move_to(&mut r, &host, 20.0, 12.0),
        [expected(
            &a,
            MateEvent::DragMove {
                position: Vec2::new(10.0, 2.0)
            }
        )]
    );
    assert!(r.route(&host, PointerInput::Refresh).unwrap().is_empty());
}
#[test]
fn invalid_input_does_not_corrupt_router_or_capture() {
    let (host, a, _) = host();
    let mut r = router();
    move_to(&mut r, &host, 12.0, 12.0);
    r.route(&host, PointerInput::Press(1)).unwrap();
    assert!(r
        .route(&host, PointerInput::Move(Vec2::new(f64::NAN, 0.0)))
        .is_err());
    assert_eq!(r.hovered(), Some(&a));
    assert!(matches!(
        r.route(&host, PointerInput::Release(1)).unwrap()[0].event,
        MateEvent::PointerClick {
            position: Vec2 { x: 2.0, y: 2.0 },
            ..
        }
    ));
    assert!(PointerRouter::new(0, 1).is_err());
}
struct RememberClick;
impl Behavior for RememberClick {
    fn on_event(&self, _: &MateContext<'_>, e: &MateEvent) -> Vec<MateAction> {
        if let MateEvent::PointerClick { position, .. } = e {
            vec![MateAction::MoveTo(*position)]
        } else {
            vec![]
        }
    }
}
#[test]
fn physical_input_to_router_behavior_action_state_scene_is_complete_and_isolated() {
    let mut host = Host::default();
    let pkg = package();
    let a = host
        .spawn_native(
            "a",
            pkg.clone(),
            NativeMate {
                state: MateState::default(),
                behaviors: vec![Box::new(RememberClick)],
                appearance: appearance(),
            },
            Presentation {
                offset: Vec2::new(10.0, 10.0),
                ..Default::default()
            },
        )
        .unwrap();
    let b = host
        .spawn(
            "b",
            pkg,
            Presentation {
                offset: Vec2::new(70.0, 70.0),
                ..Default::default()
            },
        )
        .unwrap();
    let viewport = Viewport::new([100, 100], [400, 200]).unwrap().unwrap();
    let p = viewport.to_canvas(Vec2::new(126.0, 28.0)).unwrap();
    assert_eq!(p, Vec2::new(13.0, 14.0));
    let mut r = router();
    for input in [
        PointerInput::Move(p),
        PointerInput::Press(1),
        PointerInput::Release(1),
    ] {
        for routed in r.route(&host, input).unwrap() {
            let step = host.dispatch(&routed.target, routed.event).unwrap();
            assert!(step.requests.is_empty());
        }
    }
    assert_eq!(
        host.get(&a).unwrap().runtime().state().position,
        Vec2::new(3.0, 4.0)
    );
    assert_eq!(
        host.scenes().unwrap()[0].scene.nodes[0]
            .transform
            .translation,
        Vec2::new(13.0, 14.0)
    );
    assert_eq!(
        host.get(&b).unwrap().runtime().state().position,
        Vec2::default()
    );
}

#[test]
fn viewport_letterbox_mapping_and_dpi_scaled_surface_share_coordinates() {
    let a = Viewport::new([100, 50], [400, 400]).unwrap().unwrap();
    assert_eq!(
        a.to_canvas(Vec2::new(40.0, 140.0)).unwrap(),
        Vec2::new(10.0, 10.0)
    );
    assert_eq!(
        a.to_canvas(Vec2::new(0.0, 0.0)).unwrap(),
        Vec2::new(0.0, -25.0)
    );
    let b = Viewport::new([100, 50], [800, 800]).unwrap().unwrap();
    assert_eq!(
        b.to_canvas(Vec2::new(80.0, 280.0)).unwrap(),
        Vec2::new(10.0, 10.0)
    );
    assert!(b.to_canvas(Vec2::new(f64::INFINITY, 0.0)).is_err());
}
#[test]
fn viewport_fractional_scale_preserves_round_trip_location() {
    let v = Viewport::new([100, 100], [151, 201]).unwrap().unwrap();
    let p = v.to_canvas(Vec2::new(75.5, 100.5)).unwrap();
    assert_eq!(p, Vec2::new(50.0, 50.0));
}
#[test]
fn minimized_and_oversized_viewports_are_handled_before_buffer_allocation() {
    assert!(Viewport::new([100, 100], [0, 100]).unwrap().is_none());
    assert!(Viewport::new([100, 100], [100, 0]).unwrap().is_none());
    assert!(Viewport::new([0, 100], [100, 100]).is_err());
    assert!(Viewport::new([1, 1], [u32::MAX, u32::MAX]).is_err());
    assert!(Viewport::new([1, 1], [4096, 4096]).is_ok());
    assert!(Viewport::new([1, 1], [4096, 4097]).is_err());
}
#[test]
fn viewport_rgb_flattens_alpha_and_fills_letterbox_using_hand_calculated_pixels() {
    let renderer = CanvasRenderer::new(RenderOptions {
        width: 1,
        height: 1,
        background: [255, 0, 0, 128],
        ..Default::default()
    })
    .unwrap();
    let v = Viewport::new([1, 1], [3, 1]).unwrap().unwrap();
    let mut pixels = vec![0xffffffff; 3];
    v.write_rgb(renderer.frame(), &mut pixels, [0, 0, 255])
        .unwrap();
    assert_eq!(pixels, [0x0000ff, 0x80007f, 0x0000ff]);
}
#[test]
fn viewport_rejects_wrong_buffer_and_frame_dimensions_without_partial_writes() {
    let renderer = CanvasRenderer::new(RenderOptions {
        width: 1,
        height: 1,
        ..Default::default()
    })
    .unwrap();
    let v = Viewport::new([1, 1], [2, 2]).unwrap().unwrap();
    let mut pixels = [123_u32; 3];
    assert!(v.write_rgb(renderer.frame(), &mut pixels, [0; 3]).is_err());
    assert_eq!(pixels, [123; 3]);
    let v = Viewport::new([2, 2], [3, 1]).unwrap().unwrap();
    assert!(v.write_rgb(renderer.frame(), &mut pixels, [0; 3]).is_err());
    assert_eq!(pixels, [123; 3]);
}
#[test]
fn fixed_clock_accumulates_partial_intervals_and_catches_up_with_exact_steps() {
    let mut clock = FrameClock::default();
    assert_eq!(clock.advance(Duration::from_secs(20)).unwrap(), 0);
    assert_eq!(
        clock
            .advance(Duration::from_secs(20) + Duration::from_millis(10))
            .unwrap(),
        0
    );
    assert_eq!(clock.next_in(), FIXED_STEP - Duration::from_millis(10));
    assert_eq!(
        clock
            .advance(Duration::from_secs(20) + FIXED_STEP * 3)
            .unwrap(),
        3
    );
    assert_eq!(clock.next_in(), FIXED_STEP);
}
#[test]
fn clock_discards_stall_backlog_and_reset_does_not_replay_paused_time() {
    let mut clock = FrameClock::default();
    clock.advance(Duration::ZERO).unwrap();
    let stalled = FIXED_STEP * 100 + Duration::from_nanos(3);
    assert_eq!(clock.advance(stalled).unwrap(), MAX_CATCH_UP);
    assert_eq!(clock.advance(stalled).unwrap(), 0);
    assert_eq!(clock.next_in(), FIXED_STEP - Duration::from_nanos(3));
    clock.reset();
    assert_eq!(clock.advance(Duration::from_secs(1000)).unwrap(), 0);
    assert_eq!(
        clock
            .advance(Duration::from_secs(1000) + FIXED_STEP)
            .unwrap(),
        1
    );
}
#[test]
fn clock_rejects_time_reversal_atomically() {
    let mut clock = FrameClock::default();
    clock.advance(Duration::from_secs(10)).unwrap();
    assert!(clock.advance(Duration::from_secs(9)).is_err());
    assert_eq!(
        clock.advance(Duration::from_secs(10) + FIXED_STEP).unwrap(),
        1
    );
}

#[test]
fn viewport_pixel_centers_preserve_colored_rows_under_resize_and_letterbox() {
    let fixture = Fixture::new();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            2,
            2,
            ExtendedColorType::Rgba8,
        )
        .unwrap();
    fs::write(fixture.0.join("sprites/neko.png"), png).unwrap();
    let pkg = LoadedPackage::load(&fixture.0).unwrap();
    let scene = SpriteAppearance {
        asset: "sprites/neko.png".into(),
        size: Vec2::new(2.0, 2.0),
        animations: BTreeMap::new(),
    }
    .scene(&MateState::default());
    let mut renderer = CanvasRenderer::new(RenderOptions {
        width: 2,
        height: 2,
        ..Default::default()
    })
    .unwrap();
    renderer
        .render_layers(&[mochi_renderer_software::RenderLayer {
            package: &pkg,
            scene: &scene,
        }])
        .unwrap();
    let viewport = Viewport::new([2, 2], [4, 6]).unwrap().unwrap();
    let mut pixels = vec![0; 24];
    viewport
        .write_rgb(renderer.frame(), &mut pixels, [1, 2, 3])
        .unwrap();
    assert_eq!(
        pixels,
        [
            0x010203, 0x010203, 0x010203, 0x010203, 0xff0000, 0xff0000, 0x00ff00, 0x00ff00,
            0xff0000, 0xff0000, 0x00ff00, 0x00ff00, 0x0000ff, 0x0000ff, 0xffffff, 0xffffff,
            0x0000ff, 0x0000ff, 0xffffff, 0xffffff, 0x010203, 0x010203, 0x010203, 0x010203,
        ]
    );
}

#[test]
fn viewport_matches_reference_sampling_for_all_alpha_values_and_varied_sizes() {
    let fixture = Fixture::new();
    let rgba: Vec<u8> = (0..=255_u8)
        .flat_map(|a| [a.wrapping_mul(73), 255 - a, a.wrapping_mul(19), a])
        .collect();
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&rgba, 16, 16, ExtendedColorType::Rgba8)
        .unwrap();
    fs::write(fixture.0.join("sprites/neko.png"), png).unwrap();
    let package = LoadedPackage::load(&fixture.0).unwrap();
    let scene = SpriteAppearance {
        asset: "sprites/neko.png".into(),
        size: Vec2::new(16.0, 16.0),
        animations: BTreeMap::new(),
    }
    .scene(&MateState::default());
    let mut renderer = CanvasRenderer::new(RenderOptions {
        width: 16,
        height: 16,
        ..Default::default()
    })
    .unwrap();
    renderer
        .render_layers(&[mochi_renderer_software::RenderLayer {
            package: &package,
            scene: &scene,
        }])
        .unwrap();
    // Downscaling, fractional/integer upscaling, odd letterbox boundaries and
    // very thin surfaces exercise row reuse and every alpha rounding value.
    let sizes = (1..=35).flat_map(|w| (1..=35).map(move |h| [w, h]));
    for surface in sizes.chain([[1, 1024], [1024, 1], [1280, 720]]) {
        let viewport = Viewport::new([16, 16], surface).unwrap().unwrap();
        for matte in [[0, 0, 0], [255, 255, 255], [19, 103, 211]] {
            let mut actual = vec![0xdeadbeef; (surface[0] * surface[1]) as usize];
            viewport
                .write_rgb(renderer.frame(), &mut actual, matte)
                .unwrap();
            for (index, actual) in actual.into_iter().enumerate() {
                let p = viewport
                    .to_canvas(Vec2::new(
                        (index % surface[0] as usize) as f64 + 0.5,
                        (index / surface[0] as usize) as f64 + 0.5,
                    ))
                    .unwrap();
                let mut rgb = matte;
                if p.x >= 0.0 && p.y >= 0.0 && p.x < 16.0 && p.y < 16.0 {
                    let source = renderer
                        .frame()
                        .pixel(p.x.floor() as u32, p.y.floor() as u32)
                        .unwrap();
                    let alpha = u32::from(source[3]);
                    for c in 0..3 {
                        rgb[c] = ((u32::from(source[c]) * alpha
                            + u32::from(matte[c]) * (255 - alpha)
                            + 127)
                            / 255) as u8;
                    }
                }
                let expected =
                    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
                assert_eq!(
                    actual, expected,
                    "surface={surface:?} pixel={index} matte={matte:?}"
                );
            }
        }
    }
}

#[test]
fn viewport_regions_preserve_neighbors_and_reject_invalid_bounds_atomically() {
    let renderer = CanvasRenderer::new(RenderOptions {
        width: 7,
        height: 5,
        background: [113, 89, 71, 137],
        ..Default::default()
    })
    .unwrap();
    for surface in [[21, 15], [23, 19], [3, 11], [19, 3]] {
        let viewport = Viewport::new([7, 5], surface).unwrap().unwrap();
        let mut expected = vec![0; (surface[0] * surface[1]) as usize];
        viewport
            .write_rgb(renderer.frame(), &mut expected, [7, 13, 19])
            .unwrap();
        for x0 in 0..surface[0] {
            for y0 in 0..surface[1] {
                let x1 = (x0 + 3).min(surface[0]);
                let y1 = (y0 + 5).min(surface[1]);
                let mut pixels = vec![0xdeadbeef; expected.len()];
                viewport
                    .write_rgb_region(renderer.frame(), &mut pixels, [7, 13, 19], [x0, y0, x1, y1])
                    .unwrap();
                for (i, &pixel) in pixels.iter().enumerate() {
                    let x = i as u32 % surface[0];
                    let y = i as u32 / surface[0];
                    assert_eq!(
                        pixel,
                        if x >= x0 && x < x1 && y >= y0 && y < y1 {
                            expected[i]
                        } else {
                            0xdeadbeef
                        }
                    );
                }
            }
        }
        for region in [
            [2, 0, 1, 1],
            [0, 2, 1, 1],
            [0, 0, surface[0] + 1, 1],
            [0, 0, 1, surface[1] + 1],
        ] {
            let mut pixels = vec![0xdeadbeef; expected.len()];
            assert!(viewport
                .write_rgb_region(renderer.frame(), &mut pixels, [0; 3], region)
                .is_err());
            assert!(pixels.iter().all(|&p| p == 0xdeadbeef));
        }
    }
}
