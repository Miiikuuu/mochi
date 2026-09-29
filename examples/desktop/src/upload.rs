//! Reusable bounded X11 PutImage strips; no window or renderer state.
use std::error::Error;
use x11rb::{
    protocol::xproto::{ConnectionExt as _, Gcontext, ImageFormat, Pixmap, Rectangle},
    rust_connection::RustConnection,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
pub struct Upload {
    size: u16,
    strip: Vec<u8>,
}
impl Upload {
    pub fn new(size: u16) -> Self {
        Self {
            size,
            strip: Vec::new(),
        }
    }
    pub fn write(
        &mut self,
        conn: &RustConnection,
        backing: Pixmap,
        gc: Gcontext,
        packed: &[u8],
        damage: Rectangle,
    ) -> Result<u64> {
        if packed.len() != usize::from(self.size).pow(2) * 4
            || damage.x < 0
            || damage.y < 0
            || damage.x as u32 + u32::from(damage.width) > u32::from(self.size)
            || damage.y as u32 + u32::from(damage.height) > u32::from(self.size)
        {
            return Err("invalid X11 upload bounds".into());
        }
        if damage.width == 0 || damage.height == 0 {
            return Ok(0);
        }
        for offset in (0..damage.height).step_by(16) {
            let rows = 16.min(damage.height - offset);
            self.strip.clear();
            for row in 0..rows {
                let start = ((damage.y as usize + usize::from(offset + row))
                    * usize::from(self.size)
                    + damage.x as usize)
                    * 4;
                self.strip
                    .extend_from_slice(&packed[start..start + usize::from(damage.width) * 4]);
            }
            conn.put_image(
                ImageFormat::Z_PIXMAP,
                backing,
                gc,
                damage.width,
                rows,
                damage.x,
                damage.y + offset as i16,
                0,
                32,
                &self.strip,
            )?;
        }
        Ok(u64::from(damage.width) * u64::from(damage.height) * 4)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::connection::Connection;
    #[test]
    #[ignore = "requires local X11; uses offscreen pixmaps only"]
    fn wire_uploads_match_server_pixels_and_clear_damage() {
        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let pixmap = conn.generate_id().unwrap();
        let gc = conn.generate_id().unwrap();
        let presented = conn.generate_id().unwrap();
        conn.create_pixmap(32, pixmap, root, 257, 257)
            .unwrap()
            .check()
            .unwrap();
        conn.create_pixmap(32, presented, root, 257, 257)
            .unwrap()
            .check()
            .unwrap();
        conn.create_gc(gc, pixmap, &Default::default())
            .unwrap()
            .check()
            .unwrap();
        let mut upload = Upload::new(257);
        let full = Rectangle {
            x: 0,
            y: 0,
            width: 257,
            height: 257,
        };
        for n in 1..=4 {
            let pixels = vec![n; 257 * 257 * 4];
            upload.write(&conn, pixmap, gc, &pixels, full).unwrap();
            conn.copy_area(
                pixmap,
                presented,
                gc,
                full.x,
                full.y,
                full.x,
                full.y,
                full.width,
                full.height,
            )
            .unwrap();
        }
        let pixels = conn
            .get_image(ImageFormat::Z_PIXMAP, pixmap, 0, 0, 257, 257, u32::MAX)
            .unwrap()
            .reply()
            .unwrap()
            .data;
        assert_eq!(pixels, vec![4; 257 * 257 * 4]);
        let mut expected = pixels;
        let damage = Rectangle {
            x: 2,
            y: 3,
            width: 4,
            height: 5,
        };
        for y in 3..8 {
            expected[(y * 257 + 2) * 4..(y * 257 + 6) * 4].fill(0);
        }
        upload.write(&conn, pixmap, gc, &expected, damage).unwrap();
        conn.copy_area(
            pixmap,
            presented,
            gc,
            damage.x,
            damage.y,
            damage.x,
            damage.y,
            damage.width,
            damage.height,
        )
        .unwrap();
        let actual = conn
            .get_image(ImageFormat::Z_PIXMAP, pixmap, 0, 0, 257, 257, u32::MAX)
            .unwrap()
            .reply()
            .unwrap()
            .data;
        assert_eq!(
            actual, expected,
            "damage clears old pixels and preserves everything outside it"
        );
        let shown = conn
            .get_image(ImageFormat::Z_PIXMAP, presented, 0, 0, 257, 257, u32::MAX)
            .unwrap()
            .reply()
            .unwrap()
            .data;
        assert_eq!(
            shown, expected,
            "partial presentation must clear damage and preserve untouched pixels"
        );
        conn.free_pixmap(presented).unwrap().check().unwrap();
        conn.free_gc(gc).unwrap().check().unwrap();
        conn.free_pixmap(pixmap).unwrap().check().unwrap();
    }
}
