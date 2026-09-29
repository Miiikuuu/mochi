use crate::{MateEvent, MateState, Namespace, SchemaError, Value, Vec2};

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityRequest {
    pub capability: Namespace,
    pub scope: String,
    pub payload: Value,
}
impl CapabilityRequest {
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.scope.trim().is_empty() || self.scope.len() > 1024 {
            return Err(SchemaError(
                "capability scope must contain 1..=1024 bytes".into(),
            ));
        }
        self.payload.validate()
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MateAction {
    MoveTo(Vec2),
    MoveBy(Vec2),
    SetVelocity(Vec2),
    PlayAnimation(String),
    SetCurrentState(String),
    SetState { key: Namespace, value: Value },
    EmitEvent(MateEvent),
    RequestCapability(CapabilityRequest),
}

/// Pure effects returned to the runtime; nothing is executed here.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Event(MateEvent),
    Capability(CapabilityRequest),
}

/// Apply one action atomically; invalid actions leave `state` unchanged.
pub fn apply_action(
    state: &mut MateState,
    action: &MateAction,
) -> Result<Option<Effect>, SchemaError> {
    match action {
        MateAction::MoveTo(position) => {
            position.validate()?;
            state.position = *position;
        }
        MateAction::MoveBy(offset) => {
            offset.validate()?;
            let next = Vec2::new(state.position.x + offset.x, state.position.y + offset.y);
            next.validate()?;
            state.position = next;
        }
        MateAction::SetVelocity(velocity) => {
            velocity.validate()?;
            state.velocity = *velocity;
        }
        MateAction::PlayAnimation(name) => {
            crate::state::validate_label(name)?;
            state.current_animation = name.clone();
        }
        MateAction::SetCurrentState(name) => {
            crate::state::validate_label(name)?;
            state.current_state = name.clone();
        }
        MateAction::SetState { key, value } => {
            value.validate()?;
            state.values.insert(key.clone(), value.clone());
        }
        MateAction::EmitEvent(event) => {
            event.validate()?;
            return Ok(Some(Effect::Event(event.clone())));
        }
        MateAction::RequestCapability(request) => {
            request.validate()?;
            return Ok(Some(Effect::Capability(request.clone())));
        }
    }
    Ok(None)
}
