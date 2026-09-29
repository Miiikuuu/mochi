//! Bounded, content-addressed prefiltering for the native host. Package pixels
//! stay immutable; cache entries never retain packages or animation state.
use crate::{
    raster::{self, ImageView, Sprite},
    Frame, RenderError, SamplingMode,
};
use mochi_package::ContentHash;
use std::collections::BTreeMap;

const MAX_ENTRIES: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    source: ContentHash,
    source_size: [u32; 2],
    size: [u32; 2],
}
struct Entry {
    pixels: Vec<u8>,
    alpha_bounds: [u32; 4],
    used: u64,
}
pub(crate) struct ScaledCache {
    entries: BTreeMap<Key, Entry>,
    bytes: usize,
    limit: usize,
    clock: u64,
}
impl ScaledCache {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            bytes: 0,
            limit,
            clock: 0,
        }
    }
    pub(crate) fn draw(
        &mut self,
        sprite: &Sprite<'_>,
        frame: &mut Frame,
        sampling: SamplingMode,
    ) -> Result<(), RenderError> {
        // Arbitrary rotations retain the bounded reference sampler.
        if sprite.sin != 0.0 || sprite.cos != 1.0 {
            return raster::draw(sprite, frame, sampling);
        }
        let source_size = [sprite.image.width(), sprite.image.height()];
        let size = if sampling == SamplingMode::Coverage {
            [
                sprite
                    .span_x
                    .abs()
                    .floor()
                    .max(1.0)
                    .min(f64::from(source_size[0])) as u32,
                sprite
                    .span_y
                    .abs()
                    .floor()
                    .max(1.0)
                    .min(f64::from(source_size[1])) as u32,
            ]
        } else {
            source_size
        };
        let key = Key {
            source: sprite.source_hash,
            source_size,
            size,
        };
        let bytes = if size == source_size {
            0
        } else {
            size[0] as usize * size[1] as usize * 4
        };
        if bytes > self.limit {
            return raster::draw(sprite, frame, sampling);
        }
        self.clock = self.clock.wrapping_add(1);
        if !self.entries.contains_key(&key) {
            // Evict before allocating the replacement, bounding retained plus
            // in-flight filtered pixels by this renderer's configured limit.
            while self.entries.len() >= MAX_ENTRIES || self.bytes > self.limit - bytes {
                let oldest = *self
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.used)
                    .expect("nonempty bounded cache")
                    .0;
                self.bytes -= self.entries.remove(&oldest).unwrap().pixels.len();
            }
            let pixels = if bytes == 0 {
                Vec::new()
            } else {
                area_resize(sprite.image, size)?
            };
            let image = if bytes == 0 {
                sprite.image
            } else {
                ImageView {
                    width: size[0],
                    height: size[1],
                    pixels: &pixels,
                }
            };
            let bounds = alpha_bounds(image);
            self.bytes += pixels.len();
            self.entries.insert(
                key,
                Entry {
                    pixels,
                    alpha_bounds: bounds,
                    used: self.clock,
                },
            );
        }
        let entry = self.entries.get_mut(&key).unwrap();
        entry.used = self.clock;
        if entry.alpha_bounds[0] == entry.alpha_bounds[2] {
            return Ok(());
        }
        let image = if entry.pixels.is_empty() {
            sprite.image
        } else {
            ImageView {
                width: size[0],
                height: size[1],
                pixels: &entry.pixels,
            }
        };
        let mut prepared = Sprite { image, ..*sprite };
        // Transparent padding contributes nothing. Keep a one-pixel guard for
        // fractional boundaries and floating-point rounding, including mirrors.
        for (axis, span, translation) in [
            (0, sprite.span_x, sprite.node.transform.translation.x),
            (1, sprite.span_y, sprite.node.transform.translation.y),
        ] {
            let a =
                translation + span * (f64::from(entry.alpha_bounds[axis]) / f64::from(size[axis]));
            let b = translation
                + span * (f64::from(entry.alpha_bounds[axis + 2]) / f64::from(size[axis]));
            prepared.bounds[axis] = (a.min(b).floor() - 1.0).clamp(
                f64::from(sprite.bounds[axis]),
                f64::from(sprite.bounds[axis + 2]),
            ) as u32;
            prepared.bounds[axis + 2] = (a.max(b).ceil() + 1.0).clamp(
                f64::from(prepared.bounds[axis]),
                f64::from(sprite.bounds[axis + 2]),
            ) as u32;
        }
        raster::draw(&prepared, frame, sampling)
    }
}

fn alpha_bounds(image: ImageView<'_>) -> [u32; 4] {
    let mut bounds = [image.width(), image.height(), 0, 0];
    for (i, rgba) in image.rgba8().chunks_exact(4).enumerate() {
        if rgba[3] != 0 {
            let x = i as u32 % image.width();
            let y = i as u32 / image.width();
            bounds = [
                bounds[0].min(x),
                bounds[1].min(y),
                bounds[2].max(x + 1),
                bounds[3].max(y + 1),
            ];
        }
    }
    if bounds[2] == 0 {
        [0; 4]
    } else {
        bounds
    }
}

/// Exact area integration in encoded channel space and premultiplied alpha.
/// Independent of destination position/opacity: siblings and later loops reuse
/// the same filtered pixels. Floor-sized output keeps subsequent sampling in
/// the analytic magnification path, without quantizing movement coordinates.
fn area_resize(source: ImageView<'_>, size: [u32; 2]) -> Result<Vec<u8>, RenderError> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(size[0] as usize * size[1] as usize * 4)
        .map_err(|_| RenderError::AllocationFailed)?;
    let sx = f64::from(source.width()) / f64::from(size[0]);
    let sy = f64::from(source.height()) / f64::from(size[1]);
    for y in 0..size[1] {
        let top = f64::from(y) * sy;
        let bottom = f64::from(y + 1) * sy;
        for x in 0..size[0] {
            let left = f64::from(x) * sx;
            let right = f64::from(x + 1) * sx;
            let mut alpha = 0.0;
            let mut rgb = [0.0; 3];
            for iy in top.floor() as u32..(bottom.ceil() as u32).min(source.height()) {
                let wy = (bottom.min(f64::from(iy + 1)) - top.max(f64::from(iy))).max(0.0);
                for ix in left.floor() as u32..(right.ceil() as u32).min(source.width()) {
                    let weight =
                        wy * (right.min(f64::from(ix + 1)) - left.max(f64::from(ix))).max(0.0);
                    let index = (iy as usize * source.width() as usize + ix as usize) * 4;
                    let rgba = &source.rgba8()[index..index + 4];
                    let a = f64::from(rgba[3]) * weight;
                    alpha += a;
                    for c in 0..3 {
                        rgb[c] += f64::from(rgba[c]) * a;
                    }
                }
            }
            let a = (alpha / (sx * sy)).round().clamp(0.0, 255.0) as u8;
            if a == 0 {
                output.extend_from_slice(&[0; 4]);
            } else {
                output.extend_from_slice(&[
                    (rgb[0] / alpha).round() as u8,
                    (rgb[1] / alpha).round() as u8,
                    (rgb[2] / alpha).round() as u8,
                    a,
                ]);
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderOptions;
    use mochi_core::{SceneContent, SceneNode, Transform, Vec2};
    use mochi_package::LoadedPackage;
    use std::path::Path;

    #[test]
    fn prefilter_integrates_alpha_without_bleeding_transparent_rgb() {
        let pixels = [
            255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255, 255, 0,
        ];
        let image = ImageView {
            width: 4,
            height: 1,
            pixels: &pixels,
        };
        assert_eq!(
            area_resize(image, [2, 1]).unwrap(),
            [255, 0, 0, 128, 0, 0, 255, 128]
        );
        assert_eq!(area_resize(image, [1, 1]).unwrap(), [128, 0, 128, 128]);
        assert_eq!(alpha_bounds(image), [0, 0, 3, 1]);
    }

    #[test]
    fn cache_eviction_is_bounded_and_regeneration_keeps_pixels_identical() {
        let pkg = LoadedPackage::load(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/neko/neko.mochi"),
        )
        .unwrap();
        let pixels = [180, 120, 60, 173].repeat(64 * 64);
        let node = SceneNode {
            id: "test".into(),
            transform: Transform::default(),
            opacity: 0.55,
            content: SceneContent::Sprite {
                asset: "sprites/neko.png".into(),
                size: Vec2::new(64.0, 64.0),
                animation: "idle".into(),
            },
        };
        let mut sprite = Sprite {
            node: &node,
            image: ImageView {
                width: 64,
                height: 64,
                pixels: &pixels,
            },
            source_hash: pkg.image("sprites/neko.png").unwrap().source_hash(),
            sin: 0.0,
            cos: 1.0,
            span_x: 1.0,
            span_y: 1.0,
            bounds: [0, 0, 8, 8],
        };
        let options = RenderOptions {
            width: 8,
            height: 8,
            ..Default::default()
        };
        let mut cache = ScaledCache::new(32);
        let mut initial = Frame::clear(options).unwrap();
        cache
            .draw(&sprite, &mut initial, SamplingMode::Coverage)
            .unwrap();
        for width in 2..=8 {
            sprite.span_x = f64::from(width);
            cache
                .draw(
                    &sprite,
                    &mut Frame::clear(options).unwrap(),
                    SamplingMode::Coverage,
                )
                .unwrap();
            assert!(cache.bytes <= 32);
        }
        sprite.span_x = 1.0;
        let mut repeated = Frame::clear(options).unwrap();
        cache
            .draw(&sprite, &mut repeated, SamplingMode::Coverage)
            .unwrap();
        assert_eq!(initial.rgba8(), repeated.rgba8());
        // Metadata-only entries have their own cap, even when byte usage is zero.
        for w in 1..=32 {
            for h in 1..=32 {
                sprite.image.width = w;
                sprite.image.height = h;
                sprite.image.pixels = &pixels[..(w * h * 4) as usize];
                cache
                    .draw(
                        &sprite,
                        &mut Frame::clear(options).unwrap(),
                        SamplingMode::Nearest,
                    )
                    .unwrap();
                assert!(cache.entries.len() <= MAX_ENTRIES && cache.bytes <= 32);
            }
        }
    }
}
