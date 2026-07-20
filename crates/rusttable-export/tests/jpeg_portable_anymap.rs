use rusttable_color::ColorEncoding;
use rusttable_export::encoders::{jpeg, portable_anymap};
use rusttable_export::{CanonicalArtifact, ExportMetadata};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDescriptor, ImageDimensions, Orientation, OwnedImage,
    PixelFormat, SampleType, StorageLayout,
};

fn artifact(
    dimensions: ImageDimensions,
    sample: SampleType,
    channels: ChannelLayout,
    alpha: AlphaMode,
    order: ByteOrder,
    bytes: Vec<u8>,
    metadata: ExportMetadata,
) -> CanonicalArtifact<'static> {
    let format = PixelFormat::new(sample, channels, alpha, order, StorageLayout::Interleaved)
        .expect("pixel format");
    let descriptor = if bytes.len()
        == ImageDescriptor::new(dimensions, format, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor")
            .byte_length()
    {
        ImageDescriptor::new(dimensions, format, ColorEncoding::Srgb, Orientation::Normal)
            .expect("descriptor")
    } else {
        let stride = bytes.len() / usize::try_from(dimensions.height()).expect("height");
        ImageDescriptor::with_strides(
            dimensions,
            format,
            ColorEncoding::Srgb,
            None,
            Orientation::Normal,
            &[stride],
        )
        .expect("padded descriptor")
    };
    let image = Box::leak(Box::new(OwnedImage::new(descriptor, bytes).expect("image")));
    CanonicalArtifact::new(image, metadata)
}

#[test]
fn jpeg_honors_quality_sampling_progressive_restart_and_density() {
    let image = artifact(
        ImageDimensions::new(17, 9).expect("dimensions"),
        SampleType::U8,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Native,
        (0..17 * 9 * 3)
            .map(|value| u8::try_from(value % 256))
            .collect::<Result<Vec<_>, _>>()
            .expect("pixel values"),
        ExportMetadata::default(),
    );
    let settings = jpeg::Settings {
        quality: 73,
        subsampling: jpeg::ChromaSubsampling::FourTwoZero,
        progressive: true,
        optimize_huffman: false,
        restart_interval: 4,
        density: Some(
            rusttable_export::Density::new(300, 300, rusttable_export::DensityUnit::Inch)
                .expect("density"),
        ),
        ..jpeg::Settings::default()
    };
    let (bytes, receipt) = jpeg::Encoder::new(settings)
        .encode_to_vec(&image)
        .expect("JPEG");
    assert_eq!(receipt.dimensions, ImageDimensions::new(17, 9).unwrap());
    assert_eq!(receipt.quality, 73);
    assert_eq!(receipt.subsampling, jpeg::ChromaSubsampling::FourTwoZero);
    assert!(receipt.progressive);
    assert_eq!(receipt.encoded_bytes, bytes.len() as u64);
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    assert_eq!(&bytes[bytes.len() - 2..], &[0xff, 0xd9]);
    assert!(bytes.windows(2).any(|window| window == [0xff, 0xdd]));
    assert!(bytes.windows(2).any(|window| window == [0xff, 0xc2]));
}

#[test]
fn jpeg_streams_padded_grayscale_and_chunks_large_xmp() {
    let image = artifact(
        ImageDimensions::new(2, 2).expect("dimensions"),
        SampleType::U8,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Native,
        vec![1, 2, 0xaa, 0xbb, 3, 4, 0xcc, 0xdd],
        ExportMetadata::default().with_xmp(vec![b'x'; 100_000]),
    );
    let mut bytes = Vec::new();
    let receipt = jpeg::Encoder::new(jpeg::Settings::default())
        .encode_to_writer(&image, &mut bytes)
        .expect("JPEG");
    assert_eq!(receipt.dimensions, ImageDimensions::new(2, 2).unwrap());
    assert!(
        bytes
            .windows(b"http://ns.adobe.com/xmp/extension/\0".len())
            .any(|window| window == b"http://ns.adobe.com/xmp/extension/\0")
    );
}

#[test]
fn ppm_and_pgm_are_exact_and_big_endian() {
    let ppm = artifact(
        ImageDimensions::new(2, 1).expect("dimensions"),
        SampleType::U8,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Native,
        vec![1, 2, 3, 4, 5, 6],
        ExportMetadata::default(),
    );
    let (ppm_bytes, ppm_receipt) =
        portable_anymap::Encoder::new(portable_anymap::Settings::default())
            .encode_to_vec(&ppm)
            .expect("PPM");
    assert_eq!(&ppm_bytes[..11], b"P6\n2 1\n255\n");
    assert_eq!(&ppm_bytes[11..], &[1, 2, 3, 4, 5, 6]);
    assert_eq!(ppm_receipt.payload_bytes, 6);

    let pgm = artifact(
        ImageDimensions::new(2, 1).expect("dimensions"),
        SampleType::U16,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Little,
        vec![0x34, 0x12, 0xcd, 0xab],
        ExportMetadata::default(),
    );
    let settings = portable_anymap::Settings {
        format: portable_anymap::Format::Pgm,
        integer_depth: portable_anymap::IntegerDepth::Sixteen,
        ..portable_anymap::Settings::default()
    };
    let (pgm_output, _) = portable_anymap::Encoder::new(settings)
        .encode_to_vec(&pgm)
        .expect("PGM");
    assert_eq!(&pgm_output[..13], b"P5\n2 1\n65535\n");
    assert_eq!(&pgm_output[13..], &[0x12, 0x34, 0xab, 0xcd]);
}

#[test]
fn pfm_is_little_endian_bottom_up_and_has_explicit_non_finite_policy() {
    let pfm = artifact(
        ImageDimensions::new(1, 2).expect("dimensions"),
        SampleType::F32,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Big,
        [1.0f32.to_be_bytes(), 2.0f32.to_be_bytes()].concat(),
        ExportMetadata::default(),
    );
    let settings = portable_anymap::Settings {
        format: portable_anymap::Format::Pfm,
        ..portable_anymap::Settings::default()
    };
    let (bytes, receipt) = portable_anymap::Encoder::new(settings)
        .encode_to_vec(&pfm)
        .expect("PFM");
    assert_eq!(&bytes[..12], b"Pf\n1 2\n-1.0\n");
    assert_eq!(&bytes[12..], &[0u8, 0, 0, 0x40, 0, 0, 0x80, 0x3f]);
    assert_eq!(receipt.non_finite_count, 0);

    let non_finite = artifact(
        ImageDimensions::new(1, 1).expect("dimensions"),
        SampleType::F32,
        ChannelLayout::Gray,
        AlphaMode::None,
        ByteOrder::Little,
        f32::NAN.to_le_bytes().to_vec(),
        ExportMetadata::default(),
    );
    assert!(matches!(
        portable_anymap::Encoder::new(settings).encode_to_vec(&non_finite),
        Err(portable_anymap::Error::NonFinite { .. })
    ));
    let canonical = portable_anymap::Settings {
        non_finite: portable_anymap::NonFinitePolicy::Canonicalize,
        ..settings
    };
    let (_, receipt) = portable_anymap::Encoder::new(canonical)
        .encode_to_vec(&non_finite)
        .expect("canonicalized PFM");
    assert_eq!(receipt.non_finite_count, 1);
}

#[test]
fn portable_anymap_rejects_metadata_and_jpeg_rejects_alpha() {
    let image = artifact(
        ImageDimensions::new(1, 1).expect("dimensions"),
        SampleType::U8,
        ChannelLayout::Rgb,
        AlphaMode::None,
        ByteOrder::Native,
        vec![1, 2, 3],
        ExportMetadata::default().with_icc_profile(vec![1]),
    );
    assert!(matches!(
        portable_anymap::Encoder::new(portable_anymap::Settings::default()).encode_to_vec(&image),
        Err(portable_anymap::Error::UnsupportedMetadata)
    ));

    let alpha = artifact(
        ImageDimensions::new(1, 1).expect("dimensions"),
        SampleType::U8,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        vec![1, 2, 3, 4],
        ExportMetadata::default(),
    );
    assert!(matches!(
        jpeg::Encoder::new(jpeg::Settings::default()).encode_to_vec(&alpha),
        Err(jpeg::Error::UnsupportedLayout(ChannelLayout::Rgba))
    ));
}
