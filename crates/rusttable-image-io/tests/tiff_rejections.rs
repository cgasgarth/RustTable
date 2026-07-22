use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_image::{DecodeLimits, ImageInput, ImageInputError};
use rusttable_image_io::FileImageInput;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

fn decode_base64(encoded: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut quartet = [0u8; 4];
    let mut count = 0;
    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        quartet[count] = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            _ => panic!("fixture contains invalid base64"),
        };
        count += 1;
        if count == 4 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
            if quartet[2] != 64 {
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
            }
            if quartet[3] != 64 {
                output.push((quartet[2] << 6) | quartet[3]);
            }
            count = 0;
        }
    }
    assert_eq!(count, 0);
    output
}

fn fixture(name: &str) -> Vec<u8> {
    decode_base64(match name {
        "bigtiff" => include_str!("fixtures/bigtiff.tiff.b64"),
        "multi" => include_str!("fixtures/multi-page.tiff.b64"),
        "16bit" => include_str!("fixtures/rgb-16bit.tiff.b64"),
        "malformed-offset" => include_str!("fixtures/malformed-offset.tiff.b64"),
        "rgb" => include_str!("fixtures/rgb-2x1.tiff.b64"),
        _ => panic!("unknown fixture"),
    })
}

fn input(max_source_bytes: u64) -> FileImageInput {
    FileImageInput::new(DecodeLimits::new(max_source_bytes, 2, 2, 4, 16).unwrap())
}

fn with_bytes<T>(name: &str, bytes: Vec<u8>, operation: impl FnOnce(&Path) -> T) -> T {
    let unique = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusttable-tiff-reject-{name}-{}-{unique}.fixture",
        std::process::id()
    ));
    fs::write(&path, bytes).unwrap();
    let result = operation(&path);
    fs::remove_file(path).unwrap();
    result
}

#[test]
fn truncated_bigtiff_is_malformed() {
    let result = with_bytes("bigtiff", fixture("bigtiff"), |path| {
        input(1_000_000).probe_path(path)
    });

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput { .. })
    ));
}

#[test]
fn recursive_page_chain_is_rejected() {
    let result = with_bytes("multi", fixture("multi"), |path| {
        input(1_000_000).probe_path(path)
    });

    assert!(
        matches!(result, Err(ImageInputError::MalformedInput { message, .. }) if message.contains("cycle"))
    );
}

#[test]
fn high_bit_depth_is_accepted_by_probe() {
    let result = with_bytes("16bit", fixture("16bit"), |path| {
        input(1_000_000).probe_path(path)
    });

    assert_eq!(result.expect("16-bit TIFF probes").dimensions().width(), 2);
}

#[test]
fn source_limit_precedes_tiff_dispatch() {
    let bytes = fixture("rgb");
    let result = with_bytes("too-large", bytes.clone(), |path| {
        input(u64::try_from(bytes.len() - 1).unwrap()).probe_path(path)
    });

    assert_eq!(
        result,
        Err(ImageInputError::SourceTooLarge {
            limit: u64::try_from(bytes.len() - 1).unwrap(),
            actual: u64::try_from(bytes.len()).unwrap(),
        })
    );
}

#[test]
fn tiff_dimension_and_decoded_byte_limits_are_checked_before_decode() {
    let width = with_bytes("width-limit", fixture("rgb"), |path| {
        FileImageInput::new(DecodeLimits::new(1_000_000, 1, 1, 1, 4).unwrap()).probe_path(path)
    });
    assert_eq!(
        width,
        Err(ImageInputError::WidthLimit {
            actual: 2,
            limit: 1,
        })
    );

    let pixels = with_bytes("pixel-limit", fixture("rgb"), |path| {
        FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 1, 4).unwrap()).probe_path(path)
    });
    assert_eq!(
        pixels,
        Err(ImageInputError::PixelLimit {
            actual: 2,
            limit: 1,
        })
    );

    let bytes = with_bytes("decoded-byte-limit", fixture("rgb"), |path| {
        FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 2, 4).unwrap()).probe_path(path)
    });
    assert_eq!(
        bytes,
        Err(ImageInputError::DecodedByteLimit {
            actual: 6,
            limit: 4,
        })
    );
}

#[test]
fn truncated_tiff_is_malformed() {
    let result = with_bytes("truncated", fixture("rgb")[..20].to_vec(), |path| {
        input(1_000_000).probe_path(path)
    });

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: rusttable_image::InputFormat::Tiff,
            ..
        })
    ));
}

#[test]
fn malformed_strip_offset_is_typed_without_panicking() {
    let result = with_bytes("malformed-offset", fixture("malformed-offset"), |path| {
        input(1_000_000).decode_path(path)
    });

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: rusttable_image::InputFormat::Tiff,
            ..
        })
    ));
}
