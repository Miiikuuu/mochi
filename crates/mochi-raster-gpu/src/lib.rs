//! Bounded offscreen triangles. No windows, packages, filesystem or model code.
use mochi_core::Mesh;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
};
use wgpu::util::DeviceExt;

type Result<T> = std::result::Result<T, String>;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

// wgpu 24 routes some Device::poll / driver failures to a fatal panic rather
// than the error callback. Confine unwind recovery to GPU operations; the
// poisoned device is never reused. Scene/evaluator code is outside this guard.
fn gpu_call<T>(failure: &OnceLock<String>, call: impl FnOnce() -> T) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|payload| {
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("unknown GPU panic");
        let _ = failure.set(format!("GPU backend panic: {message}"));
        failure.get().expect("failure recorded").clone()
    })
}

pub struct Texture<'a> {
    pub key: [u8; 32],
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}
#[derive(Clone, Copy)]
pub struct Clip {
    pub tx: f64,
    pub ty: f64,
    pub cos: f64,
    pub sin: f64,
    pub sx: f64,
    pub sy: f64,
}
struct Shared {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    resolve_native: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    name: String,
    failure: Arc<OnceLock<String>>,
    #[cfg(feature = "test-hooks")]
    lose_on_readback: AtomicBool,
}
impl Shared {
    fn get() -> Result<Arc<Self>> {
        static SHARED: OnceLock<Result<Arc<Shared>>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                gpu_call(&OnceLock::new(), || {
                    pollster::block_on(Self::new()).map(Arc::new)
                })?
            })
            .clone()
    }
    async fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .ok_or("no GPU adapter")?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return Err("only a CPU GPU adapter is available".into());
        }
        let name = adapter.get_info().name;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Mochi shared offscreen device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits {
                        max_texture_dimension_2d: 4096,
                        ..wgpu::Limits::downlevel_defaults()
                    },
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        // wgpu's default uncaptured-error handler panics. Record a shared,
        // permanent failure instead; renderers can finish the same frame on CPU.
        let failure = Arc::new(OnceLock::new());
        let lost = failure.clone();
        device.set_device_lost_callback(move |reason, message| {
            let _ = lost.set(format!("device lost ({reason:?}): {message}"));
        });
        let errors = failure.clone();
        device.on_uncaptured_error(Box::new(move |error| {
            let _ = errors.set(format!("GPU backend error: {error}"));
        }));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Spine PMA tint and camera clipping"),
            source: wgpu::ShaderSource::Wgsl(include_str!("spine.wgsl").into()),
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Spine triangles"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 56,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp PMA"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let make_resolve = |entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("PMA float to output bytes"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("resolve_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let resolve = make_resolve("resolve_fs");
        let resolve_native = make_resolve("resolve_native_fs");
        if let Some(reason) = failure.get() {
            return Err(reason.clone());
        }
        Ok(Self {
            device,
            queue,
            layout,
            pipeline,
            resolve,
            resolve_native,
            sampler,
            name,
            failure,
            #[cfg(feature = "test-hooks")]
            lose_on_readback: AtomicBool::new(false),
        })
    }
    fn check(&self) -> Result<()> {
        match self.failure.get() {
            Some(reason) => Err(reason.clone()),
            None => Ok(()),
        }
    }
    fn fail(&self, reason: String) -> String {
        let _ = self.failure.set(reason);
        self.failure.get().expect("failure recorded").clone()
    }
    fn texture(
        &self,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    }
}

pub struct Raster {
    shared: Arc<Shared>,
    width: u32,
    height: u32,
    target: wgpu::Texture,
    resolved: wgpu::Texture,
    resolve_group: wgpu::BindGroup,
    readback: wgpu::Buffer,
    busy: Arc<AtomicBool>,
    textures: BTreeMap<[u8; 32], wgpu::BindGroup>,
}
impl Raster {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err("GPU canvas dimensions exceed budget".into());
        }
        let shared = Shared::get()?;
        let failure = shared.failure.clone();
        gpu_call(&failure, || Self::with_shared(width, height, shared))?
    }
    fn with_shared(width: u32, height: u32, shared: Arc<Shared>) -> Result<Self> {
        shared.check()?;
        let row = (width * 4).div_ceil(256) * 256;
        let target = shared.texture(
            width,
            height,
            FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
        );
        let resolved = shared.texture(
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let resolve_group = shared.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &shared.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &target.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shared.sampler),
                },
            ],
        });
        let readback = shared.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded frame readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        shared.check()?;
        Ok(Self {
            shared,
            width,
            height,
            target,
            resolved,
            resolve_group,
            readback,
            busy: Arc::new(AtomicBool::new(false)),
            textures: BTreeMap::new(),
        })
    }
    pub fn adapter_name(&self) -> &str {
        &self.shared.name
    }
    /// Permanent device/backend failure shared by every raster on this device.
    /// Input validation errors do not set this state.
    pub fn failure_reason(&self) -> Option<&str> {
        self.shared.failure.get().map(String::as_str)
    }
    /// Destroy only this process's shared wgpu device at its next readback.
    /// Explicit diagnostic builds only; never resets the OS graphics driver.
    #[cfg(feature = "test-hooks")]
    pub fn test_lose_device_on_readback(&self) {
        self.shared.lose_on_readback.store(true, Ordering::Release);
    }
    pub fn clear_textures(&mut self) {
        self.textures.clear();
    }

    /// Existing pixels are straight alpha; textures and internal composition are
    /// PMA. Returns straight RGBA only after the complete GPU draw succeeds.
    pub fn draw(
        &mut self,
        meshes: &[Mesh],
        textures: &[Texture<'_>],
        clip: Clip,
        opacity: f32,
        previous: &[u8],
    ) -> Result<Vec<u8>> {
        Ok(self
            .submit(meshes, textures, clip, opacity, previous, false)?
            .finish()?
            .rgba8)
    }

    /// Submit without a CPU wait. At most one outstanding readback per raster;
    /// dropping its ticket cancels mapping and safely releases the slot.
    pub fn submit(
        &mut self,
        meshes: &[Mesh],
        textures: &[Texture<'_>],
        clip: Clip,
        opacity: f32,
        previous: &[u8],
        native_bgra: bool,
    ) -> Result<PendingFrame> {
        let failure = self.shared.failure.clone();
        gpu_call(&failure, || {
            self.submit_inner(meshes, textures, clip, opacity, previous, native_bgra)
        })?
    }
    fn submit_inner(
        &mut self,
        meshes: &[Mesh],
        textures: &[Texture<'_>],
        clip: Clip,
        opacity: f32,
        previous: &[u8],
        native_bgra: bool,
    ) -> Result<PendingFrame> {
        self.shared.check()?;
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("GPU readback already pending".into());
        }
        let mut lease = ReadbackLease {
            buffer: self.readback.clone(),
            busy: self.busy.clone(),
            failure: self.shared.failure.clone(),
            mapping: false,
        };
        if previous.len() != self.width as usize * self.height as usize * 4
            || textures.len() > 8
            || meshes.len() > 2048
            || !opacity.is_finite()
            || !(0.0..=1.0).contains(&opacity)
            || [clip.tx, clip.ty, clip.cos, clip.sin, clip.sx, clip.sy]
                .iter()
                .any(|x| !x.is_finite())
            || clip.sx == 0.
            || clip.sy == 0.
        {
            return Err("invalid GPU draw inputs".into());
        }
        let count: usize = meshes
            .iter()
            .map(|m| m.vertices.len() * 12 + m.indices.len() + 3)
            .sum();
        if count > 1048576 {
            return Err("GPU geometry budget exceeded".into());
        }
        let mut vertices: Vec<[f32; 14]> = Vec::new();
        let mut indices = Vec::new();
        let mut draws: Vec<(usize, std::ops::Range<u32>)> = Vec::new();
        for mesh in meshes {
            if mesh.page >= textures.len()
                || mesh.indices.len() % 3 != 0
                || mesh.indices.iter().any(|i| *i >= mesh.vertices.len())
            {
                return Err("invalid GPU indices".into());
            }
            let base = vertices.len() as u32;
            for v in &mesh.vertices {
                if v.iter().any(|x| !x.is_finite()) {
                    return Err("nonfinite GPU vertex".into());
                }
                let mut vertex = [0.; 14];
                vertex[..12].copy_from_slice(v);
                vertex[0] = v[0] / self.width as f32 * 2. - 1.;
                vertex[1] = 1. - v[1] / self.height as f32 * 2.;
                let dx = f64::from(v[0]) - clip.tx;
                let dy = f64::from(v[1]) - clip.ty;
                vertex[12] = ((clip.cos * dx + clip.sin * dy) / clip.sx) as f32;
                vertex[13] = ((-clip.sin * dx + clip.cos * dy) / clip.sy) as f32;
                for c in [2, 3, 4, 5, 8, 9, 10] {
                    vertex[c] *= opacity;
                }
                if vertex.iter().any(|x| !x.is_finite()) {
                    return Err("GPU transform overflow".into());
                }
                vertices.push(vertex);
            }
            let start = indices.len() as u32;
            indices.extend(mesh.indices.iter().map(|i| base + *i as u32));
            if let Some((page, range)) = draws.last_mut().filter(|(page, _)| *page == mesh.page) {
                let _ = page;
                range.end = indices.len() as u32;
            } else {
                draws.push((mesh.page, start..indices.len() as u32));
            }
        }
        self.textures
            .retain(|key, _| textures.iter().any(|t| &t.key == key));
        let mut texture_bytes = 0u64;
        for t in textures {
            if t.width == 0 || t.height == 0 || t.width > 4096 || t.height > 4096 {
                return Err("GPU texture dimensions exceed budget".into());
            }
            let bytes = u64::from(t.width) * u64::from(t.height) * 4;
            texture_bytes += bytes;
            if t.width == 0
                || t.height == 0
                || t.width > 4096
                || t.height > 4096
                || bytes != t.rgba.len() as u64
                || texture_bytes > 64 * 1024 * 1024
            {
                return Err("GPU texture budget exceeded".into());
            }
            if self.textures.contains_key(&t.key) {
                continue;
            }
            let texture = self.shared.texture(
                t.width,
                t.height,
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );
            self.shared.queue.write_texture(
                texture.as_image_copy(),
                t.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(t.width * 4),
                    rows_per_image: Some(t.height),
                },
                texture.size(),
            );
            let group = self
                .shared
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cached immutable atlas"),
                    layout: &self.shared.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(
                                &texture.create_view(&Default::default()),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.shared.sampler),
                        },
                    ],
                });
            self.textures.insert(t.key, group);
        }
        let clear = previous.chunks_exact(4).all(|p| p[3] == 0);
        let crop = readback_bounds(meshes, self.width, self.height, !clear);
        let row_bytes = (crop[2] * 4).div_ceil(256) * 256;
        let readback_bytes = u64::from(row_bytes) * u64::from(crop[3]);
        if !clear {
            let initial: Vec<u16> = previous
                .chunks_exact(4)
                .flat_map(|p| {
                    let a = f32::from(p[3]) / 255.;
                    [
                        f32::from(p[0]) / 255. * a,
                        f32::from(p[1]) / 255. * a,
                        f32::from(p[2]) / 255. * a,
                        a,
                    ]
                    .map(|v| half::f16::from_f32(v).to_bits())
                })
                .collect();
            self.shared.queue.write_texture(
                self.target.as_image_copy(),
                bytemuck::cast_slice(&initial),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 8),
                    rows_per_image: Some(self.height),
                },
                self.target.size(),
            );
        }
        let vertex_buffer =
            self.shared
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
        let index_buffer =
            self.shared
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
        let mut encoder = self
            .shared
            .device
            .create_command_encoder(&Default::default());
        let view = self.target.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if clear {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shared.pipeline);
            pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for (page, range) in draws {
                pass.set_bind_group(0, &self.textures[&textures[page].key], &[]);
                pass.draw_indexed(range, 0, 0..1);
            }
        }
        let resolved_view = self.resolved.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolve RGBA8"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resolved_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(if native_bgra {
                &self.shared.resolve_native
            } else {
                &self.shared.resolve
            });
            pass.set_bind_group(0, &self.resolve_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.resolved,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: crop[0],
                    y: crop[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(crop[3]),
                },
            },
            wgpu::Extent3d {
                width: crop[2],
                height: crop[3],
                depth_or_array_layers: 1,
            },
        );
        let submission = self.shared.queue.submit([encoder.finish()]);
        self.shared.check()?;
        let (tx, rx) = std::sync::mpsc::channel();
        lease.mapping = true;
        lease
            .buffer
            .slice(..readback_bytes)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        Ok(PendingFrame {
            lease,
            shared: self.shared.clone(),
            submission,
            receiver: rx,
            width: self.width,
            height: self.height,
            crop,
            row_bytes,
            readback_bytes,
            native_bgra,
        })
    }
}

/// Pixel-space conservative bounds; one-pixel guard covers raster edge rounding.
fn readback_bounds(meshes: &[Mesh], width: u32, height: u32, has_background: bool) -> [u32; 4] {
    if has_background {
        return [0, 0, width, height];
    }
    let mut min = [width as f32, height as f32];
    let mut max = [0_f32; 2];
    for mesh in meshes {
        // Unreferenced vertices cannot contribute pixels.
        for &index in &mesh.indices {
            let v = &mesh.vertices[index];
            for c in 0..2 {
                min[c] = min[c].min(v[c]);
                max[c] = max[c].max(v[c]);
            }
        }
    }
    let x = (min[0].floor() - 1.).clamp(0., width as f32) as u32;
    let y = (min[1].floor() - 1.).clamp(0., height as f32) as u32;
    let right = (max[0].ceil() + 1.).clamp(0., width as f32) as u32;
    let bottom = (max[1].ceil() + 1.).clamp(0., height as f32) as u32;
    if right <= x || bottom <= y {
        [0, 0, 1, 1]
    } else {
        [x, y, right - x, bottom - y]
    }
}

struct ReadbackLease {
    buffer: wgpu::Buffer,
    busy: Arc<AtomicBool>,
    failure: Arc<OnceLock<String>>,
    mapping: bool,
}
impl Drop for ReadbackLease {
    fn drop(&mut self) {
        if self.mapping {
            let _ = gpu_call(&self.failure, || self.buffer.unmap());
        }
        self.busy.store(false, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransferStats {
    /// CPU wall time awaiting a submitted batch, not GPU execution duration.
    pub wait: std::time::Duration,
    pub copy: std::time::Duration,
    /// Includes row alignment, excludes untouched transparent regions.
    pub bytes: u64,
}

pub struct ReadbackFrame {
    /// BGRA premultiplied when native_bgra is true, otherwise straight RGBA.
    pub rgba8: Vec<u8>,
    pub stats: TransferStats,
    pub native_bgra: bool,
}

pub struct PendingFrame {
    lease: ReadbackLease,
    shared: Arc<Shared>,
    submission: wgpu::SubmissionIndex,
    receiver: std::sync::mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>>,
    width: u32,
    height: u32,
    crop: [u32; 4],
    row_bytes: u32,
    readback_bytes: u64,
    native_bgra: bool,
}
impl PendingFrame {
    pub fn finish(self) -> Result<ReadbackFrame> {
        let failure = self.shared.failure.clone();
        gpu_call(&failure, || self.finish_inner())?
    }
    fn finish_inner(self) -> Result<ReadbackFrame> {
        #[cfg(feature = "test-hooks")]
        if self.shared.lose_on_readback.swap(false, Ordering::AcqRel) {
            self.shared.device.destroy();
        }
        self.shared.check()?;
        let started = std::time::Instant::now();
        self.shared
            .device
            .poll(wgpu::Maintain::WaitForSubmissionIndex(
                self.submission.clone(),
            ));
        self.shared.check()?;
        // Poll normally delivers the map callback. Do not wait forever on a
        // disconnected/missing callback after the driver has returned from poll.
        self.receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| {
                self.shared
                    .fail(format!("GPU readback callback failed: {e}"))
            })?
            .map_err(|e| {
                self.shared
                    .fail(format!("GPU readback mapping failed: {e}"))
            })?;
        let wait = started.elapsed();
        let started = std::time::Instant::now();
        let mut rgba8 = vec![0; self.width as usize * self.height as usize * 4];
        {
            let data = self
                .lease
                .buffer
                .slice(..self.readback_bytes)
                .get_mapped_range();
            for (y, row) in data.chunks_exact(self.row_bytes as usize).enumerate() {
                let at =
                    ((y + self.crop[1] as usize) * self.width as usize + self.crop[0] as usize) * 4;
                rgba8[at..at + self.crop[2] as usize * 4]
                    .copy_from_slice(&row[..self.crop[2] as usize * 4]);
            }
        }
        self.shared.check()?;
        Ok(ReadbackFrame {
            rgba8,
            native_bgra: self.native_bgra,
            stats: TransferStats {
                wait,
                copy: started.elapsed(),
                bytes: self.readback_bytes,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gpu_unwind_is_reported_and_first_failure_is_retained() {
        let failure = OnceLock::new();
        let error = gpu_call(&failure, || panic!("simulated fatal poll error")).unwrap_err();
        assert!(error.contains("simulated fatal poll error"));
        let second = gpu_call(&failure, || panic!("secondary cleanup failure")).unwrap_err();
        assert_eq!(error, second);
    }
    #[test]
    #[ignore = "requires hardware GPU; creates its own isolated device"]
    fn gpu_backend_callback_records_errors_without_panicking() {
        let shared = pollster::block_on(Shared::new()).unwrap();
        let _invalid = shared.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("intentional invalid usage for callback regression"),
            size: 16,
            usage: wgpu::BufferUsages::empty(),
            mapped_at_creation: false,
        });
        assert!(shared.check().unwrap_err().contains("GPU backend error"));
    }
    #[test]
    fn readback_bounds_cover_edges_clip_to_canvas_and_ignore_unused_vertices() {
        let mut vertices = vec![[0.; 12]; 4];
        vertices[0][..2].copy_from_slice(&[-2., 3.2]);
        vertices[1][..2].copy_from_slice(&[5.1, 3.2]);
        vertices[2][..2].copy_from_slice(&[5.1, 8.3]);
        vertices[3][..2].copy_from_slice(&[1000., 1000.]);
        let meshes = [Mesh {
            page: 0,
            vertices,
            indices: vec![0, 1, 2],
        }];
        assert_eq!(readback_bounds(&meshes, 64, 64, false), [0, 2, 7, 8]);
        assert_eq!(readback_bounds(&meshes, 4, 4, false), [0, 2, 4, 2]);
        assert_eq!(readback_bounds(&meshes, 64, 64, true), [0, 0, 64, 64]);
        assert_eq!(readback_bounds(&[], 64, 64, false), [0, 0, 1, 1]);
    }
}
