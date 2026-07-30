use std::fs;
use std::path::Path;

use rusttable_image::{DecodeLimits, ImageInput, InputFormat, PixelLayout};
use rusttable_image_io::FileImageInput;

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
        "rgb" => include_str!("fixtures/rgb-2x1.tiff.b64"),
        "rgba" => include_str!("fixtures/rgba-1x2.tiff.b64"),
        "gray" => include_str!("fixtures/gray-2x1.tiff.b64"),
        _ => panic!("unknown fixture"),
    })
}

fn input() -> FileImageInput {
    FileImageInput::new(DecodeLimits::new(1_000_000, 2, 2, 4, 16).expect("valid test limits"))
}

fn with_bytes<T>(name: &str, bytes: Vec<u8>, operation: impl FnOnce(&Path) -> T) -> T {
    let path = std::env::temp_dir().join(format!("rusttable-tiff-{name}.fixture"));
    fs::write(&path, bytes).expect("fixture should be writable");
    let result = operation(&path);
    fs::remove_file(path).expect("fixture should be removable");
    result
}

#[test]
fn little_endian_rgb8_is_decoded_from_signature() {
    let bytes = fixture("rgb");
    let probe = with_bytes("rgb-wrong-extension.jpg", bytes.clone(), |path| {
        input().probe_path(path)
    })
    .expect("RGB TIFF should probe");
    let image = with_bytes("rgb-wrong-extension.png", bytes, |path| {
        input().decode_path(path)
    })
    .expect("RGB TIFF should decode");

    assert_eq!(probe.format(), InputFormat::Tiff);
    assert_eq!(probe.dimensions().width(), 2);
    assert_eq!(probe.dimensions().height(), 1);
    assert_eq!(image.layout(), PixelLayout::Rgba8StraightAlpha);
    assert_eq!(image.pixels(), &[255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn big_endian_rgba8_preserves_straight_alpha() {
    let image = with_bytes("rgba", fixture("rgba"), |path| input().decode_path(path))
        .expect("RGBA TIFF should decode");

    assert_eq!(image.dimensions().width(), 1);
    assert_eq!(image.dimensions().height(), 2);
    assert_eq!(image.pixels(), &[255, 0, 0, 128, 0, 255, 0, 64]);
}

#[test]
fn grayscale8_replicates_samples_and_adds_opaque_alpha() {
    let image = with_bytes("gray", fixture("gray"), |path| input().decode_path(path))
        .expect("grayscale TIFF should decode");

    assert_eq!(image.pixels(), &[17, 17, 17, 255, 231, 231, 231, 255]);
}
