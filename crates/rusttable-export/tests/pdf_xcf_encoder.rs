use rusttable_color::ColorEncoding;
use rusttable_export::encoders::{pdf, xcf};
use rusttable_export::{CanonicalArtifact, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn make_artifact(
    sample: SampleType,
    channels: ChannelLayout,
    bytes: Vec<u8>,
    metadata: ExportMetadata,
) -> (OwnedImage, CanonicalArtifact<'static>) {
    let dimensions = ImageDimensions::new(65, 3).expect("dimensions");
    let alpha = if matches!(channels, ChannelLayout::Rgba) {
        AlphaMode::Straight
    } else {
        AlphaMode::None
    };
    let format = PixelFormat::new(
        sample,
        channels,
        alpha,
        ByteOrder::Big,
        StorageLayout::Interleaved,
    )
    .expect("format");
    let descriptor =
        ImageDescriptor::new(dimensions, format, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor");
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    (image.clone(), CanonicalArtifact::new(image, metadata))
}

struct Cancelled;
impl rusttable_export::EncodeCancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[test]
fn pdf_is_one_page_deterministic_and_independently_inspectable() {
    let metadata = ExportMetadata::default()
        .with_icc_profile([1, 2, 3, 4])
        .with_xmp(b"<x:xmpmeta/>")
        .with_text("Title", "Receipt");
    let (image, artifact) = make_artifact(
        SampleType::U8,
        ChannelLayout::Rgba,
        vec![17; 65 * 3 * 4],
        metadata,
    );
    let settings = pdf::Settings {
        page: pdf::PageSize::A4,
        margin: 12.0,
        placement: pdf::Placement::Fit,
        ..pdf::Settings::default()
    };
    let encoder = pdf::Encoder::new(settings);
    let (first, receipt) = encoder.encode_to_vec(&artifact).expect("PDF");
    let (second, second_receipt) = encoder.encode_to_vec(&artifact).expect("PDF repeat");
    assert_eq!(first, second);
    assert_eq!(receipt.output_sha256, second_receipt.output_sha256);
    let inspection = pdf::inspect(&first).expect("PDF inspection");
    assert_eq!(inspection.page_count, 1);
    assert_eq!(
        inspection.image_dimensions,
        ImageDimensions::new(65, 3).unwrap()
    );
    assert!(inspection.has_soft_mask);
    assert!(inspection.has_icc_profile);
    assert!(inspection.has_xmp);
    drop(image);
}

#[test]
fn pdf_rejects_non_u8_and_cancellation_publishes_no_bytes() {
    let (image, artifact) = make_artifact(
        SampleType::U16,
        ChannelLayout::Rgb,
        [0, 1].repeat(65 * 3 * 3),
        ExportMetadata::default(),
    );
    assert!(matches!(
        pdf::Encoder::new(pdf::Settings::default()).encode_to_vec(&artifact),
        Err(pdf::Error::UnsupportedSample(SampleType::U16))
    ));
    assert!(matches!(
        pdf::Encoder::new(pdf::Settings::default()).encode_to_vec_with_budget(
            &artifact,
            rusttable_export::EncodeBudget::default(),
            &Cancelled
        ),
        Err(pdf::Error::Cancelled)
    ));
    drop(image);
}

#[test]
fn xcf_round_trips_rle_and_raw_across_integer_precisions() {
    for (precision, sample, bytes) in [
        (xcf::Precision::U8, SampleType::U8, vec![9; 65 * 3 * 3]),
        (
            xcf::Precision::U16,
            SampleType::U16,
            [0, 9].repeat(65 * 3 * 3),
        ),
    ] {
        let (image, artifact) = make_artifact(
            sample,
            ChannelLayout::Rgb,
            bytes,
            ExportMetadata::default().with_icc_profile([9, 8, 7]),
        );
        for compression in [xcf::Compression::Rle, xcf::Compression::Raw] {
            let settings = xcf::Settings {
                precision,
                channels: ChannelLayout::Rgb,
                compression,
                ..xcf::Settings::default()
            };
            let (encoded, receipt) = xcf::Encoder::new(settings)
                .encode_to_vec(&artifact)
                .expect("XCF");
            let inspection = xcf::inspect(&encoded).expect("XCF inspection");
            assert_eq!(inspection.layer_count, 1);
            assert_eq!(inspection.layer_offset, (0, 0));
            assert_eq!(inspection.tile_count, 2);
            assert_eq!(inspection.precision, precision);
            assert_eq!(inspection.profile_sha256, Some(sha256(&[9, 8, 7])));
            assert_eq!(inspection.decoded_sha256, receipt.decoded_sha256);
        }
        drop(image);
    }
}

#[test]
fn xcf_supports_float_and_rejects_non_finite_samples() {
    let mut bytes = Vec::new();
    for _ in 0..(65 * 3) {
        for value in [0.0_f32, 0.25, 0.5, 1.0] {
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
    }
    let (image, artifact) = make_artifact(
        SampleType::F32,
        ChannelLayout::Rgba,
        bytes,
        ExportMetadata::default(),
    );
    let settings = xcf::Settings {
        precision: xcf::Precision::F32,
        channels: ChannelLayout::Rgba,
        ..xcf::Settings::default()
    };
    let (encoded, receipt) = xcf::Encoder::new(settings)
        .encode_to_vec(&artifact)
        .expect("float XCF");
    assert_eq!(
        xcf::inspect(&encoded).unwrap().decoded_sha256,
        receipt.decoded_sha256
    );
    drop(image);

    let mut bad = Vec::new();
    for _ in 0..(65 * 3 * 4) {
        bad.extend_from_slice(&f32::NAN.to_bits().to_be_bytes());
    }
    let (image, artifact) = make_artifact(
        SampleType::F32,
        ChannelLayout::Rgba,
        bad,
        ExportMetadata::default(),
    );
    assert!(matches!(
        xcf::Encoder::new(settings).encode_to_vec(&artifact),
        Err(xcf::Error::NonFiniteSample)
    ));
    drop(image);
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}
