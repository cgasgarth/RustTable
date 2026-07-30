use rusttable_color::ColorEncoding;
use rusttable_export::encoders::{jpegxl, webp};
use rusttable_export::{CanonicalArtifact, EncodeBudget, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(channels: ChannelLayout, bytes: Vec<u8>) -> OwnedImage {
    let descriptor = ImageDescriptor::new(
        ImageDimensions::new(2, 2).expect("dimensions"),
        PixelFormat::new(
            SampleType::U8,
            channels,
            if channels == ChannelLayout::Rgba {
                AlphaMode::Straight
            } else {
                AlphaMode::None
            },
            ByteOrder::Native,
            StorageLayout::Interleaved,
        )
        .expect("format"),
        ColorEncoding::Srgb,
        Orientation::Normal,
    )
    .expect("descriptor");
    OwnedImage::new(descriptor, bytes).expect("image")
}

#[test]
fn webp_encodes_rgba_and_preserves_metadata_chunks() {
    let metadata = ExportMetadata::default()
        .with_icc_profile(vec![1, 2, 3, 4])
        .with_exif(vec![b'I', b'I', 42, 0])
        .with_xmp(b"<x:xmpmeta/>");
    let image = artifact(
        ChannelLayout::Rgba,
        vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 1, 2, 3, 0],
    );
    let artifact = CanonicalArtifact::new(&image, metadata);
    let (bytes, receipt) = webp::Encoder::new(webp::Settings::default())
        .encode_to_vec_with_budget(
            &artifact,
            EncodeBudget::new(1024 * 1024, 2, 2).unwrap(),
            &NoCancel,
        )
        .expect("WebP");
    assert_eq!(&bytes[..4], b"RIFF");
    assert!(receipt.chunks.iter().any(|chunk| chunk == "ICCP"));
    assert!(receipt.chunks.iter().any(|chunk| chunk == "EXIF"));
    assert!(receipt.chunks.iter().any(|chunk| chunk == "XMP "));
    assert_eq!(receipt.threads, 2);
}

#[test]
fn jpegxl_encodes_lossless_container_with_exif_and_xmp() {
    let image = artifact(
        ChannelLayout::Rgb,
        vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
    );
    let metadata = ExportMetadata::default()
        .with_exif(vec![b'I', b'I', 42, 0])
        .with_xmp(b"<x:xmpmeta/>");
    let artifact = CanonicalArtifact::new(&image, metadata);
    let settings = jpegxl::Settings {
        lossless: true,
        ..jpegxl::Settings::default()
    };
    let (bytes, receipt) = jpegxl::Encoder::new(settings)
        .encode_to_vec(&artifact)
        .expect("JPEG XL");
    assert_eq!(&bytes[4..8], b"JXL ");
    assert!(receipt.boxes.iter().any(|name| name == "jxlp"));
    assert!(receipt.boxes.iter().any(|name| name == "brob"));
    assert!(receipt.container);
}

#[test]
fn encoders_reject_cancelled_work_before_codec_call() {
    let image = artifact(ChannelLayout::Rgb, vec![0; 12]);
    let artifact = CanonicalArtifact::new(&image, ExportMetadata::default());
    let cancelled = Cancelled;
    assert!(matches!(
        webp::Encoder::new(webp::Settings::default()).encode_to_vec_with_budget(
            &artifact,
            EncodeBudget::default(),
            &cancelled,
        ),
        Err(webp::Error::Cancelled)
    ));
    assert!(matches!(
        jpegxl::Encoder::new(jpegxl::Settings::default()).encode_to_vec_with_budget(
            &artifact,
            EncodeBudget::default(),
            &cancelled,
        ),
        Err(jpegxl::Error::Cancelled)
    ));
}

#[test]
fn encoders_report_bounded_output_limits_without_returning_partial_bytes() {
    let image = artifact(ChannelLayout::Rgb, vec![0; 12]);
    let artifact = CanonicalArtifact::new(&image, ExportMetadata::default());
    let webp_settings = webp::Settings {
        max_output_bytes: 1,
        ..webp::Settings::default()
    };
    assert!(matches!(
        webp::Encoder::new(webp_settings).encode_to_vec(&artifact),
        Err(webp::Error::OutputLimit { .. })
    ));
    let jpegxl_settings = jpegxl::Settings {
        max_output_bytes: 1,
        ..jpegxl::Settings::default()
    };
    assert!(matches!(
        jpegxl::Encoder::new(jpegxl_settings).encode_to_vec(&artifact),
        Err(jpegxl::Error::OutputLimit { .. })
    ));
}

struct NoCancel;
impl rusttable_export::EncodeCancellation for NoCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

struct Cancelled;
impl rusttable_export::EncodeCancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}
