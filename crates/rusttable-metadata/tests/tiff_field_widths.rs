use rusttable_image::InputFormat;
use rusttable_metadata::{ExifMetadataInput, MetadataInput, MetadataInputError, MetadataLimits};

const VALUE_DATA_OFFSET: usize = 8 + 2 + (8 * 12) + 4;

fn input() -> ExifMetadataInput {
    ExifMetadataInput::new(MetadataLimits::new(4096, 2048, 32, 32, 4, 32, 128).unwrap())
}

fn put_u16(bytes: &mut Vec<u8>, value: u16, little: bool) {
    let encoded = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    bytes.extend_from_slice(&encoded);
}

fn put_u32(bytes: &mut Vec<u8>, value: u32, little: bool) {
    let encoded = if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    };
    bytes.extend_from_slice(&encoded);
}

fn put_entry(bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32, little: bool) {
    put_u16(bytes, tag, little);
    put_u16(bytes, kind, little);
    put_u32(bytes, count, little);
    put_u32(bytes, value, little);
}

fn typed_width_fixture(little: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(160);
    bytes.extend_from_slice(if little { b"II*\0" } else { b"MM\0*" });
    put_u32(&mut bytes, 8, little);
    put_u16(&mut bytes, 8, little);

    // Inline SBYTE, SSHORT, SLONG, and IFD values.
    put_entry(&mut bytes, 0xc001, 6, 1, 0x0000_00fe, little);
    put_entry(&mut bytes, 0xc002, 8, 1, 0x0000_fffe, little);
    put_entry(&mut bytes, 0xc003, 9, 1, 0xffff_fffd, little);
    put_entry(&mut bytes, 0xc004, 13, 1, 0x1234_5678, little);

    // Offset-backed values exercise every corrected width with a count that
    // exceeds the four-byte inline value slot.
    let sbyte_offset = u32::try_from(VALUE_DATA_OFFSET).unwrap();
    let sshort_offset = sbyte_offset + 8;
    let slong_offset = sshort_offset + 6;
    let ifd_offset = slong_offset + 8;
    put_entry(&mut bytes, 0xc011, 6, 8, sbyte_offset, little);
    put_entry(&mut bytes, 0xc012, 8, 3, sshort_offset, little);
    put_entry(&mut bytes, 0xc013, 9, 2, slong_offset, little);
    put_entry(&mut bytes, 0xc014, 13, 2, ifd_offset, little);
    put_u32(&mut bytes, 0, little);

    bytes.extend_from_slice(&[0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87]);
    put_u16(&mut bytes, 0xffff, little);
    put_u16(&mut bytes, 0, little);
    put_u16(&mut bytes, 0x7fff, little);
    put_u32(&mut bytes, u32::MAX, little);
    put_u32(&mut bytes, i32::MAX as u32, little);
    put_u32(&mut bytes, 8, little);
    put_u32(&mut bytes, 16, little);
    bytes
}

fn jpeg_with_exif(tiff: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe1];
    let segment_length = u16::try_from(tiff.len() + 8).unwrap();
    bytes.extend_from_slice(&segment_length.to_be_bytes());
    bytes.extend_from_slice(b"Exif\0\0");
    bytes.extend_from_slice(tiff);
    bytes.extend_from_slice(&[0xff, 0xd9]);
    bytes
}

#[test]
fn corrected_signed_and_ifd_widths_parse_inline_and_offset_values_in_little_endian() {
    let metadata = input()
        .read_bytes(InputFormat::Tiff, &typed_width_fixture(true))
        .expect("little-endian TIFF field widths parse");
    assert!(metadata.is_empty());
}

#[test]
fn corrected_signed_and_ifd_widths_parse_inline_and_offset_values_in_big_endian() {
    let metadata = input()
        .read_bytes(InputFormat::Tiff, &typed_width_fixture(false))
        .expect("big-endian TIFF field widths parse");
    assert!(metadata.is_empty());
}

#[test]
fn corrected_widths_are_applied_inside_jpeg_app1() {
    let tiff = typed_width_fixture(false);
    let metadata = input()
        .read_bytes(InputFormat::Jpeg, &jpeg_with_exif(&tiff))
        .expect("JPEG APP1 TIFF field widths parse");
    assert!(metadata.is_empty());
}

#[test]
fn truncated_inline_value_returns_a_typed_structure_error() {
    let mut source = typed_width_fixture(true);
    source.truncate(8 + 2 + (8 * 12) + 3);
    assert!(matches!(
        input().read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::TiffStructureTruncated { .. })
    ));
}

#[test]
fn truncated_offset_value_returns_a_typed_value_error() {
    let mut source = typed_width_fixture(true);
    source.truncate(VALUE_DATA_OFFSET + 7);
    assert!(matches!(
        input().read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::TiffValueTruncated {
            kind: 6,
            count: 8,
            ..
        })
    ));
}

#[test]
fn offset_beyond_payload_returns_a_typed_value_error() {
    let mut source = typed_width_fixture(true);
    // The first offset-backed SBYTE entry stores its offset at byte 66.
    source[66..70].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        input().read_bytes(InputFormat::Tiff, &source),
        Err(MetadataInputError::TiffValueTruncated {
            kind: 6,
            count: 8,
            ..
        })
    ));
}

#[test]
fn maximum_tiff_count_is_checked_without_narrowing_or_wraparound() {
    let mut source = typed_width_fixture(true);
    // The first offset-backed SBYTE entry stores its count at byte 62.
    source[62..66].copy_from_slice(&u32::MAX.to_le_bytes());
    let error = input().read_bytes(InputFormat::Tiff, &source).unwrap_err();
    let MetadataInputError::ValueTooLarge { limit, actual } = error else {
        panic!("expected a typed value limit error");
    };
    assert_eq!(limit, 128);
    assert_eq!(actual, u64::from(u32::MAX));
}
