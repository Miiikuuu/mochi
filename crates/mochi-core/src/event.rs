use crate::{Namespace, SchemaError, Value, Vec2};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MateEvent {
    Tick {
        delta: Duration,
    },
    PointerEnter,
    PointerLeave,
    PointerClick {
        position: Vec2,
        button: u16,
    },
    DragStart {
        position: Vec2,
    },
    DragMove {
        position: Vec2,
    },
    DragEnd {
        position: Vec2,
    },
    ScreenChanged {
        origin: Vec2,
        size: Vec2,
        scale_factor: f64,
    },
    Timer {
        id: Namespace,
    },
    Custom {
        name: Namespace,
        payload: Value,
    },
}
impl MateEvent {
    pub fn validate(&self) -> Result<(), SchemaError> {
        match self {
            Self::PointerClick { position, .. }
            | Self::DragStart { position }
            | Self::DragMove { position }
            | Self::DragEnd { position } => position.validate(),
            Self::ScreenChanged {
                origin,
                size,
                scale_factor,
            } => {
                origin.validate()?;
                size.validate()?;
                if size.x <= 0.0
                    || size.y <= 0.0
                    || !scale_factor.is_finite()
                    || *scale_factor <= 0.0
                {
                    return Err(SchemaError(
                        "screen dimensions and scale must be positive and finite".into(),
                    ));
                }
                Ok(())
            }
            Self::Custom { payload, .. } => payload.validate(),
            _ => Ok(()),
        }
    }
}
