use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_image::{
    DecodeLimits, ImageInputError, InputFormat, Orientation, UnsupportedImageFeature,
};
use rusttable_image_io::{
    JpegComponentModel, JpegDecodeError, JpegDecodeRequest, JpegDecoder, JpegPixelData,
    RawByteSource, RawDecodeLimits, RawSourceError,
};

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
    assert_eq!(count, 0);
    output
}

fn jpeg() -> Vec<u8> {
    decode_base64(include_str!("fixtures/rgb-2x1.jpg.b64"))
}

fn progressive_jpeg() -> Vec<u8> {
    decode_base64(include_str!("fixtures/progressive-2x1.jpg.b64"))
}

fn image_limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 64, 64, 4_096, 16_384).expect("valid image limits")
}

fn raw_limits() -> RawDecodeLimits {
    RawDecodeLimits::new(1_000_000, 64, 64, 4_096, 16_384).expect("valid JPEG limits")
}

fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len() + 2).expect("test segment fits");
    let mut bytes = vec![0xff, marker];
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn exif_orientation(value: u16) -> Vec<u8> {
    let mut payload = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
    payload.extend_from_slice(&0x0112_u16.to_le_bytes());
    payload.extend_from_slice(&3_u16.to_le_bytes());
    payload.extend_from_slice(&1_u32.to_le_bytes());
    payload.extend_from_slice(&value.to_le_bytes());
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload
}

fn precision12_fixture() -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xc1, &[12, 0, 1, 0, 1, 1, 1, 0x11, 0]));
    bytes.extend(segment(0xda, &[1, 1, 0, 0, 0x3f, 0]));
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

#[test]
fn header_probe_reports_dimensions_precision_components_orientation_and_metadata() {
    let mut bytes = vec![0xff, 0xd8];
    let exif = segment(0xe1, &exif_orientation(6));
    bytes.extend_from_slice(&exif);
    bytes.extend_from_slice(&jpeg()[2..]);

    let header = JpegDecoder::new()
        .probe_bytes(&bytes, image_limits())
        .expect("valid JPEG header should probe");

    assert_eq!(header.dimensions.width(), 2);
    assert_eq!(header.dimensions.height(), 1);
    assert_eq!(header.precision, 8);
    assert_eq!(header.components, JpegComponentModel::Ycbcr);
    assert_eq!(header.orientation, Orientation::Rotate90);
    assert_eq!(header.output_dimensions().width(), 1);
    assert_eq!(header.output_dimensions().height(), 2);
    assert_eq!(header.metadata.len(), 1);
    assert_eq!(header.metadata[0].data, exif[4..]);
}

#[test]
fn full_decode_returns_typed_rgb_pixels_and_a_deterministic_receipt() {
    let result = JpegDecoder::new()
        .decode_bytes(&jpeg(), &JpegDecodeRequest::new(raw_limits()))
        .expect("valid JPEG should decode");

    assert_eq!(result.header.coding_process.to_string(), "baseline");
    assert_eq!(result.header.components, JpegComponentModel::Ycbcr);
    assert_eq!(result.header.scans, 1);
    assert_eq!(result.header.restart_interval, 0);
    assert_eq!(result.receipt.backend, "jpeg-decoder-0.3.2");
    assert_eq!(result.receipt.source_bytes, jpeg().len() as u64);
    assert_eq!(result.receipt.output_bytes, 6);
    assert!(matches!(result.pixels, Some(JpegPixelData::RgbU8(values)) if values.len() == 6));
}

#[test]
fn progressive_dct_decodes_through_the_same_boundary() {
    let result = JpegDecoder::new()
        .decode_bytes(&progressive_jpeg(), &JpegDecodeRequest::new(raw_limits()))
        .expect("progressive JPEG should decode");

    assert_eq!(result.header.coding_process.to_string(), "progressive");
    assert!(result.header.scans > 1);
    assert!(matches!(result.pixels, Some(JpegPixelData::RgbU8(values)) if values.len() == 6));
}

#[test]
fn region_requests_are_rejected_before_pixel_allocation() {
    let error = JpegDecoder::new()
        .decode_bytes(
            &jpeg(),
            &JpegDecodeRequest::new(raw_limits()).region(0, 0, 1, 1),
        )
        .expect_err("JPEG ROI decode is intentionally unsupported");

    assert_eq!(error, JpegDecodeError::UnsupportedRegion);
}

#[test]
fn header_mode_does_not_decode_entropy_data() {
    let result = JpegDecoder::new()
        .decode_bytes(&jpeg(), &JpegDecodeRequest::new(raw_limits()).header())
        .expect("header-only JPEG request should succeed");

    assert!(result.pixels.is_none());
    assert_eq!(result.receipt.output_bytes, 0);
    assert_eq!(result.receipt.scans, 1);
}

#[test]
fn truncated_and_hostile_segments_are_rejected_without_fallback() {
    let mut truncated = jpeg();
    truncated.truncate(truncated.len() - 2);
    let truncated_error = JpegDecoder::new()
        .probe_bytes(&truncated, image_limits())
        .expect("header probe need not consume entropy")
        .dimensions;
    assert_eq!(truncated_error.width(), 2);
    assert!(matches!(
        JpegDecoder::new().decode_bytes(&truncated, &JpegDecodeRequest::new(raw_limits())),
        Err(JpegDecodeError::Input(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        }))
    ));

    let hostile = [0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff];
    assert!(matches!(
        JpegDecoder::new().probe_bytes(&hostile, image_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));
}

#[test]
fn unsupported_precision_is_reported_stably_after_header_validation() {
    let bytes = precision12_fixture();
    let header = JpegDecoder::new()
        .probe_bytes(&bytes, image_limits())
        .expect("12-bit sequential header should be inspectable");
    assert_eq!(header.precision, 12);
    assert_eq!(header.sof.marker(), 0xc1);
    assert!(matches!(
        JpegDecoder::new().decode_bytes(&bytes, &JpegDecodeRequest::new(raw_limits())),
        Err(JpegDecodeError::Input(
            ImageInputError::UnsupportedFeature {
                format: InputFormat::Jpeg,
                reason: UnsupportedImageFeature::BitDepth,
            }
        ))
    ));
}

#[test]
fn cancellation_and_source_mutation_publish_no_result() {
    let cancellation = rusttable_image_io::RawCancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        JpegDecoder::new().decode_bytes(
            &jpeg(),
            &JpegDecodeRequest::new(raw_limits()).with_cancellation(cancellation),
        ),
        Err(JpegDecodeError::Cancelled)
    );

    let source = ChangingSource::new(jpeg());
    assert!(matches!(
        JpegDecoder::new().decode_source(&source, &JpegDecodeRequest::new(raw_limits())),
        Err(JpegDecodeError::Source(RawSourceError::Changed))
    ));
}

struct ChangingSource {
    bytes: Vec<u8>,
    revisions: AtomicUsize,
}

impl ChangingSource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            revisions: AtomicUsize::new(0),
        }
    }
}

impl RawByteSource for ChangingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        Ok(self.bytes.len() as u64)
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        let value = self.revisions.fetch_add(1, Ordering::Relaxed);
        Ok([u8::try_from(value).unwrap_or(u8::MAX); 32])
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(RawSourceError::Read {
                offset,
                requested: buffer.len(),
            })?;
        buffer.copy_from_slice(self.bytes.get(start..end).ok_or(RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?);
        Ok(())
    }
}
