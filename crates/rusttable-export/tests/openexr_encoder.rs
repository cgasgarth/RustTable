use rusttable_color::ColorEncoding;
use rusttable_export::encoders::openexr::{
    self, Compression, Encoder, LineOrder, NonFinitePolicy, Settings, Storage,
};
use rusttable_export::{CanonicalArtifact, EncodeBudget, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(
    sample: SampleType,
    channels: ChannelLayout,
    bytes: Vec<u8>,
) -> CanonicalArtifact<'static> {
    let alpha = if matches!(channels, ChannelLayout::GrayA | ChannelLayout::Rgba) {
        AlphaMode::Straight
    } else {
        AlphaMode::None
    };
    let format = PixelFormat::new(
        sample,
        channels,
        alpha,
        ByteOrder::Little,
        StorageLayout::Interleaved,
    )
    .expect("format");
    let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
    let descriptor = ImageDescriptor::new(
        dimensions,
        format,
        ColorEncoding::LinearSrgb,
        Orientation::Normal,
    )
    .expect("descriptor");
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    CanonicalArtifact::new(image, ExportMetadata::default())
}

fn f32_bytes(channels: usize) -> Vec<u8> {
    (0..6 * channels)
        .flat_map(|index| {
            (f32::from(u16::try_from(index).expect("test index")) / 10.0).to_le_bytes()
        })
        .collect()
}

#[test]
fn exports_all_contract_layouts_and_samples_with_independent_inspection() {
    for channels in [
        ChannelLayout::Gray,
        ChannelLayout::GrayA,
        ChannelLayout::Rgb,
        ChannelLayout::Rgba,
    ] {
        let channel_count = channels.channels();
        for sample in [SampleType::F16, SampleType::F32] {
            let bytes = if sample == SampleType::F16 {
                [0x00, 0x3c].repeat(6 * channel_count)
            } else {
                f32_bytes(channel_count)
            };
            let artifact = artifact(sample, channels, bytes);
            let settings = Settings {
                sample_type: sample,
                channels,
                compression: Compression::Rle,
                ..Settings::default()
            };
            let (encoded, receipt) = Encoder::new(settings)
                .encode_to_vec_with_budget(
                    &artifact,
                    EncodeBudget::default(),
                    &rusttable_export::NeverCancel,
                )
                .expect("OpenEXR");
            let inspection = openexr::inspect(&encoded).expect("inspection");
            assert_eq!((inspection.width, inspection.height), (3, 2));
            assert_eq!(receipt.channels, channels);
            assert_eq!(receipt.sample_type, sample);
            assert_eq!(receipt.encoded_bytes, encoded.len() as u64);
            assert_eq!(inspection.storage, "scanlines");
        }
    }
}

#[test]
fn tiled_zip_output_preserves_padded_rows_and_metadata_attributes() {
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Big,
        StorageLayout::Interleaved,
    )
    .expect("format");
    let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
    let descriptor = ImageDescriptor::with_strides(
        dimensions,
        format,
        ColorEncoding::DisplayP3,
        None,
        Orientation::Normal,
        &[40],
    )
    .expect("descriptor");
    let mut bytes = Vec::new();
    for row in 0..2 {
        for value in 0..3 * 3 {
            let value = u16::try_from(row * 10 + value).expect("test value");
            bytes.extend_from_slice(&f32::from(value).to_be_bytes());
        }
        bytes.extend_from_slice(&[0xaa; 4]);
    }
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    let metadata = ExportMetadata::default()
        .with_icc_profile([1, 2, 3, 4])
        .with_xmp(b"<xmp/>");
    let artifact = CanonicalArtifact::new(image, metadata);
    let settings = Settings {
        sample_type: SampleType::F32,
        channels: ChannelLayout::Rgb,
        compression: Compression::Zip16,
        storage: Storage::Tiles {
            width: 2,
            height: 2,
        },
        line_order: LineOrder::Decreasing,
        ..Settings::default()
    };
    let (encoded, receipt) = Encoder::new(settings)
        .encode_to_vec(&artifact)
        .expect("tiled OpenEXR");
    let inspection = openexr::inspect(&encoded).expect("inspection");
    assert_eq!(inspection.tile_size, Some((2, 2)));
    assert_eq!(inspection.line_order, "decreasing");
    assert!(
        inspection
            .attribute_names
            .iter()
            .any(|name| name == "rusttable.xmp")
    );
    assert!(receipt.metadata_sha256.is_some());
}

#[test]
fn rejects_non_finite_samples_unless_explicitly_preserved() {
    let artifact = artifact(
        SampleType::F32,
        ChannelLayout::Gray,
        f32::NAN.to_le_bytes().repeat(6),
    );
    let settings = Settings {
        sample_type: SampleType::F32,
        channels: ChannelLayout::Gray,
        non_finite: NonFinitePolicy::Reject,
        ..Settings::default()
    };
    assert!(matches!(
        Encoder::new(settings).encode_to_vec(&artifact),
        Err(openexr::Error::Unsupported("non-finite sample"))
    ));
    let settings = Settings {
        non_finite: NonFinitePolicy::PreserveNonFinite,
        ..settings
    };
    assert!(Encoder::new(settings).encode_to_vec(&artifact).is_ok());
}

#[test]
fn settings_reject_lossy_f32_and_oversized_tiles() {
    let settings = Settings {
        compression: Compression::Pxr24,
        ..Settings::default()
    };
    assert!(matches!(
        settings.validate(),
        Err(openexr::SettingsError::LossyCompressionForF32)
    ));
    let settings = Settings {
        storage: Storage::Tiles {
            width: 0,
            height: 16,
        },
        ..Settings::default()
    };
    assert!(matches!(
        settings.validate(),
        Err(openexr::SettingsError::ZeroTile)
    ));
}
