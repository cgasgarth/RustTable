use std::num::NonZeroU32;
use std::path::PathBuf;

use rusttable_image::{DecodedImage, ImageDimensions, OutputLimits};
use rusttable_metadata::{
    ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational,
};

pub fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).expect("dimensions"),
        vec![255, 0, 0, 255, 0, 255, 0, 255],
    )
    .expect("image")
}

pub fn metadata() -> ImageMetadata {
    ImageMetadata::from_entries([
        MetadataEntry::CameraMake(text("Canon")),
        MetadataEntry::CameraModel(text("EOS R")),
        MetadataEntry::LensModel(text("RF 50mm")),
        MetadataEntry::CaptureDateTimeOriginal(text("2024:01:02 03:04:05")),
        MetadataEntry::Orientation(Orientation::RightTop),
        MetadataEntry::ExposureTime(PositiveRational::new(1, 125).expect("rational")),
        MetadataEntry::FNumber(PositiveRational::new(28, 10).expect("rational")),
        MetadataEntry::IsoSpeed(NonZeroU32::new(400).expect("ISO")),
        MetadataEntry::FocalLength(PositiveRational::new(50, 1).expect("rational")),
    ])
    .expect("unique metadata fields")
}

pub fn text(value: &str) -> MetadataText {
    MetadataText::new(value).expect("metadata text")
}

pub fn metadata_limits() -> rusttable_metadata::MetadataOutputLimits {
    rusttable_metadata::MetadataOutputLimits::new(16_384, 10, 4_096, 16_384)
        .expect("metadata limits")
}

pub fn output_limits() -> OutputLimits {
    OutputLimits::new(1_000_000).expect("output limit")
}

pub fn destination(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-metadata-output-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}
