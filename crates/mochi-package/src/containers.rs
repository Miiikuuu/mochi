//! Small framing checks before codec entry. Pixel decoding stays in maintained codecs.
use crate::{error, PackageError};
use image::ImageFormat;

pub(crate) fn validate(bytes: &[u8], format: ImageFormat) -> Result<(), PackageError> {
    match format {
        ImageFormat::Png => png(bytes),
        ImageFormat::WebP => webp(bytes),
        ImageFormat::Jpeg if bytes.ends_with(b"\xff\xd9") => Ok(()),
        ImageFormat::Jpeg => Err(error("missing JPEG end marker")),
        _ => Err(error("unsupported image container")),
    }
}

fn png(bytes: &[u8]) -> Result<(), PackageError> {
    let mut rest = &bytes[8..]; // caller has checked the PNG signature
    let mut first = true;
    while rest.len() >= 12 {
        let length = u32::from_be_bytes(rest[..4].try_into().unwrap()) as u64;
        if length + 12 > rest.len() as u64 {
            return Err(error("truncated PNG chunk"));
        }
        let length = length as usize;
        let kind = &rest[4..8];
        if first && (kind != b"IHDR" || length != 13) {
            return Err(error("PNG must begin with IHDR"));
        }
        first = false;
        let expected = u32::from_be_bytes(rest[8 + length..12 + length].try_into().unwrap());
        if crc32fast::hash(&rest[4..8 + length]) != expected {
            return Err(error("PNG chunk checksum mismatch"));
        }
        if matches!(kind, b"acTL" | b"fcTL" | b"fdAT") {
            return Err(error("animated PNG is not supported"));
        }
        if kind == b"IEND" {
            return if length == 0 && rest.len() == 12 {
                Ok(())
            } else {
                Err(error("invalid PNG end or trailing bytes"))
            };
        }
        rest = &rest[length + 12..];
    }
    Err(error("missing PNG end chunk"))
}

fn u24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

fn webp(bytes: &[u8]) -> Result<(), PackageError> {
    // Signature checked by the caller. Require exact RIFF and chunk lengths;
    // malformed lengths must not drive allocations or seeks beyond the snapshot.
    let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u64;
    if size + 8 != bytes.len() as u64 {
        return Err(error("WebP RIFF size mismatch"));
    }
    let mut rest = &bytes[12..];
    let mut canvas = None;
    let mut frame = None;
    while rest.len() >= 8 {
        let size = u32::from_le_bytes(rest[4..8].try_into().unwrap()) as u64;
        let padded = size + (size & 1);
        if padded + 8 > rest.len() as u64 {
            return Err(error("truncated WebP chunk"));
        }
        let data = &rest[8..8 + size as usize];
        match &rest[..4] {
            b"VP8X" => {
                if canvas.is_some() || data.len() != 10 {
                    return Err(error("invalid WebP extended header"));
                }
                if data[0] & 2 != 0 {
                    return Err(error("animated WebP is not supported"));
                }
                canvas = Some((u24(&data[4..7]) + 1, u24(&data[7..10]) + 1));
            }
            b"ANIM" | b"ANMF" => return Err(error("animated WebP is not supported")),
            kind @ (b"VP8 " | b"VP8L") => {
                if frame.is_some() {
                    return Err(error("multiple WebP pixel chunks"));
                }
                frame = Some(if kind == b"VP8 " {
                    if data.len() < 10 || data[0] & 1 != 0 || data[3..6] != [0x9d, 0x01, 0x2a] {
                        return Err(error("invalid WebP VP8 header"));
                    }
                    (
                        u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff),
                        u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff),
                    )
                } else {
                    if data.len() < 5 || data[0] != 0x2f {
                        return Err(error("invalid WebP VP8L header"));
                    }
                    let packed = u32::from_le_bytes(data[1..5].try_into().unwrap());
                    ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
                });
            }
            _ => {}
        }
        rest = &rest[8 + padded as usize..];
    }
    if !rest.is_empty() || frame.is_none() || canvas.is_some_and(|size| Some(size) != frame) {
        return Err(error(
            "invalid WebP chunks or inconsistent canvas/frame dimensions",
        ));
    }
    Ok(())
}
