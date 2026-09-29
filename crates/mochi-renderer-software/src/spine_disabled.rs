//! Explicit unsupported backend. No Spine runtime is linked in the public lane.
use crate::RenderLayer;
#[cfg(feature = "gpu")]
use crate::{RenderError, RenderOptions};

#[derive(Default)]
pub(crate) struct SpineCache {
    #[cfg(feature = "gpu")]
    pub(crate) native_bgra: bool,
}

impl SpineCache {
    pub(crate) fn retain(&mut self, _layers: &[RenderLayer<'_>]) {}

    #[cfg(feature = "gpu")]
    pub(crate) fn enable_gpu(&mut self, _options: RenderOptions) -> Result<String, RenderError> {
        Err(RenderError::InvalidOptions(
            "Spine support is not included in this build".into(),
        ))
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn disable_gpu(&mut self) {}

    #[cfg(feature = "gpu-test-hooks")]
    pub(crate) fn test_lose_device_on_readback(&self) -> Result<(), RenderError> {
        Err(RenderError::InvalidOptions(
            "Spine support is not included in this build".into(),
        ))
    }
}
