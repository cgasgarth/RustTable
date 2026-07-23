use rusttable_core::ImageMetadata;
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataLimits};

fn input() -> ExifMetadataInput {
    ExifMetadataInput::new(MetadataLimits::new(64 * 1024, 16 * 1024, 32, 32, 4, 128, 4096).unwrap())
}

#[test]
fn metadata_input_is_object_safe_and_returns_empty_for_valid_no_exif_inputs() {
    let input: Box<dyn MetadataInput> = Box::new(input());
    let jpeg = [0xff, 0xd8, 0xff, 0xd9];
    let metadata = input
        .read_bytes(InputFormat::Jpeg, &jpeg)
        .expect("valid JPEG");
    assert_eq!(metadata, ImageMetadata::empty());
}

#[test]
fn open_exr_remains_metadata_optional() {
    assert_eq!(
        input()
            .read_bytes(InputFormat::OpenExr, b"decoder-validated OpenEXR")
            .expect("OpenEXR has no container EXIF path"),
        ImageMetadata::empty()
    );
}

#[test]
fn limits_reject_zero_caps() {
    assert!(MetadataLimits::new(0, 1, 1, 1, 1, 1, 1).is_err());
}
