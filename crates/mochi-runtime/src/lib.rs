//! Synchronous deterministic event processing, independent of any GUI loop.
use mochi_core::*;
use std::collections::{BTreeSet, VecDeque};
use std::fmt;
use std::time::Duration;

pub const MAX_EVENTS_PER_STEP: usize = 64;
pub const MAX_ACTIONS_PER_EVENT: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    InvalidData(SchemaError),
    EventLimit,
    ActionLimit,
    TimeOverflow,
    SequenceOverflow,
}
impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(e) => e.fmt(f),
            Self::EventLimit => f.write_str("event budget exceeded; step rolled back"),
            Self::ActionLimit => f.write_str("action budget exceeded; step rolled back"),
            Self::TimeOverflow => f.write_str("logical clock overflow"),
            Self::SequenceOverflow => f.write_str("capability sequence overflow"),
        }
    }
}
impl std::error::Error for RuntimeError {}
impl From<SchemaError> for RuntimeError {
    fn from(e: SchemaError) -> Self {
        Self::InvalidData(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Declaration only. The host still must authorize; Runtime has no executor.
    AwaitingHostDecision,
    RejectedUndeclared,
}
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityIntent {
    pub instance_id: String,
    pub sequence: u64,
    pub request: CapabilityRequest,
    pub status: CapabilityStatus,
}
#[derive(Debug, Clone, PartialEq)]
pub struct EventTrace {
    pub event: MateEvent,
    pub actions: Vec<MateAction>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutput {
    pub trace: Vec<EventTrace>,
    pub state: MateState,
    pub scene: Scene,
    pub capability_requests: Vec<CapabilityIntent>,
    pub elapsed: Duration,
}

/// Owns one isolated Mate instance. A host can own any number of Runtimes.
pub struct Runtime {
    instance_id: String,
    state: MateState,
    behaviors: Vec<Box<dyn Behavior>>,
    appearance: Box<dyn Appearance>,
    capabilities: BTreeSet<Namespace>,
    elapsed: Duration,
    next_request: u64,
}
impl Runtime {
    pub fn new(
        instance_id: impl Into<String>,
        state: MateState,
        capabilities: BTreeSet<Namespace>,
        behaviors: Vec<Box<dyn Behavior>>,
        appearance: Box<dyn Appearance>,
    ) -> Result<Self, RuntimeError> {
        let instance_id = instance_id.into();
        if instance_id.trim().is_empty() || instance_id.len() > 256 {
            return Err(SchemaError("instance id must contain 1..=256 bytes".into()).into());
        }
        state.validate()?;
        appearance.scene(&state).validate()?;
        Ok(Self {
            instance_id,
            state,
            behaviors,
            appearance,
            capabilities,
            elapsed: Duration::ZERO,
            next_request: 0,
        })
    }
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
    pub fn state(&self) -> &MateState {
        &self.state
    }
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }
    pub fn scene(&self) -> Result<Scene, RuntimeError> {
        let scene = self.appearance.scene(&self.state);
        scene.validate()?;
        Ok(scene)
    }

    /// FIFO emitted events; behaviors see the same pre-action snapshot per event.
    /// All state, time and request numbering commit only after the whole step and
    /// final scene validate. Trusted trait implementations must obey purity.
    pub fn process(&mut self, event: MateEvent) -> Result<StepOutput, RuntimeError> {
        let mut state = self.state.clone();
        let mut elapsed = self.elapsed;
        let mut next_request = self.next_request;
        let mut queue = VecDeque::from([event]);
        let mut trace = Vec::new();
        let mut requests = Vec::new();
        while let Some(event) = queue.pop_front() {
            if trace.len() >= MAX_EVENTS_PER_STEP {
                return Err(RuntimeError::EventLimit);
            }
            event.validate()?;
            if let MateEvent::Tick { delta } = &event {
                elapsed = elapsed
                    .checked_add(*delta)
                    .ok_or(RuntimeError::TimeOverflow)?;
            }
            let context = MateContext {
                instance_id: &self.instance_id,
                state: &state,
                elapsed,
            };
            let mut actions = Vec::new();
            for behavior in &self.behaviors {
                let produced = behavior.on_event(&context, &event);
                if actions.len().saturating_add(produced.len()) > MAX_ACTIONS_PER_EVENT {
                    return Err(RuntimeError::ActionLimit);
                }
                actions.extend(produced);
            }
            for action in &actions {
                match apply_action(&mut state, action)? {
                    Some(Effect::Event(event)) => {
                        if trace.len() + 1 + queue.len() >= MAX_EVENTS_PER_STEP {
                            return Err(RuntimeError::EventLimit);
                        }
                        queue.push_back(event);
                    }
                    Some(Effect::Capability(request)) => {
                        let status = if self.capabilities.contains(&request.capability) {
                            CapabilityStatus::AwaitingHostDecision
                        } else {
                            CapabilityStatus::RejectedUndeclared
                        };
                        requests.push(CapabilityIntent {
                            instance_id: self.instance_id.clone(),
                            sequence: next_request,
                            request,
                            status,
                        });
                        next_request = next_request
                            .checked_add(1)
                            .ok_or(RuntimeError::SequenceOverflow)?;
                    }
                    None => {}
                }
            }
            trace.push(EventTrace { event, actions });
        }
        state.validate()?;
        let scene = self.appearance.scene(&state);
        scene.validate()?;
        self.state = state.clone();
        self.elapsed = elapsed;
        self.next_request = next_request;
        Ok(StepOutput {
            trace,
            state,
            scene,
            capability_requests: requests,
            elapsed,
        })
    }
}
