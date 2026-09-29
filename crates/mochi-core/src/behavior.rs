use crate::{MateAction, MateEvent, MateState, Namespace, SchemaError, Value, Vec2};
use std::time::Duration;

pub struct MateContext<'a> {
    pub instance_id: &'a str,
    pub state: &'a MateState,
    /// Logical time after incorporating the current Tick, never wall time.
    pub elapsed: Duration,
}

/// Trusted host code must be pure: no I/O, clocks, randomness or interior state.
/// This trait is an API boundary, NOT an in-process security sandbox.
pub trait Behavior {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction>;
}

pub struct IdleBehavior;
impl Behavior for IdleBehavior {
    fn on_event(&self, _: &MateContext<'_>, _: &MateEvent) -> Vec<MateAction> {
        Vec::new()
    }
}

/// Deterministic horizontal patrol. Direction lives in per-Mate namespaced state.
pub struct WanderBehavior {
    min_x: f64,
    max_x: f64,
    speed: f64,
    direction_key: Namespace,
}
impl WanderBehavior {
    pub fn new(min_x: f64, max_x: f64, speed: f64) -> Result<Self, SchemaError> {
        let width = max_x - min_x;
        if !min_x.is_finite()
            || !max_x.is_finite()
            || width <= 0.0
            || !(width * 2.0).is_finite()
            || !speed.is_finite()
            || speed <= 0.0
        {
            return Err(SchemaError(
                "wander requires finite min_x < max_x and positive finite speed".into(),
            ));
        }
        Ok(Self {
            min_x,
            max_x,
            speed,
            direction_key: Namespace::new("mochi.wander.direction")?,
        })
    }
}
impl Behavior for WanderBehavior {
    fn on_event(&self, context: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        let MateEvent::Tick { delta } = event else {
            return Vec::new();
        };
        if delta.is_zero() {
            return Vec::new();
        }
        let width = self.max_x - self.min_x;
        let x = context.state.position.x.clamp(self.min_x, self.max_x) - self.min_x;
        let left = matches!(
            context.state.values.get(&self.direction_key),
            Some(Value::Integer(-1))
        );
        let phase = if left { 2.0 * width - x } else { x };
        let phase = (phase + self.speed * delta.as_secs_f64()) % (2.0 * width);
        let (x, direction) = if phase < width {
            (phase, 1)
        } else {
            (2.0 * width - phase, -1)
        };
        vec![
            MateAction::MoveTo(Vec2::new(self.min_x + x, context.state.position.y)),
            MateAction::SetVelocity(Vec2::new(self.speed * direction as f64, 0.0)),
            MateAction::PlayAnimation("walk".into()),
            MateAction::SetState {
                key: self.direction_key.clone(),
                value: Value::Integer(direction),
            },
        ]
    }
}
