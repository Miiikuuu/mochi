use super::package::Fixture;
use crate::*;
use mochi_broker::*;
use mochi_host::*;
use mochi_package::LoadedPackage;
use mochi_platform::{EventSource, MockClock, MockPlatform};
use mochi_renderer_software::{CanvasRenderer, RenderLayer, RenderOptions};
use mochi_schema::{MateManifest, Permission};
use std::{cell::Cell, fs, rc::Rc, sync::Arc, time::Duration};

fn package() -> Arc<LoadedPackage> {
    Arc::new(LoadedPackage::load(neko_path()).unwrap())
}
fn tick() -> MateEvent {
    MateEvent::Tick {
        delta: Duration::from_secs(1),
    }
}
fn click() -> MateEvent {
    MateEvent::PointerClick {
        position: Vec2::default(),
        button: 1,
    }
}
fn spawn(host: &mut Host, id: &str, pkg: &Arc<LoadedPackage>) -> InstanceHandle {
    host.spawn(id, pkg.clone(), Presentation::default())
        .unwrap()
}
fn x(host: &Host, h: &InstanceHandle) -> f64 {
    host.get(h).unwrap().runtime().state().position.x
}
fn rule() -> Rule {
    Rule::new(ns("mock.echo"), "demo").unwrap()
}
fn cap_package() -> Arc<LoadedPackage> {
    let fixture = Fixture::new();
    let mut manifest = MateManifest::from_toml(&manifest_text()).unwrap();
    manifest.capabilities.insert(ns("mock.echo"));
    manifest.permissions.push(Permission {
        capability: ns("mock.echo"),
        scope: "demo".into(),
    });
    fs::write(fixture.0.join("manifest.toml"), manifest.to_toml().unwrap()).unwrap();
    Arc::new(LoadedPackage::load(&fixture.0).unwrap())
}
struct RequestAndReceive;
impl Behavior for RequestAndReceive {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        match event {
            MateEvent::PointerClick { .. } => {
                vec![MateAction::RequestCapability(CapabilityRequest {
                    capability: ns("mock.echo"),
                    scope: "demo".into(),
                    payload: Value::Bool(true),
                })]
            }
            MateEvent::Custom { name, payload } if name.as_str() == "mochi.capability.result" => {
                vec![MateAction::SetState {
                    key: ns("demo.result"),
                    value: payload.clone(),
                }]
            }
            _ => vec![],
        }
    }
}
fn native(
    host: &mut Host,
    id: &str,
    pkg: &Arc<LoadedPackage>,
    behavior: Box<dyn Behavior>,
) -> InstanceHandle {
    host.spawn_native(
        id,
        pkg.clone(),
        NativeMate {
            state: MateState::default(),
            behaviors: vec![behavior],
            appearance: appearance(),
        },
        Presentation::default(),
    )
    .unwrap()
}
fn request(host: &mut Host, handle: &InstanceHandle) -> HostRequest {
    host.dispatch(handle, click()).unwrap().requests.remove(0)
}
fn pending(host: &mut Host, request: &HostRequest) -> Ticket {
    match host.submit(request, Duration::from_secs(2)).unwrap() {
        Submission::Pending(ticket) => ticket,
        other => panic!("expected pending: {other:?}"),
    }
}
fn ready(host: &mut Host, request: &HostRequest) -> CapabilityReply {
    match host.submit(request, Duration::from_secs(2)).unwrap() {
        Submission::Completed(reply) => reply,
        other => panic!("expected completed: {other:?}"),
    }
}
struct Counting {
    starts: Rc<Cell<usize>>,
    cancels: Rc<Cell<usize>>,
}
impl CapabilityHandler for Counting {
    fn start(&mut self, _: &Invocation<'_>) -> HandlerResult {
        self.starts.set(self.starts.get() + 1);
        HandlerResult::Pending
    }
    fn cancel(&mut self, _: &Ticket) {
        self.cancels.set(self.cancels.get() + 1);
    }
}
fn counted() -> (Host, Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let mut host = Host::default();
    let starts = Rc::new(Cell::new(0));
    let cancels = Rc::new(Cell::new(0));
    host.register(
        ns("mock.echo"),
        Box::new(Counting {
            starts: starts.clone(),
            cancels: cancels.clone(),
        }),
    )
    .unwrap();
    host.allow(rule()).unwrap();
    (host, starts, cancels)
}

#[test]
fn same_package_is_shared_while_targeted_tick_state_and_time_are_isolated() {
    let pkg = package();
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    let b = spawn(&mut host, "b", &pkg);
    assert!(std::ptr::eq(
        host.get(&a).unwrap().package(),
        host.get(&b).unwrap().package()
    ));
    let step = host.dispatch(&a, tick()).unwrap();
    assert!(matches!(
        step.output.trace[0].actions[0],
        MateAction::MoveTo(_)
    ));
    assert_eq!((x(&host, &a), x(&host, &b)), (10.0, 0.0));
    assert_eq!(host.get(&b).unwrap().runtime().elapsed(), Duration::ZERO);
    assert!(host.get(&b).unwrap().runtime().state().values.is_empty());
    assert!(step.requests.is_empty());
    assert_eq!(host.broker().pending_count(), 0);
}
#[test]
fn failed_spawns_leave_registry_unchanged_and_limit_is_reusable() {
    let pkg = package();
    let mut host = Host::new(1);
    assert!(host
        .spawn(" ", pkg.clone(), Presentation::default())
        .is_err());
    assert!(host.is_empty());
    let a = spawn(&mut host, "a", &pkg);
    assert!(matches!(
        host.spawn("a", pkg.clone(), Presentation::default()),
        Err(HostError::DuplicateInstance)
    ));
    assert!(matches!(
        host.spawn("b", pkg.clone(), Presentation::default()),
        Err(HostError::InstanceLimit)
    ));
    assert_eq!(host.len(), 1);
    host.remove(&a).unwrap();
    spawn(&mut host, "b", &pkg);
    assert!(matches!(
        Host::new(0).spawn("a", pkg, Presentation::default()),
        Err(HostError::InstanceLimit)
    ));
}
#[test]
fn removed_and_foreign_handles_never_target_reused_names() {
    let pkg = package();
    let mut host = Host::default();
    let mut other = Host::default();
    let old = spawn(&mut host, "same", &pkg);
    let foreign = spawn(&mut other, "same", &pkg);
    assert!(matches!(
        host.dispatch(&foreign, tick()),
        Err(HostError::ForeignHandle)
    ));
    assert!(matches!(
        host.remove(&foreign),
        Err(HostError::ForeignHandle)
    ));
    host.remove(&old).unwrap();
    let new = spawn(&mut host, "same", &pkg);
    assert_ne!(old, new);
    assert_eq!(host.find("same"), Some(new.clone()));
    assert!(matches!(
        host.dispatch(&old, tick()),
        Err(HostError::UnknownInstance)
    ));
    assert!(host.get(&old).is_err());
    assert!(host.remove(&old).is_err());
    assert!(host
        .set_presentation(&old, Presentation::default())
        .is_err());
    assert_eq!(x(&host, &new), 0.0);
}
#[test]
fn broadcast_order_is_creation_order_independent_of_name_and_z_order() {
    let pkg = package();
    let mut host = Host::default();
    let z = spawn(&mut host, "z", &pkg);
    let a = spawn(&mut host, "a", &pkg);
    let m = spawn(&mut host, "m", &pkg);
    host.set_presentation(
        &z,
        Presentation {
            z_index: 99,
            ..Default::default()
        },
    )
    .unwrap();
    let results = host.broadcast(tick());
    assert_eq!(
        results.iter().map(|(h, _)| h.clone()).collect::<Vec<_>>(),
        [z, a, m]
    );
    for (_, result) in results {
        assert_eq!(result.unwrap().output.state.position.x, 10.0);
    }
}
struct RejectTick;
impl Behavior for RejectTick {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        if matches!(event, MateEvent::Tick { .. }) {
            vec![MateAction::MoveTo(Vec2::new(f64::NAN, 0.0))]
        } else {
            vec![]
        }
    }
}
#[test]
fn broadcast_attempts_all_instances_and_preserves_successful_siblings() {
    let pkg = package();
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    let b = native(&mut host, "b", &pkg, Box::new(RejectTick));
    let c = spawn(&mut host, "c", &pkg);
    let results = host.broadcast(tick());
    assert!(results[0].1.is_ok());
    assert!(results[1].1.is_err());
    assert!(results[2].1.is_ok());
    assert_eq!(
        (x(&host, &a), x(&host, &b), x(&host, &c)),
        (10.0, 0.0, 10.0)
    );
    assert_eq!(host.get(&b).unwrap().runtime().elapsed(), Duration::ZERO);
}
struct LocalEvents;
impl Behavior for LocalEvents {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        match event {
            MateEvent::PointerEnter => vec![MateAction::EmitEvent(MateEvent::Custom {
                name: ns("ros2.node_offline"),
                payload: Value::Bool(true),
            })],
            MateEvent::Custom { name, payload } => vec![MateAction::SetState {
                key: name.clone(),
                value: payload.clone(),
            }],
            _ => vec![],
        }
    }
}
#[test]
fn emitted_custom_events_stay_with_the_origin_instance() {
    let pkg = package();
    let mut host = Host::default();
    let a = native(&mut host, "a", &pkg, Box::new(LocalEvents));
    let b = native(&mut host, "b", &pkg, Box::new(LocalEvents));
    let step = host.dispatch(&a, MateEvent::PointerEnter).unwrap();
    assert_eq!(step.output.trace.len(), 2);
    assert_eq!(
        step.output.state.values[&ns("ros2.node_offline")],
        Value::Bool(true)
    );
    assert!(host.get(&b).unwrap().runtime().state().values.is_empty());
}
#[test]
fn presentation_offsets_and_opacity_do_not_modify_runtime_scene_or_state() {
    let pkg = package();
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    let step = host.dispatch(&a, tick()).unwrap();
    host.set_presentation(
        &a,
        Presentation {
            offset: Vec2::new(7.0, 11.0),
            opacity: 0.5,
            z_index: 8,
        },
    )
    .unwrap();
    let scenes = host.scenes().unwrap();
    assert_eq!(
        scenes[0].scene.nodes[0].transform.translation,
        Vec2::new(17.0, 11.0)
    );
    assert_eq!(scenes[0].scene.nodes[0].opacity, 0.5);
    assert_eq!(
        host.get(&a).unwrap().runtime().scene().unwrap(),
        step.output.scene
    );
    assert_eq!(x(&host, &a), 10.0);
}
#[test]
fn equal_z_uses_creation_order_and_recreated_instance_moves_to_end() {
    let pkg = package();
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    spawn(&mut host, "b", &pkg);
    let c = spawn(&mut host, "c", &pkg);
    host.set_presentation(
        &c,
        Presentation {
            z_index: -1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        host.scenes()
            .unwrap()
            .iter()
            .map(|s| s.instance_id)
            .collect::<Vec<_>>(),
        ["c", "a", "b"]
    );
    host.remove(&a).unwrap();
    spawn(&mut host, "a", &pkg);
    assert_eq!(
        host.scenes()
            .unwrap()
            .iter()
            .map(|s| s.instance_id)
            .collect::<Vec<_>>(),
        ["c", "b", "a"]
    );
}
#[test]
fn invalid_presentation_is_atomic_and_projection_overflow_is_reported() {
    let pkg = package();
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    for p in [
        Presentation {
            opacity: 1.1,
            ..Default::default()
        },
        Presentation {
            offset: Vec2::new(f64::NAN, 0.0),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            host.set_presentation(&a, p),
            Err(HostError::InvalidPresentation)
        ));
        assert!(matches!(
            host.spawn("bad", pkg.clone(), p),
            Err(HostError::InvalidPresentation)
        ));
        assert_eq!(
            host.get(&a).unwrap().presentation(),
            Presentation::default()
        );
    }
    let b = host
        .spawn_native(
            "big",
            pkg,
            NativeMate {
                state: MateState {
                    position: Vec2::new(f64::MAX, 0.0),
                    ..Default::default()
                },
                behaviors: vec![],
                appearance: appearance(),
            },
            Presentation {
                offset: Vec2::new(f64::MAX, 0.0),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(matches!(host.scenes(), Err(HostError::Runtime(_))));
    assert_eq!(x(&host, &b), f64::MAX);
}
#[test]
fn dispatch_requests_do_not_start_handlers_and_grants_are_per_instance() {
    let pkg = cap_package();
    let (mut host, starts, _) = counted();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    let b = native(&mut host, "b", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    let ra = request(&mut host, &a);
    let rb = request(&mut host, &b);
    assert_eq!(starts.get(), 0);
    assert_eq!(host.broker().pending_count(), 0);
    assert_eq!(
        ready(&mut host, &rb).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    pending(&mut host, &ra);
    assert_eq!(starts.get(), 1);
}
#[test]
fn delayed_requests_are_bound_to_original_host_and_incarnation() {
    let pkg = cap_package();
    let mut host = Host::default();
    let mut other = Host::default();
    let old = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    native(&mut other, "a", &pkg, Box::new(RequestAndReceive));
    let r = request(&mut host, &old);
    assert!(matches!(
        other.submit(&r, Duration::ZERO),
        Err(HostError::ForeignHandle)
    ));
    host.remove(&old).unwrap();
    native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    assert!(matches!(
        host.submit(&r, Duration::ZERO),
        Err(HostError::UnknownInstance)
    ));
}
#[test]
fn out_of_order_and_duplicate_submission_do_not_reexecute_handlers() {
    let pkg = cap_package();
    let (mut host, starts, _) = counted();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    let r0 = request(&mut host, &a);
    let r1 = request(&mut host, &a);
    assert!(matches!(
        host.submit(&r1, Duration::from_secs(2)),
        Err(HostError::Broker(BrokerError::UnexpectedSequence { .. }))
    ));
    pending(&mut host, &r0);
    pending(&mut host, &r1);
    assert!(host.submit(&r0, Duration::from_secs(2)).is_err());
    assert_eq!(starts.get(), 2);
}
#[test]
fn remove_cancels_only_own_pending_requests_and_revokes_grants_on_name_reuse() {
    let pkg = cap_package();
    let (mut host, _, cancels) = counted();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    let b = native(&mut host, "b", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    host.grant(&b, rule()).unwrap();
    let ra = request(&mut host, &a);
    let rb = request(&mut host, &b);
    let ta = pending(&mut host, &ra);
    let tb = pending(&mut host, &rb);
    let replies = host.remove(&a).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].outcome(), &Outcome::SessionClosed);
    assert_eq!(cancels.get(), 1);
    assert_eq!(host.broker().pending_count(), 1);
    let new = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    assert!(host.deliver(&replies[0]).is_err());
    assert!(host.complete(&ta, Ok(Value::Bool(true))).is_err());
    let rnew = request(&mut host, &new);
    assert_eq!(
        ready(&mut host, &rnew).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    let reply_b = host.complete(&tb, Ok(Value::Bool(true))).unwrap();
    let step = host.deliver(&reply_b).unwrap();
    assert_eq!(step.instance, b);
    assert!(host.get(&new).unwrap().runtime().state().values.is_empty());
}
#[test]
fn completed_reply_cannot_reach_replacement_or_another_host() {
    let pkg = cap_package();
    let mut host = Host::default();
    let mut other = Host::default();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    native(&mut other, "a", &pkg, Box::new(RequestAndReceive));
    let request = request(&mut host, &a);
    let reply = ready(&mut host, &request);
    assert!(matches!(
        other.deliver(&reply),
        Err(HostError::Broker(BrokerError::ForeignHandle))
    ));
    host.remove(&a).unwrap();
    let new = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    assert!(host.deliver(&reply).is_err());
    assert!(host.get(&new).unwrap().runtime().state().values.is_empty());
}
#[test]
fn explicit_clock_times_out_in_creation_order_without_tick_coupling() {
    let pkg = cap_package();
    let (mut host, _, cancels) = counted();
    let a = native(&mut host, "z", &pkg, Box::new(RequestAndReceive));
    let b = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    host.grant(&b, rule()).unwrap();
    let ra = request(&mut host, &a);
    let rb = request(&mut host, &b);
    pending(&mut host, &rb);
    pending(&mut host, &ra);
    let _step = host
        .dispatch(
            &a,
            MateEvent::Tick {
                delta: Duration::from_secs(100),
            },
        )
        .unwrap();
    assert_eq!(host.broker().now(), Duration::ZERO);
    let replies = host.advance_to(Duration::from_secs(2)).unwrap();
    assert_eq!(
        replies.iter().map(|r| r.instance_id()).collect::<Vec<_>>(),
        ["z", "a"]
    );
    for reply in replies {
        assert_eq!(reply.outcome(), &Outcome::TimedOut);
        let _step = host.deliver(&reply).unwrap();
    }
    assert_eq!(cancels.get(), 2);
    assert!(host.advance_to(Duration::ZERO).is_err());
}
#[test]
fn revoke_and_disallow_preserve_original_session_routing() {
    let pkg = cap_package();
    let (mut host, _, cancels) = counted();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    let r = request(&mut host, &a);
    pending(&mut host, &r);
    let replies = host.revoke(&a, &rule()).unwrap();
    assert_eq!(replies[0].outcome(), &Outcome::Revoked);
    let _step = host.deliver(&replies[0]).unwrap();
    host.grant(&a, rule()).unwrap();
    let r = request(&mut host, &a);
    pending(&mut host, &r);
    let replies = host.disallow(&rule()).unwrap();
    assert_eq!(
        replies[0].outcome(),
        &Outcome::Denied(Denial::HostPolicyDenied)
    );
    host.allow(rule()).unwrap();
    let r = request(&mut host, &a);
    assert_eq!(
        ready(&mut host, &r).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    assert_eq!(cancels.get(), 2);
}
#[test]
fn clear_and_drop_cancel_once_and_release_owned_packages() {
    let pkg = cap_package();
    let (mut host, _, cancels) = counted();
    for name in ["b", "a"] {
        let a = native(&mut host, name, &pkg, Box::new(RequestAndReceive));
        host.grant(&a, rule()).unwrap();
        let r = request(&mut host, &a);
        pending(&mut host, &r);
    }
    let replies = host.clear();
    assert!(host.is_empty());
    assert_eq!(cancels.get(), 2);
    assert_eq!(
        replies.iter().map(|r| r.instance_id()).collect::<Vec<_>>(),
        ["b", "a"]
    );
    assert!(host.clear().is_empty());
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    host.grant(&a, rule()).unwrap();
    let r = request(&mut host, &a);
    pending(&mut host, &r);
    let weak = Arc::downgrade(&pkg);
    drop(pkg);
    assert!(weak.upgrade().is_some());
    drop(host);
    assert_eq!(cancels.get(), 3);
    assert!(weak.upgrade().is_none());
}
#[test]
fn render_failure_does_not_lose_committed_capability_requests() {
    let pkg = cap_package();
    let mut host = Host::default();
    let a = native(&mut host, "a", &pkg, Box::new(RequestAndReceive));
    let step = host.dispatch(&a, click()).unwrap();
    let mut renderer = CanvasRenderer::new(RenderOptions {
        limits: mochi_renderer_software::RenderLimits {
            max_nodes: 0,
            ..Default::default()
        },
        ..Default::default()
    })
    .unwrap();
    let scenes = host.scenes().unwrap();
    let layers: Vec<_> = scenes
        .iter()
        .map(|s| RenderLayer {
            package: s.package,
            scene: &s.scene,
        })
        .collect();
    assert!(renderer.render_layers(&layers).is_err());
    let reply = ready(&mut host, &step.requests[0]);
    let received = host.deliver(&reply).unwrap();
    assert!(received
        .output
        .state
        .values
        .contains_key(&ns("demo.result")));
}
struct RequestOnReply;
impl Behavior for RequestOnReply {
    fn on_event(&self, ctx: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        if matches!(event, MateEvent::Custom { .. }) {
            RequestAndReceive.on_event(ctx, &click())
        } else {
            RequestAndReceive.on_event(ctx, event)
        }
    }
}
#[test]
fn reply_generated_requests_are_captured_without_recursive_execution() {
    let pkg = cap_package();
    let mut host = Host::default();
    let a = native(&mut host, "a", &pkg, Box::new(RequestOnReply));
    let r = request(&mut host, &a);
    let reply = ready(&mut host, &r);
    let step = host.deliver(&reply).unwrap();
    assert_eq!(step.requests.len(), 1);
    assert_eq!(step.requests[0].intent().sequence, 1);
    assert_eq!(host.broker().pending_count(), 0);
}

fn replay(pkg: Arc<LoadedPackage>) -> Vec<Vec<u8>> {
    let mut host = Host::default();
    let a = spawn(&mut host, "a", &pkg);
    let b = spawn(&mut host, "b", &pkg);
    let c = spawn(&mut host, "c", &pkg);
    for (handle, y) in [(&a, 0.0), (&b, 32.0), (&c, 64.0)] {
        host.set_presentation(
            handle,
            Presentation {
                offset: Vec2::new(0.0, y),
                ..Default::default()
            },
        )
        .unwrap();
    }
    let mut renderer = CanvasRenderer::new(RenderOptions {
        width: 80,
        height: 96,
        ..Default::default()
    })
    .unwrap();
    let mut frames = Vec::new();
    let mut platform = MockPlatform::default();
    let mut clock = MockClock::default();
    for phase in 0..4 {
        match phase {
            1 => {
                platform.push_event(clock.advance(Duration::from_secs(1)).unwrap());
                let step = host
                    .dispatch(&a, platform.next_event().unwrap().unwrap())
                    .unwrap();
                assert_eq!(step.output.state.position.x, 10.0);
                assert_eq!((x(&host, &b), x(&host, &c)), (0.0, 0.0));
            }
            2 => {
                host.remove(&b).unwrap();
            }
            3 => {
                platform.push_event(clock.advance(Duration::from_secs(1)).unwrap());
                let results = host.broadcast(platform.next_event().unwrap().unwrap());
                assert_eq!(results.len(), 2);
                for (_, result) in results {
                    assert!(result.unwrap().requests.is_empty());
                }
                assert_eq!((x(&host, &a), x(&host, &c)), (20.0, 10.0));
            }
            _ => {}
        }
        let scenes = host.scenes().unwrap();
        let layers: Vec<_> = scenes
            .iter()
            .map(|s| RenderLayer {
                package: s.package,
                scene: &s.scene,
            })
            .collect();
        renderer.render_layers(&layers).unwrap();
        let expected = match phase {
            0 => vec![(0, 0), (0, 32), (0, 64)],
            1 => vec![(10, 0), (0, 32), (0, 64)],
            2 => vec![(10, 0), (0, 64)],
            _ => vec![(20, 0), (10, 64)],
        };
        for y in 0..96 {
            for x in 0..80 {
                let white = expected
                    .iter()
                    .any(|(sx, sy)| (*sx..*sx + 32).contains(&x) && (*sy..*sy + 32).contains(&y));
                assert_eq!(
                    renderer.frame().pixel(x, y),
                    Some(if white { [255; 4] } else { [0; 4] })
                );
            }
        }
        let mut png = Vec::new();
        renderer.frame().write_png(&mut png).unwrap();
        frames.push(png);
    }
    frames
}
#[test]
fn three_mates_mock_events_removal_and_full_pixel_png_replay() {
    let pkg = package();
    assert_eq!(replay(pkg.clone()), replay(pkg));
}
