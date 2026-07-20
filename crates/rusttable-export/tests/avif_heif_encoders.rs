use rusttable_color::ColorEncoding;
use rusttable_export::encoders::{avif, heif};
use rusttable_export::{CanonicalArtifact, EncodeBudget, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(
    channels: ChannelLayout,
    bytes: Vec<u8>,
    metadata: ExportMetadata,
) -> (OwnedImage, CanonicalArtifact<'static>) {
    let alpha = if channels == ChannelLayout::Rgba {
        AlphaMode::Straight
    } else {
        AlphaMode::None
    };
    let descriptor = ImageDescriptor::new(
        ImageDimensions::new(3, 2).expect("dimensions"),
        PixelFormat::new(
            SampleType::U8,
            channels,
            alpha,
            ByteOrder::Native,
            StorageLayout::Interleaved,
        )
        .expect("format"),
        ColorEncoding::Srgb,
        Orientation::Normal,
    )
    .expect("descriptor");
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    (image.clone(), CanonicalArtifact::new(image, metadata))
}

fn budget() -> EncodeBudget {
    EncodeBudget::new(64 * 1024 * 1024, 2, 2).expect("budget")
}

#[test]
fn avif_encodes_opaque_and_alpha_stills_with_exif_and_reopens() {
    let pixels = vec![
        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 128, 64, 32,
    ];
    let (_, rgb_artifact) = artifact(
        ChannelLayout::Rgb,
        pixels.clone(),
        ExportMetadata::default().with_exif([b'I', b'I', 42, 0]),
    );
    let (bytes, receipt) = avif::Encoder::new(avif::Settings::default())
        .encode_to_vec_with_budget(&rgb_artifact, budget(), &rusttable_export::NeverCancel)
        .expect("AVIF");
    assert_eq!(&bytes[4..8], b"ftyp");
    assert!(receipt.boxes.iter().any(|name| name == "meta"));
    assert_eq!(receipt.threads, 2);

    let (_, alpha_artifact) = artifact(
        ChannelLayout::Rgba,
        [pixels, vec![255, 128, 64, 0, 255, 192]].concat(),
        ExportMetadata::default(),
    );
    let (bytes, receipt) = avif::Encoder::new(avif::Settings::default())
        .encode_to_vec_with_budget(&alpha_artifact, budget(), &rusttable_export::NeverCancel)
        .expect("AVIF alpha");
    assert!(!bytes.is_empty());
    assert_eq!(receipt.depth, avif::OutputBitDepth::Ten);
}

#[test]
fn heif_and_heic_use_explicit_ids_and_preserve_metadata() {
    let pixels = vec![
        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 128, 64, 32,
    ];
    for id in [heif::EncoderId::Heif, heif::EncoderId::Heic] {
        let (_, artifact) = artifact(
            ChannelLayout::Rgb,
            pixels.clone(),
            ExportMetadata::default()
                .with_icc_profile([0, 1, 2, 3])
                .with_exif([b'I', b'I', 42, 0])
                .with_xmp(b"<x:xmpmeta/>"),
        );
        let settings = heif::Settings {
            encoder: id,
            subsampling: heif::ChromaSubsampling::FourTwoZero,
            ..heif::Settings::default()
        };
        let (bytes, receipt) = heif::Encoder::new(settings)
            .encode_to_vec_with_budget(&artifact, budget(), &rusttable_export::NeverCancel)
            .expect("HEIF");
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(receipt.boxes.iter().any(|name| name == "meta"));
        assert_eq!(receipt.encoder_id, id.as_str());
    }
    let rgba = [pixels, vec![255, 128, 64, 0, 255, 192]].concat();
    let (_, artifact) = artifact(ChannelLayout::Rgba, rgba, ExportMetadata::default());
    let settings = heif::Settings {
        encoder: heif::EncoderId::Heic,
        ..heif::Settings::default()
    };
    let (_, receipt) = heif::Encoder::new(settings)
        .encode_to_vec_with_budget(&artifact, budget(), &rusttable_export::NeverCancel)
        .expect("HEIC alpha");
    assert!(receipt.boxes.iter().any(|name| name == "meta"));
}

#[test]
fn codecs_reject_unsupported_or_cancelled_work_before_native_encode() {
    let (_, artifact) = artifact(ChannelLayout::Rgb, vec![0; 18], ExportMetadata::default());
    assert!(matches!(
        avif::Encoder::new(avif::Settings {
            subsampling: avif::ChromaSubsampling::FourTwoZero,
            ..avif::Settings::default()
        })
        .encode_to_vec(&artifact),
        Err(avif::Error::InvalidSettings(_))
    ));
    assert!(matches!(
        heif::Encoder::new(heif::Settings::default()).encode_to_vec_with_budget(
            &artifact,
            budget(),
            &Cancelled
        ),
        Err(heif::Error::Cancelled)
    ));
}

struct Cancelled;
impl rusttable_export::EncodeCancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}
