//! Synchronous multi-Mate orchestration. No renderer, event loop, clock reads,
//! filesystem operations or capability handlers are installed by Host.
mod types;
pub use types::*;

use mochi_broker::{
    Broker, CapabilityHandler, CapabilityReply, HandlerOutcome, Rule, Session, Submission, Ticket,
};
use mochi_core::{MateEvent, Namespace};
use mochi_package::LoadedPackage;
use mochi_runtime::{Runtime, StepOutput};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub const DEFAULT_MAX_INSTANCES: usize = 64;

pub struct Instance {
    runtime: Runtime,
    package: Arc<LoadedPackage>,
    session: Session,
    presentation: Presentation,
}
impl Instance {
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }
    pub fn package(&self) -> &LoadedPackage {
        &self.package
    }
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }
}

/// Owns instance/session lifetimes. No mutable Runtime/Broker escape hatch:
/// routing, sequence capture and detach cannot be bypassed through this API.
pub struct Host {
    owner: Arc<()>,
    instances: BTreeMap<u64, Instance>,
    broker: Broker,
    max_instances: usize,
}
impl Default for Host {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_INSTANCES)
    }
}
impl Host {
    pub fn new(max_instances: usize) -> Self {
        Self {
            owner: Arc::new(()),
            instances: BTreeMap::new(),
            broker: Broker::default(),
            max_instances,
        }
    }
    pub fn len(&self) -> usize {
        self.instances.len()
    }
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
    pub fn broker(&self) -> &Broker {
        &self.broker
    }

    /// Current handles in creation order, independent of names and visual z order.
    pub fn handles(&self) -> Vec<InstanceHandle> {
        self.instances
            .keys()
            .map(|&serial| self.handle(serial))
            .collect()
    }
    /// Explicitly resolve a name to its CURRENT incarnation.
    pub fn find(&self, instance_id: &str) -> Option<InstanceHandle> {
        self.instances
            .iter()
            .find(|(_, i)| i.runtime.instance_id() == instance_id)
            .map(|(&serial, _)| self.handle(serial))
    }
    pub fn get(&self, handle: &InstanceHandle) -> Result<&Instance, HostError> {
        self.check(handle)?;
        Ok(&self.instances[&handle.serial])
    }
    pub fn spawn(
        &mut self,
        id: impl Into<String>,
        package: Arc<LoadedPackage>,
        presentation: Presentation,
    ) -> Result<InstanceHandle, HostError> {
        let id = id.into();
        self.check_spawn(&id, presentation)?;
        let runtime = package.instantiate(id)?;
        self.insert(runtime, package, presentation)
    }
    pub fn spawn_native(
        &mut self,
        id: impl Into<String>,
        package: Arc<LoadedPackage>,
        native: NativeMate,
        presentation: Presentation,
    ) -> Result<InstanceHandle, HostError> {
        let id = id.into();
        self.check_spawn(&id, presentation)?;
        let runtime = Runtime::new(
            id,
            native.state,
            package.manifest().capabilities.clone(),
            native.behaviors,
            native.appearance,
        )?;
        self.insert(runtime, package, presentation)
    }
    /// Trusted host policy around package behavior, preserving its animation adapter.
    pub fn spawn_with_behavior(
        &mut self,
        id: impl Into<String>,
        package: Arc<LoadedPackage>,
        presentation: Presentation,
        decorate: impl FnOnce(Box<dyn mochi_core::Behavior>) -> Box<dyn mochi_core::Behavior>,
    ) -> Result<InstanceHandle, HostError> {
        let id = id.into();
        self.check_spawn(&id, presentation)?;
        let runtime = package.instantiate_with_behavior(id, decorate)?;
        self.insert(runtime, package, presentation)
    }
    fn check_spawn(&self, id: &str, presentation: Presentation) -> Result<(), HostError> {
        presentation.validate()?;
        if self.find(id).is_some() {
            return Err(HostError::DuplicateInstance);
        }
        if self.len() >= self.max_instances {
            return Err(HostError::InstanceLimit);
        }
        Ok(())
    }
    fn insert(
        &mut self,
        runtime: Runtime,
        package: Arc<LoadedPackage>,
        presentation: Presentation,
    ) -> Result<InstanceHandle, HostError> {
        let session = self.broker.attach(&runtime, package.manifest())?;
        let serial = session.serial();
        self.instances.insert(
            serial,
            Instance {
                runtime,
                package,
                session,
                presentation,
            },
        );
        Ok(self.handle(serial))
    }
    pub fn set_presentation(
        &mut self,
        handle: &InstanceHandle,
        presentation: Presentation,
    ) -> Result<(), HostError> {
        self.check(handle)?;
        presentation.validate()?;
        self.instances
            .get_mut(&handle.serial)
            .expect("checked instance")
            .presentation = presentation;
        Ok(())
    }
    /// State/time/request numbers commit per instance. Rendering is a separate
    /// operation and cannot erase the output requests of a successful step.
    pub fn dispatch(
        &mut self,
        handle: &InstanceHandle,
        event: MateEvent,
    ) -> Result<HostStep, HostError> {
        self.check(handle)?;
        let output = self
            .instances
            .get_mut(&handle.serial)
            .expect("checked instance")
            .runtime
            .process(event)?;
        Ok(Self::step(handle.clone(), output))
    }
    /// Every live instance is attempted in creation order. A failed instance
    /// rolls back its own step; successful siblings stay committed. No global transaction.
    #[must_use]
    pub fn broadcast(
        &mut self,
        event: MateEvent,
    ) -> Vec<(InstanceHandle, Result<HostStep, HostError>)> {
        self.handles()
            .into_iter()
            .map(|handle| {
                let result = self.dispatch(&handle, event.clone());
                (handle, result)
            })
            .collect()
    }
    /// Back-to-front (z_index, creation order). Presentation modifies projected
    /// nodes only. Errors do not change simulation state or any renderer frame.
    pub fn scenes(&self) -> Result<Vec<HostScene<'_>>, HostError> {
        let mut entries: Vec<_> = self.instances.iter().collect();
        entries.sort_by_key(|(serial, i)| (i.presentation.z_index, **serial));
        entries
            .into_iter()
            .map(|(_, instance)| {
                let mut scene = instance.runtime.scene()?;
                for node in &mut scene.nodes {
                    node.transform.translation.x += instance.presentation.offset.x;
                    node.transform.translation.y += instance.presentation.offset.y;
                    node.opacity *= instance.presentation.opacity;
                }
                scene
                    .validate()
                    .map_err(mochi_runtime::RuntimeError::from)?;
                Ok(HostScene {
                    instance_id: instance.runtime.instance_id(),
                    package: &instance.package,
                    scene,
                })
            })
            .collect()
    }
    /// Cancels pending work and discards grants before removing the instance.
    /// Returned SessionClosed replies are cleanup records, never deliverable to a replacement.
    pub fn remove(&mut self, handle: &InstanceHandle) -> Result<Vec<CapabilityReply>, HostError> {
        self.check(handle)?;
        let replies = self
            .broker
            .detach(&self.instances[&handle.serial].session)?;
        self.instances.remove(&handle.serial);
        Ok(replies)
    }
    /// Explicit shutdown returns terminal replies in creation/request order.
    pub fn clear(&mut self) -> Vec<CapabilityReply> {
        let mut replies = Vec::new();
        for handle in self.handles() {
            replies.extend(
                self.remove(&handle)
                    .expect("host owns live broker sessions"),
            );
        }
        replies
    }

    // Host-owned policy controls. Data packages cannot invoke these methods.
    pub fn register(
        &mut self,
        capability: Namespace,
        handler: Box<dyn CapabilityHandler>,
    ) -> Result<(), HostError> {
        Ok(self.broker.register(capability, handler)?)
    }
    pub fn allow(&mut self, rule: Rule) -> Result<(), HostError> {
        Ok(self.broker.allow(rule)?)
    }
    pub fn disallow(&mut self, rule: &Rule) -> Result<Vec<CapabilityReply>, HostError> {
        Ok(self.broker.disallow(rule)?)
    }
    pub fn grant(&mut self, handle: &InstanceHandle, rule: Rule) -> Result<(), HostError> {
        self.check(handle)?;
        Ok(self
            .broker
            .grant(&self.instances[&handle.serial].session, rule)?)
    }
    pub fn revoke(
        &mut self,
        handle: &InstanceHandle,
        rule: &Rule,
    ) -> Result<Vec<CapabilityReply>, HostError> {
        self.check(handle)?;
        Ok(self
            .broker
            .revoke(&self.instances[&handle.serial].session, rule)?)
    }
    /// Explicit, ordered submission only. Dispatch never executes a handler.
    pub fn submit(
        &mut self,
        request: &HostRequest,
        timeout: Duration,
    ) -> Result<Submission, HostError> {
        self.check(&request.instance)?;
        Ok(self.broker.submit(
            &self.instances[&request.instance.serial].session,
            request.intent.clone(),
            timeout,
        )?)
    }
    pub fn complete(
        &mut self,
        ticket: &Ticket,
        outcome: HandlerOutcome,
    ) -> Result<CapabilityReply, HostError> {
        Ok(self.broker.complete(ticket, outcome)?)
    }
    /// Broker time is explicitly injected; targeted Tick events do not advance it.
    pub fn advance_to(&mut self, now: Duration) -> Result<Vec<CapabilityReply>, HostError> {
        Ok(self.broker.advance_to(now)?)
    }
    /// Original broker/session routing only, never a name lookup. The caller
    /// still owns reply delivery scheduling and must avoid duplicate delivery.
    pub fn deliver(&mut self, reply: &CapabilityReply) -> Result<HostStep, HostError> {
        let serial = reply.id().session;
        let instance = self
            .instances
            .get_mut(&serial)
            .ok_or(HostError::UnknownInstance)?;
        let output = self
            .broker
            .deliver(&instance.session, reply, &mut instance.runtime)?;
        Ok(Self::step(self.handle(serial), output))
    }
    fn step(instance: InstanceHandle, output: StepOutput) -> HostStep {
        let requests = output
            .capability_requests
            .iter()
            .map(|intent| HostRequest {
                instance: instance.clone(),
                intent: intent.clone(),
            })
            .collect();
        HostStep {
            instance,
            output,
            requests,
        }
    }
    fn handle(&self, serial: u64) -> InstanceHandle {
        InstanceHandle {
            owner: self.owner.clone(),
            serial,
        }
    }
    fn check(&self, handle: &InstanceHandle) -> Result<(), HostError> {
        if !Arc::ptr_eq(&self.owner, &handle.owner) {
            return Err(HostError::ForeignHandle);
        }
        if !self.instances.contains_key(&handle.serial) {
            return Err(HostError::UnknownInstance);
        }
        Ok(())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        // Cooperative cancellation also runs on ordinary drop. Use clear() if
        // terminal replies must be observed. Native handlers must not panic.
        let _ = self.clear();
    }
}
