//! A bounded data format compiled into existing actions, never interpreted as code.
use crate::{error, PackageError};
use mochi_core::*;
use mochi_runtime::MAX_ACTIONS_PER_EVENT;
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc};

const RANDOM_KEY: &str = "mochi.rules.random";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    #[serde(default)]
    seed: u32,
    rules: Vec<RuleSource>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSource {
    on: Trigger,
    when_state: Option<String>,
    actions: Vec<ActionSource>,
}
#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
enum Trigger {
    Tick {},
    PointerEnter {},
    PointerLeave {},
    PointerClick { button: u16 },
    DragStart {},
    DragMove {},
    DragEnd {},
    ScreenChanged {},
    Timer { id: Namespace },
    Custom { name: Namespace },
}
impl Trigger {
    fn matches(&self, event: &MateEvent) -> bool {
        match (self, event) {
            (Self::Tick {}, MateEvent::Tick { .. })
            | (Self::PointerEnter {}, MateEvent::PointerEnter)
            | (Self::PointerLeave {}, MateEvent::PointerLeave)
            | (Self::DragStart {}, MateEvent::DragStart { .. })
            | (Self::DragMove {}, MateEvent::DragMove { .. })
            | (Self::DragEnd {}, MateEvent::DragEnd { .. })
            | (Self::ScreenChanged {}, MateEvent::ScreenChanged { .. }) => true,
            (Self::PointerClick { button: expected }, MateEvent::PointerClick { button, .. }) => {
                expected == button
            }
            (Self::Timer { id: expected }, MateEvent::Timer { id }) => expected == id,
            (Self::Custom { name: expected }, MateEvent::Custom { name, .. }) => expected == name,
            _ => false,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ActionSource {
    MoveTo {
        x: f64,
        y: f64,
    },
    MoveBy {
        x: f64,
        y: f64,
    },
    SetVelocity {
        x: f64,
        y: f64,
    },
    PlayAnimation {
        name: Option<String>,
        choices: Option<Vec<String>>,
    },
    SetCurrentState {
        name: String,
    },
    SetState {
        key: Namespace,
        value: Value,
    },
    EmitEvent {
        name: Namespace,
        payload: Value,
    },
    RequestCapability {
        capability: Namespace,
        scope: String,
        payload: Value,
    },
}
impl ActionSource {
    fn compile(self) -> Result<RuleAction, PackageError> {
        let action = match self {
            Self::MoveTo { x, y } => MateAction::MoveTo(Vec2::new(x, y)),
            Self::MoveBy { x, y } => MateAction::MoveBy(Vec2::new(x, y)),
            Self::SetVelocity { x, y } => MateAction::SetVelocity(Vec2::new(x, y)),
            Self::PlayAnimation { name, choices } => match (name, choices) {
                (Some(name), None) => MateAction::PlayAnimation(name),
                (None, Some(choices)) => {
                    if choices.is_empty() || choices.len() > 16 {
                        return Err(error("animation choices require 1..=16 names"));
                    }
                    let mut seen = BTreeSet::new();
                    for name in &choices {
                        if !seen.insert(name) {
                            return Err(error("duplicate animation choice"));
                        }
                        apply_action(
                            &mut MateState::default(),
                            &MateAction::PlayAnimation(name.clone()),
                        )
                        .map_err(error)?;
                    }
                    return Ok(RuleAction::ChooseAnimation(choices));
                }
                _ => {
                    return Err(error(
                        "play_animation requires exactly one of name or choices",
                    ))
                }
            },
            Self::SetCurrentState { name } => MateAction::SetCurrentState(name),
            Self::SetState { key, value } => {
                if key.as_str().starts_with("mochi.") {
                    return Err(error("rules cannot write reserved mochi.* state keys"));
                }
                MateAction::SetState { key, value }
            }
            Self::EmitEvent { name, payload } => {
                // Do not let package rules impersonate host protocol messages.
                // Restart is the one explicitly defined package-facing command.
                if name.as_str().starts_with("mochi.")
                    && !(name.as_str() == "mochi.animation.restart"
                        && matches!(&payload, Value::Object(data) if data.is_empty()))
                {
                    return Err(error("rules cannot emit reserved mochi.* events except an empty animation restart"));
                }
                MateAction::EmitEvent(MateEvent::Custom { name, payload })
            }
            Self::RequestCapability {
                capability,
                scope,
                payload,
            } => MateAction::RequestCapability(CapabilityRequest {
                capability,
                scope,
                payload,
            }),
        };
        // Reuse the core validator. Effects remain data and are deliberately discarded.
        // Validate each action separately, not against an invented initial rule order.
        apply_action(&mut MateState::default(), &action).map_err(error)?;
        Ok(RuleAction::Fixed(action))
    }
}

enum RuleAction {
    Fixed(MateAction),
    ChooseAnimation(Vec<String>),
}

// Stable per-instance seed mixing; deliberately independent of std hashers and OS RNG.
fn initial_random(seed: u32, instance_id: &str) -> u32 {
    instance_id.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    }) ^ seed
}
fn choose(random: &mut u32, count: usize) -> usize {
    // Defined 32-bit arithmetic and high-bit range mapping; one draw per choice action.
    // This small reproducible generator is for visual variety, never security decisions.
    *random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((u64::from(*random) * count as u64) >> 32) as usize
}

struct Rule {
    on: Trigger,
    when_state: Option<String>,
    actions: Vec<RuleAction>,
}
#[derive(Clone)]
pub(crate) struct RulesBehavior {
    rules: Arc<[Rule]>,
    seed: u32,
    random_key: Namespace,
    max_actions: usize,
}
impl RulesBehavior {
    pub(crate) fn parse(text: &str) -> Result<Self, PackageError> {
        let document: Document = toml::from_str(text).map_err(error)?;
        if document.schema_version != 1 {
            return Err(error("unsupported rules schema_version"));
        }
        if document.rules.is_empty() || document.rules.len() > 64 {
            return Err(error("rules file requires 1..=64 rules"));
        }
        let mut rules = Vec::new();
        let mut count = 0;
        let mut has_choices = false;
        for (index, source) in document.rules.into_iter().enumerate() {
            count += source.actions.len();
            // Reserve one action for the existing animation adapter, even when static.
            if source.actions.is_empty()
                || source.actions.len() > 16
                || count >= MAX_ACTIONS_PER_EVENT
            {
                return Err(error("rules allow 1..=16 actions per rule, 127 in total"));
            }
            if let Some(state) = &source.when_state {
                apply_action(
                    &mut MateState::default(),
                    &MateAction::SetCurrentState(state.clone()),
                )
                .map_err(|e| error(format!("rule {} condition: {e}", index + 1)))?;
            }
            let actions: Vec<RuleAction> = source
                .actions
                .into_iter()
                .map(ActionSource::compile)
                .collect::<Result<_, _>>()
                .map_err(|e| error(format!("rule {}: {e}", index + 1)))?;
            has_choices |= actions
                .iter()
                .any(|action| matches!(action, RuleAction::ChooseAnimation(_)));
            rules.push(Rule {
                on: source.on,
                when_state: source.when_state,
                actions,
            });
        }
        if has_choices && count > MAX_ACTIONS_PER_EVENT - 2 {
            return Err(error("rules with animation choices allow 126 actions, reserving random and animation state updates"));
        }
        Ok(Self {
            rules: rules.into(),
            seed: document.seed,
            random_key: Namespace::new(RANDOM_KEY).map_err(error)?,
            max_actions: count + usize::from(has_choices),
        })
    }
    pub(crate) fn reserve_animation_actions(&self, reserved: usize) -> Result<(), PackageError> {
        if self.max_actions + reserved > MAX_ACTIONS_PER_EVENT {
            return Err(error(
                "rules exceed action budget after reserving animation transitions",
            ));
        }
        Ok(())
    }
}
impl Behavior for RulesBehavior {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        let mut random = None;
        let mut actions = Vec::new();
        for rule in self.rules.iter().filter(|rule| {
            rule.on.matches(event)
                && rule
                    .when_state
                    .as_ref()
                    .is_none_or(|state| state == &context.state.current_state)
        }) {
            for action in &rule.actions {
                match action {
                    RuleAction::Fixed(action) => actions.push(action.clone()),
                    RuleAction::ChooseAnimation(choices) => {
                        let cursor = random.get_or_insert_with(|| {
                            match context.state.values.get(&self.random_key) {
                                Some(Value::Integer(value)) => u32::try_from(*value).ok(),
                                _ => None,
                            }
                            .unwrap_or_else(|| initial_random(self.seed, context.instance_id))
                        });
                        let index = choose(cursor, choices.len());
                        actions.push(MateAction::PlayAnimation(choices[index].clone()));
                    }
                }
            }
        }
        // One state action, even if several rules drew choices. Emitted events see
        // this cursor through the normal FIFO; a failed Runtime step rolls it back.
        if let Some(random) = random {
            actions.push(MateAction::SetState {
                key: self.random_key.clone(),
                value: Value::Integer(i64::from(random)),
            });
        }
        actions
    }
}
