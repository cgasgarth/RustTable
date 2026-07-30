mod support;

use rusttable_core::{ImageMetadata, MetadataEntry, MetadataField};
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

const CONTAINER_SIGNATURE: [u8; 12] = [0, 0, 0, 12, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];

fn input(max_source_bytes: u64, max_exif_bytes: u64) -> ExifMetadataInput {
    ExifMetadataInput::new(
        MetadataLimits::new(max_source_bytes, max_exif_bytes, 16, 16, 4, 32, 128).unwrap(),
    )
}

fn box32(box_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let size = u32::try_from(payload.len() + 8).expect("fixture box fits");
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(&box_type);
    bytes.extend_from_slice(payload);
    bytes
}

fn box64(box_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = 1_u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&box_type);
    let size = u64::try_from(payload.len() + 16).expect("fixture box fits");
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn box_to_end(box_type: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = 0_u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&box_type);
    bytes.extend_from_slice(payload);
    bytes
}

fn exif_payload(prefix: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut payload = u32::try_from(prefix.len())
        .expect("fixture offset fits")
        .to_be_bytes()
        .to_vec();
    payload.extend_from_slice(prefix);
    payload.extend_from_slice(tiff);
    payload
}

fn container(boxes: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = Vec::from(CONTAINER_SIGNATURE);
    bytes.extend_from_slice(&box32(*b"ftyp", b"jxl \0\0\0\0jxl "));
    for item in boxes {
        bytes.extend_from_slice(item);
    }
    bytes
}

#[test]
fn extracts_exif_from_32_and_64_bit_boxes_with_tiff_offsets() {
    let tiff = support::tiff_with_metadata();
    let sources = [
        container(&[box32(*b"Exif", &exif_payload(&[], &tiff))]),
        container(&[box64(*b"Exif", &exif_payload(b"Exif\0\0", &tiff))]),
    ];

    for source in sources {
        let metadata = input(4096, 2048)
            .read_bytes(InputFormat::JpegXl, &source)
            .expect("JPEG XL EXIF parses");
        assert!(
            matches!(metadata.get(MetadataField::CameraModel), Some(MetadataEntry::CameraModel(value)) if value.as_str() == "EOS R")
        );
    }
}

#[test]
fn zero_sized_terminal_exif_box_extends_to_the_end_of_the_source() {
    let tiff = support::tiff_with_metadata();
    let source = container(&[box_to_end(*b"Exif", &exif_payload(&[], &tiff))]);
    let metadata = input(4096, 2048)
        .read_bytes(InputFormat::JpegXl, &source)
        .expect("terminal JPEG XL EXIF parses");

    assert!(
        matches!(metadata.get(MetadataField::CameraMake), Some(MetadataEntry::CameraMake(value)) if value.as_str() == "Canon")
    );
}

#[test]
fn bare_codestream_has_no_container_exif() {
    assert_eq!(
        input(4096, 2048)
            .read_bytes(InputFormat::JpegXl, &[0xff, 0x0a, 1, 2, 3])
            .expect("bare JPEG XL has no container metadata"),
        ImageMetadata::empty()
    );
}

#[test]
fn duplicate_exif_boxes_are_rejected() {
    let tiff = support::tiff_with_metadata();
    let payload = exif_payload(&[], &tiff);
    let source = container(&[box32(*b"Exif", &payload), box64(*b"Exif", &payload)]);

    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, &source),
        Err(MetadataInputError::DuplicateExifPayload {
            format: InputFormat::JpegXl
        })
    ));
}

#[test]
fn truncated_boxes_and_out_of_range_tiff_offsets_are_rejected() {
    let tiff = support::tiff_with_metadata();
    let mut truncated = container(&[box64(*b"Exif", &exif_payload(&[], &tiff))]);
    truncated.pop();
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, &truncated),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::JpegXl,
            ..
        })
    ));

    let source = container(&[box32(*b"Exif", &10_u32.to_be_bytes())]);
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, &source),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::JpegXl,
            ..
        })
    ));
}

#[test]
fn source_and_exif_payload_limits_are_enforced_after_the_offset() {
    let tiff = support::tiff_with_metadata();
    let source = container(&[box32(*b"Exif", &exif_payload(b"Exif\0\0", &tiff))]);
    let source_bytes = u64::try_from(source.len()).unwrap();
    let tiff_bytes = u64::try_from(tiff.len()).unwrap();

    assert!(matches!(
        input(source_bytes - 1, 2048).read_bytes(InputFormat::JpegXl, &source),
        Err(MetadataInputError::SourceTooLarge { .. })
    ));
    assert!(matches!(
        input(4096, tiff_bytes - 1).read_bytes(InputFormat::JpegXl, &source),
        Err(MetadataInputError::ExifPayloadTooLarge {
            actual,
            ..
        }) if actual == tiff_bytes
    ));
}

#[test]
fn signature_and_box_size_mismatches_are_rejected() {
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, b"not JPEG XL"),
        Err(MetadataInputError::FormatMismatch {
            format: InputFormat::JpegXl
        })
    ));

    let mut invalid_size = container(&[]);
    invalid_size.extend_from_slice(&7_u32.to_be_bytes());
    invalid_size.extend_from_slice(b"free");
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, &invalid_size),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::JpegXl,
            ..
        })
    ));

    let mut invalid_extended_size = container(&[]);
    invalid_extended_size.extend_from_slice(&1_u32.to_be_bytes());
    invalid_extended_size.extend_from_slice(b"free");
    invalid_extended_size.extend_from_slice(&15_u64.to_be_bytes());
    assert!(matches!(
        input(4096, 2048).read_bytes(InputFormat::JpegXl, &invalid_extended_size),
        Err(MetadataInputError::MalformedContainer {
            format: InputFormat::JpegXl,
            ..
        })
    ));
}
