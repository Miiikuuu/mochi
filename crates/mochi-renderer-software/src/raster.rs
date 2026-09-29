use crate::{coverage, Frame, RenderError, RenderOptions, SamplingMode};
use mochi_core::{Scene, SceneContent, SceneNode};
use mochi_package::{ContentHash, DecodedImage, LoadedPackage};

#[derive(Clone, Copy)]
pub(crate) struct ImageView<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: &'a [u8],
}
impl<'a> ImageView<'a> {
    pub(crate) fn width(self) -> u32 {
        self.width
    }
    pub(crate) fn height(self) -> u32 {
        self.height
    }
    pub(crate) fn rgba8(self) -> &'a [u8] {
        self.pixels
    }
}
impl<'a> From<&'a DecodedImage> for ImageView<'a> {
    fn from(image: &'a DecodedImage) -> Self {
        Self {
            width: image.width(),
            height: image.height(),
            pixels: image.rgba8(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Sprite<'a> {
    pub(crate) node: &'a SceneNode,
    pub(crate) image: ImageView<'a>,
    pub(crate) source_hash: ContentHash,
    pub(crate) sin: f64,
    pub(crate) cos: f64,
    pub(crate) span_x: f64,
    pub(crate) span_y: f64,
    pub(crate) bounds: [u32; 4], // x0, y0, x1 exclusive, y1 exclusive
}

#[derive(Default)]
pub(crate) struct Budget {
    pub(crate) nodes: usize,
    pub(crate) samples: u64,
}

pub(crate) enum Draw<'a> {
    Sprite(Sprite<'a>),
    #[cfg(feature = "spine")]
    Skeletal {
        package: &'a LoadedPackage,
        node: &'a SceneNode,
    },
}

pub(crate) fn prepare<'a>(
    package: &'a LoadedPackage,
    scene: &'a Scene,
    options: RenderOptions,
    budget: &mut Budget,
) -> Result<Vec<Draw<'a>>, RenderError> {
    budget.nodes = budget
        .nodes
        .checked_add(scene.nodes.len())
        .ok_or(RenderError::LimitExceeded("scene nodes"))?;
    if budget.nodes > options.limits.max_nodes {
        return Err(RenderError::LimitExceeded("scene nodes"));
    }
    scene
        .validate()
        .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
    let mut sprites = Vec::new();
    for node in &scene.nodes {
        #[cfg(feature = "spine")]
        if let SceneContent::Skeletal {
            asset,
            animation,
            offsets,
            overlay,
            ..
        } = &node.content
        {
            let resource = package
                .spine(asset)
                .map_err(|e| RenderError::InvalidScene(e.to_string()))?;
            if offsets
                .iter()
                .any(|o| !resource.model.metadata.bone_names.contains(&o.bone))
            {
                return Err(RenderError::InvalidScene(
                    "unknown skeletal bone offset".into(),
                ));
            }
            if overlay.gaze.as_ref().is_some_and(|g| {
                g.rotations
                    .iter()
                    .any(|r| !resource.model.metadata.bone_names.contains(&r.bone))
                    || g.eyes.iter().any(|e| {
                        !resource.model.metadata.bone_names.contains(&e.bone)
                            || !resource.model.metadata.bone_names.contains(&e.closure.bone)
                            || e.pupil.as_ref().is_some_and(|p| {
                                !resource.model.metadata.slot_names.contains(&p.slot)
                            })
                    })
            }) || overlay.layers.iter().any(|l| {
                !resource
                    .model
                    .metadata
                    .animations
                    .iter()
                    .any(|a| a.name == l.animation)
                    || l.foot_plant.as_ref().is_some_and(|p| {
                        !resource
                            .model
                            .metadata
                            .animations
                            .iter()
                            .any(|a| a.name == p.anchor_animation)
                            || [&p.hip, &p.feet[0], &p.feet[1]]
                                .iter()
                                .any(|n| !resource.model.metadata.bone_names.contains(n))
                    })
                    || l.rotations
                        .iter()
                        .any(|r| !resource.model.metadata.bone_names.contains(&r.bone))
                    || l.slot_alpha
                        .iter()
                        .any(|s| !resource.model.metadata.slot_names.contains(&s.slot))
                    || l.bones
                        .iter()
                        .any(|b| !resource.model.metadata.bone_names.contains(&b.bone))
                    || l.slots
                        .iter()
                        .any(|s| !resource.model.metadata.slot_names.contains(s))
            }) || overlay.spirals.iter().any(|e| {
                [&e.slot, &e.highlight, &e.clip]
                    .into_iter()
                    .any(|s| !resource.model.metadata.slot_names.contains(s))
                    || !resource
                        .model
                        .pages
                        .iter()
                        .any(|p| p.asset == e.texture && p.width == 1 && p.height == 1)
            }) || overlay
                .bones
                .iter()
                .any(|b| !resource.model.metadata.bone_names.contains(&b.bone))
                || overlay
                    .slots
                    .iter()
                    .any(|s| !resource.model.metadata.slot_names.contains(&s.slot))
            {
                return Err(RenderError::InvalidScene(
                    "unknown skeletal pose overlay channel".into(),
                ));
            }
            if !resource
                .model
                .metadata
                .animations
                .iter()
                .any(|a| &a.name == animation)
            {
                return Err(RenderError::InvalidScene(
                    "unknown skeletal animation".into(),
                ));
            }
            sprites.push(Draw::Skeletal { package, node });
            continue;
        }
        let SceneContent::Sprite { asset, size, .. } = &node.content else {
            return Err(RenderError::UnsupportedContent {
                node: node.id.clone(),
            });
        };
        let image = package
            .image(asset)
            .map_err(|_| RenderError::MissingImage {
                node: node.id.clone(),
                asset: asset.clone(),
            })?;
        let transform = &node.transform;
        if transform.scale.x == 0.0 || transform.scale.y == 0.0 {
            continue; // degenerate shape, but resource and content were still validated
        }
        let invalid = || RenderError::InvalidTransform {
            node: node.id.clone(),
        };
        let span_x = size.x * transform.scale.x;
        let span_y = size.y * transform.scale.y;
        if !span_x.is_finite() || !span_y.is_finite() || span_x == 0.0 || span_y == 0.0 {
            return Err(invalid());
        }
        let (sin, cos) = transform.rotation_radians.sin_cos();
        let tx = transform.translation.x;
        let ty = transform.translation.y;
        let corners = [
            (tx, ty),
            (tx + cos * span_x, ty + sin * span_x),
            (tx - sin * span_y, ty + cos * span_y),
            (
                tx + cos * span_x - sin * span_y,
                ty + sin * span_x + cos * span_y,
            ),
        ];
        if corners
            .iter()
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(invalid());
        }
        let x0 = corners
            .iter()
            .map(|p| p.0)
            .fold(f64::INFINITY, f64::min)
            .floor()
            .clamp(0.0, f64::from(options.width)) as u32;
        let y0 = corners
            .iter()
            .map(|p| p.1)
            .fold(f64::INFINITY, f64::min)
            .floor()
            .clamp(0.0, f64::from(options.height)) as u32;
        let x1 = corners
            .iter()
            .map(|p| p.0)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .clamp(0.0, f64::from(options.width)) as u32;
        let y1 = corners
            .iter()
            .map(|p| p.1)
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil()
            .clamp(0.0, f64::from(options.height)) as u32;
        if node.opacity == 0.0 || x0 == x1 || y0 == y1 {
            continue;
        }
        let sprite = Sprite {
            node,
            image: image.into(),
            source_hash: image.source_hash(),
            sin,
            cos,
            span_x,
            span_y,
            bounds: [x0, y0, x1, y1],
        };
        let samples_per_pixel = match options.sampling {
            SamplingMode::Nearest => 1,
            SamplingMode::Coverage if coverage::axis_aligned(&sprite) => 4,
            SamplingMode::Coverage => 16,
        };
        let samples = (u64::from(x1 - x0) * u64::from(y1 - y0))
            .checked_mul(samples_per_pixel)
            .ok_or(RenderError::LimitExceeded("sample count overflow"))?;
        budget.samples = budget
            .samples
            .checked_add(samples)
            .ok_or(RenderError::LimitExceeded("sample count overflow"))?;
        if budget.samples > options.limits.max_sampled_pixels {
            return Err(RenderError::LimitExceeded("sampled pixels"));
        }
        sprites.push(Draw::Sprite(sprite));
    }
    Ok(sprites)
}

pub(crate) fn draw(
    sprite: &Sprite<'_>,
    frame: &mut Frame,
    sampling: SamplingMode,
) -> Result<(), RenderError> {
    if sampling == SamplingMode::Coverage {
        return coverage::draw(sprite, frame);
    }
    let [x0, y0, x1, y1] = sprite.bounds;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = f64::from(x) + 0.5 - sprite.node.transform.translation.x;
            let dy = f64::from(y) + 0.5 - sprite.node.transform.translation.y;
            // Inverse R then scale. Normalized spans avoid overflow from dividing
            // by a tiny scale before dividing by a large logical sprite size.
            let u = (sprite.cos * dx + sprite.sin * dy) / sprite.span_x;
            let v = (-sprite.sin * dx + sprite.cos * dy) / sprite.span_y;
            if !u.is_finite() || !v.is_finite() {
                return Err(RenderError::InvalidTransform {
                    node: sprite.node.id.clone(),
                });
            }
            if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                continue;
            }
            // Clamp only the final rounded multiplication at the upper boundary.
            let sx = ((u * f64::from(sprite.image.width())) as u32).min(sprite.image.width() - 1);
            let sy = ((v * f64::from(sprite.image.height())) as u32).min(sprite.image.height() - 1);
            let source = (sy as usize * sprite.image.width() as usize + sx as usize) * 4;
            let destination = (y as usize * frame.width as usize + x as usize) * 4;
            over(
                &sprite.image.rgba8()[source..source + 4],
                &mut frame.rgba8[destination..destination + 4],
                sprite.node.opacity,
            );
        }
    }
    Ok(())
}

/// Source-over in encoded channel space. Round each resulting byte to nearest.
pub(crate) fn over(src: &[u8], dst: &mut [u8], opacity: f64) {
    let sa = (f64::from(src[3]) * opacity).round() as u32;
    if sa == 0 {
        return;
    }
    if sa == 255 {
        dst.copy_from_slice(src);
        return;
    }
    let da = u32::from(dst[3]);
    if da == 255 {
        // With an opaque destination, output alpha stays 255. Cancel the
        // common factor 255 before dividing, preserving byte-exact rounding.
        for channel in 0..3 {
            dst[channel] =
                ((u32::from(src[channel]) * sa + u32::from(dst[channel]) * (255 - sa) + 127) / 255)
                    as u8;
        }
        return;
    }
    let alpha = sa * 255 + da * (255 - sa);
    for channel in 0..3 {
        let numerator =
            u32::from(src[channel]) * sa * 255 + u32::from(dst[channel]) * da * (255 - sa);
        dst[channel] = ((numerator + alpha / 2) / alpha) as u8;
    }
    dst[3] = ((alpha + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::over;

    #[test]
    fn blending_fast_paths_match_general_source_over_for_all_alpha_pairs() {
        for source_alpha in 0..=255_u32 {
            for destination_alpha in 0..=255_u32 {
                for opacity in [0.0, 0.001, 0.49, 0.5, 0.8, 0.999, 1.0] {
                    let src = [
                        source_alpha as u8,
                        255 - source_alpha as u8,
                        173,
                        source_alpha as u8,
                    ];
                    let original = [
                        destination_alpha as u8,
                        71,
                        255 - destination_alpha as u8,
                        destination_alpha as u8,
                    ];
                    let mut expected = original;
                    let sa = (f64::from(source_alpha) * opacity).round() as u32;
                    let da = destination_alpha;
                    if sa != 0 {
                        let alpha = sa * 255 + da * (255 - sa);
                        for c in 0..3 {
                            let numerator = u32::from(src[c]) * sa * 255
                                + u32::from(original[c]) * da * (255 - sa);
                            expected[c] = ((numerator + alpha / 2) / alpha) as u8;
                        }
                        expected[3] = ((alpha + 127) / 255) as u8;
                    }
                    let mut actual = original;
                    over(&src, &mut actual, opacity);
                    assert_eq!(actual, expected, "source alpha={source_alpha}, destination alpha={destination_alpha}, opacity={opacity}");
                }
            }
        }
    }
}
