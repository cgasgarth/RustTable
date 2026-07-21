mod support;

use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

fn input() -> ExifMetadataInput {
    ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 16, 16, 4, 32, 128).unwrap())
}

#[test]
fn duplicate_jpeg_payloads_are_rejected() {
    let payload = support::jpeg_with_exif();
    let mut duplicate = payload[..payload.len() - 2].to_vec();
    duplicate.extend_from_slice(&payload[2..]);
    assert!(matches!(
        input().read_bytes(InputFormat::Jpeg, &duplicate),
        Err(MetadataInputError::DuplicateExifPayload { .. })
    ));
}

#[test]
fn malformed_png_chunk_is_rejected() {
    let mut source = b"\x89PNG\r\n\x1a\n".to_vec();
    source.extend_from_slice(&[0, 0, 0, 20, b'e', b'X', b'I', b'f']);
    assert!(matches!(
        input().read_bytes(InputFormat::Png, &source),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::Png,
            ..
        })
    ));
}

#[test]
fn invalid_orientation_is_rejected_as_a_typed_field_error() {
    let mut source = support::tiff_with_metadata();
    source[42] = 9;
    assert!(matches!(
        input().read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::InvalidField {
            field: "orientation"
        })
    ));
}

#[test]
fn raw_container_without_embedded_tiff_metadata_is_accepted_as_empty() {
    assert_eq!(
        input()
            .read_bytes(InputFormat::Raw, b"FUJIFILMCCD-RAW synthetic")
            .expect("RAW metadata is optional"),
        rusttable_core::ImageMetadata::empty()
    );
}
