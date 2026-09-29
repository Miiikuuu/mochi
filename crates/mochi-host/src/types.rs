use mochi_broker::BrokerError;
use mochi_core::{Appearance, Behavior, MateState, Scene, Vec2};
use mochi_package::{LoadedPackage, PackageError};
use mochi_runtime::{CapabilityIntent, RuntimeError, StepOutput};
use std::{fmt, sync::Arc};

/// Opaque, process-local identity. Names may be reused; handles may not.
#[derive(Debug, Clone)]
pub struct InstanceHandle {
    pub(crate) owner: Arc<()>,
    pub(crate) serial: u64,
}
impl PartialEq for InstanceHandle {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && self.serial == other.serial
    }
}
impl Eq for InstanceHandle {}

/// Presentation only: offset does not change simulation coordinates. Opacity
/// multiplies EACH node's opacity, not the opacity of an isolated group image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Presentation {
    pub offset: Vec2,
    pub opacity: f64,
    pub z_index: i32,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            offset: Vec2::default(),
            opacity: 1.0,
            z_index: 0,
        }
    }
}
impl Presentation {
    pub(crate) fn validate(self) -> Result<(), HostError> {
        if self.offset.validate().is_err()
            || !self.opacity.is_finite()
            || !(0.0..=1.0).contains(&self.opacity)
        {
            return Err(HostError::InvalidPresentation);
        }
        Ok(())
    }
}

/// Explicit trusted-host injection. Loaded packages cannot supply native code.
/// Host always creates a fresh Runtime, so broker sequence numbering starts at zero.
pub struct NativeMate {
    pub state: MateState,
    pub behaviors: Vec<Box<dyn Behavior>>,
    pub appearance: Box<dyn Appearance>,
}

/// Session-bound request envelope. Copying or delaying it cannot retarget it to
/// a replacement instance; submit failures leave it available for a safe retry.
#[derive(Debug, Clone)]
pub struct HostRequest {
    pub(crate) instance: InstanceHandle,
    pub(crate) intent: CapabilityIntent,
}
impl HostRequest {
    pub fn instance(&self) -> &InstanceHandle {
        &self.instance
    }
    pub fn intent(&self) -> &CapabilityIntent {
        &self.intent
    }
}

#[derive(Debug, Clone)]
#[must_use = "inspect capability requests even if rendering fails"]
pub struct HostStep {
    pub instance: InstanceHandle,
    pub output: StepOutput,
    pub requests: Vec<HostRequest>,
}

/// Borrowed resources prevent mutating/removing instances while this view is
/// live. Node IDs and asset keys are local to each layer, not globally renamed.
pub struct HostScene<'a> {
    pub instance_id: &'a str,
    pub package: &'a LoadedPackage,
    pub scene: Scene,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum HostError {
    ForeignHandle,
    UnknownInstance,
    DuplicateInstance,
    InstanceLimit,
    InvalidPresentation,
    Package(PackageError),
    Runtime(RuntimeError),
    Broker(BrokerError),
}
impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HostError {}
impl From<PackageError> for HostError {
    fn from(e: PackageError) -> Self {
        Self::Package(e)
    }
}
impl From<RuntimeError> for HostError {
    fn from(e: RuntimeError) -> Self {
        Self::Runtime(e)
    }
}
impl From<BrokerError> for HostError {
    fn from(e: BrokerError) -> Self {
        Self::Broker(e)
    }
}
