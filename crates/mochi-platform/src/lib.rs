//! Host-facing adapters. Native events must be translated before entering Core.
use mochi_core::{MateEvent, Scene};
use mochi_runtime::{Runtime, RuntimeError, StepOutput};
use std::collections::VecDeque;
use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub enum PlatformError {
    Runtime(RuntimeError),
    Adapter(String),
}
impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(e) => e.fmt(f),
            Self::Adapter(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for PlatformError {}
impl From<RuntimeError> for PlatformError {
    fn from(e: RuntimeError) -> Self {
        Self::Runtime(e)
    }
}

pub trait EventSource {
    /// Non-blocking: None means no event currently available, not permanent EOF.
    fn next_event(&mut self) -> Result<Option<MateEvent>, PlatformError>;
}
pub trait Renderer {
    /// A real backend must explicitly reject unsupported Scene primitives.
    fn render(&mut self, instance_id: &str, scene: &Scene) -> Result<(), PlatformError>;
}

/// Advance the runtime only. Returning the output before rendering keeps
/// capability intents available even if a renderer later fails.
pub fn pump_once(
    runtime: &mut Runtime,
    source: &mut impl EventSource,
) -> Result<Option<StepOutput>, PlatformError> {
    match source.next_event()? {
        Some(event) => Ok(Some(runtime.process(event)?)),
        None => Ok(None),
    }
}

#[derive(Default)]
pub struct MockPlatform {
    events: VecDeque<MateEvent>,
    pub frames: Vec<(String, Scene)>,
}
impl MockPlatform {
    pub fn push_event(&mut self, event: MateEvent) {
        self.events.push_back(event);
    }
}
impl EventSource for MockPlatform {
    fn next_event(&mut self) -> Result<Option<MateEvent>, PlatformError> {
        Ok(self.events.pop_front())
    }
}
impl Renderer for MockPlatform {
    fn render(&mut self, instance_id: &str, scene: &Scene) -> Result<(), PlatformError> {
        scene
            .validate()
            .map_err(|e| PlatformError::Adapter(e.to_string()))?;
        self.frames.push((instance_id.into(), scene.clone()));
        Ok(())
    }
}

/// Test helper; Runtime itself reads no clock. Advancing only creates a Tick.
#[derive(Default)]
pub struct MockClock {
    now: Duration,
}
impl MockClock {
    pub fn now(&self) -> Duration {
        self.now
    }
    pub fn advance(&mut self, delta: Duration) -> Result<MateEvent, PlatformError> {
        self.now = self
            .now
            .checked_add(delta)
            .ok_or_else(|| PlatformError::Adapter("mock clock overflow".into()))?;
        Ok(MateEvent::Tick { delta })
    }
}
