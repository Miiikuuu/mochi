use mochi_core::{CapabilityRequest, MateEvent, Namespace, SchemaError, Value};
use mochi_runtime::RuntimeError;
use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

/// Opaque host handle. An identity token prevents use with a different Broker.
#[derive(Debug, Clone)]
pub struct Session {
    pub(crate) owner: Arc<()>,
    pub(crate) serial: u64,
}
impl Session {
    pub fn serial(&self) -> u64 {
        self.serial
    }
}

/// Stable within one broker run; both integers are encoded as decimal strings
/// in result events so the complete u64 range is preserved by Core's Value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId {
    pub session: u64,
    pub sequence: u64,
}

/// Returned only for authorized in-flight requests, never for rejected requests.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub(crate) owner: Arc<()>,
    pub(crate) id: RequestId,
}
impl Ticket {
    pub fn id(&self) -> RequestId {
        self.id
    }
}

/// A capability/scope pair. Scope comparison is byte-exact, not prefix or glob.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rule {
    pub capability: Namespace,
    pub scope: String,
}
impl Rule {
    pub fn new(capability: Namespace, scope: impl Into<String>) -> Result<Self, BrokerError> {
        let scope = scope.into();
        validate_scope(&scope)?;
        Ok(Self { capability, scope })
    }
}
pub(crate) fn validate_scope(scope: &str) -> Result<(), BrokerError> {
    if scope.is_empty()
        || scope.len() > 1024
        || scope.trim() != scope
        || scope
            .chars()
            .any(|c| c.is_control() || c == '*' || c == '?')
    {
        return Err(BrokerError::InvalidScope);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Denial {
    InvalidRequest,
    UndeclaredCapability,
    RuntimeRejected,
    ScopeNotDeclared,
    HostPolicyDenied,
    NotGranted,
    UnsupportedCapability,
    Busy,
}
impl Denial {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::UndeclaredCapability => "undeclared_capability",
            Self::RuntimeRejected => "runtime_rejected",
            Self::ScopeNotDeclared => "scope_not_declared",
            Self::HostPolicyDenied => "host_policy_denied",
            Self::NotGranted => "not_granted",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::Busy => "busy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandlerFailure {
    pub code: Namespace,
    pub message: String,
}
pub type HandlerOutcome = Result<Value, HandlerFailure>;

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Succeeded(Value),
    Denied(Denial),
    Failed(HandlerFailure),
    TimedOut,
    Revoked,
    SessionClosed,
}
impl Outcome {
    pub fn status(&self) -> &'static str {
        match self {
            Self::Succeeded(_) => "succeeded",
            Self::Denied(_) => "denied",
            Self::Failed(_) => "failed",
            Self::TimedOut => "timed_out",
            Self::Revoked => "revoked",
            Self::SessionClosed => "session_closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    InvalidData(SchemaError),
    InvalidScope,
    ForeignHandle,
    UnknownSession,
    DuplicateInstance,
    DuplicateHandler,
    InstanceMismatch,
    SequenceExhausted,
    UnexpectedSequence { expected: u64, received: u64 },
    DeadlineOverflow,
    ClockWentBackwards,
    RequestNotPending,
    Authorization(Denial),
    WrongRecipient,
    Runtime(RuntimeError),
}
impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BrokerError {}
impl From<SchemaError> for BrokerError {
    fn from(e: SchemaError) -> Self {
        Self::InvalidData(e)
    }
}
impl From<RuntimeError> for BrokerError {
    fn from(e: RuntimeError) -> Self {
        Self::Runtime(e)
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityReply {
    pub(crate) owner: Arc<()>,
    pub(crate) id: RequestId,
    pub(crate) instance_id: String,
    pub(crate) package_id: Namespace,
    pub(crate) package_version: String,
    pub(crate) capability: Namespace,
    pub(crate) scope: String,
    pub(crate) outcome: Outcome,
}
impl CapabilityReply {
    pub fn id(&self) -> RequestId {
        self.id
    }
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
    pub fn package_id(&self) -> &Namespace {
        &self.package_id
    }
    pub fn package_version(&self) -> &str {
        &self.package_version
    }
    pub fn capability(&self) -> &Namespace {
        &self.capability
    }
    pub fn scope(&self) -> &str {
        &self.scope
    }
    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    /// A data event, not a proof of authorization. Prefer Broker::deliver to
    /// guard recipient/session routing. Event wire version is independent of
    /// package schema and Cargo package versions.
    pub fn event(&self) -> MateEvent {
        let mut fields = BTreeMap::from([
            ("schema_version".into(), Value::Integer(1)),
            ("session".into(), Value::String(self.id.session.to_string())),
            (
                "sequence".into(),
                Value::String(self.id.sequence.to_string()),
            ),
            (
                "instance_id".into(),
                Value::String(self.instance_id.clone()),
            ),
            (
                "package_id".into(),
                Value::String(self.package_id.to_string()),
            ),
            (
                "package_version".into(),
                Value::String(self.package_version.clone()),
            ),
            (
                "capability".into(),
                Value::String(self.capability.to_string()),
            ),
            ("scope".into(), Value::String(self.scope.clone())),
            ("status".into(), Value::String(self.outcome.status().into())),
        ]);
        match &self.outcome {
            Outcome::Succeeded(value) => {
                fields.insert("value".into(), value.clone());
            }
            Outcome::Denied(reason) => {
                fields.insert("reason".into(), Value::String(reason.code().into()));
            }
            Outcome::Failed(failure) => {
                fields.insert("code".into(), Value::String(failure.code.to_string()));
                fields.insert("message".into(), Value::String(failure.message.clone()));
            }
            _ => {}
        }
        MateEvent::Custom {
            name: Namespace::new("mochi.capability.result").expect("constant namespace"),
            payload: Value::Object(fields),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Submission {
    Completed(CapabilityReply),
    Pending(Ticket),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditEvent {
    Attached {
        session: u64,
        instance_id: String,
        package_id: Namespace,
        version: String,
    },
    PolicyChanged {
        rule: Rule,
        allowed: bool,
    },
    Granted {
        session: u64,
        rule: Rule,
    },
    Revoked {
        session: u64,
        rule: Rule,
    },
    Submitted {
        request: RequestId,
        capability: Namespace,
        scope: String,
    },
    Started {
        request: RequestId,
    },
    Finished {
        request: RequestId,
        status: String,
        reason: Option<String>,
    },
    Detached {
        session: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub at: Duration,
    pub event: AuditEvent,
}

/// Handler receives an authorized call. Native handlers are trusted host code;
/// they must return promptly. This interface is not a sandbox or preemption API.
pub struct Invocation<'a> {
    pub ticket: Ticket,
    pub instance_id: &'a str,
    pub request: &'a CapabilityRequest,
}
pub enum HandlerResult {
    Complete(HandlerOutcome),
    Pending,
}
pub trait CapabilityHandler {
    fn start(&mut self, call: &Invocation<'_>) -> HandlerResult;
    /// Cooperative notification; cannot undo prior effects or preempt native
    /// code. A late completion is still rejected even if cancellation is ignored.
    fn cancel(&mut self, _ticket: &Ticket) {}
}
