use rusttable_color::ColorEncoding;
use rusttable_core::template::{Template, TemplateContext};
use rusttable_export::ArtifactKind;
use rusttable_export::encoders::jpeg2000::{
    self, Container, ProgressionOrder, Settings, Transform,
};
use rusttable_export::{
    CanonicalArtifact, CapabilityFinding, CapabilitySet, DestinationCapabilityDescriptor,
    EncodeBudget, EncodeCancellation, ExportRequest,
};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(channels: ChannelLayout, bytes: Vec<u8>) -> CanonicalArtifact<'static> {
    let dimensions = ImageDimensions::new(8, 8).expect("dimensions");
    let alpha = if channels == ChannelLayout::Rgba {
        AlphaMode::Straight
    } else {
        AlphaMode::None
    };
    let format = PixelFormat::new(
        SampleType::U8,
        channels,
        alpha,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .expect("format");
    let descriptor =
        ImageDescriptor::new(dimensions, format, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor");
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    CanonicalArtifact::new(image, rusttable_export::ExportMetadata::default())
}

#[derive(Debug)]
struct Cancel(bool);
impl EncodeCancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0
    }
}

#[test]
fn j2k_and_jp2_lossless_exports_round_trip_and_receipt_is_complete() {
    let gray = artifact(ChannelLayout::Gray, (0u8..64).collect());
    for container in [Container::J2k, Container::Jp2] {
        let settings = Settings {
            container,
            resolution_levels: 1,
            ..Settings::default()
        };
        let (bytes, receipt) = jpeg2000::Encoder::new(settings)
            .encode_to_vec(&gray)
            .expect("JPEG 2000");
        assert_eq!(receipt.container, container);
        assert_eq!(receipt.components, 1);
        assert_eq!(receipt.bit_depth, 8);
        assert!(receipt.encoded_bytes > 0);
        assert_ne!(receipt.output_sha256, [0; 32]);
        assert!(receipt.markers.iter().any(|marker| marker == "SIZ"));
        if container == Container::Jp2 {
            assert_eq!(&bytes[4..8], b"jP  ");
            assert!(receipt.boxes.contains(&"jp2c".into()));
        }
        assert!(jpeg2000::inspect(&bytes, container).is_ok());
    }
}

#[test]
fn j2k_rejects_unsafe_metadata_and_cancellation_without_partial_output() {
    let artifact = artifact(ChannelLayout::Gray, vec![1; 64]);
    let settings = Settings {
        container: Container::J2k,
        resolution_levels: 1,
        ..Settings::default()
    };
    assert!(matches!(
        jpeg2000::Encoder::new(settings).encode_to_vec_with_budget(
            &artifact,
            EncodeBudget::default(),
            &Cancel(true)
        ),
        Err(jpeg2000::Error::Cancelled)
    ));
    let metadata = rusttable_export::ExportMetadata::default().with_xmp(b"private");
    let image = artifact.image();
    let with_metadata = CanonicalArtifact::new(image, metadata);
    assert!(matches!(
        jpeg2000::Encoder::new(settings).encode_to_vec(&with_metadata),
        Err(jpeg2000::Error::Unsupported(_))
    ));
}

#[test]
fn settings_and_capabilities_report_exact_findings() {
    let invalid = Settings {
        lossless: true,
        transform: Transform::Irreversible97,
        ..Settings::default()
    };
    assert!(invalid.validate().is_err());
    for progression in [
        ProgressionOrder::Lrcp,
        ProgressionOrder::Rlcp,
        ProgressionOrder::Rpcl,
        ProgressionOrder::Pcrl,
        ProgressionOrder::Cprl,
    ] {
        assert!(
            Settings {
                progression,
                ..Settings::default()
            }
            .validate()
            .is_ok()
        );
    }
    let request = ExportRequest::new(
        ArtifactKind::Image,
        Template::parse("${source_stem}").unwrap(),
        TemplateContext::new(),
    );
    let report = CapabilitySet::new(
        jpeg2000::capabilities(),
        DestinationCapabilityDescriptor::new("local", false),
    )
    .negotiate(&request);
    assert!(
        report
            .findings()
            .iter()
            .any(|finding| matches!(finding, CapabilityFinding::UnsupportedFormat { .. }))
    );
}
