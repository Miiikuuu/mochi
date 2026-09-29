use crate::{CapabilityHandler, HandlerFailure, HandlerResult, Invocation};
use mochi_core::Namespace;

/// The only provided capability implementations. No I/O, timers or threads.
pub struct MockCapability {
    mode: Mode,
}
enum Mode {
    Echo,
    Fail(String),
    Pending,
}
impl MockCapability {
    pub fn echo() -> Self {
        Self { mode: Mode::Echo }
    }
    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            mode: Mode::Fail(message.into()),
        }
    }
    /// Simulates work awaiting a host completion or a logical timeout.
    pub fn pending() -> Self {
        Self {
            mode: Mode::Pending,
        }
    }
}
impl CapabilityHandler for MockCapability {
    fn start(&mut self, call: &Invocation<'_>) -> HandlerResult {
        match &self.mode {
            Mode::Echo => HandlerResult::Complete(Ok(call.request.payload.clone())),
            Mode::Fail(message) => HandlerResult::Complete(Err(HandlerFailure {
                code: Namespace::new("mock.failed").expect("constant namespace"),
                message: message.clone(),
            })),
            Mode::Pending => HandlerResult::Pending,
        }
    }
}
