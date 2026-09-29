use mochi_core::{Scene, Vec2};
use mochi_renderer_software::Frame;
use std::fmt;

pub const MAX_SURFACE_PIXELS: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewError(pub &'static str);
impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ViewError {}

/// Aspect-preserving letterbox in physical pixels. Rendering and input share
/// the same scale/offset; a DPI factor must NOT be applied a second time.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    canvas: [u32; 2],
    surface: [u32; 2],
    scale: f64,
    offset: Vec2,
}
impl Viewport {
    /// Zero physical area means minimized/unconfigured, with no buffer to allocate.
    pub fn new(canvas: [u32; 2], surface: [u32; 2]) -> Result<Option<Self>, ViewError> {
        if canvas.contains(&0) {
            return Err(ViewError("canvas dimensions must be positive"));
        }
        let pixels = u64::from(surface[0]) * u64::from(surface[1]);
        if pixels > MAX_SURFACE_PIXELS || pixels > (isize::MAX as u64) / 4 {
            return Err(ViewError("window surface pixel budget exceeded"));
        }
        if surface.contains(&0) {
            return Ok(None);
        }
        let scale = (f64::from(surface[0]) / f64::from(canvas[0]))
            .min(f64::from(surface[1]) / f64::from(canvas[1]));
        let offset = Vec2::new(
            (f64::from(surface[0]) - f64::from(canvas[0]) * scale) / 2.0,
            (f64::from(surface[1]) - f64::from(canvas[1]) * scale) / 2.0,
        );
        Ok(Some(Self {
            canvas,
            surface,
            scale,
            offset,
        }))
    }
    pub fn surface_size(&self) -> [u32; 2] {
        self.surface
    }
    /// Physical pixels per canvas unit; includes letterbox scaling.
    pub fn scale(&self) -> f64 {
        self.scale
    }
    /// Project a copy into physical pixels. Input and simulation keep using the
    /// original logical Scene; uniform view scaling also preserves rotations.
    pub fn project_scene(&self, scene: &Scene) -> Scene {
        let mut projected = scene.clone();
        for node in &mut projected.nodes {
            node.transform.translation = Vec2::new(
                node.transform.translation.x * self.scale + self.offset.x,
                node.transform.translation.y * self.scale + self.offset.y,
            );
            node.transform.scale.x *= self.scale;
            node.transform.scale.y *= self.scale;
        }
        projected
    }
    /// Unclipped coordinates allow captured drags to continue outside the canvas.
    pub fn to_canvas(&self, physical: Vec2) -> Result<Vec2, ViewError> {
        let p = Vec2::new(
            (physical.x - self.offset.x) / self.scale,
            (physical.y - self.offset.y) / self.scale,
        );
        p.validate()
            .map_err(|_| ViewError("invalid pointer coordinate"))?;
        Ok(p)
    }
    /// Fill a softbuffer-compatible 0x00RRGGBB surface. Transparent RGBA pixels
    /// are flattened against the opaque matte; all buffer pixels are overwritten.
    pub fn write_rgb(
        &self,
        frame: &Frame,
        pixels: &mut [u32],
        matte: [u8; 3],
    ) -> Result<(), ViewError> {
        self.write_rgb_region(
            frame,
            pixels,
            matte,
            [0, 0, self.surface[0], self.surface[1]],
        )
    }
    /// Replace a half-open physical rectangle `[x0, y0, x1, y1]`, preserving
    /// pixels outside it. Sampling is identical to a complete `write_rgb`.
    /// The caller must ensure preserved pixels belong to the intended frame.
    pub fn write_rgb_region(
        &self,
        frame: &Frame,
        pixels: &mut [u32],
        matte: [u8; 3],
        region: [u32; 4],
    ) -> Result<(), ViewError> {
        if [frame.width(), frame.height()] != self.canvas {
            return Err(ViewError("frame/canvas dimensions differ"));
        }
        if pixels.len() != (u64::from(self.surface[0]) * u64::from(self.surface[1])) as usize {
            return Err(ViewError("surface buffer length differs"));
        }
        let [x0, y0, x1, y1] = region;
        if x0 > x1 || y0 > y1 || x1 > self.surface[0] || y1 > self.surface[1] {
            return Err(ViewError("surface region is out of bounds"));
        }
        if x0 == x1 || y0 == y1 {
            return Ok(());
        }
        let width = self.surface[0] as usize;
        let row_len = (x1 - x0) as usize;
        let matte_pixel = pack_rgb(matte);
        if self.canvas == self.surface {
            // Native-resolution frames already use the destination grid.
            // Avoid coordinate tables and divisions for every dirty tile.
            for y in y0 as usize..y1 as usize {
                let start = y * width + x0 as usize;
                let source = &frame.rgba8()[start * 4..(start + row_len) * 4];
                for (pixel, rgba) in pixels[start..start + row_len]
                    .iter_mut()
                    .zip(source.chunks_exact(4))
                {
                    let alpha = u32::from(rgba[3]);
                    if alpha == 255 {
                        *pixel = pack_rgb([rgba[0], rgba[1], rgba[2]]);
                    } else {
                        let mut rgb = [0; 3];
                        for c in 0..3 {
                            rgb[c] = ((u32::from(rgba[c]) * alpha
                                + u32::from(matte[c]) * (255 - alpha)
                                + 127)
                                / 255) as u8;
                        }
                        *pixel = pack_rgb(rgb);
                    }
                }
            }
            return Ok(());
        }
        // Pixel-center sampling is separable: compute X once per column, not
        // once per output pixel. Keep division/rounding identical to to_canvas.
        // u32::MAX cannot name a source pixel (the maximum index is width - 1).
        let mut columns = Vec::new();
        columns
            .try_reserve_exact(row_len)
            .map_err(|_| ViewError("cannot allocate viewport column map"))?;
        for x in x0..x1 {
            let source_x = (f64::from(x) + 0.5 - self.offset.x) / self.scale;
            columns.push(if source_x >= 0.0 && source_x < f64::from(self.canvas[0]) {
                source_x.floor() as u32
            } else {
                u32::MAX
            });
        }
        let stride = self.canvas[0] as usize * 4;
        let mut previous_y = None;
        for y in y0 as usize..y1 as usize {
            let start = y * width + x0 as usize;
            let source_y = (y as f64 + 0.5 - self.offset.y) / self.scale;
            if source_y < 0.0 || source_y >= f64::from(self.canvas[1]) {
                pixels[start..start + row_len].fill(matte_pixel);
                previous_y = None;
                continue;
            }
            let source_y = source_y.floor() as usize;
            if previous_y == Some(source_y) {
                // Nearest-neighbor upscaling often repeats complete rows.
                pixels.copy_within(start - width..start - width + row_len, start);
                continue;
            }
            let source = &frame.rgba8()[source_y * stride..(source_y + 1) * stride];
            for (pixel, &x) in pixels[start..start + row_len].iter_mut().zip(&columns) {
                *pixel = if x == u32::MAX {
                    matte_pixel
                } else {
                    let offset = x as usize * 4;
                    let rgba = &source[offset..offset + 4];
                    let alpha = u32::from(rgba[3]);
                    if alpha == 255 {
                        // The ordinary preview canvas is already opaque.
                        pack_rgb([rgba[0], rgba[1], rgba[2]])
                    } else {
                        let mut rgb = [0; 3];
                        for c in 0..3 {
                            rgb[c] = ((u32::from(rgba[c]) * alpha
                                + u32::from(matte[c]) * (255 - alpha)
                                + 127)
                                / 255) as u8;
                        }
                        pack_rgb(rgb)
                    }
                };
            }
            previous_y = Some(source_y);
        }
        Ok(())
    }
}

fn pack_rgb(rgb: [u8; 3]) -> u32 {
    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}
