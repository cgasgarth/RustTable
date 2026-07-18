mod support;

use rusttable_core::{MetadataEntry, MetadataField};
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataLimits};

#[test]
fn extracts_exif_from_a_bounded_png_exif_chunk() {
    let input = ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 4, 4, 4, 32, 128).unwrap());
    let metadata = input
        .read_bytes(InputFormat::Png, &support::png_with_exif())
        .expect("PNG EXIF parses");
    assert!(
        matches!(metadata.get(MetadataField::CameraModel), Some(MetadataEntry::CameraModel(value)) if value.as_str() == "EOS R")
    );
}
