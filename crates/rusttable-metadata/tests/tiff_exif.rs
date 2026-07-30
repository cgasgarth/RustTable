use rusttable_core::{ImageMetadata, MetadataEntry, MetadataField, Orientation};
use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

fn input() -> ExifMetadataInput {
    ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 16, 16, 4, 32, 128).unwrap())
}

fn u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn entry(bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32) {
    u16(bytes, tag);
    u16(bytes, kind);
    u32(bytes, count);
    u32(bytes, value);
}

fn tiff_with_metadata() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(220);
    bytes.extend_from_slice(b"II*\0");
    u32(&mut bytes, 8);
    u16(&mut bytes, 4);
    entry(&mut bytes, 0x010f, 2, 6, 62);
    entry(&mut bytes, 0x0110, 2, 6, 68);
    entry(&mut bytes, 0x0112, 3, 1, 1);
    entry(&mut bytes, 0x8769, 4, 1, 74);
    u32(&mut bytes, 0);
    bytes.extend_from_slice(b"Canon\0EOS R\0");
    u16(&mut bytes, 6);
    entry(&mut bytes, 0x9003, 2, 20, 152);
    entry(&mut bytes, 0x829a, 5, 1, 172);
    entry(&mut bytes, 0x829d, 5, 1, 180);
    entry(&mut bytes, 0x920a, 5, 1, 188);
    entry(&mut bytes, 0x8833, 3, 1, 400);
    entry(&mut bytes, 0xa434, 2, 8, 196);
    u32(&mut bytes, 0);
    bytes.extend_from_slice(b"2024:01:02 03:04:05\0");
    u32(&mut bytes, 1);
    u32(&mut bytes, 125);
    u32(&mut bytes, 28);
    u32(&mut bytes, 10);
    u32(&mut bytes, 50);
    u32(&mut bytes, 1);
    bytes.extend_from_slice(b"RF 50mm\0");
    bytes
}

fn entry_ref(metadata: &ImageMetadata, field: MetadataField) -> &MetadataEntry {
    metadata.get(field).expect("expected canonical field")
}

#[test]
fn extracts_canonical_fields_from_classic_tiff() {
    let metadata = input()
        .read_bytes(InputFormat::Tiff, &tiff_with_metadata())
        .expect("metadata parses");
    assert_eq!(metadata.len(), 9);
    assert!(
        matches!(entry_ref(&metadata, MetadataField::CameraMake), MetadataEntry::CameraMake(value) if value.as_str() == "Canon")
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::CameraModel), MetadataEntry::CameraModel(value) if value.as_str() == "EOS R")
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::LensModel), MetadataEntry::LensModel(value) if value.as_str() == "RF 50mm")
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::CaptureDateTimeOriginal), MetadataEntry::CaptureDateTimeOriginal(value) if value.as_str() == "2024:01:02 03:04:05")
    );
    assert!(matches!(
        entry_ref(&metadata, MetadataField::Orientation),
        MetadataEntry::Orientation(Orientation::TopLeft)
    ));
    assert!(
        matches!(entry_ref(&metadata, MetadataField::ExposureTime), MetadataEntry::ExposureTime(value) if value.numerator() == 1 && value.denominator() == 125)
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::FNumber), MetadataEntry::FNumber(value) if value.numerator() == 14 && value.denominator() == 5)
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::IsoSpeed), MetadataEntry::IsoSpeed(value) if value.get() == 400)
    );
    assert!(
        matches!(entry_ref(&metadata, MetadataField::FocalLength), MetadataEntry::FocalLength(value) if value.numerator() == 50)
    );
}

#[test]
fn rejects_a_signature_mismatch_before_parsing() {
    let error = input()
        .read_bytes(InputFormat::Png, &tiff_with_metadata())
        .unwrap_err();
    assert_eq!(
        error,
        MetadataInputError::FormatMismatch {
            format: InputFormat::Png
        }
    );
}
