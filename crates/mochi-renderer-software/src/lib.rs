//! Headless Sprite rendering and package-scoped composition. No filesystem paths or GUI.
mod coverage;
mod raster;
mod scaled_cache;
#[cfg(feature = "spine")]
mod spine;
#[cfg(not(feature = "spine"))]
#[path = "spine_disabled.rs"]
mod spine;

use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
use mochi_core::Scene;
use mochi_package::LoadedPackage;
use mochi_platform::{PlatformError, Renderer};
#[cfg(feature = "gpu")]
pub use mochi_raster_gpu::TransferStats as GpuTransferStats;
use std::{fmt, io::Write, sync::OnceLock};

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    InvalidOptions(String),
    InvalidScene(String),
    /// The GPU was disabled; retry the same scene on the software path.
    GpuUnavailable(String),
    WrongInstance {
        expected: String,
        actual: String,
    },
    UnsupportedContent {
        node: String,
    },
    MissingImage {
        node: String,
        asset: String,
    },
    InvalidTransform {
        node: String,
    },
    LimitExceeded(&'static str),
    AllocationFailed,
    PngEncoding(String),
}
impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(message) => write!(f, "invalid render options: {message}"),
            Self::InvalidScene(message) => write!(f, "invalid scene: {message}"),
            Self::GpuUnavailable(message) => write!(f, "GPU unavailable; using CPU: {message}"),
            Self::WrongInstance { expected, actual } => {
                write!(f, "renderer is bound to {expected}, received {actual}")
            }
            Self::UnsupportedContent { node } => write!(f, "unsupported content in node {node}"),
            Self::MissingImage { node, asset } => {
                write!(f, "node {node}: no decoded image for {asset}")
            }
            Self::InvalidTransform { node } => {
                write!(f, "node {node}: transform cannot be represented")
            }
            Self::LimitExceeded(limit) => write!(f, "render limit exceeded: {limit}"),
            Self::AllocationFailed => write!(f, "could not allocate frame buffer"),
            Self::PngEncoding(message) => write!(f, "PNG encoding failed: {message}"),
        }
    }
}
impl std::error::Error for RenderError {}

/// Host budgets. Work counts clipped bounding-box pixels times the sampling
/// bound (nearest: 1; analytic coverage: 4; coverage fallback: 16).
#[derive(Debug, Clone, Copy)]
pub struct RenderLimits {
    pub max_width: u32,
    pub max_height: u32,
    pub max_frame_bytes: u64,
    pub max_nodes: usize,
    pub max_sampled_pixels: u64,
}
impl Default for RenderLimits {
    fn default() -> Self {
        Self {
            max_width: 4096,
            max_height: 4096,
            max_frame_bytes: 64 * 1024 * 1024,
            max_nodes: 256,
            max_sampled_pixels: 16 * 1024 * 1024,
        }
    }
}

/// Host-selected spatial filtering. Simulation and source assets are unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SamplingMode {
    /// Pixel-center nearest-neighbor sampling, preserving the original output.
    #[default]
    Nearest,
    /// Area coverage for axis-aligned magnification; bounded 4x4 sampling for
    /// rotation/minification. Mix samples in premultiplied alpha space.
    Coverage,
}

/// Fixed viewport: origin (0, 0), positive Y downward, one logical unit per pixel.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    pub background: [u8; 4],
    pub limits: RenderLimits,
    pub sampling: SamplingMode,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 256,
            height: 128,
            background: [0; 4],
            limits: RenderLimits::default(),
            sampling: SamplingMode::Nearest,
        }
    }
}
impl RenderOptions {
    fn frame_bytes(self) -> Result<usize, RenderError> {
        if self.width == 0 || self.height == 0 {
            return Err(RenderError::InvalidOptions(
                "canvas dimensions must be positive".into(),
            ));
        }
        if self.width > self.limits.max_width || self.height > self.limits.max_height {
            return Err(RenderError::LimitExceeded("canvas dimensions"));
        }
        let bytes = u64::from(self.width)
            .checked_mul(u64::from(self.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(RenderError::LimitExceeded("frame byte size overflow"))?;
        if bytes > self.limits.max_frame_bytes || bytes > isize::MAX as u64 {
            return Err(RenderError::LimitExceeded("frame bytes"));
        }
        Ok(bytes as usize)
    }
}

/// Immutable pixels. RGBA access and PNG output use straight alpha.
/// Native BGRA is opt-in; recovering straight color from PMA8 is lossy.
pub struct Frame {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
    native_bgra: Option<Vec<u8>>,
    straight_cache: OnceLock<Vec<u8>>,
}
impl Frame {
    fn clear(options: RenderOptions) -> Result<Self, RenderError> {
        let count = options.frame_bytes()?;
        let mut rgba8 = Vec::new();
        rgba8
            .try_reserve_exact(count)
            .map_err(|_| RenderError::AllocationFailed)?;
        rgba8.resize(count, 0);
        for pixel in rgba8.chunks_exact_mut(4) {
            pixel.copy_from_slice(&options.background);
        }
        Ok(Self {
            width: options.width,
            height: options.height,
            rgba8,
            native_bgra: None,
            straight_cache: OnceLock::new(),
        })
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn rgba8(&self) -> &[u8] {
        match &self.native_bgra {
            Some(bytes) => self.straight_cache.get_or_init(|| {
                bytes
                    .chunks_exact(4)
                    .flat_map(Self::straight_pixel)
                    .collect()
            }),
            None => &self.rgba8,
        }
    }
    /// Native upload bytes when the renderer's preference was applicable.
    pub fn premultiplied_bgra8(&self) -> Option<&[u8]> {
        self.native_bgra.as_deref()
    }
    fn straight_pixel(p: &[u8]) -> [u8; 4] {
        let alpha = u32::from(p[3]);
        if alpha == 0 {
            return [0; 4];
        }
        let straight = |c| ((u32::from(c) * 255 + alpha / 2) / alpha).min(255) as u8;
        [straight(p[2]), straight(p[1]), straight(p[0]), p[3]]
    }
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * 4;
        if let Some(bytes) = &self.native_bgra {
            return Some(Self::straight_pixel(&bytes[offset..offset + 4]));
        }
        Some(
            self.rgba8[offset..offset + 4]
                .try_into()
                .expect("validated frame layout"),
        )
    }
    /// The caller owns output I/O. A failed writer may contain a partial PNG.
    pub fn write_png(&self, writer: impl Write) -> Result<(), RenderError> {
        PngEncoder::new(writer)
            .write_image(
                self.rgba8(),
                self.width,
                self.height,
                ExtendedColorType::Rgba8,
            )
            .map_err(|e| RenderError::PngEncoding(e.to_string()))
    }
}

/// Bound to exactly one host-selected instance and imported package.
/// The last successful frame is retained; failed draws never publish partial pixels.
pub struct SoftwareRenderer<'a> {
    instance_id: String,
    package: &'a LoadedPackage,
    canvas: CanvasRenderer,
}
impl<'a> SoftwareRenderer<'a> {
    pub fn new(
        instance_id: impl Into<String>,
        package: &'a LoadedPackage,
        options: RenderOptions,
    ) -> Result<Self, RenderError> {
        let instance_id = instance_id.into();
        if instance_id.trim().is_empty() || instance_id.len() > 256 {
            return Err(RenderError::InvalidOptions(
                "instance id must contain 1..=256 bytes".into(),
            ));
        }
        Ok(Self {
            instance_id,
            package,
            canvas: CanvasRenderer::new(options)?,
        })
    }
    /// Initial frame is the configured background; subsequent frames are committed draws.
    pub fn frame(&self) -> &Frame {
        self.canvas.frame()
    }

    pub fn render_frame(&mut self, instance_id: &str, scene: &Scene) -> Result<(), RenderError> {
        if instance_id != self.instance_id {
            return Err(RenderError::WrongInstance {
                expected: self.instance_id.clone(),
                actual: instance_id.into(),
            });
        }
        self.canvas.render_layers(&[RenderLayer {
            package: self.package,
            scene,
        }])
    }
}
impl Renderer for SoftwareRenderer<'_> {
    fn render(&mut self, instance_id: &str, scene: &Scene) -> Result<(), PlatformError> {
        self.render_frame(instance_id, scene)
            .map_err(|e| PlatformError::Adapter(e.to_string()))
    }
}

/// Each layer resolves its asset keys against its own immutable package.
/// Slice order is painter order. IDs need be unique only within each Scene.
#[derive(Clone, Copy)]
pub struct RenderLayer<'a> {
    pub package: &'a LoadedPackage,
    pub scene: &'a Scene,
}

/// Bounds even empty layers. Node and sampled-pixel budgets are global per frame.
pub const MAX_LAYERS: usize = 256;

/// Shared canvas; no per-Mate offscreen buffers or retained package references.
/// All nodes draw directly into one working frame, preserving source-over rounding.
pub struct CanvasRenderer {
    options: RenderOptions,
    frame: Frame,
    cache: Option<scaled_cache::ScaledCache>,
    spine: spine::SpineCache,
    #[cfg(feature = "gpu")]
    pending: Option<(Frame, mochi_raster_gpu::PendingFrame)>,
    #[cfg(feature = "gpu")]
    transfer: GpuTransferStats,
    #[cfg(feature = "gpu")]
    gpu_failure: Option<String>,
}
impl CanvasRenderer {
    pub fn new(options: RenderOptions) -> Result<Self, RenderError> {
        Ok(Self {
            options,
            frame: Frame::clear(options)?,
            cache: None,
            spine: spine::SpineCache::default(),
            #[cfg(feature = "gpu")]
            pending: None,
            #[cfg(feature = "gpu")]
            transfer: GpuTransferStats::default(),
            #[cfg(feature = "gpu")]
            gpu_failure: None,
        })
    }
    /// Host opt-in: prefilter axis-aligned minification once per image/size,
    /// then retain continuous coverage for translation and magnification.
    /// Cache bytes and entry count are bounded; eviction never drops frames.
    /// Zero disables the cache and preserves the original sampling exactly.
    pub fn with_scaled_cache(
        options: RenderOptions,
        max_bytes: usize,
    ) -> Result<Self, RenderError> {
        let mut renderer = Self::new(options)?;
        if max_bytes != 0 {
            renderer.cache = Some(scaled_cache::ScaledCache::new(max_bytes));
        }
        Ok(renderer)
    }
    pub fn frame(&self) -> &Frame {
        &self.frame
    }
    /// Opt in to shared-device GPU Spine rasterization. Sprites keep the
    /// existing sampling path. Initialization failure leaves software usable.
    #[cfg(feature = "gpu")]
    pub fn enable_spine_gpu(&mut self) -> Result<String, RenderError> {
        let name = self.spine.enable_gpu(self.options)?;
        self.gpu_failure = None;
        Ok(name)
    }
    /// Prefer PMA BGRA8 for a single GPU skeletal draw. Mixed scenes and CPU
    /// fallback retain straight RGBA. Native callers should read BGRA directly;
    /// requesting RGBA/PNG lazily unpremultiplies with PMA8 precision loss.
    #[cfg(feature = "gpu")]
    pub fn prefer_premultiplied_bgra(&mut self, enabled: bool) {
        self.spine.native_bgra = enabled;
    }
    /// First runtime GPU failure, retained for diagnostics. GPU stays disabled
    /// for this canvas; new canvases also reject the failed shared device.
    #[cfg(feature = "gpu")]
    pub fn gpu_fallback_reason(&self) -> Option<&str> {
        self.gpu_failure.as_deref()
    }
    #[cfg(feature = "gpu")]
    fn fail_gpu(&mut self, reason: String) -> RenderError {
        self.cancel_render();
        self.spine.disable_gpu();
        self.gpu_failure.get_or_insert_with(|| reason.clone());
        RenderError::GpuUnavailable(reason)
    }
    #[cfg(feature = "gpu-test-hooks")]
    pub fn test_lose_spine_device_on_readback(&self) -> Result<(), RenderError> {
        self.spine.test_lose_device_on_readback()
    }
    /// Any failure preserves the last complete frame, including an error during
    /// drawing a later layer. An empty layer list commits the background.
    pub fn render_layers(&mut self, layers: &[RenderLayer<'_>]) -> Result<(), RenderError> {
        self.begin_render_layers(layers)?;
        match self.finish_render() {
            #[cfg(feature = "gpu")]
            Err(RenderError::GpuUnavailable(_)) => {
                // Same immutable scene/absolute time; never dispatch another Tick.
                self.begin_render_layers(layers)
            }
            result => result,
        }
    }
    /// Queue a single GPU skeletal layer without waiting. CPU/mixed layers are
    /// completed synchronously. A queued GPU draw preserves the old frame until finish.
    pub fn begin_render_layers(&mut self, layers: &[RenderLayer<'_>]) -> Result<(), RenderError> {
        let result = self.begin_frame(layers);
        #[cfg(feature = "gpu")]
        if let Err(RenderError::GpuUnavailable(reason)) = result {
            self.fail_gpu(reason);
            // Discard the working frame and restart all layers with fresh budgets.
            return self.begin_frame(layers);
        }
        result
    }
    fn begin_frame(&mut self, layers: &[RenderLayer<'_>]) -> Result<(), RenderError> {
        #[cfg(feature = "gpu")]
        {
            if self.pending.is_some() {
                return Err(RenderError::InvalidOptions(
                    "finish or cancel the pending frame first".into(),
                ));
            }
            self.transfer = GpuTransferStats::default();
        }
        if layers.len() > MAX_LAYERS {
            return Err(RenderError::LimitExceeded("layers"));
        }
        let mut budget = raster::Budget::default();
        let mut sprites = Vec::new();
        for layer in layers {
            sprites.extend(raster::prepare(
                layer.package,
                layer.scene,
                self.options,
                &mut budget,
            )?);
        }
        let mut next = Frame::clear(self.options)?;
        self.spine.retain(layers);
        #[cfg(feature = "spine")]
        let defer = sprites.len() == 1;
        for draw in sprites {
            match draw {
                raster::Draw::Sprite(sprite) => {
                    if let Some(cache) = &mut self.cache {
                        cache.draw(&sprite, &mut next, self.options.sampling)?;
                    } else {
                        raster::draw(&sprite, &mut next, self.options.sampling)?;
                    }
                }
                #[cfg(feature = "spine")]
                raster::Draw::Skeletal { package, node } => {
                    match self.spine.draw(
                        package,
                        node,
                        &mut next,
                        self.options,
                        &mut budget,
                        defer,
                    )? {
                        spine::SpineDraw::Complete => {}
                        #[cfg(feature = "gpu")]
                        spine::SpineDraw::Pending(job) => {
                            self.pending = Some((next, job));
                            return Ok(());
                        }
                    }
                }
            }
        }
        self.frame = next;
        Ok(())
    }
    /// Commit the queued frame. On GpuUnavailable, GPU is already disabled and
    /// the previous complete frame is retained. Deferred callers must re-submit
    /// the SAME layers; render_layers performs this retry automatically.
    pub fn finish_render(&mut self) -> Result<(), RenderError> {
        #[cfg(feature = "gpu")]
        if let Some((mut next, job)) = self.pending.take() {
            let result = match job.finish() {
                Ok(result) => result,
                Err(reason) => return Err(self.fail_gpu(reason)),
            };
            if result.native_bgra {
                next.rgba8 = Vec::new();
                next.native_bgra = Some(result.rgba8);
            } else {
                next.rgba8 = result.rgba8;
            }
            self.transfer = result.stats;
            self.frame = next;
        }
        Ok(())
    }
    /// Cancel a queued GPU frame; preserve the last committed pixels.
    pub fn cancel_render(&mut self) {
        #[cfg(feature = "gpu")]
        {
            self.pending = None;
            self.transfer = GpuTransferStats::default();
        }
    }
    #[cfg(feature = "gpu")]
    pub fn gpu_transfer_stats(&self) -> GpuTransferStats {
        self.transfer
    }
}
