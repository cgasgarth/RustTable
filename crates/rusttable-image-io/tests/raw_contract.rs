use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_image_io::{
    RAWLER_BACKEND_ID, RawByteSource, RawCancellationToken, RawChannel, RawContainerKind,
    RawDecodeError, RawDecodeLimits, RawDecodeRequest, RawFrameValidationError, RawPlaneLayout,
    RawSourceError, RawlerRawDecoder, rawler_capability_manifest,
};
use rusttable_testkit::fixtures::deterministic_compressed_raf;

fn limits() -> RawDecodeLimits {
    RawDecodeLimits::new(1_000_000, 2_000, 2_000, 4_000_000, 8_000_000).expect("RAW limits")
}

#[test]
fn corpus_raf_maps_to_an_owned_validated_sensor_frame() {
    let fixture = deterministic_compressed_raf();
    let result = RawlerRawDecoder::new()
        .decode_bytes(fixture.bytes(), &RawDecodeRequest::new(limits()))
        .expect("RAW frame");

    assert_eq!(result.receipt.backend, RAWLER_BACKEND_ID);
    assert_eq!(result.receipt.container, RawContainerKind::Raf);
    assert_eq!(result.receipt.camera.maker, "FUJIFILM");
    assert_eq!(result.receipt.camera.model, "X-Pro2");
    assert_eq!(result.receipt.source.source_bytes, 70_000);
    assert!(result.receipt.source.stable_copy_used);
    assert_eq!(result.receipt.plane_count, 1);
    assert_eq!(result.receipt.sample_count, 768 * 6);

    let parts = result.frame.parts();
    assert_eq!(parts.planes[0].dimensions.width, 768);
    assert_eq!(parts.planes[0].dimensions.height, 6);
    assert_eq!(parts.bit_depth, 14);
    assert_eq!(parts.white_balance[0], Some(2.116));
    assert_eq!(parts.white_balance[1], Some(1.0));
    assert_eq!(parts.white_balance[2], Some(1.715));
    assert_eq!(parts.white_balance[3], None);
    match &parts.planes[0].layout {
        RawPlaneLayout::Mosaic(cfa) => {
            assert_eq!((cfa.width, cfa.height), (6, 6));
            assert_eq!(cfa.pattern.len(), 36);
            assert!(cfa.pattern.contains(&RawChannel::Red));
            assert!(cfa.pattern.contains(&RawChannel::Green));
            assert!(cfa.pattern.contains(&RawChannel::Blue));
        }
        RawPlaneLayout::Linear { .. } => panic!("fixture is an X-Trans mosaic"),
    }
    assert!(
        parts
            .color_matrices
            .iter()
            .all(|matrix| matrix.coefficients.iter().all(|value| value.is_finite()))
    );
}

#[test]
fn cancellation_prevents_source_copy_and_backend_work() {
    let fixture = deterministic_compressed_raf();
    let cancellation = RawCancellationToken::new();
    cancellation.cancel();
    let request = RawDecodeRequest::new(limits()).with_cancellation(cancellation);

    assert_eq!(
        RawlerRawDecoder::new().decode_bytes(fixture.bytes(), &request),
        Err(RawDecodeError::Cancelled)
    );
}

struct MutatingSource {
    bytes: Vec<u8>,
    revisions: AtomicUsize,
}

impl RawByteSource for MutatingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        u64::try_from(self.bytes.len()).map_err(|_| RawSourceError::LengthConversion)
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        let revision = self.revisions.fetch_add(1, Ordering::AcqRel);
        let mut value = [0; 32];
        value[0] = u8::try_from(revision).unwrap_or(u8::MAX);
        Ok(value)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::LengthConversion)?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(RawSourceError::LengthConversion)?;
        buffer.copy_from_slice(self.bytes.get(start..end).ok_or(RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?);
        Ok(())
    }
}

#[test]
fn source_mutation_discards_the_bounded_stable_copy() {
    let fixture = deterministic_compressed_raf();
    let source = MutatingSource {
        bytes: fixture.bytes().to_vec(),
        revisions: AtomicUsize::new(0),
    };
    let sentinel = std::env::temp_dir().join(format!(
        "rusttable-raw-copy-{}-must-not-exist",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&sentinel);

    assert_eq!(
        RawlerRawDecoder::new().decode_source(&source, &RawDecodeRequest::new(limits())),
        Err(RawDecodeError::Source(RawSourceError::Changed))
    );
    assert!(
        !sentinel.exists(),
        "the in-memory stable copy must not leak an artifact"
    );
}

#[test]
fn unsupported_camera_is_a_precise_capability_error() {
    let mut bytes = deterministic_compressed_raf().bytes().to_vec();
    bytes[560..567].copy_from_slice(b"NO-CAM\0");
    let error = RawlerRawDecoder::new()
        .decode_bytes(&bytes, &RawDecodeRequest::new(limits()))
        .expect_err("unknown camera");

    match error {
        RawDecodeError::Capability(capability) => {
            assert_eq!(capability.container, Some(RawContainerKind::Raf));
            assert_eq!(capability.maker, "FUJIFILM");
            assert_eq!(capability.model, "NO-CAM");
            assert!(!capability.detail.is_empty());
            assert!(capability.detail.len() <= 256);
        }
        other => panic!("expected capability error, got {other:?}"),
    }
}

#[test]
fn backend_declared_dimensions_are_bounded_before_pixel_decode() {
    let mut bytes = deterministic_compressed_raf().bytes().to_vec();
    bytes[452..456].copy_from_slice(&10_000_u32.to_be_bytes());
    let error = RawlerRawDecoder::new()
        .decode_bytes(&bytes, &RawDecodeRequest::new(limits()))
        .expect_err("excessive dimensions");
    assert!(matches!(
        error,
        RawDecodeError::InvalidFrame(RawFrameValidationError::DimensionLimit { .. })
    ));
}

#[test]
fn truncated_recognized_raw_is_terminal_and_deterministic() {
    let mut bytes = deterministic_compressed_raf().bytes().to_vec();
    bytes.truncate(1_032);
    let decoder = RawlerRawDecoder::new();
    let request = RawDecodeRequest::new(limits());
    let first = decoder
        .decode_bytes(&bytes, &request)
        .expect_err("truncated RAF");
    let second = decoder
        .decode_bytes(&bytes, &request)
        .expect_err("truncated RAF");
    assert_eq!(first, second);
    assert!(matches!(first, RawDecodeError::Malformed { .. }));
}

#[test]
fn manifest_is_deterministic_and_links_the_corpus() {
    let first = rawler_capability_manifest();
    let second = rawler_capability_manifest();
    assert!(std::ptr::eq(first, second));
    assert_eq!(first.sha256, second.sha256);
    assert!(first.entries().iter().any(|entry| {
        entry
            .corpus_fixtures
            .iter()
            .any(|fixture| fixture == "rusttable-testkit.raw.synthetic-compressed-raf")
    }));
}

#[test]
fn dependency_graph_has_only_the_pinned_raw_backend() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .expect("workspace lockfile");
    assert!(lock.contains("name = \"rawler\""));
    assert!(!lock.contains("name = \"rawloader\""));
    for forbidden in ["rawspeed", "libraw", "dcraw"] {
        assert!(!lock.to_ascii_lowercase().contains(forbidden));
    }
}
