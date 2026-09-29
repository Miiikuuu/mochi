//! Pure event → behavior → action → state → appearance → scene contracts.
mod action;
mod behavior;
mod event;
mod gaze;
mod pose;
mod scene;
mod state;

pub use action::*;
pub use behavior::*;
pub use event::*;
pub use gaze::*;
pub use mochi_schema::{Namespace, SchemaError, Value};
pub use pose::*;
pub use scene::*;
pub use state::*;

/// Backend-independent textured triangle data. No model runtime dependency.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    pub page: usize,
    pub vertices: Vec<[f32; 12]>,
    pub indices: Vec<usize>,
}
