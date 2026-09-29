//! Data-only animation recipes and pure adapters assembled by the package loader.
use crate::{error, PackageError};
use mochi_core::*;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

const PLAYBACK_KEY: &str = "mochi.animation.playback";
const RESTART_EVENT: &str = "mochi.animation.restart";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    default: String,
    clips: BTreeMap<String, ClipSource>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClipSource {
    mode: Mode,
    next: Option<String>,
    frames: Vec<FrameSource>,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Loop,
    Once,
}
#[cfg(feature = "spine")]
pub(crate) struct TimedClip {
    pub source: String,
    pub duration_ns: u64,
    pub looping: bool,
    pub next: Option<String>,
    pub tail_loop: Option<TailLoop>,
}
#[derive(Clone, Copy)]
pub(crate) struct TailLoop {
    pub start_ns: u64,
    pub half_cycle_ns: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameSource {
    asset: String,
    duration_ms: u64,
}
struct Clip {
    mode: Mode,
    next: Option<String>,
    tail_loop: Option<TailLoop>,
    // Exclusive, cumulative integer nanosecond boundaries.
    frames: Vec<(String, u64)>,
    duration_ns: u64,
}
pub(crate) struct AnimationSet {
    default: String,
    clips: BTreeMap<String, Clip>,
    key: Namespace,
}
impl AnimationSet {
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.clips.contains_key(name)
    }
    /// Shared integer clock for skeletal clips; source is an animation name,
    /// not a pre-rendered image. No frame sampling is performed here.
    #[cfg(feature = "spine")]
    pub(crate) fn timed(
        default: String,
        sources: BTreeMap<String, TimedClip>,
    ) -> Result<Self, PackageError> {
        if sources.is_empty() || sources.len() > 64 || !sources.contains_key(&default) {
            return Err(error(
                "timed animation requires 1..=64 clips and a defined default",
            ));
        }
        for (name, clip) in &sources {
            if name.trim().is_empty()
                || name.len() > 256
                || !(1..=3_600_000_000_000).contains(&clip.duration_ns)
            {
                return Err(error("invalid timed animation"));
            }
            if let Some(next) = &clip.next {
                if clip.looping || sources.get(next).is_none_or(|c| !c.looping) {
                    return Err(error("next requires a once clip and a defined loop target"));
                }
            }
            if let Some(tail) = clip.tail_loop {
                if clip.looping
                    || clip.next.is_some()
                    || tail.start_ns >= clip.duration_ns
                    || tail.half_cycle_ns == 0
                    || tail.half_cycle_ns > 3_600_000_000_000
                {
                    return Err(error("invalid timed tail loop"));
                }
            }
        }
        Ok(Self {
            default,
            clips: sources
                .into_iter()
                .map(|(name, source)| {
                    (
                        name,
                        Clip {
                            mode: if source.looping {
                                Mode::Loop
                            } else {
                                Mode::Once
                            },
                            next: source.next,
                            tail_loop: source.tail_loop,
                            frames: vec![(source.source, source.duration_ns)],
                            duration_ns: source.duration_ns,
                        },
                    )
                })
                .collect(),
            key: Namespace::new(PLAYBACK_KEY).map_err(error)?,
        })
    }
    pub(crate) fn pose(&self, state: &MateState) -> (&str, u64) {
        let clip = self.clip(&state.current_animation);
        (&clip.frames[0].0, clip.source_time(self.phase(state)))
    }
    pub(crate) fn parse(text: &str) -> Result<Self, PackageError> {
        let document: Document = toml::from_str(text).map_err(error)?;
        if document.schema_version != 1 {
            return Err(error("unsupported animation schema_version"));
        }
        if document.clips.is_empty() || document.clips.len() > 64 {
            return Err(error("animation requires 1..=64 clips"));
        }
        if !document.clips.contains_key(&document.default) {
            return Err(error("default animation must name a defined clip"));
        }
        for source in document.clips.values() {
            if let Some(next) = &source.next {
                if source.mode != Mode::Once
                    || document
                        .clips
                        .get(next)
                        .is_none_or(|clip| clip.mode != Mode::Loop)
                {
                    return Err(error("next requires a once clip and a defined loop target"));
                }
            }
        }
        let mut clips = BTreeMap::new();
        let mut count = 0;
        for (name, source) in document.clips {
            if name.trim().is_empty() || name.len() > 256 {
                return Err(error("invalid animation name"));
            }
            count += source.frames.len();
            if source.frames.is_empty() || source.frames.len() > 256 || count > 1024 {
                return Err(error(
                    "animation allows 1..=256 frames per clip, 1024 in total",
                ));
            }
            let mut end = 0;
            let mut frames = Vec::new();
            for frame in source.frames {
                if !(1..=60_000).contains(&frame.duration_ms) {
                    return Err(error("frame duration_ms must be an integer in 1..=60000"));
                }
                // At most 256 minutes per clip: comfortably fits u64 and i64.
                end += frame.duration_ms * 1_000_000;
                frames.push((frame.asset, end));
            }
            clips.insert(
                name,
                Clip {
                    mode: source.mode,
                    next: source.next,
                    tail_loop: None,
                    frames,
                    duration_ns: end,
                },
            );
        }
        Ok(Self {
            default: document.default,
            clips,
            key: Namespace::new(PLAYBACK_KEY).map_err(error)?,
        })
    }
    pub(crate) fn assets(&self) -> impl Iterator<Item = &str> {
        self.clips
            .values()
            .flat_map(|clip| clip.frames.iter().map(|(asset, _)| asset.as_str()))
    }
    pub(crate) fn reserved_actions(&self) -> usize {
        if self.clips.values().any(|clip| clip.next.is_some()) {
            2
        } else {
            1
        }
    }
    fn clip(&self, name: &str) -> &Clip {
        self.clips.get(name).unwrap_or(&self.clips[&self.default])
    }
    fn phase(&self, state: &MateState) -> u64 {
        let Some(Value::Object(cursor)) = state.values.get(&self.key) else {
            return 0;
        };
        match (cursor.get("clip"), cursor.get("phase_ns")) {
            (Some(Value::String(name)), Some(Value::Integer(phase)))
                if name == &state.current_animation && *phase >= 0 =>
            {
                let clip = self.clip(name);
                let phase = *phase as u64;
                match clip.mode {
                    Mode::Loop => phase % clip.duration_ns,
                    Mode::Once => clip.phase(phase as u128),
                }
            }
            _ => 0,
        }
    }
    fn cursor(&self, name: &str, phase: u64) -> Value {
        Value::Object(BTreeMap::from([
            ("clip".into(), Value::String(name.into())),
            ("phase_ns".into(), Value::Integer(phase as i64)),
        ]))
    }
    pub(crate) fn initial_state(&self) -> MateState {
        MateState {
            current_animation: self.default.clone(),
            values: BTreeMap::from([(self.key.clone(), self.cursor(&self.default, 0))]),
            ..MateState::default()
        }
    }
}

impl Clip {
    fn phase(&self, elapsed: u128) -> u64 {
        if let Some(tail) = self.tail_loop {
            let entry = u128::from(self.duration_ns);
            if elapsed > entry {
                let cycle = u128::from(tail.half_cycle_ns) * 2;
                return (entry + (elapsed - entry) % cycle) as u64;
            }
        }
        elapsed.min(u128::from(self.duration_ns)) as u64
    }

    fn source_time(&self, phase: u64) -> u64 {
        let Some(tail) = self.tail_loop else {
            return phase;
        };
        if phase <= self.duration_ns {
            return phase;
        }
        let cycle = tail.half_cycle_ns * 2;
        let t = (phase - self.duration_ns) % cycle;
        let toward_start = t.min(cycle - t) as f64 / tail.half_cycle_ns as f64;
        let eased = toward_start * toward_start * (3.0 - 2.0 * toward_start);
        let span = self.duration_ns - tail.start_ns;
        self.duration_ns - (span as f64 * eased).round() as u64
    }
}

pub(crate) struct AnimatedBehavior {
    pub(crate) inner: Box<dyn Behavior>,
    pub(crate) animations: Arc<AnimationSet>,
}
impl Behavior for AnimatedBehavior {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        let set = &self.animations;
        let mut phase = set.phase(context.state);
        let mut name = context.state.current_animation.as_str();
        let mut transition = None;
        if let MateEvent::Tick { delta } = event {
            let clip = set.clip(name);
            let elapsed = u128::from(phase) + delta.as_nanos();
            phase = match clip.mode {
                Mode::Loop => (elapsed % u128::from(clip.duration_ns)) as u64,
                Mode::Once => clip.phase(elapsed),
            };
            if elapsed >= u128::from(clip.duration_ns) {
                if let Some(next) = &clip.next {
                    name = next;
                    phase = ((elapsed - u128::from(clip.duration_ns))
                        % u128::from(set.clip(next).duration_ns))
                        as u64;
                    transition = Some(MateAction::PlayAnimation(next.clone()));
                }
            }
        }
        if matches!(event, MateEvent::Custom { name, payload: Value::Object(data) }
            if name.as_str() == RESTART_EVENT && data.is_empty())
        {
            phase = 0;
        }
        let mut actions = Vec::new();
        if let Some(transition) = transition {
            actions.push(transition);
        }
        let delegated = self.inner.on_event(context, event);
        // Behaviors normally share a pre-action snapshot. Inspect the delegate's
        // ordered actions here so an animation switch takes effect in this event.
        for action in &delegated {
            if let MateAction::PlayAnimation(next) = action {
                if next != name {
                    name = next;
                    phase = 0;
                }
            }
        }
        let cursor = set.cursor(name, phase);
        actions.extend(delegated);
        // This key belongs to the adapter, including on events without a Tick.
        if context.state.values.get(&set.key) != Some(&cursor)
            || actions
                .iter()
                .any(|a| matches!(a, MateAction::SetState { key, .. } if key == &set.key))
        {
            actions.push(MateAction::SetState {
                key: set.key.clone(),
                value: cursor,
            });
        }
        actions
    }
}

pub(crate) struct AnimatedAppearance {
    pub(crate) animations: Arc<AnimationSet>,
    pub(crate) size: Vec2,
}
impl Appearance for AnimatedAppearance {
    fn scene(&self, state: &MateState) -> Scene {
        let clip = self.animations.clip(&state.current_animation);
        let phase = self.animations.phase(state);
        // Once clips hold their final frame at the inclusive terminal phase.
        let index = clip
            .frames
            .partition_point(|(_, end)| *end <= phase)
            .min(clip.frames.len() - 1);
        Scene {
            nodes: vec![SceneNode {
                id: "body".into(),
                transform: Transform {
                    translation: state.position,
                    ..Transform::default()
                },
                opacity: 1.0,
                content: SceneContent::Sprite {
                    asset: clip.frames[index].0.clone(),
                    size: self.size,
                    animation: state.current_animation.clone(),
                },
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochi_runtime::{Runtime, RuntimeError};
    use std::time::Duration;

    const SOURCE: &str = r#"
        schema_version = 1
        default = 'idle'
        [clips.idle]
        mode = 'loop'
        frames = [{ asset = 'a', duration_ms = 100 }, { asset = 'b', duration_ms = 200 }]
        [clips.once]
        mode = 'once'
        frames = [{ asset = 'c', duration_ms = 100 }, { asset = 'd', duration_ms = 200 }]
    "#;
    struct Commands(Vec<MateAction>);
    impl Behavior for Commands {
        fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
            if matches!(event, MateEvent::PointerEnter) {
                self.0.clone()
            } else {
                vec![]
            }
        }
    }
    fn runtime(commands: Vec<MateAction>) -> Runtime {
        runtime_source(SOURCE, commands)
    }
    fn runtime_source(source: &str, commands: Vec<MateAction>) -> Runtime {
        let animations = Arc::new(AnimationSet::parse(source).unwrap());
        Runtime::new(
            "animated",
            animations.initial_state(),
            Default::default(),
            vec![Box::new(AnimatedBehavior {
                inner: Box::new(Commands(commands)),
                animations: animations.clone(),
            })],
            Box::new(AnimatedAppearance {
                animations,
                size: Vec2::new(1.0, 1.0),
            }),
        )
        .unwrap()
    }
    fn tick(runtime: &mut Runtime, ns: u64) {
        runtime
            .process(MateEvent::Tick {
                delta: Duration::from_nanos(ns),
            })
            .unwrap();
    }
    fn asset(runtime: &Runtime) -> String {
        match &runtime.scene().unwrap().nodes[0].content {
            SceneContent::Sprite { asset, .. } => asset.clone(),
            _ => panic!("expected sprite"),
        }
    }
    fn restart() -> MateEvent {
        MateEvent::Custom {
            name: Namespace::new(RESTART_EVENT).unwrap(),
            payload: Value::Object(BTreeMap::new()),
        }
    }
    #[test]
    fn next_clip_transitions_at_boundary_and_preserves_remainder_across_ticks() {
        let source = SOURCE.replace("mode = 'once'", "mode = 'once'\nnext = 'idle'");
        let make = || {
            let mut mate = runtime_source(&source, vec![MateAction::PlayAnimation("once".into())]);
            mate.process(MateEvent::PointerEnter).unwrap();
            mate
        };
        let mut exact = make();
        tick(&mut exact, 299_999_999);
        assert_eq!(exact.state().current_animation, "once");
        assert_eq!(asset(&exact), "d");
        tick(&mut exact, 1);
        assert_eq!(exact.state().current_animation, "idle");
        assert_eq!(asset(&exact), "a");
        let mut whole = make();
        tick(&mut whole, 750_000_000);
        tick(&mut exact, 450_000_000);
        assert_eq!(whole.state(), exact.state());
        assert_eq!(asset(&whole), "b");
        whole.process(MateEvent::PointerEnter).unwrap();
        tick(&mut whole, 100_000_000);
        whole.process(restart()).unwrap();
        assert_eq!(asset(&whole), "c");
        let mut huge = make();
        huge.process(MateEvent::Tick {
            delta: Duration::MAX,
        })
        .unwrap();
        assert_eq!(huge.state().current_animation, "idle");
        assert_eq!(
            huge.state().values[&Namespace::new(PLAYBACK_KEY).unwrap()],
            AnimationSet::parse(&source).unwrap().cursor(
                "idle",
                ((Duration::MAX.as_nanos() - 300_000_000) % 300_000_000) as u64
            )
        );
    }
    #[test]
    fn next_rejects_missing_targets_loop_sources_and_once_cycles() {
        for source in [
            SOURCE.replace("mode = 'once'", "mode = 'once'\nnext = 'missing'"),
            SOURCE.replace("mode = 'loop'", "mode = 'loop'\nnext = 'idle'"),
            SOURCE.replace("mode = 'once'", "mode = 'once'\nnext = 'once'"),
        ] {
            assert!(AnimationSet::parse(&source).is_err());
        }
    }
    #[test]
    fn unequal_durations_use_exact_half_open_boundaries_and_wrap() {
        let mut mate = runtime(vec![]);
        tick(&mut mate, 99_999_999);
        assert_eq!(asset(&mate), "a");
        tick(&mut mate, 1);
        assert_eq!(asset(&mate), "b");
        tick(&mut mate, 199_999_999);
        assert_eq!(asset(&mate), "b");
        tick(&mut mate, 1);
        assert_eq!(asset(&mate), "a");
    }
    #[test]
    fn once_holds_last_frame_and_custom_event_restarts_it() {
        let mut mate = runtime(vec![MateAction::PlayAnimation("once".into())]);
        mate.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(asset(&mate), "c");
        tick(&mut mate, 100_000_000);
        assert_eq!(asset(&mate), "d");
        tick(&mut mate, 9_000_000_000);
        let terminal = mate.state().clone();
        tick(&mut mate, 1_000_000_000);
        assert_eq!(mate.state(), &terminal);
        mate.process(restart()).unwrap();
        assert_eq!(asset(&mate), "c");
        assert_eq!(mate.elapsed(), Duration::from_millis(10_100));
    }
    #[test]
    fn repeated_play_continues_but_ordered_switch_away_and_back_resets() {
        let mut same = runtime(vec![MateAction::PlayAnimation("idle".into())]);
        tick(&mut same, 150_000_000);
        same.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(asset(&same), "b");
        let mut switches = runtime(vec![
            MateAction::PlayAnimation("once".into()),
            MateAction::PlayAnimation("idle".into()),
        ]);
        tick(&mut switches, 150_000_000);
        switches.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(asset(&switches), "a");
    }
    #[test]
    fn unknown_clip_uses_default_without_changing_the_requested_label() {
        let mut mate = runtime(vec![MateAction::PlayAnimation("git.dirty".into())]);
        tick(&mut mate, 150_000_000);
        mate.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(mate.state().current_animation, "git.dirty");
        assert_eq!(asset(&mate), "a");
        tick(&mut mate, 100_000_000);
        assert_eq!(asset(&mate), "b");
    }
    #[test]
    fn injected_tick_partition_and_huge_delta_preserve_integer_phase() {
        let mut whole = runtime(vec![]);
        let mut parts = runtime(vec![]);
        tick(&mut whole, 257_000_003);
        for n in [1, 56_999_999, 200_000_003] {
            tick(&mut parts, n);
        }
        assert_eq!(whole.state(), parts.state());
        assert_eq!(whole.scene().unwrap(), parts.scene().unwrap());
        assert_eq!(whole.elapsed(), parts.elapsed());
        let mut huge = runtime(vec![]);
        huge.process(MateEvent::Tick {
            delta: Duration::MAX,
        })
        .unwrap();
        let expected = (Duration::MAX.as_nanos() % 300_000_000) as u64;
        let mut small = runtime(vec![]);
        tick(&mut small, expected);
        assert_eq!(huge.state(), small.state());
        let before = huge.state().clone();
        assert_eq!(
            huge.process(MateEvent::Tick {
                delta: Duration::from_nanos(1)
            }),
            Err(RuntimeError::TimeOverflow)
        );
        assert_eq!(huge.state(), &before);
    }
    #[test]
    fn emitted_restart_is_fifo_and_invalid_action_rolls_back_playback() {
        let mut mate = runtime(vec![MateAction::EmitEvent(restart())]);
        tick(&mut mate, 150_000_000);
        let step = mate.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(step.trace.len(), 2);
        assert_eq!(asset(&mate), "a");
        let mut broken = runtime(vec![
            MateAction::PlayAnimation("once".into()),
            MateAction::MoveTo(Vec2::new(f64::NAN, 0.0)),
        ]);
        tick(&mut broken, 150_000_000);
        let before = broken.state().clone();
        assert!(broken.process(MateEvent::PointerEnter).is_err());
        assert_eq!(broken.state(), &before);
        assert_eq!(asset(&broken), "b");
    }
    #[test]
    fn reserved_cursor_is_owned_by_adapter_and_malformed_state_falls_back() {
        let set = AnimationSet::parse(SOURCE).unwrap();
        let mut state = set.initial_state();
        state.values.insert(set.key.clone(), Value::Integer(-10));
        assert_eq!(set.phase(&state), 0);
        let mut mate = runtime(vec![MateAction::SetState {
            key: set.key.clone(),
            value: Value::Bool(true),
        }]);
        tick(&mut mate, 150_000_000);
        let before = mate.state().clone();
        mate.process(MateEvent::PointerEnter).unwrap();
        assert_eq!(mate.state(), &before);
    }
    #[test]
    fn unrelated_events_and_invalid_restart_payload_do_not_advance_animation() {
        let mut mate = runtime(vec![]);
        tick(&mut mate, 150_000_000);
        let before = mate.state().clone();
        for name in ["agent.thinking", RESTART_EVENT] {
            let output = mate
                .process(MateEvent::Custom {
                    name: Namespace::new(name).unwrap(),
                    payload: Value::Bool(true),
                })
                .unwrap();
            assert!(output.trace[0].actions.is_empty());
            assert_eq!(&output.state, &before);
        }
        tick(&mut mate, 0);
        assert_eq!(mate.state(), &before);
    }
}
