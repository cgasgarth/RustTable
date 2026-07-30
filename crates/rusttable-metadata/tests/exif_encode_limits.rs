use std::num::NonZeroU32;

use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational};
use rusttable_metadata::{
    CanonicalExifOutput, MetadataOutput, MetadataOutputError, MetadataOutputLimits,
    MetadataOutputLimitsError,
};

fn text(value: &str) -> MetadataText {
    MetadataText::new(value).unwrap()
}

fn metadata() -> ImageMetadata {
    ImageMetadata::from_entries([
        MetadataEntry::CameraMake(text("Canon")),
        MetadataEntry::ExposureTime(PositiveRational::new(1, 125).unwrap()),
        MetadataEntry::IsoSpeed(NonZeroU32::new(400).unwrap()),
    ])
    .unwrap()
}

#[test]
fn zero_and_inconsistent_limits_are_rejected() {
    assert!(matches!(
        MetadataOutputLimits::new(0, 10, 16, 32),
        Err(MetadataOutputLimitsError::ZeroLimit { .. })
    ));
    assert!(matches!(
        MetadataOutputLimits::new(32, 10, 33, 64),
        Err(MetadataOutputLimitsError::Inconsistent { .. })
    ));
}

#[test]
fn each_bounded_resource_has_a_typed_failure() {
    let metadata = metadata();
    let baseline =
        CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap())
            .encode_exif(&metadata)
            .unwrap()
            .unwrap();
    let payload_len = baseline.len() as u64;

    let error = CanonicalExifOutput::new(
        MetadataOutputLimits::new(payload_len - 1, 10, payload_len - 1, 4096).unwrap(),
    )
    .encode_exif(&metadata)
    .unwrap_err();
    assert!(matches!(error, MetadataOutputError::PayloadLimit { .. }));

    let error = CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 2, 4096, 4096).unwrap())
        .encode_exif(&metadata)
        .unwrap_err();
    assert!(matches!(error, MetadataOutputError::IfdEntryLimit { .. }));

    let error = CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 10, 1, 4096).unwrap())
        .encode_exif(&metadata)
        .unwrap_err();
    assert!(matches!(error, MetadataOutputError::ValueLimit { .. }));

    let error = CanonicalExifOutput::new(
        MetadataOutputLimits::new(4096, 10, 4096, payload_len - 1).unwrap(),
    )
    .encode_exif(&metadata)
    .unwrap_err();
    assert!(matches!(error, MetadataOutputError::AllocationLimit { .. }));
}

#[test]
fn unsupported_text_and_rational_values_fail_closed() {
    let text_metadata =
        ImageMetadata::from_entries([MetadataEntry::CameraMake(text("Café"))]).unwrap();
    let error = CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap())
        .encode_exif(&text_metadata)
        .unwrap_err();
    assert!(matches!(
        error,
        MetadataOutputError::UnrepresentableText { .. }
    ));

    let rational = PositiveRational::new(u64::from(u32::MAX) + 1, 1).unwrap();
    let rational_metadata =
        ImageMetadata::from_entries([MetadataEntry::FocalLength(rational)]).unwrap();
    let error = CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap())
        .encode_exif(&rational_metadata)
        .unwrap_err();
    assert!(matches!(
        error,
        MetadataOutputError::UnrepresentableRational { .. }
    ));
}

#[test]
fn maximum_representable_values_and_oversized_values_are_bounded() {
    let limits = MetadataOutputLimits::new(16_384, 10, 4_096, 16_384).unwrap();
    let maximum = ImageMetadata::from_entries([
        MetadataEntry::CameraMake(MetadataText::new(&"A".repeat(4_095)).unwrap()),
        MetadataEntry::Orientation(Orientation::LeftBottom),
        MetadataEntry::ExposureTime(PositiveRational::new(u64::from(u32::MAX), 1).unwrap()),
        MetadataEntry::IsoSpeed(std::num::NonZeroU32::new(u32::MAX).unwrap()),
    ])
    .unwrap();
    assert!(
        CanonicalExifOutput::new(limits)
            .encode_exif(&maximum)
            .is_ok()
    );

    let oversized = ImageMetadata::from_entries([MetadataEntry::CameraMake(
        MetadataText::new(&"A".repeat(4_096)).unwrap(),
    )])
    .unwrap();
    let error =
        CanonicalExifOutput::new(MetadataOutputLimits::new(16_384, 10, 4_096, 16_384).unwrap())
            .encode_exif(&oversized)
            .unwrap_err();
    assert!(matches!(error, MetadataOutputError::ValueLimit { .. }));

    assert!(matches!(
        MetadataOutputLimits::new(u64::from(u32::MAX) + 1, 10, 16, u64::from(u32::MAX) + 1),
        Err(MetadataOutputLimitsError::NotRepresentable { .. })
    ));
    assert!(matches!(
        MetadataOutputLimits::new(64, u32::from(u16::MAX) + 1, 16, 64),
        Err(MetadataOutputLimitsError::NotRepresentable { .. })
    ));
}
