//! Textured triangles from the official Spine evaluator. PMA throughout filtering.
use crate::{raster::Budget, Frame, RenderError, RenderLayer, RenderOptions};
use mochi_core::{SceneContent, SceneNode};
use mochi_package::{ContentHash, LoadedPackage};
use mochi_spine::{Evaluator, Mesh};
use std::collections::BTreeMap;

pub(crate) enum SpineDraw {
    Complete,
    #[cfg(feature = "gpu")]
    Pending(mochi_raster_gpu::PendingFrame),
}

#[derive(Default)]
pub(crate) struct SpineCache {
    evaluators: BTreeMap<ContentHash, Evaluator>,
    #[cfg(feature = "gpu")]
    gpu: Option<mochi_raster_gpu::Raster>,
    #[cfg(feature = "gpu")]
    pub(crate) native_bgra: bool,
}
impl SpineCache {
    #[cfg(feature = "gpu")]
    pub(crate) fn enable_gpu(&mut self, options: RenderOptions) -> Result<String, RenderError> {
        let gpu = mochi_raster_gpu::Raster::new(options.width, options.height)
            .map_err(RenderError::InvalidOptions)?;
        let name = gpu.adapter_name().to_owned();
        self.gpu = Some(gpu);
        Ok(name)
    }
    #[cfg(feature = "gpu")]
    pub(crate) fn disable_gpu(&mut self) {
        self.gpu = None;
    }
    #[cfg(feature = "gpu-test-hooks")]
    pub(crate) fn test_lose_device_on_readback(&self) -> Result<(), RenderError> {
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| RenderError::InvalidOptions("GPU is not enabled".into()))?;
        gpu.test_lose_device_on_readback();
        Ok(())
    }
    pub(crate) fn retain(&mut self, layers: &[RenderLayer<'_>]) {
        self.evaluators
            .retain(|hash, _| layers.iter().any(|l| l.package.content_hash() == *hash));
        #[cfg(feature = "gpu")]
        if self.evaluators.is_empty() {
            if let Some(gpu) = &mut self.gpu {
                gpu.clear_textures();
            }
        }
    }
    pub(crate) fn draw(
        &mut self,
        package: &LoadedPackage,
        node: &SceneNode,
        frame: &mut Frame,
        options: RenderOptions,
        budget: &mut Budget,
        _defer: bool,
    ) -> Result<SpineDraw, RenderError> {
        let SceneContent::Skeletal {
            asset,
            animation,
            time_ns,
            size,
            offsets,
            overlay,
        } = &node.content
        else {
            unreachable!()
        };
        let resource = package
            .spine(asset)
            .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
        if node.opacity == 0.0 || node.transform.scale.x == 0.0 || node.transform.scale.y == 0.0 {
            return Ok(SpineDraw::Complete);
        }
        let key = package.content_hash();
        if !self.evaluators.contains_key(&key) {
            if self.evaluators.len() >= 8 {
                return Err(RenderError::LimitExceeded("Spine evaluators (8)"));
            }
            self.evaluators.insert(
                key,
                Evaluator::new(&resource.model)
                    .map_err(|e| RenderError::InvalidScene(e.to_string()))?,
            );
        }
        let offsets: Vec<_> = offsets
            .iter()
            .map(|o| mochi_spine::BoneOffset {
                bone: &o.bone,
                rotation_degrees: o.rotation_degrees,
                local_translation: [o.local_translation.x, o.local_translation.y],
                world_translation: [o.world_translation.x, o.world_translation.y],
            })
            .collect();
        let mut meshes = self
            .evaluators
            .get_mut(&key)
            .expect("inserted")
            .pose_with_overlay(animation, *time_ns as f64 / 1e9, &offsets, overlay)
            .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
        let t = &node.transform;
        let (sin, cos) = t.rotation_radians.sin_cos();
        let sx = size.x * t.scale.x;
        let sy = size.y * t.scale.y;
        if !sx.is_finite() || !sy.is_finite() || sx == 0.0 || sy == 0.0 {
            return Err(RenderError::InvalidTransform {
                node: node.id.clone(),
            });
        }
        for mesh in &mut meshes {
            for v in &mut mesh.vertices {
                let x = (f64::from(v[0]) - resource.camera.x) / resource.camera.width * sx;
                let y = (1.0 - (f64::from(v[1]) - resource.camera.y) / resource.camera.height) * sy;
                v[0] = (t.translation.x + cos * x - sin * y) as f32;
                v[1] = (t.translation.y + sin * x + cos * y) as f32;
                if !v[0].is_finite() || !v[1].is_finite() {
                    return Err(RenderError::InvalidTransform {
                        node: node.id.clone(),
                    });
                }
            }
        }
        #[cfg(feature = "gpu")]
        if let Some(gpu) = &mut self.gpu {
            // Account the same work budget before sending any geometry to GPU.
            for mesh in &meshes {
                for indices in mesh.indices.chunks_exact(3) {
                    let [x0, y0, x1, y1] = triangle_bounds(
                        &mesh.vertices[indices[0]],
                        &mesh.vertices[indices[1]],
                        &mesh.vertices[indices[2]],
                        options,
                    );
                    budget.samples = budget.samples.saturating_add(
                        x1.saturating_sub(x0) as u64 * y1.saturating_sub(y0) as u64 * 4,
                    );
                }
            }
            if budget.samples > options.limits.max_sampled_pixels {
                return Err(RenderError::LimitExceeded("sampled pixels"));
            }
            let textures = resource
                .model
                .pages
                .iter()
                .map(|page| {
                    let image = package
                        .image(&page.asset)
                        .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
                    Ok(mochi_raster_gpu::Texture {
                        key: *image.source_hash().as_bytes(),
                        width: image.width(),
                        height: image.height(),
                        rgba: image.rgba8(),
                    })
                })
                .collect::<Result<Vec<_>, RenderError>>()?;
            let pending = gpu
                .submit(
                    &meshes,
                    &textures,
                    mochi_raster_gpu::Clip {
                        tx: t.translation.x,
                        ty: t.translation.y,
                        cos,
                        sin,
                        sx,
                        sy,
                    },
                    node.opacity as f32,
                    &frame.rgba8,
                    _defer && self.native_bgra,
                )
                .map_err(|reason| match gpu.failure_reason() {
                    Some(_) => RenderError::GpuUnavailable(reason),
                    None => RenderError::InvalidScene(reason),
                })?;
            if _defer {
                return Ok(SpineDraw::Pending(pending));
            }
            frame.rgba8 = pending.finish().map_err(RenderError::GpuUnavailable)?.rgba8;
            return Ok(SpineDraw::Complete);
        }
        // A single bounded working buffer, no retained animation frames.
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(frame.rgba8.len())
            .map_err(|_| RenderError::AllocationFailed)?;
        for p in frame.rgba8.chunks_exact(4) {
            let a = f32::from(p[3]) / 255.0;
            buffer.extend([
                f32::from(p[0]) / 255.0 * a,
                f32::from(p[1]) / 255.0 * a,
                f32::from(p[2]) / 255.0 * a,
                a,
            ]);
        }
        let clip = Clip {
            tx: t.translation.x,
            ty: t.translation.y,
            cos,
            sin,
            sx,
            sy,
        };
        for mesh in &meshes {
            let image = package
                .image(&resource.model.pages[mesh.page].asset)
                .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
            let texture = Texture {
                pixels: image.rgba8(),
                width: image.width() as usize,
                height: image.height() as usize,
            };
            raster(
                mesh,
                &texture,
                &clip,
                node.opacity as f32,
                &mut buffer,
                options,
                budget,
            )?;
        }
        for (p, src) in frame.rgba8.chunks_exact_mut(4).zip(buffer.chunks_exact(4)) {
            if src[3] <= 0.0 {
                continue;
            }
            let a = src[3].clamp(0.0, 1.0);
            p[3] = (a * 255.0).round() as u8;
            for c in 0..3 {
                p[c] = if p[3] == 0 {
                    0
                } else {
                    (src[c] / a * 255.0).round().clamp(0.0, 255.0) as u8
                };
            }
        }
        Ok(SpineDraw::Complete)
    }
}
struct Clip {
    tx: f64,
    ty: f64,
    cos: f64,
    sin: f64,
    sx: f64,
    sy: f64,
}
impl Clip {
    fn contains(&self, x: f64, y: f64) -> bool {
        let dx = x - self.tx;
        let dy = y - self.ty;
        let u = (self.cos * dx + self.sin * dy) / self.sx;
        let v = (-self.sin * dx + self.cos * dy) / self.sy;
        (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v)
    }
}
struct Texture<'a> {
    pixels: &'a [u8],
    width: usize,
    height: usize,
}
impl Texture<'_> {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let tx = x as usize;
        let ty = y as usize;
        let fx = x - tx as f32;
        let fy = y - ty as f32;
        let indices = [
            (ty * self.width + tx) * 4,
            (ty * self.width + (tx + 1).min(self.width - 1)) * 4,
            (((ty + 1).min(self.height - 1)) * self.width + tx) * 4,
            (((ty + 1).min(self.height - 1)) * self.width + (tx + 1).min(self.width - 1)) * 4,
        ];
        let weights = [
            (1.0 - fx) * (1.0 - fy) / 255.0,
            fx * (1.0 - fy) / 255.0,
            (1.0 - fx) * fy / 255.0,
            fx * fy / 255.0,
        ];
        let mut out = [0.; 4];
        for (index, weight) in indices.into_iter().zip(weights) {
            for (c, value) in out.iter_mut().enumerate() {
                *value += f32::from(self.pixels[index + c]) * weight;
            }
        }
        out
    }
}
// Evaluate edge signs in f64: f32 cancellation can exclude a shared edge
// from both adjacent triangles and leave holes in otherwise solid regions.
fn edge(a: &[f32; 12], b: &[f32; 12], x: f32, y: f32) -> f64 {
    (f64::from(b[0]) - f64::from(a[0])) * (f64::from(y) - f64::from(a[1]))
        - (f64::from(b[1]) - f64::from(a[1])) * (f64::from(x) - f64::from(a[0]))
}
fn top_left(a: &[f32; 12], b: &[f32; 12]) -> bool {
    b[1] > a[1] || (b[1] == a[1] && b[0] < a[0])
}
fn triangle_bounds(
    a: &[f32; 12],
    b: &[f32; 12],
    c: &[f32; 12],
    options: RenderOptions,
) -> [usize; 4] {
    let x0 = (a[0].min(b[0]).min(c[0]) - 0.5)
        .ceil()
        .clamp(0.0, options.width as f32) as usize;
    let x1 =
        ((a[0].max(b[0]).max(c[0]) - 0.5).floor() + 1.0).clamp(0.0, options.width as f32) as usize;
    let y0 = (a[1].min(b[1]).min(c[1]) - 0.5)
        .ceil()
        .clamp(0.0, options.height as f32) as usize;
    let y1 =
        ((a[1].max(b[1]).max(c[1]) - 0.5).floor() + 1.0).clamp(0.0, options.height as f32) as usize;
    [x0, y0, x1, y1]
}
fn raster(
    mesh: &Mesh,
    texture: &Texture<'_>,
    clip: &Clip,
    opacity: f32,
    buffer: &mut [f32],
    options: RenderOptions,
    budget: &mut Budget,
) -> Result<(), RenderError> {
    for indices in mesh.indices.chunks_exact(3) {
        let a = &mesh.vertices[indices[0]];
        let mut b = &mesh.vertices[indices[1]];
        let mut c = &mesh.vertices[indices[2]];
        let mut area = edge(a, b, c[0], c[1]);
        if !area.is_finite() {
            return Err(RenderError::InvalidScene("unrepresentable triangle".into()));
        }
        if area.abs() < 1e-8 {
            continue;
        }
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
            area = -area;
        }
        let [x0, y0, x1, y1] = triangle_bounds(a, b, c, options);
        let samples = (x1.saturating_sub(x0) as u64) * (y1.saturating_sub(y0) as u64) * 4;
        budget.samples = budget
            .samples
            .checked_add(samples)
            .ok_or(RenderError::LimitExceeded("sample count overflow"))?;
        if budget.samples > options.limits.max_sampled_pixels {
            return Err(RenderError::LimitExceeded("sampled pixels"));
        }
        let tl = [top_left(b, c), top_left(c, a), top_left(a, b)];
        let inv = 1.0 / area;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let e = [edge(b, c, px, py), edge(c, a, px, py), edge(a, b, px, py)];
                if e.iter()
                    .zip(tl)
                    .any(|(v, top)| *v < 0.0 || (*v == 0.0 && !top))
                    || !clip.contains(px as f64, py as f64)
                {
                    continue;
                }
                let w = [
                    (e[0] * inv) as f32,
                    (e[1] * inv) as f32,
                    (e[2] * inv) as f32,
                ];
                let color = texture.sample(
                    a[6] * w[0] + b[6] * w[1] + c[6] * w[2],
                    a[7] * w[0] + b[7] * w[1] + c[7] * w[2],
                );
                // Spine emits constant attachment tint, including clipping vertices.
                let alpha = (color[3] * a[5] * opacity).clamp(0.0, 1.0);
                let at = (y * options.width as usize + x) * 4;
                for channel in 0..3 {
                    let source = ((color[3] - color[channel]) * a[8 + channel]
                        + color[channel] * a[2 + channel])
                        * opacity;
                    buffer[at + channel] = source + buffer[at + channel] * (1.0 - alpha);
                }
                buffer[at + 3] = alpha + buffer[at + 3] * (1.0 - alpha);
            }
        }
    }
    Ok(())
}
