use rusttable_image::{DecodeLimits, ImageInputError, InputFormat};
use rusttable_image_io::{ImageDecoderRegistry, PROBE_BUDGET_BYTES, ProbeOutcome};

fn limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("valid test limits")
}

#[test]
fn standard_registry_has_stable_unique_decoder_identities() {
    let descriptors = ImageDecoderRegistry::standard().descriptors();
    assert_eq!(descriptors.len(), 5);
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.identity().id())
            .collect::<Vec<_>>(),
        vec![
            "rusttable.decoder.jpeg.v1",
            "rusttable.decoder.png.v1",
            "rusttable.decoder.openexr.v1",
            "rusttable.decoder.raw.v1",
            "rusttable.decoder.tiff.v1"
        ]
    );
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.identity().version() == 1)
    );
    assert_eq!(PROBE_BUDGET_BYTES, 64 * 1024);
}

fn decode_base64(encoded: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut quartet = [0_u8; 4];
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

#[test]
fn standard_registry_selects_png_without_a_path_or_extension() {
    let bytes = decode_base64(include_str!("fixtures/rgba-2x1.png.b64"));
    let registry = ImageDecoderRegistry::standard();
    let probe = registry
        .probe_bytes(&bytes, limits())
        .expect("PNG signature should select the PNG decoder");
    let decoded = registry
        .decode_bytes(&bytes, limits())
        .expect("PNG signature should decode through the PNG decoder");

    assert_eq!(probe.format(), InputFormat::Png);
    assert_eq!(decoded.dimensions(), probe.dimensions());
}

#[test]
fn standard_registry_verifies_jpeg_decoded_dimensions() {
    let bytes = decode_base64(include_str!("fixtures/rgb-2x1.jpg.b64"));
    let registry = ImageDecoderRegistry::standard();
    let probe = registry
        .probe_bytes(&bytes, limits())
        .expect("JPEG signature should select the JPEG decoder");
    let decoded = registry
        .decode_bytes(&bytes, limits())
        .expect("JPEG signature should decode through the JPEG decoder");

    assert_eq!(probe.format(), InputFormat::Jpeg);
    assert_eq!(decoded.dimensions(), probe.dimensions());
}

#[test]
fn recognized_png_signature_reports_malformed_input() {
    let bytes = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            ..
        })
    ));
}

#[test]
fn png_header_probe_is_structural_and_does_not_need_a_complete_payload() {
    let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    bytes.extend_from_slice(&13_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&3_u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes.extend_from_slice(&[0; 4]);
    let result = ImageDecoderRegistry::standard().probe(
        &bytes,
        DecodeLimits::new(1_000_000, 2, 3, 6, 24).expect("valid limits"),
    );

    assert!(matches!(
        result,
        ProbeOutcome::Match { decoder, probe }
            if decoder.format() == InputFormat::Png
                && probe.dimensions().width() == 2
                && probe.dimensions().height() == 3
    ));
}

#[test]
fn png_signature_with_invalid_ihdr_is_malformed_before_backend_dispatch() {
    let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    bytes.extend_from_slice(&12_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&[0; 12]);

    assert!(matches!(
        ImageDecoderRegistry::standard().probe(&bytes, limits()),
        ProbeOutcome::MalformedRecognized { decoder, .. }
            if decoder.format() == InputFormat::Png
    ));
}

#[test]
fn standard_registry_selects_jpeg_and_classic_tiff_by_signature() {
    let registry = ImageDecoderRegistry::standard();
    let jpeg = decode_base64(include_str!("fixtures/rgb-2x1.jpg.b64"));
    let tiff = decode_base64(include_str!("fixtures/rgb-2x1.tiff.b64"));

    let jpeg_probe = registry
        .probe_bytes(&jpeg, limits())
        .expect("JPEG signature should select the JPEG decoder");
    let tiff_probe = registry
        .probe_bytes(&tiff, limits())
        .expect("classic TIFF signature should select the TIFF decoder");

    assert_eq!(jpeg_probe.format(), InputFormat::Jpeg);
    assert_eq!(tiff_probe.format(), InputFormat::Tiff);
}

#[test]
fn camera_raw_signature_selects_the_raw_decoder_before_decode() {
    let bytes = b"FUJIFILMCCD-RAW";
    let result = ImageDecoderRegistry::standard().probe(bytes, limits());

    assert!(matches!(
        result,
        ProbeOutcome::MalformedRecognized { decoder, error: ImageInputError::MalformedInput { format: InputFormat::Raw, .. } }
            if decoder.format() == InputFormat::Raw
    ));
}

#[test]
fn malformed_bigtiff_signature_never_falls_back() {
    let bytes = decode_base64(include_str!("fixtures/bigtiff.tiff.b64"));
    let registry = ImageDecoderRegistry::standard();

    let probe = registry.probe_bytes(&bytes, limits());
    let decode = registry.decode_bytes(&bytes, limits());

    assert!(matches!(
        probe,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            ..
        })
    ));
    assert!(matches!(
        decode,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            ..
        })
    ));
}

#[test]
fn unsupported_signature_is_rejected() {
    let bytes = *b"GIF89a";
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::UnsupportedSignature { signature }) if signature == bytes
    ));
}

#[test]
fn openexr_magic_is_registered_without_shadowing_raw() {
    use exr::prelude::{f16, write_rgb_file};

    let path = std::env::temp_dir().join(format!("rusttable-registry-{}.exr", std::process::id()));
    write_rgb_file(&path, 2, 1, |x, _| {
        let value = f16::from_f32(f32::from(u16::try_from(x).unwrap()));
        (value, value, value)
    })
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let registry = ImageDecoderRegistry::standard();
    let probe = registry.probe_bytes(&bytes, limits()).unwrap();
    assert_eq!(probe.format(), InputFormat::OpenExr);
    assert_eq!(
        registry.select(b"FUJIFILMCCD-RAW").unwrap().format(),
        InputFormat::Raw
    );
}

#[test]
fn matched_malformed_jpeg_never_falls_back_to_another_decoder() {
    let bytes = [0xff, 0xd8, 0xff];
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));
}
