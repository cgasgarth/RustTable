mod support;

use std::fs;

use rusttable_image::{
    DecodeLimits, ImageInput, ImageOutput, InputFormat, JpegQuality, OutputOptions,
};
use rusttable_image_io::{FileImageInput, FileImageOutput};
use rusttable_metadata::{
    ExifMetadataInput, ImageMetadata, MetadataImageOutput, MetadataInput, MetadataLimits,
};

#[test]
fn metadata_jpeg_places_one_canonical_app1_after_soi_and_round_trips() {
    let destination = support::destination("canonical.jpg");
    let receipt = FileImageOutput::new(support::output_limits())
        .write_new_with_metadata(
            &support::image(),
            &support::metadata(),
            &destination,
            OutputOptions::Jpeg {
                quality: JpegQuality::new(90).expect("quality"),
            },
            support::metadata_limits(),
        )
        .expect("metadata JPEG output");
    let bytes = fs::read(&destination).expect("output bytes");
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Jpeg);
    assert_eq!(&bytes[..4], &[0xff, 0xd8, 0xff, 0xe1]);
    let segment_length = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    assert_eq!(&bytes[6..12], b"Exif\0\0");
    let segment_end = 6 + segment_length;
    assert!(segment_end < bytes.len());
    assert_eq!(bytes.last_chunk::<2>().expect("JPEG EOI"), &[0xff, 0xd9]);
    assert_eq!(
        bytes
            .windows(6)
            .filter(|window| *window == b"Exif\0\0")
            .count(),
        1
    );

    let metadata_input = ExifMetadataInput::new(
        MetadataLimits::new(1_000_000, 4_096, 16, 16, 4, 32, 16_384).expect("input limits"),
    );
    assert_eq!(
        metadata_input
            .read_bytes(InputFormat::Jpeg, &bytes)
            .expect("EXIF round trip"),
        support::metadata()
    );
    let decoded =
        FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("decode limits"))
            .decode_path(&destination)
            .expect("JPEG decode");
    assert_eq!(decoded.dimensions(), support::image().dimensions());
    assert!(
        decoded
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 255)
    );
    fs::remove_file(destination).expect("cleanup");
}

#[test]
fn empty_metadata_reuses_the_byte_compatible_ordinary_path() {
    let ordinary = support::destination("empty-ordinary.jpg");
    let metadata = support::destination("empty-metadata.jpg");
    let output = FileImageOutput::new(support::output_limits());
    let quality = JpegQuality::new(90).expect("quality");
    let ordinary_receipt = output
        .write_new(
            &support::image(),
            &ordinary,
            OutputOptions::Jpeg { quality },
        )
        .expect("ordinary output");
    let metadata_receipt = output
        .write_new_with_metadata(
            &support::image(),
            &ImageMetadata::empty(),
            &metadata,
            OutputOptions::Jpeg { quality },
            support::metadata_limits(),
        )
        .expect("empty metadata output");
    assert_eq!(
        fs::read(&ordinary).expect("ordinary bytes"),
        fs::read(&metadata).expect("metadata bytes")
    );
    assert_eq!(ordinary_receipt.format(), metadata_receipt.format());
    assert_eq!(ordinary_receipt.dimensions(), metadata_receipt.dimensions());
    assert_eq!(
        ordinary_receipt.encoded_byte_length(),
        metadata_receipt.encoded_byte_length()
    );
    fs::remove_file(ordinary).expect("cleanup");
    fs::remove_file(metadata).expect("cleanup");
}
