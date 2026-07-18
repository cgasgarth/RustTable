mod support;

use std::fs;

use rusttable_image::{DecodeLimits, ImageInput, InputFormat, OutputOptions};
use rusttable_image_io::{FileImageInput, FileImageOutput};
use rusttable_metadata::{
    ExifMetadataInput, ImageMetadata, MetadataImageOutput, MetadataInput, MetadataLimits,
};

#[test]
fn metadata_png_places_one_crc_valid_exif_chunk_after_ihdr_and_round_trips() {
    let destination = support::destination("canonical.jpg");
    let output = FileImageOutput::new(support::output_limits());
    output
        .write_new_with_metadata(
            &support::image(),
            &support::metadata(),
            &destination,
            OutputOptions::Png,
            support::metadata_limits(),
        )
        .expect("metadata PNG output");
    let bytes = fs::read(&destination).expect("output bytes");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let first_length = u32::from_be_bytes(bytes[8..12].try_into().expect("IHDR length"));
    let ihdr_end = 8 + 12 + usize::try_from(first_length).expect("IHDR length fits");
    assert_eq!(&bytes[12..16], b"IHDR");
    let exif_length = u32::from_be_bytes(
        bytes[ihdr_end..ihdr_end + 4]
            .try_into()
            .expect("eXIf length"),
    );
    assert_eq!(&bytes[ihdr_end + 4..ihdr_end + 8], b"eXIf");
    let exif_end = ihdr_end + 12 + usize::try_from(exif_length).expect("eXIf length fits");
    let exif_data = &bytes[ihdr_end + 8..exif_end - 4];
    let expected_crc = crc32_parts(b"eXIf", exif_data);
    assert_eq!(&bytes[exif_end - 4..exif_end], &expected_crc.to_be_bytes());
    assert_eq!(&bytes[exif_end + 4..exif_end + 8], b"IDAT");
    assert_eq!(
        bytes.windows(4).filter(|window| *window == b"eXIf").count(),
        1
    );

    let metadata_input = ExifMetadataInput::new(
        MetadataLimits::new(1_000_000, 4_096, 16, 16, 4, 32, 16_384).expect("input limits"),
    );
    assert_eq!(
        metadata_input
            .read_bytes(InputFormat::Png, &bytes)
            .expect("EXIF round trip"),
        support::metadata()
    );
    let decoded =
        FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("decode limits"))
            .decode_path(&destination)
            .expect("PNG decode");
    assert_eq!(decoded.pixels(), support::image().pixels());
    fs::remove_file(destination).expect("cleanup");
}

fn crc32_parts(first: &[u8], second: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in first.iter().chain(second) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[test]
fn empty_metadata_png_has_no_exif_chunk() {
    let destination = support::destination("empty.png");
    FileImageOutput::new(support::output_limits())
        .write_new_with_metadata(
            &support::image(),
            &ImageMetadata::empty(),
            &destination,
            OutputOptions::Png,
            support::metadata_limits(),
        )
        .expect("empty metadata output");
    let bytes = fs::read(&destination).expect("output bytes");
    assert!(!bytes.windows(4).any(|window| window == b"eXIf"));
    fs::remove_file(destination).expect("cleanup");
}
