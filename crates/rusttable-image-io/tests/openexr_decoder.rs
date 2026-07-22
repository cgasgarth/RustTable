use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

use exr::image::write::layers::WritableLayers;
use exr::meta::attribute::{AttributeValue, Chromaticities, IntegerBounds};
use exr::prelude::{
    Blocks, Compression, Encoding, Image, ImageAttributes, Layer, LayerAttributes, LineOrder,
    SpecificChannels, Vec2, WritableImage, f16,
};
use rusttable_image::{ChannelLayout, Roi};
use rusttable_image_io::{
    EXR_BACKEND_ID, ExrChannelMapping, ExrCompression, ExrDecodeError, ExrDecodeLimits,
    ExrDecodeRequest, ExrDecoder, ExrSampleData, ExrSampleType, RawByteSource,
    RawCancellationToken, RawSourceError,
};

fn limits() -> ExrDecodeLimits {
    ExrDecodeLimits {
        max_source_bytes: 2_000_000,
        max_width: 128,
        max_height: 128,
        max_pixels: 16_384,
        max_decoded_bytes: 512 * 1024,
        max_decompressed_bytes: 512 * 1024,
        max_header_bytes: 64 * 1024,
        max_metadata_bytes: 16 * 1024,
        max_parts: 8,
        max_channels_per_part: 32,
        max_attributes_per_part: 128,
        max_levels_per_axis: 16,
        max_chunks: 4_096,
    }
}

fn encoding(compression: Compression, blocks: Blocks) -> Encoding {
    Encoding {
        compression,
        blocks,
        line_order: LineOrder::Increasing,
    }
}

fn write<ImageChannels>(image: &Image<ImageChannels>) -> Vec<u8>
where
    for<'image> ImageChannels: WritableLayers<'image>,
{
    let mut cursor = Cursor::new(Vec::new());
    image
        .write()
        .non_parallel()
        .to_buffered(&mut cursor)
        .expect("OpenEXR fixture");
    cursor.into_inner()
}

fn rgba_f16(compression: Compression, blocks: Blocks) -> Vec<u8> {
    let channels = SpecificChannels::rgba(|position: Vec2<usize>| {
        let index = position.y() * 3 + position.x();
        (
            f16::from_f32(as_f32(index) + 0.25),
            f16::from_f32(as_f32(index) + 0.5),
            f16::from_f32(as_f32(index) + 0.75),
            f16::from_f32(1.0),
        )
    });
    write(&Image::from_layer(Layer::new(
        (3, 2),
        LayerAttributes::named("main"),
        encoding(compression, blocks),
        channels,
    )))
}

fn decode(bytes: &[u8]) -> rusttable_image_io::ExrDecodeResult {
    ExrDecoder::new()
        .decode_bytes(bytes, &ExrDecodeRequest::new(limits()))
        .expect("OpenEXR decodes")
}

#[test]
fn lossless_compressions_preserve_half_bits_and_lossy_modes_remain_finite() {
    let lossless = [
        (Compression::Uncompressed, ExrCompression::None),
        (Compression::RLE, ExrCompression::Rle),
        (Compression::ZIP1, ExrCompression::Zips),
        (Compression::ZIP16, ExrCompression::Zip),
        (Compression::PIZ, ExrCompression::Piz),
        (Compression::PXR24, ExrCompression::Pxr24),
    ];
    let expected = match decode(&rgba_f16(Compression::Uncompressed, Blocks::ScanLines))
        .pixels
        .unwrap()
        .samples
    {
        ExrSampleData::F16(values) => values,
        ExrSampleData::F32(_) => panic!("uniform half must stay half"),
    };
    for (compression, normalized) in lossless {
        let result = decode(&rgba_f16(compression, Blocks::ScanLines));
        assert_eq!(result.receipt.compression, normalized);
        assert_eq!(result.receipt.backend, EXR_BACKEND_ID);
        assert_eq!(
            result.pixels.unwrap().samples,
            ExrSampleData::F16(expected.clone())
        );
    }
    for (compression, normalized) in [
        (Compression::B44, ExrCompression::B44),
        (Compression::B44A, ExrCompression::B44A),
    ] {
        let result = decode(&rgba_f16(compression, Blocks::Tiles(Vec2(4, 4))));
        assert_eq!(result.receipt.compression, normalized);
        match result.pixels.unwrap().samples {
            ExrSampleData::F16(values) => assert!(
                values
                    .iter()
                    .all(|value| f16::from_bits(*value).is_finite())
            ),
            ExrSampleData::F32(_) => panic!("uniform half must stay half"),
        }
    }
}

#[test]
fn mixed_channels_promote_to_f32_and_explicit_layer_selection_is_receipted() {
    let channels = SpecificChannels::build()
        .with_channel::<f16>("beauty.R")
        .with_channel::<f32>("beauty.G")
        .with_channel::<f16>("beauty.B")
        .with_channel::<f16>("beauty.A")
        .with_channel::<f16>("raw.R")
        .with_channel::<f16>("raw.G")
        .with_channel::<f16>("raw.B")
        .with_pixel_fn(|position| {
            let base = as_f32(position.x()) + 10.0;
            (
                f16::from_f32(base),
                base + 0.5,
                f16::from_f32(base + 1.0),
                f16::ONE,
                f16::from_f32(base + 20.0),
                f16::from_f32(base + 21.0),
                f16::from_f32(base + 22.0),
            )
        });
    let bytes = write(&Image::from_layer(Layer::new(
        (2, 1),
        LayerAttributes::named("composite"),
        encoding(Compression::ZIP1, Blocks::ScanLines),
        channels,
    )));
    let default = decode(&bytes);
    assert_eq!(default.receipt.layer, "beauty");
    assert_eq!(default.receipt.sample_type, ExrSampleType::F32);
    assert_eq!(default.pixels.as_ref().unwrap().layout, ChannelLayout::Rgba);
    assert_eq!(
        default.pixels.unwrap().samples,
        ExrSampleData::F32(vec![10.0, 10.5, 11.0, 1.0, 11.0, 11.5, 12.0, 1.0])
    );

    let raw = ExrDecoder::new()
        .decode_bytes(&bytes, &ExrDecodeRequest::new(limits()).layer("raw"))
        .unwrap();
    assert_eq!(raw.receipt.layer, "raw");
    assert_eq!(raw.receipt.sample_type, ExrSampleType::F16);
    assert_eq!(raw.pixels.unwrap().layout, ChannelLayout::Rgb);
}

#[test]
fn multipart_default_and_explicit_part_selection_follow_header_order() {
    let first = Layer::new(
        (2, 1),
        LayerAttributes::named("first"),
        encoding(Compression::ZIP1, Blocks::ScanLines),
        SpecificChannels::rgb(|_: Vec2<usize>| (1.0_f32, 2.0_f32, 3.0_f32)),
    );
    let second = Layer::new(
        (2, 1),
        LayerAttributes::named("second"),
        encoding(Compression::PIZ, Blocks::ScanLines),
        SpecificChannels::rgb(|_: Vec2<usize>| (4.0_f32, 5.0_f32, 6.0_f32)),
    );
    let image = Image::empty(ImageAttributes::with_size((2, 1)))
        .with_layer(first)
        .with_layer(second);
    let bytes = write(&image);

    let default = decode(&bytes);
    assert_eq!(default.header.parts.len(), 2);
    assert_eq!(default.receipt.part_index, 0);
    assert_eq!(default.receipt.part_name, "first");
    let explicit = ExrDecoder::new()
        .decode_bytes(&bytes, &ExrDecodeRequest::new(limits()).part(1))
        .unwrap();
    assert_eq!(explicit.receipt.part_index, 1);
    assert_eq!(explicit.receipt.part_name, "second");
    assert_eq!(explicit.receipt.compression, ExrCompression::Piz);
    assert_eq!(
        explicit.pixels.unwrap().samples,
        ExrSampleData::F32(vec![4.0, 5.0, 6.0, 4.0, 5.0, 6.0])
    );
}

#[test]
fn negative_data_window_display_fill_metadata_and_roi_are_exact() {
    let channels = SpecificChannels::rgba(|position: Vec2<usize>| {
        let value = as_f32(position.y() * 2 + position.x()) + 1.0;
        (value, value + 10.0, value + 20.0, 0.5_f32)
    });
    let mut layer_attributes = LayerAttributes::named("negative").with_position(Vec2(-1, 0));
    layer_attributes.white_luminance = Some(100.0);
    layer_attributes.adopted_neutral = Some(Vec2(0.3457, 0.3585));
    layer_attributes
        .other
        .insert("xmp".into(), AttributeValue::Text("<x:xmpmeta/>".into()));
    let mut image_attributes = ImageAttributes::new(IntegerBounds::new(Vec2(-2, -1), Vec2(5, 4)));
    image_attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.64, 0.33),
        green: Vec2(0.3, 0.6),
        blue: Vec2(0.15, 0.06),
        white: Vec2(0.3127, 0.329),
    });
    let bytes = write(&Image::new(
        image_attributes,
        Layer::new(
            (2, 2),
            layer_attributes,
            encoding(Compression::ZIP1, Blocks::ScanLines),
            channels,
        ),
    ));
    let full = decode(&bytes);
    let pixels = full.pixels.as_ref().unwrap();
    assert_eq!(
        (
            pixels.origin,
            pixels.dimensions.width(),
            pixels.dimensions.height()
        ),
        ([-2, -1], 5, 4)
    );
    assert_eq!(full.receipt.data_window.x, -1);
    assert_eq!(full.receipt.metadata.white_luminance, Some(100.0));
    assert_eq!(full.receipt.metadata.xmp.as_ref().unwrap().bytes, 12);
    let ExrSampleData::F32(values) = &pixels.samples else {
        panic!("f32")
    };
    assert_eq!(&values[..4], &[0.0; 4]);
    assert_eq!(&values[(6 * 4)..(7 * 4)], &[1.0, 11.0, 21.0, 0.5]);

    let roi = Roi::new(1, 1, 3, 2).unwrap();
    let region = ExrDecoder::new()
        .decode_bytes(&bytes, &ExrDecodeRequest::new(limits()).region(roi))
        .unwrap();
    let region_pixels = region.pixels.unwrap();
    assert_eq!(region_pixels.origin, [-1, 0]);
    let ExrSampleData::F32(region_values) = region_pixels.samples else {
        panic!("f32")
    };
    let expected = [&values[(6 * 4)..(9 * 4)], &values[(11 * 4)..(14 * 4)]].concat();
    assert_eq!(region_values, expected);
}

#[test]
fn explicit_channel_mapping_and_header_only_mode_are_deterministic() {
    let bytes = rgba_f16(Compression::ZIP16, Blocks::ScanLines);
    let request = ExrDecodeRequest::new(limits()).channels(ExrChannelMapping::Gray {
        gray: "G".to_owned(),
        alpha: Some("A".to_owned()),
    });
    let gray = ExrDecoder::new().decode_bytes(&bytes, &request).unwrap();
    assert_eq!(gray.pixels.unwrap().layout, ChannelLayout::GrayA);
    assert_eq!(gray.receipt.channels, ["G", "A"]);

    let header = ExrDecoder::new()
        .decode_bytes(&bytes, &ExrDecodeRequest::new(limits()).header())
        .unwrap();
    assert!(header.pixels.is_none());
    assert!(header.receipt.header_only);
    assert_eq!(header.receipt.output_bytes, 0);
}

#[test]
fn limits_malformed_chunk_table_and_unsupported_uint_are_precise() {
    let bytes = rgba_f16(Compression::ZIP1, Blocks::ScanLines);
    let mut constrained = limits();
    constrained.max_channels_per_part = 3;
    assert!(matches!(
        ExrDecoder::new().inspect_bytes(&bytes, constrained),
        Err(ExrDecodeError::Limit {
            kind: "channel count",
            actual: 4,
            limit: 3
        })
    ));

    let mut malformed = bytes.clone();
    let header = ExrDecoder::new().inspect_bytes(&bytes, limits()).unwrap();
    let chunk_count = usize::try_from(header.parts[0].chunk_count).unwrap();
    let table_start = find_chunk_table_start(&bytes);
    assert!(chunk_count > 0);
    malformed[table_start..table_start + 8].copy_from_slice(&0_u64.to_le_bytes());
    assert!(matches!(
        ExrDecoder::new().inspect_bytes(&malformed, limits()),
        Err(ExrDecodeError::Malformed(message)) if message.contains("chunk offset")
    ));

    let uint = SpecificChannels::build()
        .with_channel::<u32>("Y")
        .with_pixel_fn(|_| (7_u32,));
    let bytes = write(&Image::from_layer(Layer::new(
        (1, 1),
        LayerAttributes::named("uint"),
        encoding(Compression::RLE, Blocks::ScanLines),
        uint,
    )));
    assert!(matches!(
        ExrDecoder::new().decode_bytes(&bytes, &ExrDecodeRequest::new(limits())),
        Err(ExrDecodeError::UnsupportedSampleType { channel }) if channel == "Y"
    ));
}

#[test]
fn cancellation_and_source_mutation_publish_no_result() {
    let bytes = rgba_f16(Compression::ZIP1, Blocks::ScanLines);
    let cancellation = RawCancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        ExrDecoder::new().decode_bytes(
            &bytes,
            &ExrDecodeRequest::new(limits()).with_cancellation(cancellation)
        ),
        Err(ExrDecodeError::Cancelled)
    );

    let source = ChangingSource {
        bytes,
        revisions: AtomicUsize::new(0),
    };
    assert_eq!(
        ExrDecoder::new().decode_source(&source, &ExrDecodeRequest::new(limits())),
        Err(ExrDecodeError::Source(RawSourceError::Changed))
    );
}

fn find_chunk_table_start(bytes: &[u8]) -> usize {
    let mut cursor = 8;
    loop {
        let name_end = bytes[cursor..].iter().position(|byte| *byte == 0).unwrap();
        cursor += name_end + 1;
        if name_end == 0 {
            return cursor;
        }
        let kind_end = bytes[cursor..].iter().position(|byte| *byte == 0).unwrap();
        cursor += kind_end + 1;
        let size = usize::try_from(i32::from_le_bytes(
            bytes[cursor..cursor + 4].try_into().unwrap(),
        ))
        .unwrap();
        cursor += 4 + size;
    }
}

struct ChangingSource {
    bytes: Vec<u8>,
    revisions: AtomicUsize,
}

impl RawByteSource for ChangingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        Ok(u64::try_from(self.bytes.len()).unwrap())
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        Ok([u8::try_from(self.revisions.fetch_add(1, Ordering::Relaxed)).unwrap_or(u8::MAX); 32])
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::LengthConversion)?;
        let end = start + buffer.len();
        buffer.copy_from_slice(self.bytes.get(start..end).ok_or(RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?);
        Ok(())
    }
}

fn as_f32(value: usize) -> f32 {
    f32::from(u16::try_from(value).expect("small generated fixture coordinate"))
}
