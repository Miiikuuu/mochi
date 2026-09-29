use super::package::Fixture;
use crate::*;
use mochi_package::{DecodeLimits, LoadedPackage};
use sha2::{Digest, Sha256};
use std::{fs, time::Duration};

const PNG: &[u8] = include_bytes!("../fixtures/rgba.png");
const JPEG: &[u8] = include_bytes!("../fixtures/rgb.jpg");
const WEBP: &[u8] = include_bytes!("../fixtures/rgba.webp");
const LOSSY_WEBP: &[u8] = include_bytes!("../fixtures/rgba-lossy.webp");

fn with_sprite(extension: &str, bytes: &[u8]) -> Fixture {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("sprites/neko.png")).unwrap();
    fs::write(fixture.0.join(format!("sprites/neko.{extension}")), bytes).unwrap();
    fixture.replace("manifest.toml", "neko.png", &format!("neko.{extension}"));
    fixture
}

fn rejection(fixture: &Fixture, limits: DecodeLimits) -> String {
    LoadedPackage::load_with_limits(&fixture.0, limits)
        .err()
        .expect("package must be rejected")
        .to_string()
}

#[test]
fn png_jpeg_and_webp_decode_to_immutable_rgba() {
    for (ext, bytes) in [
        ("png", PNG),
        ("jpg", JPEG),
        ("jpeg", JPEG),
        ("webp", WEBP),
        ("webp", LOSSY_WEBP),
    ] {
        let fixture = with_sprite(ext, bytes);
        let package = LoadedPackage::load(&fixture.0).unwrap();
        let image = package.image(&format!("sprites/neko.{ext}")).unwrap();
        assert_eq!((image.width(), image.height()), (2, 2));
        assert_eq!(image.rgba8().len(), 16);
        assert_eq!(
            *image.source_hash().as_bytes(),
            <[u8; 32]>::from(Sha256::digest(bytes))
        );
        if ext == "png" {
            assert_eq!(
                image.rgba8(),
                [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 255, 255, 255, 255]
            );
        }
    }
}

#[test]
fn resource_lookup_is_exact_package_local_and_has_no_path_fallback() {
    let fixture = with_sprite("png", PNG);
    let package = LoadedPackage::load(&fixture.0).unwrap();
    for key in [
        "sprites/absent.png",
        "Sprites/neko.png",
        "sprites/./neko.png",
        "../sprites/neko.png",
        "https://example.com/a.png",
    ] {
        assert!(package.image(key).is_err(), "{key}");
    }
    assert!(package
        .image(fixture.0.join("sprites/neko.png").to_str().unwrap())
        .is_err());
}

#[test]
fn sixteen_bit_png_is_normalized_to_rgba8() {
    let fixture = with_sprite("png", include_bytes!("../fixtures/gray16.png"));
    let package = LoadedPackage::load(&fixture.0).unwrap();
    assert_eq!(
        package.image("sprites/neko.png").unwrap().rgba8(),
        [255, 255, 255, 255]
    );
}

#[test]
fn truncated_containers_and_png_trailers_are_rejected() {
    for (ext, bytes) in [("png", PNG), ("jpg", JPEG), ("webp", WEBP)] {
        let fixture = with_sprite(ext, &bytes[..bytes.len() - 1]);
        assert!(LoadedPackage::load(&fixture.0).is_err(), "{ext}");
    }
    let mut trailing = PNG.to_vec();
    trailing.extend_from_slice(b"unexpected trailer");
    assert!(LoadedPackage::load(&with_sprite("png", &trailing).0).is_err());
    let mut bad_crc = PNG.to_vec();
    *bad_crc.last_mut().unwrap() ^= 1;
    assert!(rejection(&with_sprite("png", &bad_crc), DecodeLimits::default()).contains("checksum"));
}

#[test]
fn webp_chunk_lengths_and_canvas_frame_mismatch_fail_before_decode() {
    let mut bad_length = WEBP.to_vec();
    bad_length[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(
        rejection(&with_sprite("webp", &bad_length), DecodeLimits::default())
            .contains("truncated WebP chunk")
    );
    let mut bad_size = LOSSY_WEBP.to_vec();
    assert_eq!(&bad_size[12..16], b"VP8X");
    // Canvas width becomes 1, while the compressed VP8 frame still says 2.
    bad_size[24..27].copy_from_slice(&[0, 0, 0]);
    assert!(
        rejection(&with_sprite("webp", &bad_size), DecodeLimits::default())
            .contains("inconsistent canvas/frame")
    );
}

#[test]
fn import_survives_overwrite_rename_and_source_removal() {
    let fixture = with_sprite("png", PNG);
    let package = LoadedPackage::load(&fixture.0).unwrap();
    let content_hash = package.content_hash();
    fs::write(fixture.0.join("sprites/neko.png"), b"replacement bytes").unwrap();
    let renamed = fixture.0.with_extension("renamed.mochi");
    fs::rename(&fixture.0, &renamed).unwrap();
    fs::remove_dir_all(&renamed).unwrap();
    assert_eq!(package.content_hash(), content_hash);
    assert_eq!(package.assets()["sprites/neko.png"], PNG);
    assert_eq!(package.image("sprites/neko.png").unwrap().rgba8()[0], 255);
    let mut mate = package.instantiate("after-removal").unwrap();
    let output = mate
        .process(MateEvent::Tick {
            delta: Duration::from_secs(1),
        })
        .unwrap();
    let node = &output.scene.nodes[0];
    let SceneContent::Sprite { asset, .. } = &node.content else {
        panic!("expected Sprite")
    };
    assert_eq!(node.transform.translation.x, 10.0);
    assert_eq!(package.image(asset).unwrap().rgba8().len(), 16);
}

#[test]
fn content_identity_ignores_location_and_binds_configuration_and_asset_keys() {
    let first = with_sprite("png", PNG);
    let second = with_sprite("png", PNG);
    let original = LoadedPackage::load(&first.0).unwrap().content_hash();
    assert_eq!(
        original,
        LoadedPackage::load(&second.0).unwrap().content_hash()
    );
    second.replace("behavior.toml", "speed = 10.0", "speed = 20.0");
    assert_ne!(
        original,
        LoadedPackage::load(&second.0).unwrap().content_hash()
    );
    second.replace("behavior.toml", "speed = 20.0", "speed = 10.0");
    fs::rename(
        second.0.join("sprites/neko.png"),
        second.0.join("sprites/other.png"),
    )
    .unwrap();
    second.replace("manifest.toml", "neko.png", "other.png");
    assert_ne!(
        original,
        LoadedPackage::load(&second.0).unwrap().content_hash()
    );
    assert_eq!(original.to_string().len(), 64);
}

#[test]
fn identical_keys_in_two_packages_do_not_share_pixels_or_state() {
    let first = with_sprite("png", PNG);
    let second = Fixture::new(); // original 1×1 white Neko
    let a = LoadedPackage::load(&first.0).unwrap();
    let b = LoadedPackage::load(&second.0).unwrap();
    assert_ne!(a.content_hash(), b.content_hash());
    assert_eq!(a.image("sprites/neko.png").unwrap().rgba8().len(), 16);
    assert_eq!(
        b.image("sprites/neko.png").unwrap().rgba8(),
        [255, 255, 255, 255]
    );
    let mut x = a.instantiate("x").unwrap();
    let y = a.instantiate("y").unwrap();
    x.process(MateEvent::Tick {
        delta: Duration::from_secs(1),
    })
    .unwrap();
    assert_eq!(y.state().position.x, 0.0);
    assert_eq!(a.image("sprites/neko.png").unwrap().rgba8()[0], 255);
}

#[test]
fn neko_content_hash_matches_the_v1_framing_vector() {
    // Independently computed using Python hashlib and little-endian u64 lengths.
    assert_eq!(
        LoadedPackage::load(neko_path())
            .unwrap()
            .content_hash()
            .to_string(),
        "e7cb343e0aa4a8db302f625e2c457b101fd32deaa8413463554137eb1f520c5f"
    );
}

#[test]
fn renamed_text_wrong_format_and_corrupt_pixels_are_rejected() {
    for bad in [
        b"#!/bin/sh\necho never-execute".as_slice(),
        JPEG,
        &PNG[..30],
    ] {
        let fixture = with_sprite("png", bad);
        assert!(rejection(&fixture, DecodeLimits::default()).contains("image sprites/neko.png"));
    }
    let mut corrupt = PNG.to_vec();
    let idat = corrupt.windows(4).position(|w| w == b"IDAT").unwrap();
    corrupt[idat + 4] ^= 0xff;
    assert!(LoadedPackage::load(&with_sprite("png", &corrupt).0).is_err());
}

#[test]
fn unreferenced_and_animation_mapped_assets_must_also_decode() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("sprites/unused.png"), b"not an image").unwrap();
    assert!(rejection(&fixture, DecodeLimits::default()).contains("unused.png"));
    fixture.replace(
        "manifest.toml",
        "height = 32.0",
        "height = 32.0\n[components.config.animations]\nsleep = 'sprites/unused.png'",
    );
    assert!(rejection(&fixture, DecodeLimits::default()).contains("unused.png"));
}

#[test]
fn animated_formats_are_rejected_instead_of_silently_using_first_frame() {
    for (ext, bytes) in [
        ("png", include_bytes!("../fixtures/animated.png").as_slice()),
        (
            "webp",
            include_bytes!("../fixtures/animated.webp").as_slice(),
        ),
    ] {
        let fixture = with_sprite(ext, bytes);
        assert!(rejection(&fixture, DecodeLimits::default()).contains("animated"));
    }
}

#[test]
fn dimension_bomb_is_rejected_from_its_header() {
    let fixture = with_sprite("png", include_bytes!("../fixtures/huge-header.png"));
    let message = rejection(&fixture, DecodeLimits::default());
    assert!(
        message.contains("dimensions") || message.contains("limit"),
        "{message}"
    );
    for limits in [
        DecodeLimits {
            max_width: 1,
            ..Default::default()
        },
        DecodeLimits {
            max_height: 1,
            ..Default::default()
        },
    ] {
        assert!(!rejection(&with_sprite("png", PNG), limits).is_empty());
    }
}

#[test]
fn per_image_and_package_budgets_check_before_output_allocation() {
    let fixture = with_sprite("png", PNG);
    let limits = DecodeLimits {
        max_image_bytes: 15,
        ..Default::default()
    };
    assert!(rejection(&fixture, limits).contains("image RGBA byte limit"));
    let limits = DecodeLimits {
        max_image_bytes: 16,
        max_total_image_bytes: 16,
        ..Default::default()
    };
    LoadedPackage::load_with_limits(&fixture.0, limits).unwrap();
    fs::write(fixture.0.join("sprites/second.png"), PNG).unwrap();
    assert!(rejection(&fixture, limits).contains("package RGBA byte limit"));
    LoadedPackage::load_with_limits(
        &fixture.0,
        DecodeLimits {
            max_total_image_bytes: 32,
            ..limits
        },
    )
    .unwrap();
}

#[test]
fn decode_working_buffer_and_zero_budgets_are_enforced() {
    let fixture = with_sprite("jpg", JPEG);
    // 2×2 RGB native buffer (12) + RGBA8 output (16).
    let limits = DecodeLimits {
        max_decoder_bytes: 27,
        ..Default::default()
    };
    assert!(rejection(&fixture, limits).contains("decode buffer byte limit"));
    for limits in [
        DecodeLimits {
            max_image_bytes: 0,
            ..Default::default()
        },
        DecodeLimits {
            max_total_image_bytes: 0,
            ..Default::default()
        },
        DecodeLimits {
            max_width: 0,
            ..Default::default()
        },
    ] {
        assert!(!rejection(&fixture, limits).is_empty());
    }
}

#[test]
fn sounds_remain_opaque_and_cannot_resolve_as_images() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.0.join("sounds")).unwrap();
    fs::write(
        fixture.0.join("sounds/opaque.wav"),
        b"opaque data, never decoded or executed",
    )
    .unwrap();
    let package = LoadedPackage::load(&fixture.0).unwrap();
    assert!(package.assets().contains_key("sounds/opaque.wav"));
    assert!(package.image("sounds/opaque.wav").is_err());
    let old = package.content_hash();
    fs::write(fixture.0.join("sounds/opaque.wav"), b"different bytes").unwrap();
    assert_ne!(old, LoadedPackage::load(&fixture.0).unwrap().content_hash());
}

#[test]
fn hard_linked_resources_are_rejected() {
    let fixture = Fixture::new();
    fs::hard_link(
        fixture.0.join("sprites/neko.png"),
        fixture.0.join("sprites/linked.png"),
    )
    .unwrap();
    assert!(rejection(&fixture, DecodeLimits::default()).contains("one link"));
}

#[cfg(unix)]
#[test]
fn symlinked_root_directories_and_special_files_are_rejected() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let link = fixture.0.with_extension("link.mochi");
    symlink(&fixture.0, &link).unwrap();
    let result = LoadedPackage::load(&link);
    fs::remove_file(link).unwrap();
    assert!(result.is_err());
    symlink(".", fixture.0.join("sprites/linked")).unwrap();
    assert!(rejection(&fixture, DecodeLimits::default())
        .contains("symlinks are forbidden: sprites/linked"));
    fs::remove_file(fixture.0.join("sprites/linked")).unwrap();
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        fixture.0.join("sprites/pipe.png"),
        rustix::fs::Mode::RUSR,
    )
    .unwrap();
    assert!(LoadedPackage::load(&fixture.0).is_err());
}
