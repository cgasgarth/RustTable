use std::num::NonZeroU32;

use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational};
use rusttable_image::InputFormat;
use rusttable_metadata::{
    CanonicalExifOutput, ExifMetadataInput, MetadataInput, MetadataLimits, MetadataOutput,
    MetadataOutputLimits,
};

fn all_metadata() -> ImageMetadata {
    ImageMetadata::from_entries([
        MetadataEntry::CameraMake(MetadataText::new("Canon").unwrap()),
        MetadataEntry::CameraModel(MetadataText::new("EOS R").unwrap()),
        MetadataEntry::LensModel(MetadataText::new("RF 50mm").unwrap()),
        MetadataEntry::CaptureDateTimeOriginal(MetadataText::new("2024:01:02 03:04:05").unwrap()),
        MetadataEntry::Orientation(Orientation::RightTop),
        MetadataEntry::ExposureTime(PositiveRational::new(1, 125).unwrap()),
        MetadataEntry::FNumber(PositiveRational::new(28, 10).unwrap()),
        MetadataEntry::IsoSpeed(NonZeroU32::new(400).unwrap()),
        MetadataEntry::FocalLength(PositiveRational::new(50, 1).unwrap()),
    ])
    .unwrap()
}

#[test]
fn successful_payloads_round_trip_through_the_bounded_reader() {
    let limits = MetadataOutputLimits::new(4096, 10, 4096, 4096).unwrap();
    let encoded = CanonicalExifOutput::new(limits)
        .encode_exif(&all_metadata())
        .unwrap()
        .unwrap();
    let input =
        ExifMetadataInput::new(MetadataLimits::new(4096, 4096, 16, 16, 4, 32, 4096).unwrap());
    let decoded = input
        .read_bytes(InputFormat::Tiff, encoded.as_bytes())
        .unwrap();
    assert_eq!(decoded, all_metadata());
}
