mod support;

use rusttable_core::{MetadataEntry, MetadataField};
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataLimits};

#[test]
fn extracts_exif_from_a_bounded_app1_segment() {
    let input = ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 4, 4, 4, 32, 128).unwrap());
    let metadata = input
        .read_bytes(InputFormat::Jpeg, &support::jpeg_with_exif())
        .expect("JPEG EXIF parses");
    assert!(
        matches!(metadata.get(MetadataField::CameraMake), Some(MetadataEntry::CameraMake(value)) if value.as_str() == "Canon")
    );
}
