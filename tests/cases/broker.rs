use crate::*;
use mochi_broker::*;
use mochi_runtime::{CapabilityIntent, CapabilityStatus};
use mochi_schema::{MateManifest, Permission};
use std::{cell::Cell, rc::Rc, time::Duration};

fn rule() -> Rule {
    Rule::new(ns("mock.echo"), "demo").unwrap()
}
fn manifest() -> MateManifest {
    let mut manifest = MateManifest::from_toml(&manifest_text()).unwrap();
    manifest.capabilities.insert(ns("mock.echo"));
    manifest.permissions.push(Permission {
        capability: ns("mock.echo"),
        scope: "demo".into(),
    });
    manifest
}
struct RequestAndReceive;
impl Behavior for RequestAndReceive {
    fn on_event(&self, _: &MateContext<'_>, event: &MateEvent) -> Vec<MateAction> {
        match event {
            MateEvent::PointerClick { .. } => {
                vec![MateAction::RequestCapability(CapabilityRequest {
                    capability: ns("mock.echo"),
                    scope: "demo".into(),
                    payload: Value::String("hello".into()),
                })]
            }
            MateEvent::Custom { name, payload } if name.as_str() == "mochi.capability.result" => {
                vec![
                    MateAction::SetState {
                        key: ns("demo.result"),
                        value: payload.clone(),
                    },
                    MateAction::PlayAnimation("received".into()),
                ]
            }
            _ => vec![],
        }
    }
}
fn mate(id: &str, manifest: &MateManifest) -> Runtime {
    Runtime::new(
        id,
        MateState::default(),
        manifest.capabilities.clone(),
        vec![Box::new(RequestAndReceive)],
        appearance(),
    )
    .unwrap()
}
fn intent(instance_id: &str, sequence: u64) -> CapabilityIntent {
    CapabilityIntent {
        instance_id: instance_id.into(),
        sequence,
        status: CapabilityStatus::AwaitingHostDecision,
        request: CapabilityRequest {
            capability: ns("mock.echo"),
            scope: "demo".into(),
            payload: Value::String("hello".into()),
        },
    }
}
fn ready(result: Result<Submission, BrokerError>) -> CapabilityReply {
    match result.unwrap() {
        Submission::Completed(reply) => reply,
        Submission::Pending(_) => panic!("expected ready"),
    }
}
fn ticket(result: Result<Submission, BrokerError>) -> Ticket {
    match result.unwrap() {
        Submission::Pending(ticket) => ticket,
        Submission::Completed(reply) => panic!("unexpected completion: {reply:?}"),
    }
}
fn authorized(handler: Box<dyn CapabilityHandler>) -> (Broker, Session, Runtime) {
    let manifest = manifest();
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker.register(ns("mock.echo"), handler).unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    broker.allow(rule()).unwrap();
    broker.grant(&session, rule()).unwrap();
    (broker, session, mate)
}
fn timeout() -> Duration {
    Duration::from_secs(2)
}
struct CountHandler {
    starts: Rc<Cell<usize>>,
    cancels: Rc<Cell<usize>>,
    pending: bool,
}
impl CapabilityHandler for CountHandler {
    fn start(&mut self, call: &Invocation<'_>) -> HandlerResult {
        self.starts.set(self.starts.get() + 1);
        if self.pending {
            HandlerResult::Pending
        } else {
            HandlerResult::Complete(Ok(call.request.payload.clone()))
        }
    }
    fn cancel(&mut self, _: &Ticket) {
        self.cancels.set(self.cancels.get() + 1);
    }
}

#[test]
fn default_denial_and_host_allow_never_invoke_handler_without_session_grant() {
    let starts = Rc::new(Cell::new(0));
    let manifest = manifest();
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker
        .register(
            ns("mock.echo"),
            Box::new(CountHandler {
                starts: starts.clone(),
                cancels: Rc::new(Cell::new(0)),
                pending: false,
            }),
        )
        .unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    assert_eq!(
        ready(broker.submit(&session, intent("a", 0), timeout())).outcome(),
        &Outcome::Denied(Denial::HostPolicyDenied)
    );
    assert_eq!(
        broker.grant(&session, rule()),
        Err(BrokerError::Authorization(Denial::HostPolicyDenied))
    );
    broker.allow(rule()).unwrap();
    assert_eq!(
        ready(broker.submit(&session, intent("a", 1), timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    assert_eq!(starts.get(), 0);
    broker.grant(&session, rule()).unwrap();
    assert!(matches!(
        ready(broker.submit(&session, intent("a", 2), timeout())).outcome(),
        Outcome::Succeeded(_)
    ));
    assert_eq!(starts.get(), 1);
}

#[test]
fn runtime_request_broker_reply_event_state_and_scene_complete_the_loop() {
    let (mut broker, session, mut mate) = authorized(Box::new(MockCapability::echo()));
    let step = mate
        .process(MateEvent::PointerClick {
            position: Vec2::default(),
            button: 1,
        })
        .unwrap();
    let reply = ready(broker.submit(&session, step.capability_requests[0].clone(), timeout()));
    assert_eq!(
        reply.id(),
        RequestId {
            session: session.serial(),
            sequence: 0
        }
    );
    assert_eq!(reply.package_id(), &ns("mochi.neko"));
    assert_eq!(reply.package_version(), "0.1.0");
    let output = broker.deliver(&session, &reply, &mut mate).unwrap();
    let Value::Object(fields) = &output.state.values[&ns("demo.result")] else {
        panic!("expected event envelope")
    };
    assert_eq!(fields["status"], Value::String("succeeded".into()));
    assert_eq!(fields["sequence"], Value::String("0".into()));
    assert_eq!(fields["schema_version"], Value::Integer(1));
    assert_eq!(fields["value"], Value::String("hello".into()));
    assert!(output.capability_requests.is_empty());
    assert!(
        matches!(&output.scene.nodes[0].content, SceneContent::Sprite { animation, .. } if animation == "received")
    );
}

#[test]
fn forged_runtime_status_cannot_bypass_manifest_or_permission_checks() {
    let mut manifest = manifest();
    manifest.capabilities.clear();
    manifest.permissions.clear();
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker
        .register(ns("mock.echo"), Box::new(MockCapability::echo()))
        .unwrap();
    broker.allow(rule()).unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    assert_eq!(
        ready(broker.submit(&session, intent("a", 0), timeout())).outcome(),
        &Outcome::Denied(Denial::UndeclaredCapability)
    );
    assert_eq!(
        broker.grant(&session, rule()),
        Err(BrokerError::Authorization(Denial::UndeclaredCapability))
    );
}

#[test]
fn runtime_rejection_remains_rejected_even_with_broker_grant() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::echo()));
    let mut request = intent("a", 0);
    request.status = CapabilityStatus::RejectedUndeclared;
    assert_eq!(
        ready(broker.submit(&session, request, timeout())).outcome(),
        &Outcome::Denied(Denial::RuntimeRejected)
    );
}

#[test]
fn exact_scope_does_not_authorize_child_sibling_or_case_variant() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::echo()));
    for (i, scope) in ["demo/child", "demo-other", "Demo"].into_iter().enumerate() {
        let other = Rule::new(ns("mock.echo"), scope).unwrap();
        broker.allow(other.clone()).unwrap();
        assert_eq!(
            broker.grant(&session, other),
            Err(BrokerError::Authorization(Denial::ScopeNotDeclared))
        );
        let mut request = intent("a", i as u64);
        request.request.scope = scope.into();
        assert_eq!(
            ready(broker.submit(&session, request, timeout())).outcome(),
            &Outcome::Denied(Denial::ScopeNotDeclared)
        );
    }
}

#[test]
fn separately_declared_scope_requires_its_own_user_grant() {
    let mut manifest = manifest();
    manifest.permissions.push(Permission {
        capability: ns("mock.echo"),
        scope: "other".into(),
    });
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker
        .register(ns("mock.echo"), Box::new(MockCapability::echo()))
        .unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    broker.allow(rule()).unwrap();
    broker.grant(&session, rule()).unwrap();
    let other = Rule::new(ns("mock.echo"), "other").unwrap();
    broker.allow(other).unwrap();
    let mut request = intent("a", 0);
    request.request.scope = "other".into();
    assert_eq!(
        ready(broker.submit(&session, request, timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
}

#[test]
fn invalid_scopes_and_wildcard_permissions_never_expand_authority() {
    for scope in [
        "", "*", "demo/*", "demo?", " demo", "demo ", "demo\n", "a\0b",
    ] {
        assert!(Rule::new(ns("mock.echo"), scope).is_err());
    }
    let mut manifest = manifest();
    manifest.permissions[0].scope = "*".into();
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker
        .register(ns("mock.echo"), Box::new(MockCapability::echo()))
        .unwrap();
    broker.allow(rule()).unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    assert_eq!(
        broker.grant(&session, rule()),
        Err(BrokerError::Authorization(Denial::ScopeNotDeclared))
    );
    let mut request = intent("a", 0);
    request.request.scope = "*".into();
    assert_eq!(
        ready(broker.submit(&session, request, timeout())).outcome(),
        &Outcome::Denied(Denial::InvalidRequest)
    );
}

#[test]
fn pending_completes_once_and_rejects_duplicate_submission_or_completion() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    let ticket = ticket(broker.submit(&session, intent("a", 0), timeout()));
    assert!(matches!(
        broker.submit(&session, intent("a", 0), timeout()),
        Err(BrokerError::UnexpectedSequence {
            expected: 1,
            received: 0
        })
    ));
    let reply = broker.complete(&ticket, Ok(Value::Bool(true))).unwrap();
    assert_eq!(reply.outcome(), &Outcome::Succeeded(Value::Bool(true)));
    assert!(matches!(
        broker.complete(&ticket, Ok(Value::Bool(false))),
        Err(BrokerError::RequestNotPending)
    ));
    assert_eq!(broker.pending_count(), 0);
}

#[test]
fn revoke_cancels_pending_once_and_late_completion_cannot_resurrect_it() {
    let starts = Rc::new(Cell::new(0));
    let cancels = Rc::new(Cell::new(0));
    let (mut broker, session, _) = authorized(Box::new(CountHandler {
        starts: starts.clone(),
        cancels: cancels.clone(),
        pending: true,
    }));
    let ticket = ticket(broker.submit(&session, intent("a", 0), timeout()));
    let replies = broker.revoke(&session, &rule()).unwrap();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].outcome(), &Outcome::Revoked);
    assert_eq!(cancels.get(), 1);
    assert!(broker.revoke(&session, &rule()).unwrap().is_empty());
    assert_eq!(cancels.get(), 1);
    broker.grant(&session, rule()).unwrap();
    assert!(matches!(
        broker.complete(&ticket, Ok(Value::Bool(true))),
        Err(BrokerError::RequestNotPending)
    ));
    assert_eq!(starts.get(), 1);
}

#[test]
fn revoked_grant_denies_new_requests() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::echo()));
    broker.revoke(&session, &rule()).unwrap();
    assert_eq!(
        ready(broker.submit(&session, intent("a", 0), timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
}

#[test]
fn policy_withdrawal_cancels_work_and_does_not_restore_old_grants_on_reallow() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    let ticket = ticket(broker.submit(&session, intent("a", 0), timeout()));
    let replies = broker.disallow(&rule()).unwrap();
    assert_eq!(
        replies[0].outcome(),
        &Outcome::Denied(Denial::HostPolicyDenied)
    );
    broker.allow(rule()).unwrap();
    assert_eq!(
        ready(broker.submit(&session, intent("a", 1), timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    assert!(matches!(
        broker.complete(&ticket, Ok(Value::Bool(true))),
        Err(BrokerError::RequestNotPending)
    ));
}

#[test]
fn timeout_at_deadline_cancels_once_and_late_completion_is_ignored() {
    let cancels = Rc::new(Cell::new(0));
    let (mut broker, session, _) = authorized(Box::new(CountHandler {
        starts: Rc::new(Cell::new(0)),
        cancels: cancels.clone(),
        pending: true,
    }));
    let ticket = ticket(broker.submit(&session, intent("a", 0), timeout()));
    assert!(broker
        .advance_to(Duration::from_secs(1))
        .unwrap()
        .is_empty());
    let replies = broker.advance_to(timeout()).unwrap();
    assert_eq!(replies[0].outcome(), &Outcome::TimedOut);
    assert_eq!(cancels.get(), 1);
    assert!(broker.advance_to(timeout()).unwrap().is_empty());
    assert!(matches!(
        broker.complete(&ticket, Ok(Value::Bool(true))),
        Err(BrokerError::RequestNotPending)
    ));
}

#[test]
fn zero_timeout_does_not_call_the_handler() {
    let starts = Rc::new(Cell::new(0));
    let (mut broker, session, _) = authorized(Box::new(CountHandler {
        starts: starts.clone(),
        cancels: Rc::new(Cell::new(0)),
        pending: false,
    }));
    assert_eq!(
        ready(broker.submit(&session, intent("a", 0), Duration::ZERO)).outcome(),
        &Outcome::TimedOut
    );
    assert_eq!(starts.get(), 0);
}

#[test]
fn deadlines_sort_by_time_then_request_key() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    ticket(broker.submit(&session, intent("a", 0), Duration::from_secs(3)));
    ticket(broker.submit(&session, intent("a", 1), Duration::from_secs(1)));
    ticket(broker.submit(&session, intent("a", 2), Duration::from_secs(1)));
    let replies = broker.advance_to(Duration::from_secs(4)).unwrap();
    assert_eq!(
        replies.iter().map(|r| r.id().sequence).collect::<Vec<_>>(),
        vec![1, 2, 0]
    );
}

#[test]
fn time_regression_and_overflow_leave_request_sequence_unchanged() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    broker.advance_to(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        broker.advance_to(Duration::ZERO),
        Err(BrokerError::ClockWentBackwards)
    ));
    assert!(matches!(
        broker.submit(&session, intent("a", 0), Duration::MAX),
        Err(BrokerError::DeadlineOverflow)
    ));
    let ticket = ticket(broker.submit(&session, intent("a", 0), timeout()));
    assert_eq!(ticket.id().sequence, 0);
    assert_eq!(broker.now(), Duration::from_secs(1));
}

#[test]
fn bindings_permissions_results_and_revocation_are_isolated_between_mates() {
    let (mut broker, a, mut mate_a) = authorized(Box::new(MockCapability::pending()));
    let manifest = manifest();
    let mut mate_b = mate("b", &manifest);
    let b = broker.attach(&mate_b, &manifest).unwrap();
    assert_eq!(
        ready(broker.submit(&b, intent("b", 0), timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
    broker.grant(&b, rule()).unwrap();
    let ta = ticket(broker.submit(&a, intent("a", 0), timeout()));
    let tb = ticket(broker.submit(&b, intent("b", 1), timeout()));
    let reply_a = broker.revoke(&a, &rule()).unwrap().remove(0);
    assert_eq!(reply_a.id(), ta.id());
    assert_eq!(broker.pending_count(), 1);
    let reply_b = broker
        .complete(&tb, Ok(Value::String("only b".into())))
        .unwrap();
    assert!(matches!(
        broker.deliver(&a, &reply_b, &mut mate_a),
        Err(BrokerError::WrongRecipient)
    ));
    broker.deliver(&b, &reply_b, &mut mate_b).unwrap();
    assert!(!mate_a.state().values.contains_key(&ns("demo.result")));
    assert!(mate_b.state().values.contains_key(&ns("demo.result")));
}

#[test]
fn detach_and_restart_do_not_reuse_grants_tickets_or_reply_routes() {
    let (mut broker, old, old_runtime) = authorized(Box::new(MockCapability::pending()));
    assert!(matches!(
        broker.attach(&old_runtime, &manifest()),
        Err(BrokerError::DuplicateInstance)
    ));
    let ticket = ticket(broker.submit(&old, intent("a", 0), timeout()));
    let reply = broker.detach(&old).unwrap().remove(0);
    assert_eq!(reply.outcome(), &Outcome::SessionClosed);
    let mut restarted = mate("a", &manifest());
    let new = broker.attach(&restarted, &manifest()).unwrap();
    assert_ne!(new.serial(), old.serial());
    assert!(matches!(
        broker.grant(&old, rule()),
        Err(BrokerError::UnknownSession)
    ));
    assert!(matches!(
        broker.complete(&ticket, Ok(Value::Bool(true))),
        Err(BrokerError::RequestNotPending)
    ));
    assert!(matches!(
        broker.deliver(&new, &reply, &mut restarted),
        Err(BrokerError::WrongRecipient)
    ));
    assert_eq!(
        ready(broker.submit(&new, intent("a", 0), timeout())).outcome(),
        &Outcome::Denied(Denial::NotGranted)
    );
}

#[test]
fn foreign_broker_handles_cannot_alias_identical_serial_numbers() {
    let (mut a, sa, _) = authorized(Box::new(MockCapability::pending()));
    let (mut b, sb, _) = authorized(Box::new(MockCapability::pending()));
    assert_eq!(sa.serial(), sb.serial());
    assert!(matches!(
        b.grant(&sa, rule()),
        Err(BrokerError::ForeignHandle)
    ));
    let ticket = ticket(a.submit(&sa, intent("a", 0), timeout()));
    assert!(matches!(
        b.complete(&ticket, Ok(Value::Bool(true))),
        Err(BrokerError::ForeignHandle)
    ));
}

#[test]
fn wrong_instance_and_out_of_order_requests_do_not_consume_sequence() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::echo()));
    assert!(matches!(
        broker.submit(&session, intent("someone-else", 0), timeout()),
        Err(BrokerError::InstanceMismatch)
    ));
    assert!(matches!(
        broker.submit(&session, intent("a", 2), timeout()),
        Err(BrokerError::UnexpectedSequence {
            expected: 0,
            received: 2
        })
    ));
    assert!(matches!(
        ready(broker.submit(&session, intent("a", 0), timeout())).outcome(),
        Outcome::Succeeded(_)
    ));
}

#[test]
fn modifying_source_manifest_cannot_expand_attached_authority() {
    let mut manifest = manifest();
    let mate = mate("a", &manifest);
    let mut broker = Broker::default();
    broker
        .register(ns("mock.echo"), Box::new(MockCapability::echo()))
        .unwrap();
    let session = broker.attach(&mate, &manifest).unwrap();
    manifest.permissions.push(Permission {
        capability: ns("mock.echo"),
        scope: "added-later".into(),
    });
    let added = Rule::new(ns("mock.echo"), "added-later").unwrap();
    broker.allow(added.clone()).unwrap();
    assert_eq!(
        broker.grant(&session, added),
        Err(BrokerError::Authorization(Denial::ScopeNotDeclared))
    );
}

#[test]
fn pending_capacity_is_bounded_and_recovers_after_completion() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    let mut tickets = Vec::new();
    for sequence in 0..MAX_PENDING as u64 {
        tickets.push(ticket(broker.submit(
            &session,
            intent("a", sequence),
            timeout(),
        )));
    }
    assert_eq!(
        ready(broker.submit(&session, intent("a", MAX_PENDING as u64), timeout())).outcome(),
        &Outcome::Denied(Denial::Busy)
    );
    broker.complete(&tickets[0], Ok(Value::Bool(true))).unwrap();
    ticket(broker.submit(&session, intent("a", MAX_PENDING as u64 + 1), timeout()));
    assert_eq!(broker.pending_count(), MAX_PENDING);
}

#[test]
fn handler_failure_and_invalid_result_return_valid_terminal_events() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::fail("expected failure")));
    let reply = ready(broker.submit(&session, intent("a", 0), timeout()));
    assert!(matches!(reply.outcome(), Outcome::Failed(f) if f.code.as_str() == "mock.failed"));
    reply.event().validate().unwrap();
    let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
    for sequence in 0..2 {
        let ticket = ticket(broker.submit(&session, intent("a", sequence), timeout()));
        let value = if sequence == 0 {
            Value::Float(f64::NAN)
        } else {
            let mut value = Value::Bool(true);
            for _ in 0..32 {
                value = Value::Array(vec![value]);
            }
            value
        };
        let reply = broker.complete(&ticket, Ok(value)).unwrap();
        assert!(
            matches!(reply.outcome(), Outcome::Failed(f) if f.code.as_str() == "broker.invalid_result")
        );
        reply.event().validate().unwrap();
    }
}

#[test]
fn malformed_request_is_denied_without_entering_handler() {
    let starts = Rc::new(Cell::new(0));
    let (mut broker, session, _) = authorized(Box::new(CountHandler {
        starts: starts.clone(),
        cancels: Rc::new(Cell::new(0)),
        pending: false,
    }));
    let mut request = intent("a", 0);
    request.request.payload = Value::Float(f64::NAN);
    assert_eq!(
        ready(broker.submit(&session, request, timeout())).outcome(),
        &Outcome::Denied(Denial::InvalidRequest)
    );
    assert_eq!(starts.get(), 0);
}

#[test]
fn audit_is_bounded_contains_no_payloads_and_exposes_truncation() {
    let (mut broker, session, _) = authorized(Box::new(MockCapability::echo()));
    for sequence in 0..100 {
        let mut request = intent("a", sequence);
        request.request.payload = Value::String("secret-payload-sentinel".into());
        ready(broker.submit(&session, request, timeout()));
    }
    assert_eq!(broker.audit().len(), MAX_AUDIT_RECORDS);
    assert!(broker.dropped_audit_records() > 0);
    assert!(!format!("{:?}", broker.audit()).contains("secret-payload-sentinel"));
}

#[test]
fn broker_events_and_audit_replay_deterministically() {
    fn replay() -> (Vec<MateEvent>, Vec<AuditRecord>) {
        let (mut broker, session, _) = authorized(Box::new(MockCapability::pending()));
        let t = ticket(broker.submit(&session, intent("a", 0), timeout()));
        let first = broker
            .complete(&t, Ok(Value::String("same".into())))
            .unwrap()
            .event();
        ticket(broker.submit(&session, intent("a", 1), timeout()));
        let last = broker.advance_to(timeout()).unwrap().remove(0).event();
        (vec![first, last], broker.audit().iter().cloned().collect())
    }
    assert_eq!(replay(), replay());
}

#[test]
fn handler_registration_cannot_silently_replace_an_implementation() {
    let (mut broker, _, _) = authorized(Box::new(MockCapability::echo()));
    assert_eq!(
        broker.register(ns("mock.echo"), Box::new(MockCapability::pending())),
        Err(BrokerError::DuplicateHandler)
    );
}
