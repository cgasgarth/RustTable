mod support;

use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

#[test]
fn source_and_payload_caps_are_checked_before_exif_parsing() {
    let source = support::tiff_with_metadata();
    let input = ExifMetadataInput::new(MetadataLimits::new(8, 2048, 4, 4, 4, 32, 128).unwrap());
    assert!(matches!(
        input.read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::SourceTooLarge { .. })
    ));

    let input = ExifMetadataInput::new(MetadataLimits::new(4096, 8, 4, 4, 4, 32, 128).unwrap());
    assert!(matches!(
        input.read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::ExifPayloadTooLarge { .. })
    ));
}

#[test]
fn jpeg_segment_cap_is_enforced() {
    let source = vec![0xff, 0xd8, 0xff, 0xe0, 0, 2, 0xff, 0xe0, 0, 2, 0xff, 0xd9];
    let input = ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 1, 4, 4, 32, 128).unwrap());
    assert!(matches!(
        input.read_bytes(InputFormat::Jpeg, &source),
        Err(MetadataInputError::JpegSegmentLimit { limit: 1 })
    ));
}

#[test]
fn value_cap_is_enforced_during_tiff_preflight() {
    let source = support::tiff_with_metadata();
    let input = ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 4, 4, 4, 32, 7).unwrap());
    assert!(matches!(
        input.read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::ValueTooLarge { limit: 7, .. })
    ));
}
