//! An optional static frame of the user's arrow, shared by all pet windows.
//! No per-motion work or global cursor/theme changes are needed.
use std::{error::Error, fs};
use x11rb::{
    connection::Connection,
    protocol::{
        render::{ConnectionExt as _, Pictformat, PictureWrapper},
        xproto::*,
    },
    rust_connection::RustConnection,
};
use xcursor::{parser::Image, CursorTheme};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn resource<'a>(resources: &'a str, key: &str) -> Option<&'a str> {
    resources.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == key).then_some(value.trim())
    })
}

fn first_frame(images: &[Image], size: u32) -> Result<&Image> {
    // min_by_key retains the first frame among equally close nominal sizes.
    let image = images
        .iter()
        .min_by_key(|image| image.size.abs_diff(size))
        .ok_or("cursor contains no images")?;
    // Bound the one-time X11 upload and reject invalid hotspots. A theme that
    // cannot be loaded leaves the ordinary inherited system cursor available.
    if image.width == 0
        || image.height == 0
        || image.width > 256
        || image.height > 256
        || image.xhot >= image.width
        || image.yhot >= image.height
        || image.pixels_rgba.len() != (image.width * image.height * 4) as usize
    {
        return Err("unsupported cursor image dimensions or hotspot".into());
    }
    Ok(image)
}

fn server_pixels(image: &Image, order: ImageOrder) -> Vec<u8> {
    // Xcursor stores little-endian ARGB words (B,G,R,A bytes), despite the
    // parser's historical pixels_rgba field name. Preserve premultiplication.
    let mut pixels = image.pixels_rgba.clone();
    if order == ImageOrder::MSB_FIRST {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.reverse();
        }
    }
    pixels
}

pub fn load(
    conn: &RustConnection,
    root: Window,
    format: Pictformat,
    resources: &str,
    scale: f64,
) -> Result<Cursor> {
    let theme = std::env::var("XCURSOR_THEME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| resource(resources, "Xcursor.theme").map(str::to_owned))
        .unwrap_or_else(|| "default".into());
    let size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|s| *s > 0)
        .or_else(|| resource(resources, "Xcursor.size")?.parse().ok())
        .filter(|s| *s > 0)
        .unwrap_or((24.0 * scale).round() as u32);
    let theme_files = CursorTheme::load(&theme);
    let path = theme_files
        .load_icon("left_ptr")
        .or_else(|| theme_files.load_icon("default"))
        .ok_or("current theme has no raster arrow")?;
    if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
        return Err("cursor theme image exceeds 16 MiB".into());
    }
    let images = xcursor::parser::parse_xcursor(&fs::read(path)?)
        .ok_or("cannot parse cursor theme image")?;
    let image = first_frame(&images, size)?;
    let pixels = server_pixels(image, conn.setup().image_byte_order);
    let pixmap =
        PixmapWrapper::create_pixmap(conn, 32, root, image.width as u16, image.height as u16)?;
    let gc = GcontextWrapper::create_gc(conn, pixmap.pixmap(), &CreateGCAux::new())?;
    conn.put_image(
        ImageFormat::Z_PIXMAP,
        pixmap.pixmap(),
        gc.gcontext(),
        image.width as u16,
        image.height as u16,
        0,
        0,
        0,
        32,
        &pixels,
    )?
    .check()?;
    let picture =
        PictureWrapper::create_picture(conn, pixmap.pixmap(), format, &Default::default())?;
    let cursor = conn.generate_id()?;
    conn.render_create_cursor(
        cursor,
        picture.picture(),
        image.xhot as u16,
        image.yhot as u16,
    )?
    .check()?;
    println!(
        "Desktop cursor: static theme={theme:?} size={}x{} hotspot={},{} id={cursor}",
        image.width, image.height, image.xhot, image.yhot
    );
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(size: u32, red: u8) -> Image {
        Image {
            size,
            width: 1,
            height: 1,
            xhot: 0,
            yhot: 0,
            delay: 117,
            pixels_rgba: vec![11, 22, red, 128],
            pixels_argb: vec![],
        }
    }

    #[test]
    fn theme_size_keeps_first_animation_frame() {
        let images = [image(32, 1), image(64, 2), image(64, 3), image(96, 4)];
        assert_eq!(first_frame(&images, 64).unwrap().pixels_rgba[2], 2);
        assert_eq!(first_frame(&images, 60).unwrap().pixels_rgba[2], 2);
        assert_eq!(first_frame(&images, 32).unwrap().pixels_rgba[2], 1);
    }

    #[test]
    fn cursor_channels_match_server_byte_order() {
        assert_eq!(
            server_pixels(&image(64, 33), ImageOrder::LSB_FIRST),
            [11, 22, 33, 128]
        );
        assert_eq!(
            server_pixels(&image(64, 33), ImageOrder::MSB_FIRST),
            [128, 33, 22, 11]
        );
    }

    #[test]
    fn invalid_hotspot_and_truncated_pixels_are_rejected() {
        let mut bad = image(64, 1);
        bad.xhot = bad.width;
        assert!(first_frame(&[bad], 64).is_err());
        let mut bad = image(64, 1);
        bad.pixels_rgba.pop();
        assert!(first_frame(&[bad], 64).is_err());
        assert!(first_frame(&[], 64).is_err());
    }
}
