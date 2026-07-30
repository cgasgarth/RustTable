mod fixtures;

use std::sync::atomic::{AtomicU8, Ordering};

use rusttable_image::{
    AlphaMode, DecodeLimits, ImageInputError, InputFormat, UnsupportedImageFeature,
};
use sha2::Digest as _;

use super::{
    WebPCodingMode, WebPContainer, WebPDecodeError, WebPDecodeLimits, WebPDecodeRequest,
    WebPDecoder, WebPPixelData, decode_legacy_rgba8, decode_webp_frame, decode_webp_probe,
    is_webp_signature,
};
use crate::raw::{RawByteSource, RawCancellationToken, RawSourceError};

#[test]
fn decodes_simple_lossy_fixture_with_transfer_tolerance() {
    let result = decode(fixtures::LOSSY_RED_3X3);
    assert_eq!(result.header.container, WebPContainer::Simple);
    assert_eq!(result.header.coding, WebPCodingMode::LossyVp8);
    assert!(!result.header.features.alpha);
    let WebPPixelData::RgbU8 {
        dimensions,
        samples,
    } = result.pixels.expect("pixels")
    else {
        panic!("lossy fixture must decode as RGB");
    };
    assert_eq!((dimensions.width(), dimensions.height()), (3, 3));
    for pixel in samples.as_chunks::<3>().0 {
        assert_transfer_close(pixel, &[255, 0, 0], 3);
    }
}

#[test]
fn lossless_rgb_and_thumbnail_preserve_odd_width_pixels() {
    let (source, expected) = fixtures::lossless_rgb();
    let request = WebPDecodeRequest::new(WebPDecodeLimits::standard()).thumbnail();
    let result = WebPDecoder::new()
        .decode_bytes(&source, &request)
        .expect("thumbnail decode");
    assert_eq!(result.receipt.mode, super::WebPDecodeMode::Thumbnail);
    assert_eq!(result.receipt.output_bytes, expected.len() as u64);
    assert_eq!(
        result.pixels.expect("pixels").samples(),
        expected.as_slice()
    );
}

#[test]
fn lossless_rgba_is_straight_and_exact() {
    let (source, expected) = fixtures::lossless_rgba();
    let result = decode(&source);
    let pixels = result.pixels.expect("pixels");
    assert_eq!(pixels.samples(), expected);
    assert_eq!(pixels.format().alpha(), AlphaMode::Straight);
}

#[test]
fn independent_cwebp_fixture_is_rgba_u8_4x3() {
    let result = decode(fixtures::CWEBP_RGBA_4X3);
    assert_eq!(result.header.coding, WebPCodingMode::LosslessVp8l);
    assert_eq!(
        (
            result.header.dimensions.width(),
            result.header.dimensions.height()
        ),
        (4, 3)
    );
    assert!(result.header.features.alpha);
    let pixels = result.pixels.expect("cwebp pixels");
    assert!(matches!(pixels, WebPPixelData::RgbaU8 { .. }));
    assert_eq!(pixels.samples(), fixtures::CWEBP_RGBA_4X3_DWEBP_PIXELS);
    assert_eq!(pixels.format().alpha(), AlphaMode::Straight);
    assert_eq!(&pixels.samples()[4..8], &[4, 5, 6, 7]);
    assert!(pixels.samples().as_chunks::<4>().0.iter().any(|rgba| {
        rgba[3] != 0 && rgba[3] != 255 && rgba[..3].iter().any(|channel| *channel != 0)
    }));
}

#[test]
fn extended_lossy_alpha_is_straight_and_unmodified() {
    let alpha = [0, 1, 32, 64, 96, 128, 192, 254, 255];
    let source = fixtures::lossy_alpha(&alpha);
    let result = decode(&source);
    assert_eq!(result.header.coding, WebPCodingMode::LossyVp8);
    assert!(result.header.features.alpha);
    let WebPPixelData::RgbaU8 { samples, .. } = result.pixels.expect("pixels") else {
        panic!("ALPH fixture must decode as RGBA");
    };
    assert_eq!(
        samples
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>(),
        alpha
    );
}

#[test]
fn inventories_metadata_offsets_lengths_and_hashes() {
    let (source, icc, exif, xmp) = fixtures::extended_metadata();
    let header = WebPDecoder::new()
        .inspect_bytes(&source, WebPDecodeLimits::standard())
        .expect("metadata inventory");
    assert_eq!(header.container, WebPContainer::Extended);
    assert_eq!(header.metadata.count(), 3);
    assert_eq!(
        extract(&source, header.metadata.icc_profile.expect("ICC").location),
        icc
    );
    assert_eq!(
        extract(&source, header.metadata.exif.expect("EXIF").location),
        exif
    );
    assert_eq!(
        extract(&source, header.metadata.xmp.expect("XMP").location),
        xmp
    );
}

#[test]
fn exif_xmp_and_unknown_chunks_accept_every_valid_placement() {
    const PERMUTATIONS: [[[u8; 4]; 3]; 6] = [
        [*b"EXIF", *b"XMP ", *b"VP8L"],
        [*b"EXIF", *b"VP8L", *b"XMP "],
        [*b"XMP ", *b"EXIF", *b"VP8L"],
        [*b"XMP ", *b"VP8L", *b"EXIF"],
        [*b"VP8L", *b"EXIF", *b"XMP "],
        [*b"VP8L", *b"XMP ", *b"EXIF"],
    ];
    for order in PERMUTATIONS {
        let source = fixtures::metadata_permutation(&order);
        let result = decode(&source);
        assert!(result.header.metadata.exif.is_some(), "{order:?}");
        assert!(result.header.metadata.xmp.is_some(), "{order:?}");
    }

    let with_unknown = fixtures::metadata_permutation(&[*b"XMP ", *b"JUNK", *b"VP8L", *b"EXIF"]);
    let result = decode(&with_unknown);
    assert_eq!(result.header.chunks.chunks.len(), 5);
    assert!(result.header.metadata.exif.is_some());
    assert!(result.header.metadata.xmp.is_some());
}

#[test]
fn metadata_freedom_does_not_relax_reconstruction_chunk_order() {
    assert_malformed(&fixtures::icc_after_image(), "ICCP chunk is out of order");
    assert_malformed(
        &fixtures::metadata_between_alpha_and_image(),
        "ALPH must immediately precede lossy VP8 data",
    );
}

#[test]
fn typed_frame_retains_profile_authoritative_lut_icc_bytes() {
    let icc = crate::source_color::test_profiles::lut();
    let source = fixtures::extended_with_icc(&icc);
    let frame = decode_webp_frame(&source, common_limits()).expect("typed WebP frame");
    let profile = frame
        .source_color()
        .profile()
        .expect("external ICC identity");

    assert_eq!(frame.embedded_icc(), Some(icc.as_slice()));
    assert_eq!(
        profile.sha256(),
        <[u8; 32]>::from(sha2::Sha256::digest(&icc))
    );
    assert_eq!(profile.model(), rusttable_color::ProfileModel::Lut);
    assert_eq!(
        frame.source_color().evidence(),
        rusttable_image::SourceColorEvidence::EmbeddedIcc
    );
    assert_eq!(frame.source_color().fallback_used(), None);
}

#[test]
fn header_mode_has_receipt_without_output_allocation() {
    let (source, _) = fixtures::lossless_rgb();
    let request = WebPDecodeRequest::new(WebPDecodeLimits::standard()).header();
    let result = WebPDecoder::new()
        .decode_bytes(&source, &request)
        .expect("header decode");
    assert!(result.pixels.is_none());
    assert_eq!(result.receipt.output_bytes, 0);
    assert_eq!(result.receipt.riff_declared_bytes, source.len() as u64);
    assert_eq!(result.receipt.backend, super::WEBP_BACKEND_ID);
}

#[test]
fn animation_is_typed_before_backend_output_allocation() {
    let error = WebPDecoder::new()
        .decode_bytes(
            &fixtures::animation(),
            &WebPDecodeRequest::new(WebPDecodeLimits::standard()),
        )
        .expect_err("animation must be rejected");
    assert_eq!(error, WebPDecodeError::UnsupportedAnimation);
}

#[test]
fn malformed_riff_sizes_padding_and_feature_graphs_are_rejected() {
    let (mut trailing, _) = fixtures::lossless_rgb();
    trailing.push(0);
    assert_malformed(&trailing, "trailing bytes");

    let (mut truncated, _) = fixtures::lossless_rgb();
    truncated.pop();
    assert_malformed(&truncated, "payload is truncated");

    let mut bad_padding = fixtures::odd_unknown_padding();
    let pad = bad_padding
        .windows(4)
        .position(|window| window == b"VP8L")
        .expect("VP8L follows odd unknown")
        - 1;
    bad_padding[pad] = 1;
    assert_malformed(&bad_padding, "padding byte");

    let mut bad_flags = fixtures::lossy_alpha(&[255; 9]);
    let vp8x_flags = bad_flags
        .windows(4)
        .position(|window| window == b"VP8X")
        .expect("VP8X")
        + 8;
    bad_flags[vp8x_flags] = 0;
    assert_malformed(&bad_flags, "feature flags");
}

#[test]
fn every_container_truncation_fails_without_reaching_decode() {
    let (source, _) = fixtures::lossless_rgba();
    for end in 12..source.len() {
        let error = WebPDecoder::new()
            .probe_bytes(&source[..end], WebPDecodeLimits::standard())
            .expect_err("every strict prefix must be rejected");
        assert!(matches!(error, WebPDecodeError::Malformed(_)));
    }
}

#[test]
fn limits_reject_dimension_bombs_before_decode() {
    let (source, _) = fixtures::lossless_rgb();
    let mut limits = WebPDecodeLimits::standard();
    limits.max_width = 4;
    let error = WebPDecoder::new()
        .probe_bytes(&source, limits)
        .expect_err("width must be bounded");
    assert_eq!(
        error,
        WebPDecodeError::Limit {
            kind: "width",
            actual: 5,
            limit: 4,
        }
    );
}

#[test]
fn cancellation_and_source_mutation_are_typed() {
    let (source, _) = fixtures::lossless_rgb();
    let cancellation = RawCancellationToken::new();
    cancellation.cancel();
    let cancelled = WebPDecoder::new()
        .decode_bytes(
            &source,
            &WebPDecodeRequest::new(WebPDecodeLimits::standard()).with_cancellation(cancellation),
        )
        .expect_err("cancelled request");
    assert_eq!(cancelled, WebPDecodeError::Cancelled);

    let changed = WebPDecoder::new()
        .decode_source(
            &ChangingSource::new(source),
            &WebPDecodeRequest::new(WebPDecodeLimits::standard()),
        )
        .expect_err("changed source");
    assert_eq!(changed, WebPDecodeError::Source(RawSourceError::Changed));
}

#[test]
fn registry_adapters_preserve_common_contracts() {
    let (source, expected) = fixtures::lossless_rgba();
    let limits = common_limits();
    let probe = decode_webp_probe(&source, limits).expect("probe adapter");
    assert_eq!(probe.format(), InputFormat::Webp);
    let image = decode_legacy_rgba8(&source, limits).expect("legacy adapter");
    assert_eq!(image.pixels(), expected);
    let frame = decode_webp_frame(&source, limits).expect("typed frame adapter");
    assert_eq!(frame.receipt().format(), InputFormat::Webp);
    assert_eq!(
        frame.image().descriptor().format().alpha(),
        AlphaMode::Straight
    );

    let animation = decode_webp_probe(&fixtures::animation(), limits)
        .expect_err("adapter must preserve unsupported animation");
    assert_eq!(
        animation,
        ImageInputError::UnsupportedFeature {
            format: InputFormat::Webp,
            reason: UnsupportedImageFeature::Animation,
        }
    );
}

#[test]
fn signature_requires_both_riff_and_webp() {
    assert!(is_webp_signature(fixtures::LOSSY_RED_3X3));
    assert!(!is_webp_signature(b"RIFF\0\0\0\0NOPE"));
    assert!(!is_webp_signature(b"WEBP"));
}

fn decode(source: &[u8]) -> super::WebPDecodeResult {
    WebPDecoder::new()
        .decode_bytes(
            source,
            &WebPDecodeRequest::new(WebPDecodeLimits::standard()),
        )
        .expect("fixture decode")
}

fn common_limits() -> DecodeLimits {
    DecodeLimits::new(1024 * 1024, 1024, 1024, 1024 * 1024, 4 * 1024 * 1024).expect("test limits")
}

fn assert_malformed(source: &[u8], message: &str) {
    let error = WebPDecoder::new()
        .probe_bytes(source, WebPDecodeLimits::standard())
        .expect_err("fixture must be malformed");
    assert!(
        matches!(error, WebPDecodeError::Malformed(ref actual) if actual.contains(message)),
        "unexpected error: {error}"
    );
}

fn assert_transfer_close(actual: &[u8], expected: &[u8], tolerance: u8) {
    assert_eq!(actual.len(), expected.len());
    for (&actual, &expected) in actual.iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "{actual} != {expected}"
        );
    }
}

fn extract(source: &[u8], location: super::WebPDataLocation) -> &[u8] {
    let start = usize::try_from(location.offset).expect("fixture offset");
    let length = usize::try_from(location.length).expect("fixture length");
    &source[start..start + length]
}

struct ChangingSource {
    bytes: Vec<u8>,
    revisions: AtomicU8,
}

impl ChangingSource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            revisions: AtomicU8::new(0),
        }
    }
}

impl RawByteSource for ChangingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        Ok(self.bytes.len() as u64)
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        let value = self.revisions.fetch_add(1, Ordering::AcqRel);
        Ok([value; 32])
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        let end = start + buffer.len();
        buffer.copy_from_slice(&self.bytes[start..end]);
        Ok(())
    }
}
