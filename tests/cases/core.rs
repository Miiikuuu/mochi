use crate::*;
use std::time::Duration;

#[test]
fn tick_reaches_behavior_and_produces_moveto() {
    let state = MateState::default();
    let context = MateContext {
        instance_id: "one",
        state: &state,
        elapsed: Duration::from_secs(1),
    };
    let wander = WanderBehavior::new(0.0, 100.0, 10.0).unwrap();
    let actions = wander.on_event(
        &context,
        &MateEvent::Tick {
            delta: Duration::from_secs(1),
        },
    );
    assert_eq!(actions[0], MateAction::MoveTo(Vec2::new(10.0, 0.0)));
    assert!(wander
        .on_event(&context, &MateEvent::PointerEnter)
        .is_empty());
    assert!(IdleBehavior
        .on_event(
            &context,
            &MateEvent::Tick {
                delta: Duration::from_secs(1)
            }
        )
        .is_empty());
}

#[test]
fn action_reducer_updates_all_state_fields() {
    let mut state = MateState::default();
    let actions = [
        MateAction::MoveTo(Vec2::new(10.0, 20.0)),
        MateAction::MoveBy(Vec2::new(-3.0, 5.0)),
        MateAction::SetVelocity(Vec2::new(1.0, 2.0)),
        MateAction::PlayAnimation("happy".into()),
        MateAction::SetCurrentState("playing".into()),
        MateAction::SetState {
            key: ns("custom.hunger"),
            value: Value::Integer(7),
        },
    ];
    for action in actions {
        assert_eq!(apply_action(&mut state, &action).unwrap(), None);
    }
    assert_eq!(state.position, Vec2::new(7.0, 25.0));
    assert_eq!(state.velocity, Vec2::new(1.0, 2.0));
    assert_eq!(state.current_animation, "happy");
    assert_eq!(state.current_state, "playing");
    assert_eq!(state.values[&ns("custom.hunger")], Value::Integer(7));
}

#[test]
fn invalid_action_is_atomic_and_move_overflow_is_rejected() {
    let mut state = MateState {
        position: Vec2::new(f64::MAX, 0.0),
        ..Default::default()
    };
    let before = state.clone();
    for action in [
        MateAction::MoveBy(Vec2::new(f64::MAX, 0.0)),
        MateAction::MoveTo(Vec2::new(f64::NAN, 0.0)),
        MateAction::PlayAnimation("".into()),
        MateAction::SetState {
            key: ns("test.bad"),
            value: Value::Float(f64::INFINITY),
        },
    ] {
        assert!(apply_action(&mut state, &action).is_err());
        assert_eq!(state, before);
    }
}

#[test]
fn effect_actions_are_data_and_leave_state_unchanged() {
    let mut state = MateState::default();
    let before = state.clone();
    let event = MateEvent::Custom {
        name: ns("git.dirty"),
        payload: Value::Bool(true),
    };
    assert_eq!(
        apply_action(&mut state, &MateAction::EmitEvent(event.clone())).unwrap(),
        Some(Effect::Event(event))
    );
    let request = CapabilityRequest {
        capability: ns("host.shell"),
        scope: "workspace".into(),
        payload: Value::String("touch should-never-exist".into()),
    };
    assert_eq!(
        apply_action(&mut state, &MateAction::RequestCapability(request.clone())).unwrap(),
        Some(Effect::Capability(request))
    );
    assert_eq!(state, before);
}

#[test]
fn appearance_projects_position_and_animation_to_scene() {
    let state = MateState {
        position: Vec2::new(42.0, -5.0),
        current_animation: "sleep".into(),
        ..Default::default()
    };
    let view = SpriteAppearance {
        asset: "sprites/idle.png".into(),
        size: Vec2::new(20.0, 30.0),
        animations: BTreeMap::from([("sleep".into(), "sprites/sleep.png".into())]),
    };
    let scene = view.scene(&state);
    scene.validate().unwrap();
    assert_eq!(scene.nodes[0].transform.translation, state.position);
    assert!(
        matches!(&scene.nodes[0].content, SceneContent::Sprite { asset, animation, .. } if asset == "sprites/sleep.png" && animation == "sleep")
    );
}

#[test]
fn scene_supports_text_transform_opacity_and_namespaced_extensions() {
    let mut scene = Scene {
        nodes: vec![
            SceneNode {
                id: "label".into(),
                transform: Transform {
                    translation: Vec2::new(2.0, 3.0),
                    scale: Vec2::new(2.0, 2.0),
                    rotation_radians: 0.2,
                },
                opacity: 0.5,
                content: SceneContent::Text {
                    text: "Neko".into(),
                    font_size: 14.0,
                },
            },
            SceneNode {
                id: "future".into(),
                transform: Transform::default(),
                opacity: 1.0,
                content: SceneContent::Extension {
                    kind: ns("example.svg"),
                    data: Value::String("asset-key".into()),
                },
            },
        ],
    };
    scene.validate().unwrap();
    scene.nodes[0].opacity = 1.1;
    assert!(scene.validate().is_err());
}

#[test]
fn native_independent_event_variants_validate() {
    let p = Vec2::new(10.0, 20.0);
    for event in [
        MateEvent::PointerEnter,
        MateEvent::PointerLeave,
        MateEvent::PointerClick {
            position: p,
            button: 1,
        },
        MateEvent::DragStart { position: p },
        MateEvent::DragMove { position: p },
        MateEvent::DragEnd { position: p },
        MateEvent::ScreenChanged {
            origin: p,
            size: Vec2::new(1920.0, 1080.0),
            scale_factor: 1.5,
        },
        MateEvent::Timer {
            id: ns("example.timer"),
        },
    ] {
        event.validate().unwrap();
    }
    assert!(MateEvent::ScreenChanged {
        origin: p,
        size: p,
        scale_factor: 0.0
    }
    .validate()
    .is_err());
}

#[test]
fn wander_validates_configuration() {
    for (a, b, speed) in [
        (0.0, 0.0, 1.0),
        (1.0, 0.0, 1.0),
        (0.0, 1.0, -1.0),
        (0.0, f64::MAX, 1.0),
        (0.0, 1.0, f64::NAN),
    ] {
        assert!(WanderBehavior::new(a, b, speed).is_err());
    }
}
