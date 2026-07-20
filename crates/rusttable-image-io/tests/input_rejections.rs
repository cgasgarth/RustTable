use std::fs;
use std::path::Path;

use rusttable_image::{DecodeLimits, ImageInput, ImageInputError};
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
        "gif" => include_str!("fixtures/unsupported.gif.b64"),
        "truncated" => include_str!("fixtures/truncated.png.b64"),
        _ => panic!("unknown fixture"),
    })
}

fn input(max_source_bytes: u64) -> FileImageInput {
    FileImageInput::new(DecodeLimits::new(max_source_bytes, 2, 1, 2, 8).expect("valid test limits"))
}

fn with_fixture<T>(name: &str, bytes: &[u8], operation: impl FnOnce(&Path) -> T) -> T {
    let path = std::env::temp_dir().join(format!("rusttable-image-io-reject-{name}"));
    fs::write(&path, bytes).expect("fixture should be writable");
    let result = operation(&path);
    fs::remove_file(path).expect("fixture should be removable");
    result
}

#[test]
fn unsupported_signature_is_not_selected_by_extension() {
    let result = with_fixture("gif-as-png.png", &fixture("gif"), |path| {
        input(1_000_000).probe_path(path)
    });
    assert!(matches!(
        result,
        Err(ImageInputError::UnsupportedSignature { .. })
    ));
}

#[test]
fn truncated_png_is_malformed_after_signature_dispatch() {
    let result = with_fixture("truncated", &fixture("truncated"), |path| {
        input(1_000_000).probe_path(path)
    });
    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput { .. })
    ));
}

#[test]
fn source_limit_precedes_signature_and_codec_work() {
    let payload = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 1];
    let result = with_fixture("too-large", &payload, |path| input(9).probe_path(path));
    assert_eq!(
        result,
        Err(ImageInputError::SourceTooLarge {
            limit: 9,
            actual: 10
        })
    );
}

#[test]
fn byte_probe_rejects_oversized_source_before_signature_work() {
    let payload = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 1];
    let result = input(9).probe_bytes(&payload);

    assert_eq!(
        result,
        Err(ImageInputError::SourceTooLarge {
            limit: 9,
            actual: 10
        })
    );
}

#[test]
fn byte_decode_rejects_oversized_source_before_codec_work() {
    let payload = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 1];
    let result = input(9).decode_bytes(&payload);

    assert_eq!(
        result,
        Err(ImageInputError::SourceTooLarge {
            limit: 9,
            actual: 10
        })
    );
}

#[test]
fn missing_path_is_a_typed_io_error() {
    let result = input(1_000_000).probe_path(Path::new("does-not-exist.rusttable"));
    assert!(matches!(result, Err(ImageInputError::Io { .. })));
}
