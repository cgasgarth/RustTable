use std::fs;
use std::path::Path;

use rusttable_image::{ColorEncoding, DecodeLimits, ImageInput, InputFormat, PixelLayout};
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
    assert_eq!(count, 0, "fixture base64 must have complete quartets");
    output
}

fn fixture(name: &str) -> Vec<u8> {
    decode_base64(match name {
        "png" => include_str!("fixtures/rgba-2x1.png.b64"),
        "jpeg" => include_str!("fixtures/rgb-2x1.jpg.b64"),
        _ => panic!("unknown fixture"),
    })
}

fn input() -> FileImageInput {
    FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("valid test limits"))
}

fn with_fixture<T>(name: &str, bytes: &[u8], operation: impl FnOnce(&Path) -> T) -> T {
    let path = std::env::temp_dir().join(format!("rusttable-image-io-{name}.fixture"));
    fs::write(&path, bytes).expect("fixture should be writable");
    let result = operation(&path);
    fs::remove_file(path).expect("fixture should be removable");
    result
}

#[test]
fn png_preserves_rgba_samples_with_srgb_fallback() {
    let result = with_fixture("png", &fixture("png"), |path| input().decode_path(path));
    let image = result.expect("reviewed PNG should decode");

    assert_eq!(image.dimensions().width(), 2);
    assert_eq!(image.dimensions().height(), 1);
    assert_eq!(image.layout(), PixelLayout::Rgba8StraightAlpha);
    assert_eq!(image.color_encoding(), ColorEncoding::SrgbD65);
    assert_eq!(image.pixels(), &[255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn jpeg_decodes_to_opaque_rgba8() {
    let result = with_fixture("jpeg", &fixture("jpeg"), |path| input().decode_path(path));
    let image = result.expect("reviewed JPEG should decode");

    assert_eq!(image.dimensions().width(), 2);
    assert_eq!(image.dimensions().height(), 1);
    assert_eq!(image.layout(), PixelLayout::Rgba8StraightAlpha);
    assert_eq!(image.color_encoding(), ColorEncoding::SrgbD65);
    assert_eq!(image.pixels().len(), 8);
    assert!(
        image
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
}

#[test]
fn probe_reports_signature_selected_format() {
    let result = with_fixture("png-probe.jpg", &fixture("png"), |path| {
        input().probe_path(path)
    });
    let probe = result.expect("reviewed PNG should probe");

    assert_eq!(probe.format(), InputFormat::Png);
}

#[test]
fn byte_operations_match_path_operations_for_the_same_snapshot() {
    let bytes = fixture("png");
    let path_result = with_fixture("png-byte-equivalence.jpg", &bytes, |path| {
        (input().probe_path(path), input().decode_path(path))
    });
    let probe = input().probe_bytes(&bytes).expect("byte probe");
    let image = input().decode_bytes(&bytes).expect("byte decode");
    assert_eq!(path_result.0.expect("path probe"), probe);
    assert_eq!(path_result.1.expect("path decode").pixels(), image.pixels());
}
