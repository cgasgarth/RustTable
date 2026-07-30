use std::num::NonZeroU32;

use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational};
use rusttable_metadata::{CanonicalExifOutput, MetadataOutput, MetadataOutputLimits};

fn text(value: &str) -> MetadataText {
    MetadataText::new(value).expect("valid metadata text")
}

fn all_metadata() -> ImageMetadata {
    ImageMetadata::from_entries([
        MetadataEntry::CameraMake(text("Canon")),
        MetadataEntry::CameraModel(text("EOS R")),
        MetadataEntry::LensModel(text("RF 50mm")),
        MetadataEntry::CaptureDateTimeOriginal(text("2024:01:02 03:04:05")),
        MetadataEntry::Orientation(Orientation::RightTop),
        MetadataEntry::ExposureTime(PositiveRational::new(1, 125).unwrap()),
        MetadataEntry::FNumber(PositiveRational::new(28, 10).unwrap()),
        MetadataEntry::IsoSpeed(NonZeroU32::new(400).unwrap()),
        MetadataEntry::FocalLength(PositiveRational::new(50, 1).unwrap()),
    ])
    .expect("unique fields")
}

fn output() -> CanonicalExifOutput {
    CanonicalExifOutput::new(MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap())
}

#[test]
fn empty_metadata_does_not_synthesize_an_exif_payload() {
    assert_eq!(output().encode_exif(&ImageMetadata::empty()).unwrap(), None);
}

#[test]
fn all_fields_are_deterministic_and_exactly_ordered() {
    let first = output().encode_exif(&all_metadata()).unwrap().unwrap();
    let second = output().encode_exif(&all_metadata()).unwrap().unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first.as_bytes(),
        &[
            0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x04, 0x00, 0x0f, 0x01, 0x02, 0x00,
            0x06, 0x00, 0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x10, 0x01, 0x02, 0x00, 0x06, 0x00,
            0x00, 0x00, 0x44, 0x00, 0x00, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x06, 0x00, 0x00, 0x00, 0x69, 0x87, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x4a, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, b'C', b'a', b'n', b'o', b'n', 0x00, b'E', b'O',
            b'S', b' ', b'R', 0x00, 0x06, 0x00, 0x9a, 0x82, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x98, 0x00, 0x00, 0x00, 0x9d, 0x82, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0xa0, 0x00,
            0x00, 0x00, 0x33, 0x88, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x90, 0x01, 0x00, 0x00,
            0x03, 0x90, 0x02, 0x00, 0x14, 0x00, 0x00, 0x00, 0xa8, 0x00, 0x00, 0x00, 0x0a, 0x92,
            0x05, 0x00, 0x01, 0x00, 0x00, 0x00, 0xbc, 0x00, 0x00, 0x00, 0x34, 0xa4, 0x02, 0x00,
            0x08, 0x00, 0x00, 0x00, 0xc4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00, 0x7d, 0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
            b'2', b'0', b'2', b'4', b':', b'0', b'1', b':', b'0', b'2', b' ', b'0', b'3', b':',
            b'0', b'4', b':', b'0', b'5', 0x00, 0x32, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            b'R', b'F', b' ', b'5', b'0', b'm', b'm', 0x00,
        ]
    );
}

#[test]
fn every_orientation_code_is_emitted_exactly() {
    for code in 1..=8 {
        let metadata = ImageMetadata::from_entries([MetadataEntry::Orientation(
            Orientation::from_u8(code).unwrap(),
        )])
        .unwrap();
        let bytes = output().encode_exif(&metadata).unwrap().unwrap();
        assert_eq!(bytes.as_bytes()[18], code);
    }
}

#[test]
fn reduced_rationals_are_preserved() {
    let metadata = ImageMetadata::from_entries([MetadataEntry::FNumber(
        PositiveRational::new(50, 100).unwrap(),
    )])
    .unwrap();
    let bytes = output().encode_exif(&metadata).unwrap().unwrap();
    assert!(
        bytes
            .as_bytes()
            .windows(8)
            .any(|window| window == [1, 0, 0, 0, 2, 0, 0, 0])
    );
}
