//! Spatial coverage only: no interpolation of simulation time or input state.
use crate::{
    raster::{over, Sprite},
    Frame, RenderError,
};

pub(crate) fn axis_aligned(sprite: &Sprite<'_>) -> bool {
    sprite.sin == 0.0
        && sprite.cos == 1.0
        && (sprite.image.width() == 1 || sprite.span_x.abs() >= f64::from(sprite.image.width()))
        && (sprite.image.height() == 1 || sprite.span_y.abs() >= f64::from(sprite.image.height()))
}

#[derive(Clone, Copy, Default)]
struct Tap {
    index: u32,
    weight: f64,
}

// A destination pixel overlaps at most two texels on this axis when magnified.
// Source coordinates are clipped before weighting, so sprite edges retain
// fractional coverage instead of stretching the outer texel into the background.
fn taps(pixel: u32, translation: f64, span: f64, extent: u32) -> Option<[Tap; 2]> {
    let edge = translation + span;
    let covered = ((f64::from(pixel) + 1.0).min(translation.max(edge))
        - f64::from(pixel).max(translation.min(edge)))
    .clamp(0.0, 1.0);
    if extent == 1 {
        return Some([
            Tap {
                index: 0,
                weight: covered,
            },
            Tap::default(),
        ]);
    }
    let a = (f64::from(pixel) - translation) / span;
    let b = (f64::from(pixel) + 1.0 - translation) / span;
    if !a.is_finite() || !b.is_finite() {
        return None;
    }
    let a = a.clamp(0.0, 1.0) * f64::from(extent);
    let b = b.clamp(0.0, 1.0) * f64::from(extent);
    let (lo, hi) = (a.min(b), a.max(b));
    let first = (lo.floor() as u32).min(extent - 1);
    // Huge valid scales can collapse the two inverse coordinates to one f64.
    // Preserve geometric coverage while selecting that representable texel.
    if lo == hi {
        return Some([
            Tap {
                index: first,
                weight: covered,
            },
            Tap::default(),
        ]);
    }
    let mut result = [Tap::default(); 2];
    for (offset, tap) in result.iter_mut().enumerate() {
        let index = first + offset as u32;
        if index < extent {
            let length = (hi.min(f64::from(index + 1)) - lo.max(f64::from(index))).max(0.0);
            *tap = Tap {
                index,
                weight: (length * (span.abs() / f64::from(extent))).clamp(0.0, 1.0),
            };
        }
    }
    let total = result[0].weight + result[1].weight;
    if total > 0.0 {
        for tap in &mut result {
            tap.weight *= covered / total;
        }
    }
    Some(result)
}

#[derive(Default)]
struct Mix {
    alpha: f64,
    premultiplied: [f64; 3],
}
impl Mix {
    fn add(&mut self, rgba: &[u8], weight: f64) {
        if rgba[3] == 0 {
            return;
        }
        let alpha = f64::from(rgba[3]) * weight;
        self.alpha += alpha;
        for (channel, value) in self.premultiplied.iter_mut().zip(rgba) {
            *channel += f64::from(*value) * alpha;
        }
    }
    fn blend(self, destination: &mut [u8], opacity: f64) {
        if self.alpha <= 0.0 {
            return;
        }
        let mut rgba = [0; 4];
        for (channel, value) in rgba[..3].iter_mut().zip(self.premultiplied) {
            *channel = (value / self.alpha).round().clamp(0.0, 255.0) as u8;
        }
        rgba[3] = self.alpha.round().clamp(0.0, 255.0) as u8;
        over(&rgba, destination, opacity);
    }
}

pub(crate) fn draw(sprite: &Sprite<'_>, frame: &mut Frame) -> Result<(), RenderError> {
    if axis_aligned(sprite) {
        draw_axis(sprite, frame)
    } else {
        draw_grid(sprite, frame)
    }
}

fn draw_axis(sprite: &Sprite<'_>, frame: &mut Frame) -> Result<(), RenderError> {
    let [x0, y0, x1, y1] = sprite.bounds;
    let mut columns = Vec::new();
    let invalid = || RenderError::InvalidTransform {
        node: sprite.node.id.clone(),
    };
    columns
        .try_reserve_exact((x1 - x0) as usize)
        .map_err(|_| RenderError::AllocationFailed)?;
    for x in x0..x1 {
        columns.push(
            taps(
                x,
                sprite.node.transform.translation.x,
                sprite.span_x,
                sprite.image.width(),
            )
            .ok_or_else(invalid)?,
        );
    }
    for y in y0..y1 {
        let rows = taps(
            y,
            sprite.node.transform.translation.y,
            sprite.span_y,
            sprite.image.height(),
        )
        .ok_or_else(invalid)?;
        for (offset, columns) in columns.iter().enumerate() {
            let x = x0 as usize + offset;
            let start = (y as usize * frame.width as usize + x) * 4;
            let destination = &mut frame.rgba8[start..start + 4];
            if columns[0].weight == 1.0 && rows[0].weight == 1.0 {
                let source = (rows[0].index as usize * sprite.image.width() as usize
                    + columns[0].index as usize)
                    * 4;
                over(
                    &sprite.image.rgba8()[source..source + 4],
                    destination,
                    sprite.node.opacity,
                );
                continue;
            }
            let mut mix = Mix::default();
            for row in rows.iter().filter(|tap| tap.weight > 0.0) {
                for column in columns.iter().filter(|tap| tap.weight > 0.0) {
                    let source = (row.index as usize * sprite.image.width() as usize
                        + column.index as usize)
                        * 4;
                    mix.add(
                        &sprite.image.rgba8()[source..source + 4],
                        row.weight * column.weight,
                    );
                }
            }
            mix.blend(destination, sprite.node.opacity);
        }
    }
    Ok(())
}

fn draw_grid(sprite: &Sprite<'_>, frame: &mut Frame) -> Result<(), RenderError> {
    let [x0, y0, x1, y1] = sprite.bounds;
    for y in y0..y1 {
        for x in x0..x1 {
            let mut mix = Mix::default();
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = f64::from(x) + (f64::from(sx) + 0.5) / 4.0
                        - sprite.node.transform.translation.x;
                    let dy = f64::from(y) + (f64::from(sy) + 0.5) / 4.0
                        - sprite.node.transform.translation.y;
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
                    let ix = ((u * f64::from(sprite.image.width())) as u32)
                        .min(sprite.image.width() - 1);
                    let iy = ((v * f64::from(sprite.image.height())) as u32)
                        .min(sprite.image.height() - 1);
                    let source = (iy as usize * sprite.image.width() as usize + ix as usize) * 4;
                    mix.add(&sprite.image.rgba8()[source..source + 4], 1.0 / 16.0);
                }
            }
            let start = (y as usize * frame.width as usize + x as usize) * 4;
            mix.blend(&mut frame.rgba8[start..start + 4], sprite.node.opacity);
        }
    }
    Ok(())
}
