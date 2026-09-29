//! In-memory decoding only. No paths, renderer objects or external metadata fetches.
use crate::{error, PackageError};
use image::codecs::{jpeg::JpegDecoder, png::PngDecoder, webp::WebPDecoder};
use image::{DynamicImage, ImageDecoder, ImageFormat, Limits};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, io::Cursor};

/// SHA-256 content identity; not a signature or proof of publisher trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Host policy, never supplied by a Mate Manifest. Zero budgets deny images.
#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    pub max_width: u32,
    pub max_height: u32,
    /// Strict maximum RGBA8 output size of one image.
    pub max_image_bytes: u64,
    /// Strict sum of retained RGBA8 image bytes in one package.
    pub max_total_image_bytes: u64,
    /// Checked native + RGBA buffers; decoder-internal allocations are best effort.
    pub max_decoder_bytes: u64,
}
impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_width: 2048,
            max_height: 2048,
            max_image_bytes: 16 * 1024 * 1024,
            max_total_image_bytes: 64 * 1024 * 1024,
            max_decoder_bytes: 64 * 1024 * 1024,
        }
    }
}
impl DecodeLimits {
    fn decoder_limits(self) -> Limits {
        let mut limits = Limits::default();
        limits.max_image_width = Some(self.max_width);
        limits.max_image_height = Some(self.max_height);
        limits.max_alloc = Some(self.max_decoder_bytes);
        limits
    }
}

/// Validated static RGBA8 samples, top-to-bottom rows. Sprite assets use straight
/// alpha; explicitly declared Spine atlas pages retain their source PMA samples.
/// No color profile conversion or EXIF orientation is applied.
pub struct DecodedImage {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
    source_hash: ContentHash,
}
impl DecodedImage {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
    /// Hash of the encoded source bytes, not the pixels.
    pub fn source_hash(&self) -> ContentHash {
        self.source_hash
    }
}

/// Domain-separated, length-prefixed, sorted file map. Includes TOML and sounds.
pub(crate) fn package_hash(files: &BTreeMap<String, Vec<u8>>) -> ContentHash {
    let mut hash = Sha256::new();
    hash.update(b"mochi.directory.v1\0");
    hash.update((files.len() as u64).to_le_bytes());
    for (key, bytes) in files {
        hash.update((key.len() as u64).to_le_bytes());
        hash.update(key.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    ContentHash(hash.finalize().into())
}

pub(crate) fn decode_images(
    files: &BTreeMap<String, Vec<u8>>,
    limits: DecodeLimits,
) -> Result<BTreeMap<String, DecodedImage>, PackageError> {
    let mut output = BTreeMap::new();
    let mut remaining = limits.max_total_image_bytes;
    for (key, bytes) in files.iter().filter(|(key, _)| key.starts_with("sprites/")) {
        let decoded = decode(bytes, key, limits, remaining)
            .map_err(|e| error(format!("image {key}: {e}")))?;
        remaining -= decoded.rgba8.len() as u64;
        output.insert(key.clone(), decoded);
    }
    Ok(output)
}

fn decode(
    bytes: &[u8],
    key: &str,
    limits: DecodeLimits,
    remaining: u64,
) -> Result<DecodedImage, PackageError> {
    let expected = match key.rsplit('.').next() {
        Some("png") => ImageFormat::Png,
        Some("jpg" | "jpeg") => ImageFormat::Jpeg,
        Some("webp") => ImageFormat::WebP,
        _ => return Err(error("unsupported image extension")),
    };
    if image::guess_format(bytes).map_err(error)? != expected {
        return Err(error("image content does not match extension"));
    }
    crate::containers::validate(bytes, expected)?;
    // Select only built-in decoders; no ImageReader/global decoder hook registry.
    let mut decoder: Box<dyn ImageDecoder + '_> = match expected {
        ImageFormat::Png => {
            let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits.decoder_limits())
                .map_err(error)?;
            if decoder.is_apng().map_err(error)? {
                return Err(error("animated PNG is not supported"));
            }
            Box::new(decoder)
        }
        ImageFormat::Jpeg => Box::new(JpegDecoder::new(Cursor::new(bytes)).map_err(error)?),
        ImageFormat::WebP => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(error)?;
            if decoder.has_animation() {
                return Err(error("animated WebP is not supported"));
            }
            Box::new(decoder)
        }
        _ => return Err(error("unsupported image format")),
    };
    decoder.set_limits(limits.decoder_limits()).map_err(error)?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height {
        return Err(error("image dimensions exceed limits"));
    }
    let rgba_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| error("image size overflow"))?;
    if rgba_bytes > limits.max_image_bytes {
        return Err(error("image RGBA byte limit exceeded"));
    }
    if rgba_bytes > remaining {
        return Err(error("package RGBA byte limit exceeded"));
    }
    let native_bytes = decoder.total_bytes();
    let working_bytes = native_bytes
        .checked_add(rgba_bytes)
        .ok_or_else(|| error("decode buffer size overflow"))?;
    if working_bytes > limits.max_decoder_bytes || working_bytes > isize::MAX as u64 {
        return Err(error("decode buffer byte limit exceeded"));
    }
    let pixels = DynamicImage::from_decoder(decoder)
        .map_err(error)?
        .into_rgba8();
    if pixels.dimensions() != (width, height) || pixels.as_raw().len() as u64 != rgba_bytes {
        return Err(error("decoded pixels disagree with image header"));
    }
    Ok(DecodedImage {
        width,
        height,
        rgba8: pixels.into_raw(),
        source_hash: ContentHash(Sha256::digest(bytes).into()),
    })
}
