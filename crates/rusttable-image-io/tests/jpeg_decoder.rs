use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_image::{
    DecodeLimits, ImageInputError, InputFormat, Orientation, UnsupportedImageFeature,
};
use rusttable_image_io::{
    ImageDecoderRegistry, JpegComponentModel, JpegDecodeError, JpegDecodeRequest, JpegDecoder,
    JpegMetadataSegment, JpegPixelData, RawByteSource, RawDecodeLimits, RawSourceError,
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

fn cmyk_jpeg() -> Vec<u8> {
    decode_base64(include_str!("fixtures/cmyk-1x1.jpg.b64"))
}

fn ycck_jpeg() -> Vec<u8> {
    decode_base64(include_str!("fixtures/ycck-1x1.jpg.b64"))
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

fn minimal_jpeg(
    width: u16,
    height: u16,
    components: &[(u8, u8, u8)],
    metadata: &[Vec<u8>],
    restart_interval: Option<u16>,
    entropy: &[u8],
    eoi: bool,
) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    for marker in metadata {
        bytes.extend_from_slice(marker);
    }
    let mut frame = vec![
        8,
        height.to_be_bytes()[0],
        height.to_be_bytes()[1],
        width.to_be_bytes()[0],
        width.to_be_bytes()[1],
        u8::try_from(components.len()).expect("test component count fits"),
    ];
    for &(component, sampling, quantization) in components {
        let bytes: [u8; 3] = (component, sampling, quantization).into();
        frame.extend_from_slice(&bytes);
    }
    bytes.extend(segment(0xc0, &frame));
    if let Some(interval) = restart_interval {
        bytes.extend(segment(0xdd, &interval.to_be_bytes()));
    }
    let mut scan = vec![u8::try_from(components.len()).expect("test component count fits")];
    for &(component, _, _) in components {
        scan.extend_from_slice(&[component, 0]);
    }
    scan.extend_from_slice(&[0, 63, 0]);
    bytes.extend(segment(0xda, &scan));
    bytes.extend_from_slice(entropy);
    if eoi {
        bytes.extend_from_slice(&[0xff, 0xd9]);
    }
    bytes
}

fn jpeg_with_metadata(metadata: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    for marker in metadata {
        bytes.extend_from_slice(marker);
    }
    bytes.extend_from_slice(&jpeg()[2..]);
    bytes
}

fn icc_segment(sequence: u8, total: u8, profile: &[u8]) -> Vec<u8> {
    let mut payload = b"ICC_PROFILE\0".to_vec();
    payload.extend_from_slice(&[sequence, total]);
    payload.extend_from_slice(profile);
    segment(0xe2, &payload)
}

fn large_limits() -> DecodeLimits {
    DecodeLimits::new(u64::MAX, u32::MAX, u32::MAX, u64::MAX / 4, u64::MAX / 4)
        .expect("large limits are valid")
}

fn matrix_icc_profile() -> Vec<u8> {
    const TABLE_END: usize = 132 + 7 * 12;
    let xyz = [
        (*b"rXYZ", xy_xyz(0.68, 0.32)),
        (*b"gXYZ", xy_xyz(0.265, 0.69)),
        (*b"bXYZ", xy_xyz(0.15, 0.06)),
        (*b"wtpt", xy_xyz(0.3127, 0.3290)),
    ];
    let curve_offset = TABLE_END + xyz.len() * 20;
    let size = curve_offset + 14;
    let mut profile = vec![0_u8; size];
    put_u32(&mut profile, 0, u32::try_from(size).expect("small profile"));
    profile[12..16].copy_from_slice(b"mntr");
    profile[16..20].copy_from_slice(b"RGB ");
    profile[20..24].copy_from_slice(b"XYZ ");
    profile[36..40].copy_from_slice(b"acsp");
    put_u32(&mut profile, 128, 7);
    for (index, (signature, values)) in xyz.into_iter().enumerate() {
        let offset = TABLE_END + index * 20;
        put_tag(&mut profile, index, signature, offset, 20);
        profile[offset..offset + 4].copy_from_slice(b"XYZ ");
        for (component, value) in values.into_iter().enumerate() {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "JPEG ICC fixtures intentionally encode bounded positive values into the serialized i32 format."
            )]
            let fixed = (value * 65_536.0).round() as i32;
            profile[offset + 8 + component * 4..offset + 12 + component * 4]
                .copy_from_slice(&fixed.to_be_bytes());
        }
    }
    for (index, signature) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().enumerate() {
        put_tag(&mut profile, index + 4, signature, curve_offset, 14);
    }
    profile[curve_offset..curve_offset + 4].copy_from_slice(b"curv");
    put_u32(&mut profile, curve_offset + 8, 1);
    profile[curve_offset + 12..curve_offset + 14].copy_from_slice(&563_u16.to_be_bytes());
    profile
}

fn xy_xyz(x: f32, y: f32) -> [f32; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

fn put_tag(bytes: &mut [u8], index: usize, signature: [u8; 4], offset: usize, size: usize) {
    let start = 132 + index * 12;
    bytes[start..start + 4].copy_from_slice(&signature);
    put_u32(
        bytes,
        start + 4,
        u32::try_from(offset).expect("small offset"),
    );
    put_u32(bytes, start + 8, u32::try_from(size).expect("small tag"));
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn grayscale_jpeg(value: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
    encoder
        .encode(&[value], 1, 1, image::ExtendedColorType::L8)
        .expect("grayscale fixture should encode");
    bytes
}

fn exif_orientation(value: u16) -> Vec<u8> {
    let mut payload = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
    payload.extend_from_slice(&0x0112_u16.to_le_bytes());
    payload.extend_from_slice(&3_u16.to_le_bytes());
    payload.extend_from_slice(&1_u32.to_le_bytes());
    payload.extend_from_slice(&value.to_le_bytes());
    payload.extend_from_slice(&[0, 0]);
    payload.extend_from_slice(&0_u32.to_le_bytes());
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
    assert_eq!(header.metadata[0].marker, 0xe1);
    assert_eq!(header.metadata[0].data, exif[4..]);
}

#[test]
fn all_exif_orientations_remain_typed_source_properties_on_non_square_jpegs() {
    let orientations = [
        Orientation::Normal,
        Orientation::FlipHorizontal,
        Orientation::Rotate180,
        Orientation::FlipVertical,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ];
    for (index, expected) in orientations.into_iter().enumerate() {
        let code = u16::try_from(index + 1).expect("EXIF code");
        let mut bytes = vec![0xff, 0xd8];
        bytes.extend(segment(0xe1, &exif_orientation(code)));
        bytes.extend_from_slice(&jpeg()[2..]);

        let frame = ImageDecoderRegistry::standard()
            .decode_frame_bytes(&bytes, image_limits())
            .expect("oriented JPEG frame");
        let descriptor = frame.image().descriptor();

        assert_eq!(descriptor.dimensions().width(), 2, "orientation {code}");
        assert_eq!(descriptor.dimensions().height(), 1, "orientation {code}");
        assert_eq!(descriptor.orientation(), expected, "orientation {code}");
        let expected_dimensions = if code >= 5 { (1, 2) } else { (2, 1) };
        let output_dimensions = descriptor
            .orientation()
            .output_dimensions(descriptor.dimensions());
        assert_eq!(
            (output_dimensions.width(), output_dimensions.height()),
            expected_dimensions,
            "orientation {code}"
        );
        assert_eq!(frame.rgba_f32_pixels().unwrap().len(), 2);
    }
}

#[test]
fn full_decode_returns_typed_rgb_pixels_and_a_deterministic_receipt() {
    let result = JpegDecoder::new()
        .decode_bytes(&jpeg(), &JpegDecodeRequest::new(raw_limits()))
        .expect("valid JPEG should decode");

    assert_eq!(result.header.coding_process.to_string(), "baseline");
    assert_eq!(result.header.components, JpegComponentModel::Ycbcr);
    assert_eq!(
        result
            .header
            .sampling
            .iter()
            .map(|sample| (
                sample.component_id,
                sample.horizontal,
                sample.vertical,
                sample.quantization_table,
            ))
            .collect::<Vec<_>>(),
        vec![(1, 2, 2, 0), (2, 1, 1, 1), (3, 1, 1, 1)]
    );
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
    const fn new(bytes: Vec<u8>) -> Self {
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

#[test]
fn native_metadata_inventory_retains_only_ordered_app1_and_app2_markers() {
    let markers = [
        segment(0xe0, b"jfif-opaque"),
        segment(0xe1, b"exif-payload"),
        segment(0xe2, b"icc-payload"),
        segment(0xe3, b"maker-private"),
        segment(0xee, b"adobe-private"),
        segment(0xef, b"application-private"),
        segment(0xfe, b"comment"),
    ];
    let mut bytes = vec![0xff, 0xd8];
    let mut expected = Vec::new();
    for marker in &markers {
        let offset = bytes.len() as u64;
        let payload_length = marker.len() - 4;
        bytes.extend_from_slice(marker);
        if matches!(marker[1], 0xe1 | 0xe2) {
            expected.push((marker[1], offset, payload_length, marker[4..].to_vec()));
        }
    }
    bytes.extend_from_slice(
        &minimal_jpeg(
            2,
            1,
            &[(1, 0x22, 0), (2, 0x11, 1), (3, 0x11, 1)],
            &[],
            None,
            &[],
            true,
        )[2..],
    );

    let header = JpegDecoder::new()
        .inspect_bytes(&bytes, image_limits())
        .expect("opaque metadata should not affect JPEG structure");
    let actual = header
        .metadata
        .iter()
        .map(|item| {
            (
                item.marker,
                item.offset,
                item.byte_length as usize,
                item.data.clone(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn libjpeg_marker_scan_discards_non_ff_and_stuffed_gap_bytes() {
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend_from_slice(&segment(0xe1, b"exif"));
    bytes.extend_from_slice(&[0x01, 0x02, 0xff, 0x00, 0x03]);
    bytes.extend_from_slice(&segment(0xe2, b"icc"));
    bytes.extend_from_slice(
        &minimal_jpeg(
            2,
            1,
            &[(1, 0x22, 0), (2, 0x11, 1), (3, 0x11, 1)],
            &[],
            None,
            &[],
            true,
        )[2..],
    );

    let header = ImageDecoderRegistry::standard()
        .probe_bytes(&bytes, image_limits())
        .expect("libjpeg accepts discarded bytes between marker segments");
    assert_eq!(header.dimensions().width(), 2);
    assert_eq!(header.dimensions().height(), 1);
}

#[test]
fn malformed_exif_offsets_are_ignored_without_inventing_orientation() {
    let malformed = segment(0xe1, b"Exif\0\0II*\0\xff\xff\xff\xff");
    let bytes = jpeg_with_metadata(&[malformed]);
    let header = JpegDecoder::new()
        .probe_bytes(&bytes, image_limits())
        .expect("malformed Exif remains opaque metadata");

    assert_eq!(header.orientation, Orientation::Normal);
    assert!(header.metadata.iter().any(JpegMetadataSegment::is_exif));
}

#[test]
fn maximum_jpeg_dimensions_and_zero_dimensions_follow_native_bounds() {
    let maximum = minimal_jpeg(u16::MAX, u16::MAX, &[(1, 0x11, 0)], &[], None, &[], true);
    let header = JpegDecoder::new()
        .probe_bytes(&maximum, large_limits())
        .expect("JPEG dimensions are unsigned 16-bit values");
    assert_eq!(header.dimensions.width(), u32::from(u16::MAX));
    assert_eq!(header.dimensions.height(), u32::from(u16::MAX));

    let zero = minimal_jpeg(0, 1, &[(1, 0x11, 0)], &[], None, &[], true);
    assert!(matches!(
        JpegDecoder::new().probe_bytes(&zero, image_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));
}

#[test]
fn grayscale_decode_replicates_luminance_into_opaque_rgb() {
    let value = 73;
    let result = JpegDecoder::new()
        .decode_rgba8(&grayscale_jpeg(value), image_limits())
        .expect("grayscale JPEG should decode");

    assert_eq!(result.0.width(), 1);
    assert_eq!(result.0.height(), 1);
    assert_eq!(result.2.len(), 4);
    assert_eq!(result.2[3], 255);
    assert_eq!(result.2[..3], [value, value, value]);
}

#[test]
fn native_rgb_output_rejects_cmyk_and_ycck_without_publishing_raw_planes() {
    for (bytes, model) in [
        (cmyk_jpeg(), JpegComponentModel::Cmyk),
        (ycck_jpeg(), JpegComponentModel::Ycck),
    ] {
        let header = JpegDecoder::new()
            .inspect_bytes(&bytes, image_limits())
            .expect("native header read recognizes four-component JPEG");
        assert_eq!(header.components, model);
        assert!(matches!(
            JpegDecoder::new().decode_bytes(&bytes, &JpegDecodeRequest::new(raw_limits())),
            Err(JpegDecodeError::Input(
                ImageInputError::UnsupportedFeature {
                    format: InputFormat::Jpeg,
                    reason: UnsupportedImageFeature::ColorModel,
                }
            ))
        ));
    }
}

#[test]
fn four_four_four_and_four_two_zero_sampling_are_retained() {
    for (sampling, expected) in [(0x11, (1, 1)), (0x22, (2, 2))] {
        let bytes = minimal_jpeg(
            2,
            1,
            &[(1, sampling, 0), (2, 0x11, 1), (3, 0x11, 1)],
            &[],
            None,
            &[],
            true,
        );
        let header = JpegDecoder::new()
            .probe_bytes(&bytes, image_limits())
            .expect("sampling header should probe");
        assert_eq!(
            (header.sampling[0].horizontal, header.sampling[0].vertical),
            expected
        );
    }
}

#[test]
fn restart_markers_require_dri_and_stuffed_entropy_is_accepted() {
    let stuffed = minimal_jpeg(
        2,
        1,
        &[(1, 0x11, 0)],
        &[],
        None,
        &[0x12, 0xff, 0x00, 0x34],
        true,
    );
    let header = JpegDecoder::new()
        .inspect_bytes(&stuffed, image_limits())
        .expect("stuffed entropy byte should not be a marker");
    assert_eq!(header.restart_interval, 0);

    let restart = minimal_jpeg(
        2,
        1,
        &[(1, 0x11, 0)],
        &[],
        Some(1),
        &[0x12, 0xff, 0xd0, 0x34],
        true,
    );
    let header = JpegDecoder::new()
        .inspect_bytes(&restart, image_limits())
        .expect("declared restart marker should be accepted");
    assert_eq!(header.restart_interval, 1);

    let undeclared = minimal_jpeg(
        2,
        1,
        &[(1, 0x11, 0)],
        &[],
        None,
        &[0x12, 0xff, 0xd0, 0x34],
        true,
    );
    assert!(matches!(
        JpegDecoder::new().inspect_bytes(&undeclared, image_limits()),
        Err(ImageInputError::UnsupportedFeature {
            format: InputFormat::Jpeg,
            reason: UnsupportedImageFeature::RestartInterval,
        })
    ));
}

#[test]
fn complete_inspection_requires_eoi_but_ignores_trailing_data() {
    let missing = minimal_jpeg(2, 1, &[(1, 0x11, 0)], &[], None, &[], false);
    assert!(matches!(
        JpegDecoder::new().inspect_bytes(&missing, image_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));

    let mut trailing = minimal_jpeg(2, 1, &[(1, 0x11, 0)], &[], None, &[], true);
    trailing.extend_from_slice(&segment(0xfe, b"trailing"));
    assert!(
        JpegDecoder::new()
            .inspect_bytes(&trailing, image_limits())
            .is_ok()
    );
}

#[test]
fn soi_only_and_missing_ff_marker_are_not_valid_jpeg_headers() {
    for bytes in [vec![0xff, 0xd8], vec![0xff, 0xd8, 0x00]] {
        assert!(matches!(
            JpegDecoder::new().probe_bytes(&bytes, image_limits()),
            Err(ImageInputError::MalformedInput {
                format: InputFormat::Jpeg,
                ..
            })
        ));
    }
}

#[test]
fn icc_parts_reassemble_in_any_order_and_accept_all_255_sequences() {
    let profile = matrix_icc_profile();
    let mut parts = profile
        .chunks(37)
        .enumerate()
        .map(|(index, part)| {
            (
                u8::try_from(index + 1).expect("test ICC sequence fits"),
                part,
            )
        })
        .collect::<Vec<_>>();
    let total = u8::try_from(parts.len()).expect("test ICC part count fits");
    parts.reverse();
    let markers = parts
        .iter()
        .map(|(sequence, part)| icc_segment(*sequence, total, part))
        .collect::<Vec<_>>();
    let result = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&jpeg_with_metadata(&markers), image_limits())
        .expect("reordered ICC parts should reassemble");
    assert_eq!(
        result.embedded_icc().expect("embedded ICC"),
        profile.as_slice()
    );

    let markers = (1..=255_u16)
        .map(|sequence| {
            icc_segment(
                u8::try_from(sequence).expect("255 ICC sequences fit"),
                u8::MAX,
                if sequence == 255 { &profile } else { &[] },
            )
        })
        .collect::<Vec<_>>();
    let result = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&jpeg_with_metadata(&markers), image_limits())
        .expect("all 255 ICC sequences should reassemble");
    assert_eq!(
        result.embedded_icc().expect("embedded ICC"),
        profile.as_slice()
    );
}

#[test]
fn malformed_icc_sequences_fail_closed() {
    let profile = matrix_icc_profile();
    let cases = [
        vec![icc_segment(1, 2, &profile)],
        vec![icc_segment(1, 1, &profile), icc_segment(1, 1, &[])],
        vec![icc_segment(1, 1, &profile), icc_segment(2, 2, &[])],
        vec![icc_segment(1, 1, &[])],
        vec![icc_segment(0, 1, &profile)],
        vec![icc_segment(2, 1, &profile)],
    ];
    for markers in cases {
        assert!(matches!(
            ImageDecoderRegistry::standard()
                .decode_frame_bytes(&jpeg_with_metadata(&markers), image_limits()),
            Err(ImageInputError::MalformedInput {
                format: InputFormat::Jpeg,
                ..
            })
        ));
    }
}

#[test]
fn metadata_inventory_enforces_item_and_byte_bounds() {
    let mut accepted_markers = (1..=255_u16)
        .map(|sequence| {
            icc_segment(
                u8::try_from(sequence).expect("255 ICC sequences fit"),
                u8::MAX,
                &[],
            )
        })
        .collect::<Vec<_>>();
    accepted_markers.push(segment(0xe1, b"exif"));
    let accepted = minimal_jpeg(2, 1, &[(1, 0x11, 0)], &accepted_markers, None, &[], true);
    assert_eq!(
        JpegDecoder::new()
            .inspect_bytes(&accepted, image_limits())
            .expect("metadata item limit is inclusive")
            .metadata
            .len(),
        256
    );

    let mut too_many_markers = accepted_markers;
    too_many_markers.push(segment(0xe1, b"overflow"));
    let rejected = minimal_jpeg(2, 1, &[(1, 0x11, 0)], &too_many_markers, None, &[], true);
    assert!(matches!(
        JpegDecoder::new().inspect_bytes(&rejected, image_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));

    let mut exact_bytes = vec![0xff, 0xd8];
    for _ in 0..128 {
        exact_bytes.extend(segment(0xe1, &vec![0x01; 65_533]));
    }
    exact_bytes.extend(segment(0xe2, &vec![0x02; 384]));
    let jpeg_tail = minimal_jpeg(2, 1, &[(1, 0x11, 0)], &[], None, &[], true);
    exact_bytes.extend_from_slice(&jpeg_tail[2..]);
    assert!(
        JpegDecoder::new()
            .inspect_bytes(&exact_bytes, image_limits())
            .is_ok()
    );

    let frame_offset = exact_bytes.len() - (jpeg_tail.len() - 2);
    let mut rejected = exact_bytes[..frame_offset].to_vec();
    rejected.extend(segment(0xe2, b"overflow"));
    rejected.extend_from_slice(&exact_bytes[frame_offset..]);
    assert!(matches!(
        JpegDecoder::new().inspect_bytes(&rejected, image_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));
}
