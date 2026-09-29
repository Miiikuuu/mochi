use crate::*;
use mochi_package::LoadedPackage;
use mochi_platform::{pump_once, MockClock, MockPlatform, Renderer};
use mochi_runtime::{CapabilityStatus, RuntimeError, MAX_ACTIONS_PER_EVENT};
use std::time::Duration;

fn tick(seconds: u64) -> MateEvent {
    MateEvent::Tick {
        delta: Duration::from_secs(seconds),
    }
}

#[test]
fn complete_tick_wander_state_appearance_sprite_pipeline() {
    let package = LoadedPackage::load(neko_path()).unwrap();
    let mut mate = package.instantiate("neko-1").unwrap();
    let before = mate.scene().unwrap();
    let output = mate.process(tick(1)).unwrap();
    assert_eq!(output.trace[0].event, tick(1));
    assert_eq!(
        output.trace[0].actions[0],
        MateAction::MoveTo(Vec2::new(10.0, 0.0))
    );
    assert_eq!(output.state.position.x, 10.0);
    assert_eq!(before.nodes[0].transform.translation.x, 0.0);
    assert_eq!(output.scene.nodes[0].transform.translation.x, 10.0);
    assert!(matches!(
        output.scene.nodes[0].content,
        SceneContent::Sprite { .. }
    ));
    assert!(output.capability_requests.is_empty());
}

#[test]
fn mock_platform_and_mock_clock_run_complete_headless_loop() {
    let mut mate = LoadedPackage::load(neko_path())
        .unwrap()
        .instantiate("mock-neko")
        .unwrap();
    let mut platform = MockPlatform::default();
    let mut clock = MockClock::default();
    for _ in 0..3 {
        platform.push_event(clock.advance(Duration::from_secs(1)).unwrap());
    }
    while let Some(output) = pump_once(&mut mate, &mut platform).unwrap() {
        assert!(output.capability_requests.is_empty());
        platform.render(mate.instance_id(), &output.scene).unwrap();
    }
    assert_eq!(platform.frames.len(), 3);
    assert_eq!(mate.state().position.x, 30.0);
    assert_eq!(mate.elapsed(), clock.now());
    assert!(pump_once(&mut mate, &mut platform).unwrap().is_none());
}

#[test]
fn multiple_mates_have_independent_state_clock_and_direction() {
    let package = LoadedPackage::load(neko_path()).unwrap();
    let mut a = package.instantiate("a").unwrap();
    let mut b = package.instantiate("b").unwrap();
    a.process(tick(11)).unwrap();
    assert_eq!(a.state().position.x, 90.0);
    assert_eq!(a.state().velocity.x, -10.0);
    assert_eq!(b.state(), &MateState::default());
    b.process(tick(1)).unwrap();
    assert_eq!(b.state().position.x, 10.0);
    assert_eq!(b.state().velocity.x, 10.0);
    assert_eq!(a.elapsed(), Duration::from_secs(11));
    assert_eq!(b.elapsed(), Duration::from_secs(1));
}

#[test]
fn replay_is_identical_and_large_ticks_reflect_at_bounds() {
    let package = LoadedPackage::load(neko_path()).unwrap();
    let mut a = package.instantiate("same").unwrap();
    let mut b = package.instantiate("same").unwrap();
    for event in [
        tick(0),
        tick(11),
        MateEvent::PointerEnter,
        tick(29),
        tick(103),
    ] {
        assert_eq!(a.process(event.clone()).unwrap(), b.process(event).unwrap());
        assert!((0.0..=100.0).contains(&a.state().position.x));
    }
}

struct CustomBehavior;
impl Behavior for CustomBehavior {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        match event {
            MateEvent::Custom { name, payload } => vec![MateAction::SetState {
                key: name.clone(),
                value: payload.clone(),
            }],
            _ => vec![],
        }
    }
}
#[test]
fn custom_namespaced_events_work_without_changing_core() {
    let mut mate = runtime(vec![Box::new(CustomBehavior)]);
    for name in ["agent.thinking", "git.dirty", "ros2.node_offline"] {
        mate.process(MateEvent::Custom {
            name: ns(name),
            payload: Value::Bool(true),
        })
        .unwrap();
        assert_eq!(mate.state().values[&ns(name)], Value::Bool(true));
    }
}

struct RequestBehavior;
impl Behavior for RequestBehavior {
    fn on_event(&self, _: &MateContext<'_>, _: &MateEvent) -> Vec<MateAction> {
        vec![MateAction::RequestCapability(CapabilityRequest {
            capability: ns("host.shell"),
            scope: "workspace".into(),
            payload: Value::String("echo this-is-only-data".into()),
        })]
    }
}
#[test]
fn capability_requests_are_outbox_data_never_execution_or_permission_grants() {
    let mut undeclared = runtime(vec![Box::new(RequestBehavior)]);
    let rejected = undeclared.process(tick(1)).unwrap();
    assert_eq!(
        rejected.capability_requests[0].status,
        CapabilityStatus::RejectedUndeclared
    );
    let mut declared = Runtime::new(
        "declared",
        MateState::default(),
        [ns("host.shell")].into(),
        vec![Box::new(RequestBehavior)],
        appearance(),
    )
    .unwrap();
    let first = declared.process(tick(1)).unwrap();
    let second = declared.process(tick(1)).unwrap();
    assert_eq!(
        first.capability_requests[0].status,
        CapabilityStatus::AwaitingHostDecision
    );
    assert_eq!(first.capability_requests[0].sequence, 0);
    assert_eq!(second.capability_requests[0].sequence, 1);
    assert_eq!(first.capability_requests[0].instance_id, "declared");
    assert_eq!(declared.state(), &MateState::default());
}

struct EmittingBehavior;
impl Behavior for EmittingBehavior {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        match event {
            MateEvent::PointerEnter => vec![
                MateAction::EmitEvent(MateEvent::Custom {
                    name: ns("test.first"),
                    payload: Value::Bool(true),
                }),
                MateAction::EmitEvent(MateEvent::Custom {
                    name: ns("test.second"),
                    payload: Value::Bool(true),
                }),
                MateAction::MoveTo(Vec2::new(5.0, 0.0)),
            ],
            MateEvent::Custom { name, .. } if name.as_str() == "test.first" => {
                vec![MateAction::MoveBy(Vec2::new(2.0, 0.0))]
            }
            MateEvent::Custom { name, .. } if name.as_str() == "test.second" => {
                vec![MateAction::SetState {
                    key: ns("test.observed"),
                    value: Value::Float(context.state.position.x),
                }]
            }
            _ => vec![],
        }
    }
}
#[test]
fn emitted_events_are_fifo_and_observe_previous_actions() {
    let output = runtime(vec![Box::new(EmittingBehavior)])
        .process(MateEvent::PointerEnter)
        .unwrap();
    assert_eq!(output.trace.len(), 3);
    assert!(
        matches!(&output.trace[1].event, MateEvent::Custom { name, .. } if name.as_str() == "test.first")
    );
    assert!(
        matches!(&output.trace[2].event, MateEvent::Custom { name, .. } if name.as_str() == "test.second")
    );
    assert_eq!(output.state.values[&ns("test.observed")], Value::Float(7.0));
}

struct InvalidActions;
impl Behavior for InvalidActions {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        let mut actions = RequestBehavior.on_event(context, event);
        if matches!(event, MateEvent::Tick { .. }) {
            actions.extend([
                MateAction::MoveTo(Vec2::new(5.0, 6.0)),
                MateAction::MoveBy(Vec2::new(f64::NAN, 0.0)),
            ]);
        }
        actions
    }
}
#[test]
fn failed_step_rolls_back_state_clock_and_capability_sequence() {
    let mut mate = runtime(vec![Box::new(InvalidActions)]);
    assert!(mate.process(tick(1)).is_err());
    assert_eq!(mate.state(), &MateState::default());
    assert_eq!(mate.elapsed(), Duration::ZERO);
    assert_eq!(
        mate.process(MateEvent::PointerEnter)
            .unwrap()
            .capability_requests[0]
            .sequence,
        0
    );
}

struct EventLoop;
impl Behavior for EventLoop {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        vec![
            MateAction::MoveBy(Vec2::new(1.0, 0.0)),
            MateAction::EmitEvent(event.clone()),
        ]
    }
}
#[test]
fn event_storm_is_bounded_and_rolled_back() {
    let mut mate = runtime(vec![Box::new(EventLoop)]);
    assert_eq!(mate.process(tick(1)), Err(RuntimeError::EventLimit));
    assert_eq!(mate.state().position.x, 0.0);
    assert_eq!(mate.elapsed(), Duration::ZERO);
}

struct TooManyActions;
impl Behavior for TooManyActions {
    fn on_event(&self, _: &MateContext<'_>, _: &MateEvent) -> Vec<MateAction> {
        vec![MateAction::MoveBy(Vec2::new(1.0, 0.0)); MAX_ACTIONS_PER_EVENT + 1]
    }
}
#[test]
fn action_storm_is_bounded() {
    let mut mate = runtime(vec![Box::new(TooManyActions)]);
    assert_eq!(mate.process(tick(1)), Err(RuntimeError::ActionLimit));
    assert_eq!(mate.state(), &MateState::default());
}

#[test]
fn time_overflow_is_rejected_without_commit() {
    let mut mate = runtime(vec![Box::new(IdleBehavior)]);
    mate.process(MateEvent::Tick {
        delta: Duration::MAX,
    })
    .unwrap();
    assert_eq!(mate.process(tick(1)), Err(RuntimeError::TimeOverflow));
    assert_eq!(mate.elapsed(), Duration::MAX);
}

struct ObservePosition;
impl Behavior for ObservePosition {
    fn on_event(&self, context: &MateContext<'_>, _: &MateEvent) -> Vec<MateAction> {
        vec![MateAction::SetState {
            key: ns("test.snapshot"),
            value: Value::Float(context.state.position.x),
        }]
    }
}
#[test]
fn behaviors_read_one_snapshot_and_actions_apply_in_registration_order() {
    let mut mate = runtime(vec![
        Box::new(WanderBehavior::new(0.0, 100.0, 10.0).unwrap()),
        Box::new(ObservePosition),
    ]);
    let output = mate.process(tick(1)).unwrap();
    assert_eq!(output.state.position.x, 10.0);
    assert_eq!(output.state.values[&ns("test.snapshot")], Value::Float(0.0));
}

struct BrokenAppearance;
impl Appearance for BrokenAppearance {
    fn scene(&self, state: &MateState) -> Scene {
        let mut scene = appearance().scene(state);
        if state.position.x > 0.0 {
            scene.nodes[0].opacity = f64::NAN;
        }
        scene
    }
}
#[test]
fn invalid_scene_rolls_back_entire_step() {
    let mut mate = Runtime::new(
        "test",
        MateState::default(),
        Default::default(),
        vec![Box::new(WanderBehavior::new(0.0, 100.0, 10.0).unwrap())],
        Box::new(BrokenAppearance),
    )
    .unwrap();
    assert!(mate.process(tick(1)).is_err());
    assert_eq!(mate.state(), &MateState::default());
    assert_eq!(mate.elapsed(), Duration::ZERO);
}

#[test]
fn invalid_input_is_rejected_before_behavior() {
    let mut mate = runtime(vec![Box::new(CustomBehavior)]);
    assert!(mate
        .process(MateEvent::Custom {
            name: ns("test.bad"),
            payload: Value::Float(f64::NAN)
        })
        .is_err());
    assert!(mate.state().values.is_empty());
}
