use x11rb::protocol::xproto::Rectangle;

pub const HIT_ALPHA: u8 = 8;

/// X11 ARGB32 is premultiplied, unlike the renderer's straight RGBA8.
pub fn argb(rgba: &[u8], little_endian: bool) -> Vec<u8> {
    let mut out = vec![0; rgba.len()];
    for (pixel, target) in rgba.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let a = u32::from(pixel[3]);
        if a == 0 {
            continue;
        }
        let c = |i: usize| (u32::from(pixel[i]) * a + 127) / 255;
        let word = (a << 24) | (c(0) << 16) | (c(1) << 8) | c(2);
        target.copy_from_slice(&if little_endian {
            word.to_le_bytes()
        } else {
            word.to_be_bytes()
        });
    }
    out
}

/// Per-row solid spans leave transparent gaps available to underlying apps.
#[cfg(test)]
pub fn input_shape(rgba: &[u8], width: u16) -> Vec<Rectangle> {
    shape_and_bounds(rgba, width).0
}

/// Bounds include every nonzero alpha, including pixels below hit threshold.
/// They define upload damage; omitting faint pixels would leave stale edges.
pub fn shape_and_bounds(rgba: &[u8], width: u16) -> (Vec<Rectangle>, Option<Rectangle>) {
    scan(rgba, width, true)
}

/// Stable input regions still need exact visual damage for every rendered frame.
pub fn alpha_bounds(rgba: &[u8], width: u16) -> Option<Rectangle> {
    scan(rgba, width, false).1
}

fn scan(rgba: &[u8], width: u16, input: bool) -> (Vec<Rectangle>, Option<Rectangle>) {
    let mut spans: Vec<Rectangle> = Vec::new();
    let mut previous: Vec<(u16, u16, usize)> = Vec::new();
    let mut current = Vec::new();
    let mut bounds = None;
    for (y, row) in rgba.chunks_exact(usize::from(width) * 4).enumerate() {
        current.clear();
        let mut prior = 0;
        let Some(left) = row.chunks_exact(4).position(|p| p[3] != 0) else {
            previous.clear();
            continue;
        };
        let right = row
            .chunks_exact(4)
            .rposition(|p| p[3] != 0)
            .expect("nonempty row")
            + 1;
        bounds = damage(
            bounds,
            Some(Rectangle {
                x: left as i16,
                y: y as i16,
                width: (right - left) as u16,
                height: 1,
            }),
        );
        if !input {
            continue;
        }
        let mut x = left as u16;
        while x < right as u16 {
            if row[usize::from(x) * 4 + 3] <= HIT_ALPHA {
                x += 1;
                continue;
            }
            let start = x;
            while x < right as u16 && row[usize::from(x) * 4 + 3] > HIT_ALPHA {
                x += 1;
            }
            // Extend identical spans on consecutive rows. Coverage stays exact,
            // including internal holes and the alpha hit threshold.
            let width = x - start;
            while prior < previous.len() && previous[prior].0 < start {
                prior += 1;
            }
            let index = if let Some(&(px, pw, index)) = previous
                .get(prior)
                .filter(|&&(px, pw, _)| px == start && pw == width)
            {
                debug_assert_eq!((px, pw), (start, width));
                spans[index].height += 1;
                index
            } else {
                spans.push(Rectangle {
                    x: start as i16,
                    y: y as i16,
                    width,
                    height: 1,
                });
                spans.len() - 1
            };
            current.push((start, width, index));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (spans, bounds)
}

pub fn damage(old: Option<Rectangle>, new: Option<Rectangle>) -> Option<Rectangle> {
    match (old, new) {
        (Some(a), Some(b)) => {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            Some(Rectangle {
                x,
                y,
                width: (i32::from(a.x) + i32::from(a.width))
                    .max(i32::from(b.x) + i32::from(b.width))
                    .saturating_sub(i32::from(x)) as u16,
                height: (i32::from(a.y) + i32::from(a.height))
                    .max(i32::from(b.y) + i32::from(b.height))
                    .saturating_sub(i32::from(y)) as u16,
            })
        }
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_shapes_preserve_every_pixel_hole_and_sort_order() {
        // Exhaust every 4x3 binary mask, including span split/merge and empty rows.
        for mask in 0u16..(1 << 12) {
            let mut rgba = vec![0; 12 * 4];
            for i in 0..12 {
                rgba[i * 4 + 3] = if mask & (1 << i) != 0 {
                    HIT_ALPHA + 1
                } else {
                    HIT_ALPHA
                };
            }
            let shape = input_shape(&rgba, 4);
            assert_eq!(alpha_bounds(&rgba, 4), shape_and_bounds(&rgba, 4).1);
            assert!(shape
                .windows(2)
                .all(|r| (r[0].y, r[0].x) < (r[1].y, r[1].x)));
            let mut coverage = [0u8; 12];
            for r in shape {
                assert!(r.width > 0 && r.height > 0);
                for y in r.y..r.y + r.height as i16 {
                    for x in r.x..r.x + r.width as i16 {
                        coverage[y as usize * 4 + x as usize] += 1;
                    }
                }
            }
            for (i, covered) in coverage.into_iter().enumerate() {
                assert_eq!(
                    covered,
                    u8::from(mask & (1 << i) != 0),
                    "mask={mask}, pixel={i}"
                );
            }
        }
    }
    #[test]
    fn solid_shape_uses_one_rectangle_and_does_not_bridge_empty_rows() {
        let mut rgba = vec![255; 512 * 512 * 4];
        assert_eq!(
            input_shape(&rgba, 512),
            vec![Rectangle {
                x: 0,
                y: 0,
                width: 512,
                height: 512
            }]
        );
        rgba[256 * 512 * 4..257 * 512 * 4].fill(0);
        assert_eq!(
            input_shape(&rgba, 512),
            vec![
                Rectangle {
                    x: 0,
                    y: 0,
                    width: 512,
                    height: 256
                },
                Rectangle {
                    x: 0,
                    y: 257,
                    width: 512,
                    height: 255
                },
            ]
        );
    }
    #[test]
    #[ignore = "requires local X11; uses one unmapped owned window"]
    fn compact_input_shape_matches_server_region() {
        use x11rb::{
            connection::Connection,
            protocol::{
                shape::{ConnectionExt as _, SK, SO},
                xproto::{ClipOrdering, ConnectionExt as _, CreateWindowAux, WindowClass},
            },
        };
        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id().unwrap();
        let size = 64u16;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            size,
            size,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .unwrap()
        .check()
        .unwrap();
        let mut rgba = vec![0; usize::from(size).pow(2) * 4];
        for y in 0..usize::from(size) {
            for x in 0..usize::from(size) {
                let hit = (x / 7 + y / 11) % 3 != 0
                    && (x as i32 - 32).pow(2) + (y as i32 - 32).pow(2) > 64;
                rgba[(y * usize::from(size) + x) * 4 + 3] = if hit { 9 } else { 8 };
            }
        }
        let shape = input_shape(&rgba, size);
        conn.shape_rectangles(
            SO::SET,
            SK::INPUT,
            ClipOrdering::YX_SORTED,
            window,
            0,
            0,
            &shape,
        )
        .unwrap()
        .check()
        .unwrap();
        let actual = conn
            .shape_get_rectangles(window, SK::INPUT)
            .unwrap()
            .reply()
            .unwrap();
        let mut coverage = vec![0u8; usize::from(size).pow(2)];
        for r in actual.rectangles {
            for y in r.y..r.y + r.height as i16 {
                for x in r.x..r.x + r.width as i16 {
                    coverage[y as usize * usize::from(size) + x as usize] += 1;
                }
            }
        }
        for (i, n) in coverage.into_iter().enumerate() {
            assert_eq!(n, u8::from(rgba[i * 4 + 3] > HIT_ALPHA), "pixel {i}");
        }
        conn.destroy_window(window).unwrap().check().unwrap();
    }
    #[test]
    fn damage_clears_previous_body_and_keeps_faint_pixels() {
        let mut rgba = vec![0; 4 * 4 * 4];
        rgba[27] = 1; // pixel (2, 1)
        let (shape, bounds) = shape_and_bounds(&rgba, 4);
        assert!(shape.is_empty());
        assert_eq!(
            bounds,
            Some(Rectangle {
                x: 2,
                y: 1,
                width: 1,
                height: 1
            })
        );
        let old = Some(Rectangle {
            x: 0,
            y: 0,
            width: 2,
            height: 3,
        });
        assert_eq!(
            damage(old, bounds),
            Some(Rectangle {
                x: 0,
                y: 0,
                width: 3,
                height: 3
            })
        );
        assert_eq!(damage(old, None), old);
        assert_eq!(damage(None, None), None);
    }
    #[test]
    fn premultiplication_handles_transparent_rgb_and_server_byte_order() {
        let rgba = [255, 128, 0, 128, 255, 255, 255, 0, 12, 34, 56, 255];
        assert_eq!(
            argb(&rgba, true),
            [0, 64, 128, 128, 0, 0, 0, 0, 56, 34, 12, 255]
        );
        assert_eq!(
            argb(&rgba, false),
            [128, 128, 64, 0, 0, 0, 0, 0, 255, 12, 34, 56]
        );
    }
    #[test]
    fn shape_preserves_holes_and_ignores_near_transparent_pixels() {
        let mut rgba = vec![255; 4 * 4 * 2];
        for (i, alpha) in [0, 255, 255, 0, 255, 8, 0, 9].into_iter().enumerate() {
            rgba[i * 4 + 3] = alpha;
        }
        assert_eq!(
            input_shape(&rgba, 4),
            vec![
                Rectangle {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1
                },
                Rectangle {
                    x: 0,
                    y: 1,
                    width: 1,
                    height: 1
                },
                Rectangle {
                    x: 3,
                    y: 1,
                    width: 1,
                    height: 1
                },
            ]
        );
    }
}
