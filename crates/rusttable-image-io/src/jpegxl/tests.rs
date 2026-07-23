use rusttable_image::{ChannelLayout, DecodeLimits, ImageDimensions, InputFormat, SampleType};
use sha2::{Digest, Sha256};

use super::*;

const FIXTURE_SHA256: [u8; 32] = [
    0x63, 0x79, 0xdb, 0xc6, 0xa9, 0x4c, 0x9a, 0xf4, 0xbb, 0x8a, 0x89, 0xa0, 0x22, 0xb7, 0x9a, 0x52,
    0xe7, 0xb5, 0x20, 0x09, 0x8a, 0x79, 0x89, 0x66, 0xcf, 0xca, 0xe6, 0x36, 0x09, 0xb5, 0xb4, 0x6b,
];

// One-time reference decode:
// djxl v0.12.0 lossless-4x3.jxl reference.pam
// PAM SHA-256: 52e2b86cf1683466cc78d0e1e1dfb5cf643a7e5f3ea2d0317918738afd3df18e
const DJXL_REFERENCE_RGBA8: [u8; 48] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15, 16, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
];
const DJXL_REFERENCE_F32_TOLERANCE: f32 = 1.0 / 65_535.0;

fn fixture() -> Vec<u8> {
    decode_base64(include_str!("../../tests/fixtures/lossless-4x3.jxl.b64"))
}

fn limits() -> JxlDecodeLimits {
    JxlDecodeLimits {
        max_source_bytes: 1_000_000,
        max_width: 64,
        max_height: 64,
        max_pixels: 4_096,
        max_decoded_bytes: 64 * 64 * 4 * 4,
        max_backend_alloc_bytes: 8 * 1024 * 1024,
        max_metadata_bytes: 1_000_000,
        max_boxes: 64,
        max_frames: 8,
        max_extra_channels: 8,
        max_previews: 1,
        max_name_bytes: 128,
    }
}

fn common_limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 64, 64, 4_096, 64 * 64 * 4).expect("valid limits")
}

#[test]
fn embedded_fixture_is_stable_and_bare() {
    let bytes = fixture();
    assert_eq!(bytes.len(), 206);
    assert_eq!(<[u8; 32]>::from(Sha256::digest(&bytes)), FIXTURE_SHA256);
    assert!(is_jpegxl_signature(&bytes));
    assert_eq!(&bytes[..2], &[0xff, 0x0a]);
}

#[test]
fn probe_rejects_total_source_bytes_before_backend_feed() {
    let bytes = fixture();
    let mut capped = limits();
    capped.max_source_bytes = u64::try_from(bytes.len() - 1).expect("fixture length");

    assert_eq!(
        JxlDecoder::new().probe_bytes(&bytes, capped),
        Err(JxlDecodeError::Limit {
            kind: "source bytes",
            actual: u64::try_from(bytes.len()).expect("fixture length"),
            limit: capped.max_source_bytes,
        })
    );
}

#[test]
fn probes_and_decodes_lossless_rgba_as_f32() {
    let bytes = fixture();
    let decoder = JxlDecoder::new();
    let dimensions = decoder.probe_bytes(&bytes, limits()).expect("probe");
    assert_eq!(dimensions, ImageDimensions::new(4, 3).expect("dimensions"));
    let result = decoder
        .decode_bytes(&bytes, &JxlDecodeRequest::new(limits()))
        .expect("decode");
    let pixels = result.pixels.expect("pixels");
    assert_eq!(pixels.dimensions, dimensions);
    assert_eq!(pixels.layout, ChannelLayout::Rgba);
    assert_eq!(pixels.samples.len(), 4 * 3 * 4);
    assert!(pixels.samples.iter().all(|sample| sample.is_finite()));
    assert_eq!(
        result.receipt.container.kind,
        JxlContainerKind::BareCodestream
    );
    assert_eq!(result.receipt.source_sha256, FIXTURE_SHA256);
}

#[test]
fn lossless_pixels_match_official_djxl_reference() {
    let result = JxlDecoder::new()
        .decode_bytes(&fixture(), &JxlDecodeRequest::new(limits()))
        .expect("decode");
    let pixels = result.pixels.expect("pixels");

    assert_reference_samples(&pixels.samples, &DJXL_REFERENCE_RGBA8);
}

#[test]
fn region_is_full_decode_then_checked_crop_with_truthful_receipt() {
    let region = rusttable_image::Roi::new(1, 1, 2, 2).expect("valid region");
    let result = JxlDecoder::new()
        .decode_bytes(&fixture(), &JxlDecodeRequest::new(limits()).region(region))
        .expect("region decode");
    let pixels = result.pixels.expect("cropped pixels");
    let expected = [5, 6, 7, 8, 9, 10, 11, 12, 6, 7, 8, 9, 10, 11, 12, 13];

    assert_eq!(
        pixels.dimensions,
        ImageDimensions::new(2, 2).expect("dimensions")
    );
    assert_reference_samples(&pixels.samples, &expected);
    assert_eq!(
        result.receipt.roi_behavior,
        JxlRoiBehavior::FullDecodeThenCrop {
            source: ImageDimensions::new(4, 3).expect("dimensions"),
            region,
        }
    );
    assert_eq!(result.receipt.output_bytes, 2 * 2 * 4 * 4);
}

#[test]
fn region_enforces_full_frame_decoded_byte_limit_before_crop() {
    let region = rusttable_image::Roi::new(1, 1, 1, 1).expect("valid region");
    let mut capped = limits();
    capped.max_decoded_bytes = 4 * 4;

    assert_eq!(
        JxlDecoder::new().decode_bytes(&fixture(), &JxlDecodeRequest::new(capped).region(region),),
        Err(JxlDecodeError::Limit {
            kind: "decoded bytes",
            actual: 4 * 3 * 4 * 4,
            limit: capped.max_decoded_bytes,
        })
    );
}

#[test]
fn region_applies_backend_allocation_limit_to_full_frame_render() {
    let region = rusttable_image::Roi::new(1, 1, 1, 1).expect("valid region");
    let mut capped = limits();
    capped.max_backend_alloc_bytes = 256;

    let error = JxlDecoder::new()
        .decode_bytes(&fixture(), &JxlDecodeRequest::new(capped).region(region))
        .expect_err("full-frame backend render must remain allocation-bounded");
    match error {
        JxlDecodeError::Backend(message) => {
            assert!(message.starts_with("frame render:"));
            assert!(message.contains("failed to allocate"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn decodes_equivalent_isobmff_jxlc_container() {
    let bare = fixture();
    let mut bytes = Vec::from(super::container::CONTAINER_SIGNATURE);
    append_box(&mut bytes, *b"ftyp", b"jxl \0\0\0\0jxl ");
    append_box(&mut bytes, *b"jxlc", &bare);
    assert!(is_jpegxl_signature(&bytes));
    let result = JxlDecoder::new()
        .decode_bytes(&bytes, &JxlDecodeRequest::new(limits()))
        .expect("container decode");
    assert_eq!(result.header.output_dimensions().width(), 4);
    assert_eq!(result.header.output_dimensions().height(), 3);
    assert_eq!(result.receipt.container.kind, JxlContainerKind::Isobmff);
}

#[test]
fn crate_private_adapters_match_registry_contract() {
    let bytes = fixture();
    let probe = decode_jpegxl_probe(&bytes, common_limits()).expect("registry probe");
    assert_eq!(probe.format(), InputFormat::JpegXl);
    assert_eq!(probe.dimensions().width(), 4);
    assert_eq!(probe.dimensions().height(), 3);
    let legacy = decode_legacy_rgba8(&bytes, common_limits()).expect("legacy image");
    assert_eq!(legacy.pixels().len(), 4 * 3 * 4);
    let frame = decode_jpegxl_frame(&bytes, common_limits()).expect("typed frame");
    assert_eq!(frame.sample_type(), SampleType::F32);
    assert_eq!(frame.image().descriptor().dimensions(), probe.dimensions());
    assert_eq!(frame.receipt().format(), InputFormat::JpegXl);
}

#[test]
fn jxl_adapter_retains_gray_and_lut_icc_as_profile_authoritative() {
    for (bytes, color_space, model) in [
        (
            crate::source_color::test_profiles::gray(),
            JxlColorSpace::Gray,
            rusttable_color::ProfileModel::Matrix,
        ),
        (
            crate::source_color::test_profiles::lut(),
            JxlColorSpace::Rgb,
            rusttable_color::ProfileModel::Lut,
        ),
    ] {
        let color = JxlColorEncoding::Icc(JxlIccProfile {
            color_space,
            sha256: Sha256::digest(&bytes).into(),
            bytes: bytes.clone(),
        });
        let (source, retained) =
            super::integration::source_color(&color).expect("profile-authoritative JXL color");
        let identity = source.profile().expect("external profile identity");

        assert_eq!(retained.as_deref(), Some(bytes.as_slice()));
        assert_eq!(identity.sha256(), <[u8; 32]>::from(Sha256::digest(&bytes)));
        assert_eq!(identity.model(), model);
        assert_eq!(source.transfer(), None);
        assert_eq!(source.primaries(), None);
        assert_eq!(
            source.evidence(),
            rusttable_image::SourceColorEvidence::EmbeddedIcc
        );
        assert_eq!(source.fallback_used(), None);
    }
}

#[test]
fn thumbnail_fallback_preserves_aspect_and_never_upscales() {
    let decoder = JxlDecoder::new();
    let large_bounds = decoder
        .decode_bytes(
            &fixture(),
            &JxlDecodeRequest::new(limits()).thumbnail(64, 64),
        )
        .expect("thumbnail without upscale");
    assert_eq!(
        large_bounds.pixels.expect("pixels").dimensions,
        ImageDimensions::new(4, 3).expect("dimensions")
    );
    let small_bounds = decoder
        .decode_bytes(&fixture(), &JxlDecodeRequest::new(limits()).thumbnail(2, 2))
        .expect("scaled thumbnail");
    assert_eq!(
        small_bounds.pixels.expect("pixels").dimensions,
        ImageDimensions::new(2, 2).expect("dimensions")
    );
}

#[test]
fn cancelled_request_publishes_no_decode_result() {
    let cancellation = crate::raw::RawCancellationToken::new();
    cancellation.cancel();
    let request = JxlDecodeRequest::new(limits()).with_cancellation(cancellation);
    assert_eq!(
        JxlDecoder::new().decode_bytes(&fixture(), &request),
        Err(JxlDecodeError::Cancelled)
    );
}

fn append_box(output: &mut Vec<u8>, box_type: [u8; 4], payload: &[u8]) {
    let length = u32::try_from(payload.len() + 8).expect("fixture box length");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&box_type);
    output.extend_from_slice(payload);
}

fn assert_reference_samples(actual: &[f32], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
        let expected = f32::from(expected) / 255.0;
        assert!(
            (actual - expected).abs() <= DJXL_REFERENCE_F32_TOLERANCE,
            "sample {index}: actual {actual}, djxl reference {expected}"
        );
    }
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
            _ => panic!("invalid fixture base64"),
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
