use super::package::Fixture;
use crate::*;
use mochi_package::LoadedPackage;
use mochi_runtime::{CapabilityStatus, RuntimeError};
use std::{fs, sync::Arc, time::Duration};

fn fixture(rules: &str) -> Fixture {
    let f = Fixture::new();
    f.replace(
        "manifest.toml",
        "mochi.behavior.wander",
        "mochi.behavior.rules",
    );
    f.replace("manifest.toml", "behavior.toml", "rules.toml");
    fs::remove_file(f.0.join("behavior.toml")).unwrap();
    fs::write(f.0.join("rules.toml"), rules).unwrap();
    f
}
fn one(on: &str, actions: &str) -> String {
    format!("schema_version=1\n[[rules]]\non={{{on}}}\nactions=[{actions}]\n")
}
fn instantiate(f: &Fixture) -> Runtime {
    LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("rules")
        .unwrap()
}
fn custom(name: &str) -> MateEvent {
    MateEvent::Custom {
        name: ns(name),
        payload: Value::Bool(true),
    }
}
fn click(button: u16) -> MateEvent {
    MateEvent::PointerClick {
        position: Vec2::new(12.0, 15.0),
        button,
    }
}
fn interactive_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/interactive-neko/neko.mochi")
}
#[test]
fn every_standard_event_can_be_matched_without_reading_platform_data() {
    let cases = [
        (
            "event='tick'",
            MateEvent::Tick {
                delta: Duration::ZERO,
            },
        ),
        ("event='pointer_enter'", MateEvent::PointerEnter),
        ("event='pointer_leave'", MateEvent::PointerLeave),
        ("event='pointer_click',button=2", click(2)),
        (
            "event='drag_start'",
            MateEvent::DragStart {
                position: Vec2::default(),
            },
        ),
        (
            "event='drag_move'",
            MateEvent::DragMove {
                position: Vec2::default(),
            },
        ),
        (
            "event='drag_end'",
            MateEvent::DragEnd {
                position: Vec2::default(),
            },
        ),
        (
            "event='screen_changed'",
            MateEvent::ScreenChanged {
                origin: Vec2::default(),
                size: Vec2::new(100.0, 100.0),
                scale_factor: 2.0,
            },
        ),
        (
            "event='timer',id='neko.alarm'",
            MateEvent::Timer {
                id: ns("neko.alarm"),
            },
        ),
        (
            "event='custom',name='ros2.node_offline'",
            custom("ros2.node_offline"),
        ),
    ];
    for (on, event) in cases {
        let f = fixture(&one(on, "{type='move_by',x=2.0,y=3.0}"));
        let step = instantiate(&f).process(event).unwrap();
        assert_eq!(step.state.position, Vec2::new(2.0, 3.0), "{on}");
        assert_eq!(step.trace[0].actions.len(), 1);
    }
}
#[test]
fn names_buttons_and_state_conditions_match_exactly() {
    let f=fixture("schema_version=1\n[[rules]]\non={event='pointer_click',button=1}\nwhen_state='idle'\nactions=[{type='set_current_state',name='happy'}]\n[[rules]]\non={event='pointer_click',button=1}\nwhen_state='happy'\nactions=[{type='set_current_state',name='sad'}]");
    let mut mate = instantiate(&f);
    assert!(mate.process(click(2)).unwrap().trace[0].actions.is_empty());
    assert!(mate.process(custom("pointer.click")).unwrap().trace[0]
        .actions
        .is_empty());
    assert_eq!(mate.process(click(1)).unwrap().state.current_state, "happy");
    // The second rule sees the next event's state, not the first rule's actions.
    assert_eq!(mate.process(click(1)).unwrap().state.current_state, "sad");
    assert!(mate.process(click(1)).unwrap().trace[0].actions.is_empty());
    for on in [
        "event='custom',name='git.dirty'",
        "event='timer',id='git.dirty'",
    ] {
        let f = fixture(&one(on, "{type='play_animation',name='marked'}"));
        let mut mate = instantiate(&f);
        for event in [
            custom("git.dirty.extra"),
            MateEvent::Timer {
                id: ns("git.clean"),
            },
        ] {
            assert!(mate.process(event).unwrap().trace[0].actions.is_empty());
        }
    }
}
#[test]
fn all_matching_rules_and_actions_keep_file_order() {
    let f=fixture("schema_version=1\n[[rules]]\non={event='pointer_enter'}\nactions=[{type='move_to',x=8.0,y=2.0},{type='set_state',key='neko.mood',value='first'}]\n[[rules]]\non={event='pointer_enter'}\nactions=[{type='move_by',x=3.0,y=-1.0},{type='set_state',key='neko.mood',value='last'},{type='set_velocity',x=5.0,y=6.0},{type='play_animation',name='walk'}]");
    let step = instantiate(&f).process(MateEvent::PointerEnter).unwrap();
    assert_eq!(step.trace[0].actions.len(), 6);
    assert_eq!(step.state.position, Vec2::new(11.0, 1.0));
    assert_eq!(step.state.velocity, Vec2::new(5.0, 6.0));
    assert_eq!(
        step.state.values[&ns("neko.mood")],
        Value::String("last".into())
    );
    assert_eq!(step.state.current_animation, "walk");
}
#[test]
fn emitted_events_observe_updated_state_and_keep_fifo_order() {
    let f=fixture("schema_version=1\n[[rules]]\non={event='pointer_enter'}\nactions=[{type='emit_event',name='neko.a',payload={}},{type='emit_event',name='neko.b',payload={}},{type='set_current_state',name='ready'}]\n[[rules]]\non={event='custom',name='neko.a'}\nwhen_state='ready'\nactions=[{type='move_to',x=2.0,y=0.0}]\n[[rules]]\non={event='custom',name='neko.b'}\nactions=[{type='move_by',x=3.0,y=0.0}]");
    let step = instantiate(&f).process(MateEvent::PointerEnter).unwrap();
    assert_eq!(step.trace.len(), 3);
    assert!(matches!(&step.trace[1].event,MateEvent::Custom{name,..} if name.as_str()=="neko.a"));
    assert!(matches!(&step.trace[2].event,MateEvent::Custom{name,..} if name.as_str()=="neko.b"));
    assert_eq!(step.state.position.x, 5.0);
}
#[test]
fn looping_rules_roll_back_state_time_and_capability_numbering() {
    let request =
        "{type='request_capability',capability='host.echo',scope='demo',payload='literal'}";
    let f=fixture(&format!("schema_version=1\n[[rules]]\non={{event='tick'}}\nactions=[{{type='set_state',key='neko.changed',value=true}},{{type='emit_event',name='neko.loop',payload={{}}}}]\n[[rules]]\non={{event='custom',name='neko.loop'}}\nactions=[{request},{{type='emit_event',name='neko.loop',payload={{}}}}]\n[[rules]]\non={{event='pointer_enter'}}\nactions=[{request}]"));
    let mut mate = instantiate(&f);
    let before = mate.state().clone();
    assert_eq!(
        mate.process(MateEvent::Tick {
            delta: Duration::from_secs(1)
        }),
        Err(RuntimeError::EventLimit)
    );
    assert_eq!(mate.state(), &before);
    assert_eq!(mate.elapsed(), Duration::ZERO);
    let step = mate.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(step.capability_requests[0].sequence, 0);
    assert_eq!(
        step.capability_requests[0].status,
        CapabilityStatus::RejectedUndeclared
    );
}
#[test]
fn arithmetic_overflow_rolls_back_the_complete_step() {
    let f = fixture(&one(
        "event='tick'",
        "{type='set_current_state',name='changed'},{type='move_by',x=1e308,y=0.0}",
    ));
    let mut mate = instantiate(&f);
    mate.process(MateEvent::Tick {
        delta: Duration::from_secs(1),
    })
    .unwrap();
    let state = mate.state().clone();
    assert!(mate
        .process(MateEvent::Tick {
            delta: Duration::from_secs(1)
        })
        .is_err());
    assert_eq!(mate.state(), &state);
    assert_eq!(mate.elapsed(), Duration::from_secs(1));
}
#[test]
fn rule_limits_reject_excess_and_accept_the_reserved_animation_slot() {
    let action = "{type='move_by',x=1.0,y=0.0}";
    for sizes in [vec![], vec![1; 65], vec![0], vec![17], vec![16; 8]] {
        let mut text = "schema_version=1\n".to_string();
        if sizes.is_empty() {
            text.push_str("rules=[]\n");
        }
        for size in sizes {
            text.push_str(&format!(
                "[[rules]]\non={{event='pointer_enter'}}\nactions=[{}]\n",
                vec![action; size].join(",")
            ));
        }
        assert!(LoadedPackage::load(&fixture(&text).0).is_err());
    }
    let mut text = "schema_version=1\n".to_string();
    for size in [16, 16, 16, 16, 16, 16, 16, 15] {
        text.push_str(&format!(
            "[[rules]]\non={{event='pointer_enter'}}\nactions=[{}]\n",
            vec![action; size].join(",")
        ));
    }
    let f = fixture(&text);
    let step = instantiate(&f).process(MateEvent::PointerEnter).unwrap();
    assert_eq!(step.trace[0].actions.len(), 127);
    assert_eq!(step.state.position.x, 127.0);
    // Exercise the exact Runtime ceiling with the animation adapter's extra action.
    f.replace("rules.toml", "event='pointer_enter'", "event='tick'");
    let path = f.0.join("rules.toml");
    let text = fs::read_to_string(&path).unwrap().replacen(
        action,
        "{type='play_animation',name='walk'}",
        1,
    );
    fs::write(&path, text).unwrap();
    f.replace(
        "manifest.toml",
        "mochi.appearance.sprite",
        "mochi.appearance.animated-sprite",
    );
    f.replace(
        "manifest.toml",
        "asset = \"sprites/neko.png\"",
        "file = \"animations.toml\"",
    );
    for file in [
        "animations.toml",
        "sprites/frame-0.png",
        "sprites/frame-1.png",
        "sprites/frame-2.png",
    ] {
        fs::copy(interactive_path().join(file), f.0.join(file)).unwrap();
    }
    let step = instantiate(&f)
        .process(MateEvent::Tick {
            delta: Duration::from_millis(100),
        })
        .unwrap();
    assert_eq!(step.trace[0].actions.len(), 128);
    assert_eq!(step.state.position.x, 126.0);
    assert_eq!(step.state.current_animation, "walk");

    let mut text = "schema_version=1\n".to_string();
    for _ in 0..64 {
        text.push_str(&format!(
            "[[rules]]\non={{event='pointer_enter'}}\nactions=[{action}]\n"
        ));
    }
    let f = fixture(&text);
    assert_eq!(
        instantiate(&f)
            .process(MateEvent::PointerEnter)
            .unwrap()
            .trace[0]
            .actions
            .len(),
        64
    );
}
#[test]
fn strict_wire_schema_and_unknown_fields_are_rejected() {
    let valid = one(
        "event='pointer_enter'",
        "{type='play_animation',name='wave'}",
    );
    for (from, to) in [
        ("schema_version=1", "schema_version=2"),
        ("schema_version=1", "schema_version=1\nscript='bad'"),
        ("[[rules]]", "[[rules]]\npriority=1"),
        ("event='pointer_enter'", "event='pointer_enter',button=1"),
        ("event='pointer_enter'", "event='pointer_click'"),
        (
            "event='pointer_enter'",
            "event='pointer_click',button=65536",
        ),
        ("event='pointer_enter'", "event='custom',name='bad'"),
        ("event='pointer_enter'", "event='tick',delta_ms=1"),
        (
            "event='pointer_enter'",
            "event='timer',id='neko.alarm',repeat=true",
        ),
        ("name='wave'", "name='wave',script='bad'"),
        ("[[rules]]", "[[rules]]\nwhen_state=''"),
    ] {
        let f = fixture(&valid.replace(from, to));
        assert!(LoadedPackage::load(&f.0).is_err(), "{to}");
    }
}
#[test]
fn unsupported_code_dynamic_binding_and_invalid_action_data_are_rejected() {
    for action in [
        "{type='shell',command='true'}",
        "{type='python',code='pass'}",
        "{type='javascript',code='1'}",
        "{type='move_to',position='event.position'}",
        "{type='move_by',x=nan,y=0.0}",
        "{type='set_velocity',x=inf,y=0.0}",
        "{type='set_state',key='bad',value=true}",
        "{type='set_state',key='neko.x',value={nested=nan}}",
        "{type='play_animation',name=' '}",
        "{type='set_current_state',name=''}",
        "{type='emit_event',name='bad',payload={}}",
        "{type='request_capability',capability='host.echo',scope='',payload={}}",
    ] {
        assert!(
            LoadedPackage::load(&fixture(&one("event='pointer_enter'", action)).0).is_err(),
            "{action}"
        );
    }
    let deep = format!(
        "{{type='set_state',key='neko.x',value={}0{}}}",
        "[".repeat(34),
        "]".repeat(34)
    );
    assert!(LoadedPackage::load(&fixture(&one("event='pointer_enter'", &deep)).0).is_err());
}
#[test]
fn rules_cannot_write_internal_state_or_emit_host_protocol_messages() {
    for key in [
        "mochi.animation.playback",
        "mochi.wander.direction",
        "mochi.future.key",
    ] {
        assert!(LoadedPackage::load(
            &fixture(&one(
                "event='tick'",
                &format!("{{type='set_state',key='{key}',value=1}}")
            ))
            .0
        )
        .is_err());
    }
    for (name, payload) in [
        ("mochi.capability.result", "{}"),
        ("mochi.future.event", "{}"),
        ("mochi.animation.restart", "true"),
        ("mochi.animation.restart", "{unexpected=1}"),
    ] {
        assert!(LoadedPackage::load(
            &fixture(&one(
                "event='pointer_enter'",
                &format!("{{type='emit_event',name='{name}',payload={payload}}}")
            ))
            .0
        )
        .is_err());
    }
    let f = fixture(&one(
        "event='pointer_enter'",
        "{type='set_state',key='neko.literal',value='$(echo hello)'}",
    ));
    let step = instantiate(&f).process(MateEvent::PointerEnter).unwrap();
    assert_eq!(
        step.state.values[&ns("neko.literal")],
        Value::String("$(echo hello)".into())
    );
}
#[test]
fn rules_file_binding_missing_unused_and_mixed_behaviors_are_rejected() {
    let source = one("event='tick'", "{type='move_by',x=1.0,y=0.0}");
    let f = fixture(&source);
    fs::remove_file(f.0.join("rules.toml")).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    for path in [
        "../rules.toml",
        "other.toml",
        "https://example.com/rules.toml",
    ] {
        let f = fixture(&source);
        f.replace("manifest.toml", "rules.toml", path);
        assert!(LoadedPackage::load(&f.0).is_err());
    }
    let f = Fixture::new();
    fs::write(f.0.join("rules.toml"), &source).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    let f = fixture(&source);
    fs::write(f.0.join("behavior.toml"), "schema_version=1").unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    let f = fixture(&source);
    let text = fs::read_to_string(f.0.join("manifest.toml")).unwrap()
        + "\n[[components]]\nid='mochi.behavior.idle'\n";
    fs::write(f.0.join("manifest.toml"), text).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
}
#[test]
fn rules_size_limit_hash_and_snapshot_are_preserved() {
    let f = fixture(&one("event='tick'", "{type='move_by',x=1.0,y=0.0}"));
    let first = LoadedPackage::load(&f.0).unwrap();
    f.replace("rules.toml", "x=1.0", "x=2.0");
    let second = LoadedPackage::load(&f.0).unwrap();
    assert_ne!(first.content_hash(), second.content_hash());
    assert_eq!(first.assets().len(), 1);
    fs::write(f.0.join("rules.toml"), " ".repeat(65537)).unwrap();
    assert!(LoadedPackage::load(&f.0).is_err());
    fs::remove_dir_all(&f.0).unwrap();
    let tick = MateEvent::Tick {
        delta: Duration::from_secs(1),
    };
    assert_eq!(
        first
            .instantiate("old")
            .unwrap()
            .process(tick.clone())
            .unwrap()
            .state
            .position
            .x,
        1.0
    );
    assert_eq!(
        second
            .instantiate("new")
            .unwrap()
            .process(tick)
            .unwrap()
            .state
            .position
            .x,
        2.0
    );
}
#[test]
fn clicks_switch_animation_same_event_and_replay_through_mock_platform() {
    use mochi_platform::{pump_once, MockPlatform, Renderer};
    let p = LoadedPackage::load(interactive_path()).unwrap();
    let mut a = p.instantiate("a").unwrap();
    let mut replay = p.instantiate("a").unwrap();
    let mut platform = MockPlatform::default();
    for event in [
        MateEvent::PointerEnter,
        click(1),
        MateEvent::Tick {
            delta: Duration::from_millis(150),
        },
        MateEvent::Timer {
            id: ns("neko.replay"),
        },
        custom("git.dirty"),
        MateEvent::PointerLeave,
    ] {
        platform.push_event(event);
    }
    let mut steps = vec![];
    while let Some(step) = pump_once(&mut a, &mut platform).unwrap() {
        assert_eq!(step, replay.process(step.trace[0].event.clone()).unwrap());
        assert!(step.capability_requests.is_empty());
        platform.render("a", &step.scene).unwrap();
        steps.push(step);
    }
    assert_eq!(platform.frames.len(), 6);
    assert_eq!(steps[1].state.current_animation, "wave");
    assert_eq!(steps[1].state.current_state, "greeting");
    assert_eq!(steps[1].trace.len(), 2);
    assert_eq!(
        steps[1].state.values[&ns("neko.mood")],
        Value::String("happy".into())
    );
    let asset = |step: &mochi_runtime::StepOutput| match &step.scene.nodes[0].content {
        SceneContent::Sprite { asset, .. } => asset.clone(),
        _ => panic!("expected sprite"),
    };
    assert_eq!(asset(&steps[1]), "sprites/frame-0.png");
    assert_eq!(asset(&steps[2]), "sprites/frame-2.png");
    assert_eq!(asset(&steps[3]), "sprites/frame-0.png");
    assert_eq!(steps[4].state.values[&ns("git.dirty")], Value::Bool(true));
    assert_eq!(steps[5].state.current_animation, "idle");
}
#[test]
fn host_instances_keep_rule_state_and_animation_progress_isolated() {
    use mochi_host::{Host, Presentation};
    let mut host = Host::default();
    let p = Arc::new(LoadedPackage::load(interactive_path()).unwrap());
    let a = host.spawn("a", p.clone(), Presentation::default()).unwrap();
    let b = host.spawn("b", p, Presentation::default()).unwrap();
    let _ = host.dispatch(&a, MateEvent::PointerEnter).unwrap();
    let _ = host.dispatch(&a, click(1)).unwrap();
    let _ = host
        .dispatch(
            &a,
            MateEvent::Tick {
                delta: Duration::from_millis(150),
            },
        )
        .unwrap();
    assert_eq!(
        host.get(&a).unwrap().runtime().state().current_state,
        "greeting"
    );
    assert_eq!(
        host.get(&b).unwrap().runtime().state().current_state,
        "idle"
    );
    assert_eq!(host.get(&b).unwrap().runtime().elapsed(), Duration::ZERO);
    assert!(!host
        .get(&b)
        .unwrap()
        .runtime()
        .state()
        .values
        .contains_key(&ns("neko.mood")));
}

#[test]
fn capability_rules_only_run_handlers_after_explicit_host_policy_and_grant() {
    use mochi_broker::{
        CapabilityHandler, Denial, HandlerResult, Invocation, Outcome, Rule, Submission,
    };
    use mochi_host::{Host, Presentation};
    use std::{cell::Cell, rc::Rc};
    struct Count(Rc<Cell<usize>>);
    impl CapabilityHandler for Count {
        fn start(&mut self, call: &Invocation<'_>) -> HandlerResult {
            self.0.set(self.0.get() + 1);
            HandlerResult::Complete(Ok(call.request.payload.clone()))
        }
    }
    let f = fixture("schema_version=1\n[[rules]]\non={event='pointer_click',button=1}\nactions=[{type='request_capability',capability='host.echo',scope='demo',payload={text='literal'}}]\n[[rules]]\non={event='custom',name='mochi.capability.result'}\nactions=[{type='set_state',key='neko.reply_seen',value=true}]");
    f.replace(
        "manifest.toml",
        "capabilities = []",
        "capabilities = ['host.echo']",
    );
    f.replace(
        "manifest.toml",
        "permissions = []",
        "permissions = [{capability='host.echo',scope='demo'}]",
    );
    let mut host = Host::default();
    let calls = Rc::new(Cell::new(0));
    host.register(ns("host.echo"), Box::new(Count(calls.clone())))
        .unwrap();
    let handle = host
        .spawn(
            "rules",
            Arc::new(LoadedPackage::load(&f.0).unwrap()),
            Presentation::default(),
        )
        .unwrap();
    let step = host.dispatch(&handle, click(1)).unwrap();
    assert_eq!(step.requests.len(), 1);
    assert_eq!(
        step.requests[0].intent().status,
        CapabilityStatus::AwaitingHostDecision
    );
    assert_eq!(calls.get(), 0);
    let Submission::Completed(reply) = host
        .submit(&step.requests[0], Duration::from_secs(1))
        .unwrap()
    else {
        panic!("expected denial")
    };
    assert_eq!(reply.outcome(), &Outcome::Denied(Denial::HostPolicyDenied));
    assert_eq!(calls.get(), 0);
    let rule = Rule::new(ns("host.echo"), "demo").unwrap();
    host.allow(rule.clone()).unwrap();
    let step = host.dispatch(&handle, click(1)).unwrap();
    let Submission::Completed(reply) = host
        .submit(&step.requests[0], Duration::from_secs(1))
        .unwrap()
    else {
        panic!("expected denial")
    };
    assert_eq!(reply.outcome(), &Outcome::Denied(Denial::NotGranted));
    assert_eq!(calls.get(), 0);
    host.grant(&handle, rule).unwrap();
    let step = host.dispatch(&handle, click(1)).unwrap();
    assert_eq!(calls.get(), 0);
    let Submission::Completed(reply) = host
        .submit(&step.requests[0], Duration::from_secs(1))
        .unwrap()
    else {
        panic!("expected echo")
    };
    assert_eq!(calls.get(), 1);
    assert!(matches!(reply.outcome(), Outcome::Succeeded(_)));
    let returned = host.deliver(&reply).unwrap();
    assert!(returned.requests.is_empty());
    assert_eq!(
        host.get(&handle).unwrap().runtime().state().values[&ns("neko.reply_seen")],
        Value::Bool(true)
    );
}
