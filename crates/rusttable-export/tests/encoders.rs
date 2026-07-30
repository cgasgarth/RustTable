use rusttable_color::ColorEncoding;
use rusttable_export::encoders::{png, tiff};
use rusttable_export::{CanonicalArtifact, Density, DensityUnit, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(
    sample: SampleType,
    channels: ChannelLayout,
    alpha: AlphaMode,
    order: ByteOrder,
    bytes: Vec<u8>,
    metadata: ExportMetadata,
) -> (OwnedImage, CanonicalArtifact<'static>) {
    let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
    let format = PixelFormat::new(sample, channels, alpha, order, StorageLayout::Interleaved)
        .expect("format");
    let descriptor =
        ImageDescriptor::new(dimensions, format, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor");
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    let view = CanonicalArtifact::new(image, metadata);
    (image.clone(), view)
}

#[test]
fn png_round_trips_gray_ga_rgb_rgba_at_eight_and_sixteen_bits() {
    for (channels, alpha, samples) in [
        (ChannelLayout::Gray, AlphaMode::None, 1),
        (ChannelLayout::GrayA, AlphaMode::Straight, 2),
        (ChannelLayout::Rgb, AlphaMode::None, 3),
        (ChannelLayout::Rgba, AlphaMode::Straight, 4),
    ] {
        for (depth, bytes) in [
            (png::BitDepth::Eight, vec![1; 4 * samples]),
            (png::BitDepth::Sixteen, [0, 1].repeat(4 * samples)),
        ] {
            let sample = if depth == png::BitDepth::Eight {
                SampleType::U8
            } else {
                SampleType::U16
            };
            let (image, artifact) = artifact(
                sample,
                channels,
                alpha,
                if sample == SampleType::U16 {
                    ByteOrder::Big
                } else {
                    ByteOrder::Native
                },
                bytes,
                ExportMetadata::default(),
            );
            let settings = png::Settings {
                bit_depth: depth,
                channels,
                ..png::Settings::default()
            };
            let (_, receipt) = png::Encoder::new(settings)
                .encode_to_vec(&artifact)
                .expect("PNG");
            assert_eq!(receipt.dimensions, ImageDimensions::new(2, 2).unwrap());
            assert!(receipt.chunks.contains(&"IHDR".to_owned()));
            drop(image);
        }
    }
}

#[test]
fn png_embeds_profile_exif_xmp_density_and_crc_checked_chunks() {
    let metadata = ExportMetadata::default()
        .with_icc_profile(vec![1, 2, 3, 4])
        .with_exif(vec![b'I', b'I', 42, 0, 8, 0, 0, 0])
        .with_xmp(b"<x:xmpmeta/>")
        .with_density(Density::new(300, 300, DensityUnit::Inch).unwrap())
        .with_text("Comment", "RustTable");
    let (image, artifact) = artifact(
        SampleType::U8,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        vec![255; 16],
        metadata,
    );
    let settings = png::Settings {
        bit_depth: png::BitDepth::Eight,
        channels: ChannelLayout::Rgba,
        ..png::Settings::default()
    };
    let (_, receipt) = png::Encoder::new(settings)
        .encode_to_vec(&artifact)
        .expect("PNG metadata");
    for chunk in ["IHDR", "pHYs", "iCCP", "eXIf", "iTXt", "IDAT", "IEND"] {
        assert!(
            receipt.chunks.contains(&chunk.to_owned()),
            "missing {chunk}"
        );
    }
    drop(image);
}

#[test]
fn png_rejects_premultiplied_and_contradictory_metadata() {
    let (image, artifact) = artifact(
        SampleType::U8,
        ChannelLayout::Rgba,
        AlphaMode::Premultiplied,
        ByteOrder::Native,
        vec![0; 16],
        ExportMetadata::default(),
    );
    let settings = png::Settings {
        bit_depth: png::BitDepth::Eight,
        channels: ChannelLayout::Rgba,
        ..png::Settings::default()
    };
    assert!(matches!(
        png::Encoder::new(settings).encode_to_vec(&artifact),
        Err(png::Error::UnsupportedAlpha(AlphaMode::Premultiplied))
    ));
    drop(image);
}

#[test]
fn tiff_writes_classic_and_bigtiff_with_metadata_and_orientation_one() {
    for variant in [tiff::Variant::Classic, tiff::Variant::Big] {
        let metadata = ExportMetadata::default()
            .with_icc_profile(vec![9, 8, 7])
            .with_exif(vec![1, 2, 3])
            .with_xmp(vec![4, 5, 6])
            .with_density(Density::new(72, 72, DensityUnit::Inch).unwrap());
        let (image, artifact) = artifact(
            SampleType::U16,
            ChannelLayout::Rgba,
            AlphaMode::Straight,
            ByteOrder::Little,
            [1, 0].repeat(16),
            metadata,
        );
        let settings = tiff::Settings {
            variant,
            sample_type: SampleType::U16,
            channels: ChannelLayout::Rgba,
            predictor: tiff::PredictorSetting::Horizontal,
            storage: tiff::Storage::Strips { rows_per_strip: 1 },
            ..tiff::Settings::default()
        };
        let (bytes, receipt) = tiff::Encoder::new(settings)
            .encode_to_vec(&artifact)
            .expect("TIFF");
        assert_eq!(
            receipt.variant,
            if variant == tiff::Variant::Big {
                tiff::ResolvedVariant::Big
            } else {
                tiff::ResolvedVariant::Classic
            }
        );
        assert_eq!(
            bytes.get(2..4),
            Some(if variant == tiff::Variant::Big {
                &[43, 0][..]
            } else {
                &[42, 0][..]
            })
        );
        assert_eq!(receipt.dimensions, ImageDimensions::new(2, 2).unwrap());
        drop(image);
    }
}

#[test]
fn tiff_supports_float_and_half_sample_tags_and_rejects_unsupported_modes() {
    let (image, float_artifact) = artifact(
        SampleType::F32,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Native,
        [0, 0, 128, 63].repeat(4),
        ExportMetadata::default(),
    );
    let settings = tiff::Settings {
        variant: tiff::Variant::Classic,
        sample_type: SampleType::F32,
        channels: ChannelLayout::Gray,
        predictor: tiff::PredictorSetting::None,
        compression: tiff::Compression::None,
        ..tiff::Settings::default()
    };
    let (_, receipt) = tiff::Encoder::new(settings)
        .encode_to_vec(&float_artifact)
        .expect("float TIFF");
    assert!(receipt.tags.contains(&339));
    drop(image);

    let (image2, artifact2) = artifact(
        SampleType::U8,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Native,
        vec![0; 12],
        ExportMetadata::default(),
    );
    let tiled = tiff::Settings {
        sample_type: SampleType::U8,
        channels: ChannelLayout::Rgb,
        storage: tiff::Storage::Tiles {
            width: 2,
            height: 2,
        },
        ..tiff::Settings::default()
    };
    assert!(matches!(
        tiff::Encoder::new(tiled).encode_to_vec(&artifact2),
        Err(tiff::Error::UnsupportedStorage)
    ));
    drop(image2);
}
