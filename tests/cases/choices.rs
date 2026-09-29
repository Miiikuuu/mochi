use super::package::Fixture;
use crate::*;
use mochi_package::LoadedPackage;
use mochi_runtime::RuntimeError;
use std::{fs, time::Duration};

const CHOICE: &str = "{type='play_animation',choices=['wave_a','wave_b','wave_c']}";
fn fixture(source: &str) -> Fixture {
    let f = Fixture::new();
    f.replace(
        "manifest.toml",
        "mochi.behavior.wander",
        "mochi.behavior.rules",
    );
    f.replace("manifest.toml", "behavior.toml", "rules.toml");
    fs::remove_file(f.0.join("behavior.toml")).unwrap();
    fs::write(f.0.join("rules.toml"), source).unwrap();
    f
}
fn source(actions: &str) -> String {
    format!(
        "schema_version=1\nseed=42\n[[rules]]\non={{event='pointer_enter'}}\nactions=[{actions}]\n"
    )
}
fn cursor(mate: &Runtime) -> i64 {
    match mate.state().values[&ns("mochi.rules.random")] {
        Value::Integer(value) => value,
        _ => panic!("expected integer cursor"),
    }
}
fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/interactive-neko/random.mochi")
}
fn click() -> MateEvent {
    MateEvent::PointerClick {
        position: Vec2::new(12.0, 12.0),
        button: 1,
    }
}
#[test]
fn choices_have_a_stable_golden_sequence_and_complete_replay() {
    let f = fixture(&source(CHOICE));
    let p = LoadedPackage::load(&f.0).unwrap();
    let mut a = p.instantiate("mate-a").unwrap();
    let mut replay = p.instantiate("mate-a").unwrap();
    for (expected, name) in [
        (1672796847, "wave_b"),
        (3067529282, "wave_c"),
        (2811456185, "wave_b"),
        (3884061892, "wave_c"),
        (3648264531, "wave_c"),
        (42356374, "wave_a"),
    ] {
        let step = a.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(step, replay.process(MateEvent::PointerEnter).unwrap());
        assert_eq!(cursor(&a), expected);
        assert_eq!(step.state.current_animation, name);
        assert_eq!(step.trace[0].actions.len(), 2);
        assert!(step.capability_requests.is_empty());
    }
}
#[test]
fn seed_and_instance_id_select_streams_without_sharing_progress() {
    let f = fixture(&source(CHOICE));
    let p = LoadedPackage::load(&f.0).unwrap();
    let mut a = p.instantiate("mate-a").unwrap();
    let mut b = p.instantiate("mate-b").unwrap();
    a.process(MateEvent::PointerEnter).unwrap();
    a.process(MateEvent::PointerEnter).unwrap();
    assert!(!b.state().values.contains_key(&ns("mochi.rules.random")));
    b.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(cursor(&b), 810521864);
    assert_eq!(b.state().current_animation, "wave_a");
    assert_eq!(cursor(&a), 3067529282);
    f.replace("rules.toml", "seed=42", "seed=43");
    let other = LoadedPackage::load(&f.0).unwrap();
    assert_ne!(p.content_hash(), other.content_hash());
    let mut changed = other.instantiate("mate-a").unwrap();
    changed.process(MateEvent::PointerEnter).unwrap();
    assert_ne!(cursor(&changed), 1672796847);
}
#[test]
fn unmatched_events_and_failed_conditions_do_not_consume_choices() {
    let rules = source(CHOICE).replace("[[rules]]", "[[rules]]\nwhen_state='idle'");
    let f = fixture(&rules);
    let p = LoadedPackage::load(&f.0).unwrap();
    let mut mate = p.instantiate("mate-a").unwrap();
    for event in [
        MateEvent::Tick {
            delta: Duration::from_secs(99),
        },
        click(),
        MateEvent::PointerLeave,
    ] {
        assert!(mate.process(event).unwrap().trace[0].actions.is_empty());
    }
    assert!(!mate.state().values.contains_key(&ns("mochi.rules.random")));
    mate.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(cursor(&mate), 1672796847);
    let f = fixture(&rules.replace("when_state='idle'", "when_state='missing'"));
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("mate-a")
        .unwrap();
    assert!(mate.process(MateEvent::PointerEnter).unwrap().trace[0]
        .actions
        .is_empty());
    assert!(!mate.state().values.contains_key(&ns("mochi.rules.random")));
}
#[test]
fn multiple_choices_and_emitted_events_consume_in_action_and_fifo_order() {
    let f = fixture(&format!(
        "{}\n[[rules]]\non={{event='custom',name='neko.next'}}\nactions=[{CHOICE}]",
        source(&format!(
            "{CHOICE},{{type='emit_event',name='neko.next',payload={{}}}},{CHOICE}"
        ))
    ));
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("mate-a")
        .unwrap();
    let step = mate.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(step.trace.len(), 2);
    assert_eq!(step.trace[0].actions.len(), 4); // 3 requested actions + one cursor commit
    assert!(matches!(&step.trace[0].actions[0],MateAction::PlayAnimation(name) if name=="wave_b"));
    assert!(matches!(&step.trace[0].actions[2],MateAction::PlayAnimation(name) if name=="wave_c"));
    assert!(matches!(&step.trace[1].actions[0],MateAction::PlayAnimation(name) if name=="wave_b"));
    assert_eq!(cursor(&mate), 2811456185);
}
#[test]
fn runtime_failures_roll_back_choice_progress_before_retry() {
    let fail = "{type='move_to',x=1e308,y=0.0},{type='move_by',x=1e308,y=0.0}";
    let cycle = "{type='emit_event',name='neko.fail',payload={}}";
    for tail in [fail, cycle] {
        let f = fixture(&format!(
            "{}\n[[rules]]\non={{event='custom',name='neko.fail'}}\nactions=[{CHOICE},{tail}]",
            source(CHOICE)
        ));
        let p = LoadedPackage::load(&f.0).unwrap();
        let mut a = p.instantiate("mate-a").unwrap();
        let mut replay = p.instantiate("mate-a").unwrap();
        assert_eq!(
            a.process(MateEvent::PointerEnter).unwrap(),
            replay.process(MateEvent::PointerEnter).unwrap()
        );
        let before = a.state().clone();
        let error = a
            .process(MateEvent::Custom {
                name: ns("neko.fail"),
                payload: Value::Bool(true),
            })
            .unwrap_err();
        if tail == cycle {
            assert_eq!(error, RuntimeError::EventLimit);
        }
        assert_eq!(a.state(), &before);
        assert_eq!(
            a.process(MateEvent::PointerEnter).unwrap(),
            replay.process(MateEvent::PointerEnter).unwrap()
        );
    }
}
#[test]
fn choice_and_seed_validation_rejects_ambiguous_empty_duplicate_or_oversized_input() {
    for action in [
        "{type='play_animation'}",
        "{type='play_animation',name='wave',choices=['wave']}",
        "{type='play_animation',choices=[]}",
        "{type='play_animation',choices=['a','a']}",
        "{type='play_animation',choices=[' ' ]}",
        "{type='play_animation',choices='wave'}",
        "{type='play_animation',choices=[1]}",
        "{type='play_animation',choices=['a'],weights=[1]}",
    ] {
        assert!(
            LoadedPackage::load(&fixture(&source(action)).0).is_err(),
            "{action}"
        );
    }
    let seventeen = (0..17)
        .map(|i| format!("'a{i}'"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(LoadedPackage::load(
        &fixture(&source(&format!(
            "{{type='play_animation',choices=[{seventeen}]}}"
        )))
        .0
    )
    .is_err());
    assert!(LoadedPackage::load(
        &fixture(&source(&format!(
            "{{type='play_animation',choices=['{}']}}",
            "a".repeat(257)
        )))
        .0
    )
    .is_err());
    for seed in ["-1", "4294967296", "1.5", "'42'"] {
        assert!(LoadedPackage::load(
            &fixture(&source(CHOICE).replace("seed=42", &format!("seed={seed}"))).0
        )
        .is_err());
    }
    for seed in ["0", "4294967295"] {
        LoadedPackage::load(
            &fixture(&source(CHOICE).replace("seed=42", &format!("seed={seed}"))).0,
        )
        .unwrap();
    }
}
#[test]
fn fixed_animations_keep_their_old_state_and_single_candidate_is_valid() {
    let f = fixture(&source("{type='play_animation',name='wave'}"));
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("mate-a")
        .unwrap();
    let output = mate.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(output.trace[0].actions.len(), 1);
    assert!(output.state.values.is_empty());
    let f = fixture(&source("{type='play_animation',choices=['only']}"));
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("mate-a")
        .unwrap();
    for expected in [1672796847, 3067529282] {
        mate.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(mate.state().current_animation, "only");
        assert_eq!(cursor(&mate), expected);
    }
}
#[test]
fn animation_choice_and_cursor_updates_fit_the_exact_runtime_budget() {
    fn rules(count: usize) -> String {
        let mut text = "schema_version=1\n".to_string();
        let mut actions = vec![CHOICE.to_string()];
        actions.extend((1..count).map(|_| "{type='move_by',x=1.0,y=0.0}".into()));
        for part in actions.chunks(16) {
            text.push_str(&format!(
                "[[rules]]\non={{event='pointer_enter'}}\nactions=[{}]\n",
                part.join(",")
            ));
        }
        text
    }
    assert!(LoadedPackage::load(&fixture(&rules(127)).0).is_err());
    let f = fixture(&rules(126));
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
    for path in [
        "animations.toml",
        "sprites/frame-0.png",
        "sprites/frame-1.png",
        "sprites/frame-2.png",
    ] {
        fs::copy(sample().join(path), f.0.join(path)).unwrap();
    }
    let mut mate = LoadedPackage::load(&f.0)
        .unwrap()
        .instantiate("mate-a")
        .unwrap();
    let output = mate.process(MateEvent::PointerEnter).unwrap();
    assert_eq!(output.trace[0].actions.len(), 128);
    assert_eq!(output.state.position.x, 125.0);
}
#[test]
fn chosen_clip_plays_without_redrawing_choices_and_explicit_restart_replays_it() {
    let p = LoadedPackage::load(sample()).unwrap();
    let mut mate = p.instantiate("interactive-neko").unwrap();
    let first = mate.process(click()).unwrap();
    assert_eq!(first.state.current_animation, "wave_c");
    let value = cursor(&mate);
    let tick = mate
        .process(MateEvent::Tick {
            delta: Duration::from_millis(250),
        })
        .unwrap();
    assert_eq!(tick.state.current_animation, "wave_c");
    assert_eq!(cursor(&mate), value);
    let asset = |scene: &Scene| match &scene.nodes[0].content {
        SceneContent::Sprite { asset, .. } => asset.clone(),
        _ => panic!("expected sprite"),
    };
    assert_eq!(asset(&first.scene), "sprites/frame-2.png");
    assert_eq!(asset(&tick.scene), "sprites/frame-0.png");
    let repeated = mate.process(click()).unwrap(); // The known stream draws wave_c again.
    assert_eq!(repeated.state.current_animation, "wave_c");
    assert_eq!(asset(&repeated.scene), "sprites/frame-2.png");
    assert_eq!(repeated.trace.len(), 2);
}
#[test]
fn choices_remain_usable_after_package_directory_deletion() {
    let f = fixture(&source(CHOICE));
    let p = LoadedPackage::load(&f.0).unwrap();
    fs::remove_dir_all(&f.0).unwrap();
    let mut mate = p.instantiate("mate-a").unwrap();
    assert_eq!(
        mate.process(MateEvent::PointerEnter)
            .unwrap()
            .state
            .current_animation,
        "wave_b"
    );
}
