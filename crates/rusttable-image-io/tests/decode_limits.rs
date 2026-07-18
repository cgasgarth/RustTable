use std::fs;
use std::path::Path;

use rusttable_image::{DecodeLimits, ImageInput, ImageInputError};
use rusttable_image_io::FileImageInput;

fn png() -> Vec<u8> {
    decode(include_str!("fixtures/rgba-2x1.png.b64"))
}

fn png_1x2() -> Vec<u8> {
    decode(include_str!("fixtures/rgba-1x2.png.b64"))
}

fn decode(encoded: &str) -> Vec<u8> {
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
    output
}

fn input(limits: DecodeLimits) -> FileImageInput {
    FileImageInput::new(limits)
}

fn with_png<T>(name: &str, operation: impl FnOnce(&Path) -> T) -> T {
    with_bytes(name, png(), operation)
}

fn with_png_1x2<T>(name: &str, operation: impl FnOnce(&Path) -> T) -> T {
    with_bytes(name, png_1x2(), operation)
}

fn with_bytes<T>(name: &str, bytes: Vec<u8>, operation: impl FnOnce(&Path) -> T) -> T {
    let path = std::env::temp_dir().join(format!("rusttable-image-io-limit-{name}.fixture"));
    fs::write(&path, bytes).expect("fixture should be writable");
    let result = operation(&path);
    fs::remove_file(path).expect("fixture should be removable");
    result
}

fn limits(
    max_source_bytes: u64,
    max_width: u32,
    max_height: u32,
    max_pixels: u64,
    max_bytes: u64,
) -> DecodeLimits {
    DecodeLimits::new(
        max_source_bytes,
        max_width,
        max_height,
        max_pixels,
        max_bytes,
    )
    .expect("valid test limits")
}

#[test]
fn width_limit_is_checked_before_decode() {
    let result = with_png("width", |path| {
        input(limits(1_000_000, 1, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::WidthLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn height_limit_is_checked_before_decode() {
    let result = with_png_1x2("height", |path| {
        input(limits(1_000_000, 1, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::HeightLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn pixel_limit_is_checked_before_decode() {
    let result = with_png("pixels", |path| {
        input(limits(1_000_000, 2, 1, 1, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::PixelLimit {
            actual: 2,
            limit: 1
        })
    );
}

#[test]
fn decoded_byte_limit_is_checked_before_decode() {
    let result = with_png("bytes", |path| {
        input(limits(1_000_000, 2, 1, 2, 4)).probe_path(path)
    });
    assert_eq!(
        result,
        Err(ImageInputError::DecodedByteLimit {
            actual: 8,
            limit: 4
        })
    );
}
