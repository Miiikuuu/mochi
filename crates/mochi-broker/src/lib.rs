//! Host-side authorization and request lifecycle. Core/Runtime never depend on
//! this crate. Only mock handlers are shipped; there is no OS capability backend.
mod mock;
mod types;
pub use mock::*;
pub use types::*;

use mochi_core::{CapabilityRequest, Namespace, Value};
use mochi_runtime::{CapabilityIntent, CapabilityStatus, Runtime, StepOutput};
use mochi_schema::MateManifest;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::Duration,
};

pub const MAX_PENDING: usize = 64;
pub const MAX_AUDIT_RECORDS: usize = 256;

struct Binding {
    instance_id: String,
    manifest: MateManifest,
    grants: BTreeSet<Rule>,
    next_sequence: u64,
}
struct Pending {
    request: CapabilityRequest,
    deadline: Duration,
}

pub struct Broker {
    owner: Arc<()>,
    next_session: u64,
    sessions: BTreeMap<u64, Binding>,
    policy: BTreeSet<Rule>,
    handlers: BTreeMap<Namespace, Box<dyn CapabilityHandler>>,
    pending: BTreeMap<RequestId, Pending>,
    now: Duration,
    audit: VecDeque<AuditRecord>,
    dropped_audit: u64,
}
impl Default for Broker {
    fn default() -> Self {
        Self {
            owner: Arc::new(()),
            next_session: 0,
            sessions: BTreeMap::new(),
            policy: BTreeSet::new(),
            handlers: BTreeMap::new(),
            pending: BTreeMap::new(),
            now: Duration::ZERO,
            audit: VecDeque::new(),
            dropped_audit: 0,
        }
    }
}
impl Broker {
    pub fn now(&self) -> Duration {
        self.now
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
    pub fn audit(&self) -> &VecDeque<AuditRecord> {
        &self.audit
    }
    pub fn dropped_audit_records(&self) -> u64 {
        self.dropped_audit
    }

    pub fn register(
        &mut self,
        capability: Namespace,
        handler: Box<dyn CapabilityHandler>,
    ) -> Result<(), BrokerError> {
        if self.handlers.contains_key(&capability) {
            return Err(BrokerError::DuplicateHandler);
        }
        self.handlers.insert(capability, handler);
        Ok(())
    }
    /// A host-owned immutable snapshot of the manifest. Updating a package or
    /// restarting a Runtime requires detach + attach, and new explicit grants.
    pub fn attach(
        &mut self,
        runtime: &Runtime,
        manifest: &MateManifest,
    ) -> Result<Session, BrokerError> {
        manifest.validate()?;
        if self
            .sessions
            .values()
            .any(|binding| binding.instance_id == runtime.instance_id())
        {
            return Err(BrokerError::DuplicateInstance);
        }
        let serial = self.next_session;
        self.next_session = serial
            .checked_add(1)
            .ok_or(BrokerError::SequenceExhausted)?;
        self.sessions.insert(
            serial,
            Binding {
                instance_id: runtime.instance_id().into(),
                manifest: manifest.clone(),
                grants: BTreeSet::new(),
                next_sequence: 0,
            },
        );
        self.record(AuditEvent::Attached {
            session: serial,
            instance_id: runtime.instance_id().into(),
            package_id: manifest.id.clone(),
            version: manifest.version.to_string(),
        });
        Ok(Session {
            owner: self.owner.clone(),
            serial,
        })
    }
    /// Host policy is independent of user grants. Allowing a rule never grants it.
    pub fn allow(&mut self, rule: Rule) -> Result<(), BrokerError> {
        types::validate_scope(&rule.scope)?;
        if self.policy.insert(rule.clone()) {
            self.record(AuditEvent::PolicyChanged {
                rule,
                allowed: true,
            });
        }
        Ok(())
    }
    /// Remove host policy and permanently invalidate matching session grants.
    pub fn disallow(&mut self, rule: &Rule) -> Result<Vec<CapabilityReply>, BrokerError> {
        types::validate_scope(&rule.scope)?;
        if self.policy.remove(rule) {
            self.record(AuditEvent::PolicyChanged {
                rule: rule.clone(),
                allowed: false,
            });
        }
        for binding in self.sessions.values_mut() {
            binding.grants.remove(rule);
        }
        let ids = self
            .pending
            .iter()
            .filter(|(_, pending)| matches_rule(&pending.request, rule))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        Ok(ids
            .into_iter()
            .map(|id| self.finish(id, Outcome::Denied(Denial::HostPolicyDenied)))
            .collect())
    }
    /// Host attests explicit user authorization for exactly this live session.
    /// Data packages have no access to this method. No prompt/UI is provided here.
    pub fn grant(&mut self, session: &Session, rule: Rule) -> Result<(), BrokerError> {
        self.check_session(session)?;
        types::validate_scope(&rule.scope)?;
        self.authorize_declaration(session.serial, &rule)
            .map_err(BrokerError::Authorization)?;
        if !self.policy.contains(&rule) {
            return Err(BrokerError::Authorization(Denial::HostPolicyDenied));
        }
        if !self.handlers.contains_key(&rule.capability) {
            return Err(BrokerError::Authorization(Denial::UnsupportedCapability));
        }
        if self
            .sessions
            .get_mut(&session.serial)
            .expect("checked session")
            .grants
            .insert(rule.clone())
        {
            self.record(AuditEvent::Granted {
                session: session.serial,
                rule,
            });
        }
        Ok(())
    }
    pub fn revoke(
        &mut self,
        session: &Session,
        rule: &Rule,
    ) -> Result<Vec<CapabilityReply>, BrokerError> {
        self.check_session(session)?;
        types::validate_scope(&rule.scope)?;
        self.sessions
            .get_mut(&session.serial)
            .expect("checked session")
            .grants
            .remove(rule);
        self.record(AuditEvent::Revoked {
            session: session.serial,
            rule: rule.clone(),
        });
        let ids = self
            .pending
            .iter()
            .filter(|(id, pending)| {
                id.session == session.serial && matches_rule(&pending.request, rule)
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        Ok(ids
            .into_iter()
            .map(|id| self.finish(id, Outcome::Revoked))
            .collect())
    }
    pub fn detach(&mut self, session: &Session) -> Result<Vec<CapabilityReply>, BrokerError> {
        self.check_session(session)?;
        let ids = self
            .pending
            .keys()
            .filter(|id| id.session == session.serial)
            .copied()
            .collect::<Vec<_>>();
        let replies = ids
            .into_iter()
            .map(|id| self.finish(id, Outcome::SessionClosed))
            .collect();
        self.sessions.remove(&session.serial);
        self.record(AuditEvent::Detached {
            session: session.serial,
        });
        Ok(replies)
    }

    /// Requests must be submitted in the Runtime's exact sequence, including
    /// denials. Transport/routing errors do not consume a sequence number.
    pub fn submit(
        &mut self,
        session: &Session,
        intent: CapabilityIntent,
        timeout: Duration,
    ) -> Result<Submission, BrokerError> {
        self.check_session(session)?;
        let binding = &self.sessions[&session.serial];
        if intent.instance_id != binding.instance_id {
            return Err(BrokerError::InstanceMismatch);
        }
        if intent.sequence != binding.next_sequence {
            return Err(BrokerError::UnexpectedSequence {
                expected: binding.next_sequence,
                received: intent.sequence,
            });
        }
        let next = intent
            .sequence
            .checked_add(1)
            .ok_or(BrokerError::SequenceExhausted)?;
        let deadline = self
            .now
            .checked_add(timeout)
            .ok_or(BrokerError::DeadlineOverflow)?;
        let id = RequestId {
            session: session.serial,
            sequence: intent.sequence,
        };
        self.sessions
            .get_mut(&session.serial)
            .expect("checked session")
            .next_sequence = next;
        self.record(AuditEvent::Submitted {
            request: id,
            capability: intent.request.capability.clone(),
            scope: intent.request.scope.clone(),
        });
        let denial = self.check_request(session.serial, &intent);
        if let Err(reason) = denial {
            return Ok(Submission::Completed(self.reply(
                id,
                &intent.request,
                Outcome::Denied(reason),
            )));
        }
        if timeout.is_zero() {
            return Ok(Submission::Completed(self.reply(
                id,
                &intent.request,
                Outcome::TimedOut,
            )));
        }
        if self.pending.len() >= MAX_PENDING {
            return Ok(Submission::Completed(self.reply(
                id,
                &intent.request,
                Outcome::Denied(Denial::Busy),
            )));
        }
        let ticket = Ticket {
            owner: self.owner.clone(),
            id,
        };
        let instance_id = self.sessions[&session.serial].instance_id.clone();
        // Reserve before calling a handler. Handler::start must return promptly;
        // logical timeouts do not interrupt a blocking native function.
        self.pending.insert(
            id,
            Pending {
                request: intent.request.clone(),
                deadline,
            },
        );
        self.record(AuditEvent::Started { request: id });
        let result = self
            .handlers
            .get_mut(&intent.request.capability)
            .expect("authorized handler")
            .start(&Invocation {
                ticket: ticket.clone(),
                instance_id: &instance_id,
                request: &intent.request,
            });
        match result {
            HandlerResult::Complete(outcome) => Ok(Submission::Completed(
                self.finish(id, validated_outcome(outcome)),
            )),
            HandlerResult::Pending => Ok(Submission::Pending(ticket)),
        }
    }
    /// Complete host work by its broker-issued ticket. Late, duplicate, foreign
    /// or revoked tickets are rejected, and cannot become successful replies.
    pub fn complete(
        &mut self,
        ticket: &Ticket,
        outcome: HandlerOutcome,
    ) -> Result<CapabilityReply, BrokerError> {
        self.check_owner(&ticket.owner)?;
        if !self.pending.contains_key(&ticket.id) {
            return Err(BrokerError::RequestNotPending);
        }
        Ok(self.finish(ticket.id, validated_outcome(outcome)))
    }
    /// Inject monotonic host time. At deadline equality timeout wins. Timeouts
    /// are reported in (deadline, session, sequence) order, independently of maps.
    pub fn advance_to(&mut self, now: Duration) -> Result<Vec<CapabilityReply>, BrokerError> {
        if now < self.now {
            return Err(BrokerError::ClockWentBackwards);
        }
        self.now = now;
        let mut expired = self
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(id, p)| (p.deadline, *id))
            .collect::<Vec<_>>();
        expired.sort();
        Ok(expired
            .into_iter()
            .map(|(_, id)| self.finish(id, Outcome::TimedOut))
            .collect())
    }
    /// Route only to the original live broker session and matching Runtime id.
    /// Replies are data, so a host must still avoid delivering the same one twice.
    pub fn deliver(
        &self,
        session: &Session,
        reply: &CapabilityReply,
        runtime: &mut Runtime,
    ) -> Result<StepOutput, BrokerError> {
        self.check_session(session)?;
        self.check_owner(&reply.owner)?;
        if reply.id.session != session.serial
            || runtime.instance_id() != reply.instance_id
            || self.sessions[&session.serial].instance_id != reply.instance_id
        {
            return Err(BrokerError::WrongRecipient);
        }
        Ok(runtime.process(reply.event())?)
    }

    fn check_owner(&self, owner: &Arc<()>) -> Result<(), BrokerError> {
        if Arc::ptr_eq(&self.owner, owner) {
            Ok(())
        } else {
            Err(BrokerError::ForeignHandle)
        }
    }
    fn check_session(&self, session: &Session) -> Result<(), BrokerError> {
        self.check_owner(&session.owner)?;
        if self.sessions.contains_key(&session.serial) {
            Ok(())
        } else {
            Err(BrokerError::UnknownSession)
        }
    }
    fn authorize_declaration(&self, serial: u64, rule: &Rule) -> Result<(), Denial> {
        let manifest = &self.sessions[&serial].manifest;
        if !manifest.capabilities.contains(&rule.capability) {
            return Err(Denial::UndeclaredCapability);
        }
        if !manifest
            .permissions
            .iter()
            .any(|p| p.capability == rule.capability && p.scope == rule.scope)
        {
            return Err(Denial::ScopeNotDeclared);
        }
        Ok(())
    }
    fn check_request(&self, serial: u64, intent: &CapabilityIntent) -> Result<(), Denial> {
        if intent.request.validate().is_err()
            || types::validate_scope(&intent.request.scope).is_err()
        {
            return Err(Denial::InvalidRequest);
        }
        let rule = Rule {
            capability: intent.request.capability.clone(),
            scope: intent.request.scope.clone(),
        };
        self.authorize_declaration(serial, &rule)?;
        if intent.status != CapabilityStatus::AwaitingHostDecision {
            return Err(Denial::RuntimeRejected);
        }
        if !self.policy.contains(&rule) {
            return Err(Denial::HostPolicyDenied);
        }
        if !self.sessions[&serial].grants.contains(&rule) {
            return Err(Denial::NotGranted);
        }
        if !self.handlers.contains_key(&rule.capability) {
            return Err(Denial::UnsupportedCapability);
        }
        Ok(())
    }
    fn finish(&mut self, id: RequestId, outcome: Outcome) -> CapabilityReply {
        let pending = self.pending.remove(&id).expect("internal pending request");
        if matches!(
            outcome,
            Outcome::TimedOut | Outcome::Revoked | Outcome::SessionClosed | Outcome::Denied(_)
        ) {
            if let Some(handler) = self.handlers.get_mut(&pending.request.capability) {
                handler.cancel(&Ticket {
                    owner: self.owner.clone(),
                    id,
                });
            }
        }
        self.reply(id, &pending.request, outcome)
    }
    fn reply(
        &mut self,
        id: RequestId,
        request: &CapabilityRequest,
        outcome: Outcome,
    ) -> CapabilityReply {
        let binding = &self.sessions[&id.session];
        let reply = CapabilityReply {
            owner: self.owner.clone(),
            id,
            instance_id: binding.instance_id.clone(),
            package_id: binding.manifest.id.clone(),
            package_version: binding.manifest.version.to_string(),
            capability: request.capability.clone(),
            scope: request.scope.clone(),
            outcome,
        };
        let reason = match &reply.outcome {
            Outcome::Denied(reason) => Some(reason.code().into()),
            Outcome::Failed(failure) => Some(failure.code.to_string()),
            _ => None,
        };
        self.record(AuditEvent::Finished {
            request: id,
            status: reply.outcome.status().into(),
            reason,
        });
        reply
    }
    fn record(&mut self, event: AuditEvent) {
        if self.audit.len() == MAX_AUDIT_RECORDS {
            self.audit.pop_front();
            self.dropped_audit = self.dropped_audit.saturating_add(1);
        }
        self.audit.push_back(AuditRecord {
            at: self.now,
            event,
        });
    }
}
fn matches_rule(request: &CapabilityRequest, rule: &Rule) -> bool {
    request.capability == rule.capability && request.scope == rule.scope
}
fn validated_outcome(outcome: HandlerOutcome) -> Outcome {
    let valid = match &outcome {
        // Account for the extra Object layer in the result event envelope.
        Ok(value) => Value::Object(BTreeMap::from([("value".into(), value.clone())]))
            .validate()
            .is_ok(),
        Err(failure) => !failure.message.trim().is_empty() && failure.message.len() <= 1024,
    };
    if !valid {
        return Outcome::Failed(HandlerFailure {
            code: Namespace::new("broker.invalid_result").expect("constant namespace"),
            message: "handler returned invalid result data".into(),
        });
    }
    match outcome {
        Ok(value) => Outcome::Succeeded(value),
        Err(failure) => Outcome::Failed(failure),
    }
}
