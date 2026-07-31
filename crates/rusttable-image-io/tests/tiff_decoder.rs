use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_image::{DecodeLimits, Orientation, Roi};
use rusttable_image_io::tiff::{TiffNativeSampleFormat, half_bits_to_f32};
use rusttable_image_io::{
    ImageDecoderRegistry, RawByteSource, RawCancellationToken, RawSourceError, TIFF_BACKEND_ID,
    TiffChunkKind, TiffContainer, TiffDecodeError, TiffDecodeLimits, TiffDecodeRequest,
    TiffDecoder, TiffPhotometric, TiffSampleData, TiffSampleFormat, TiffStorageLayout,
};
use tiff::encoder::colortype::{Gray8, Gray16, Gray32, Gray32Float, GrayI16, RGB8, RGBA8};
use tiff::encoder::{Compression, DeflateLevel, Predictor, TiffEncoder};
use tiff::tags::Tag;

fn limits() -> TiffDecodeLimits {
    TiffDecodeLimits {
        max_source_bytes: 1_000_000,
        max_width: 64,
        max_height: 64,
        max_pixels: 4_096,
        max_decoded_bytes: 64 * 1024,
        max_decompressed_bytes: 64 * 1024,
        max_temporary_bytes: 64 * 1024,
        max_metadata_bytes: 4_096,
        max_ifd_value_bytes: 4_096,
        max_pages: 16,
        max_tags: 1_024,
        max_chunks: 1_024,
    }
}

fn gray8(
    width: u32,
    height: u32,
    samples: &[u8],
    compression: Compression,
    predictor: Predictor,
) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor)
            .expect("encoder")
            .with_compression(compression)
            .with_predictor(predictor);
        let mut image = encoder
            .new_image::<Gray8>(width, height)
            .expect("gray image");
        image.rows_per_strip(2).expect("strip rows");
        image.write_data(samples).expect("gray data");
    }
    cursor.into_inner()
}

fn decode(bytes: &[u8]) -> rusttable_image_io::TiffDecodeResult {
    TiffDecoder::new()
        .decode_bytes(bytes, &TiffDecodeRequest::new(limits()))
        .expect("TIFF decodes")
}

fn decode_native(bytes: &[u8]) -> rusttable_image_io::tiff::TiffNativeRaster {
    TiffDecoder::new()
        .decode_native_bytes(bytes, &TiffDecodeRequest::new(limits()))
        .expect("native TIFF decodes")
}

fn rgb8(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cursor)
        .expect("encoder")
        .write_image::<RGB8>(width, height, samples)
        .expect("RGB data");
    cursor.into_inner()
}

fn rgba8(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cursor)
        .expect("encoder")
        .write_image::<RGBA8>(width, height, samples)
        .expect("RGBA data");
    cursor.into_inner()
}

fn gray32_float(width: u32, height: u32, samples: &[f32]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cursor)
        .expect("encoder")
        .write_image::<Gray32Float>(width, height, samples)
        .expect("float data");
    cursor.into_inner()
}

fn gray16(width: u32, height: u32, samples: &[u16]) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    TiffEncoder::new(&mut cursor)
        .expect("encoder")
        .write_image::<Gray16>(width, height, samples)
        .expect("16-bit data");
    cursor.into_inner()
}

fn patch_short_tag(bytes: &mut [u8], wanted: u16, value: u16) {
    assert_eq!(&bytes[..2], b"II", "test helper expects little-endian TIFF");
    let ifd = u32::from_le_bytes(bytes[4..8].try_into().expect("IFD offset")) as usize;
    let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().expect("IFD count"));
    for index in 0..usize::from(count) {
        let entry = ifd + 2 + index * 12;
        let tag = u16::from_le_bytes(bytes[entry..entry + 2].try_into().expect("tag"));
        if tag == wanted {
            assert_eq!(
                u16::from_le_bytes(bytes[entry + 2..entry + 4].try_into().expect("type")),
                3,
                "test tag should be SHORT",
            );
            bytes[entry + 8..entry + 10].copy_from_slice(&value.to_le_bytes());
            return;
        }
    }
    panic!("tag {wanted} is absent");
}

fn half_float_tiff(samples: &[u16]) -> Vec<u8> {
    let ifd_offset = 8_u32;
    let entry_count = 10_u16;
    let data_offset = usize::try_from(ifd_offset).expect("IFD offset fits")
        + 2
        + usize::from(entry_count) * 12
        + 4;
    let mut bytes = vec![0_u8; data_offset + samples.len() * 2];
    bytes[..2].copy_from_slice(b"II");
    bytes[2..4].copy_from_slice(&42_u16.to_le_bytes());
    bytes[4..8].copy_from_slice(&ifd_offset.to_le_bytes());
    bytes[8..10].copy_from_slice(&entry_count.to_le_bytes());
    let mut entry = 10;
    let mut write_entry = |tag: u16, field_type: u16, value: u32| {
        bytes[entry..entry + 2].copy_from_slice(&tag.to_le_bytes());
        bytes[entry + 2..entry + 4].copy_from_slice(&field_type.to_le_bytes());
        bytes[entry + 4..entry + 8].copy_from_slice(&1_u32.to_le_bytes());
        bytes[entry + 8..entry + 12].copy_from_slice(&value.to_le_bytes());
        entry += 12;
    };
    write_entry(256, 3, u32::try_from(samples.len()).expect("width"));
    write_entry(257, 3, 1);
    write_entry(258, 3, 16);
    write_entry(259, 3, 1);
    write_entry(262, 3, 1);
    write_entry(273, 4, u32::try_from(data_offset).expect("sample offset"));
    write_entry(277, 3, 1);
    write_entry(278, 3, 1);
    write_entry(
        279,
        4,
        u32::try_from(samples.len() * 2).expect("sample bytes"),
    );
    write_entry(339, 3, 3);
    bytes[entry..entry + 4].copy_from_slice(&0_u32.to_le_bytes());
    for (index, sample) in samples.iter().copied().enumerate() {
        let offset = data_offset + index * 2;
        bytes[offset..offset + 2].copy_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[test]
fn classic_and_bigtiff_decode_exact_unsigned_samples() {
    let classic = gray8(
        3,
        2,
        &[1, 2, 3, 4, 5, 6],
        Compression::Uncompressed,
        Predictor::None,
    );
    let mut big = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new_big(&mut big).expect("BigTIFF encoder");
        encoder
            .write_image::<Gray16>(2, 1, &[0x1234, 0xabcd])
            .expect("BigTIFF image");
    }

    let classic = decode(&classic);
    let big = decode(&big.into_inner());

    assert_eq!(classic.header.container, TiffContainer::Classic);
    assert_eq!(classic.page.chunks.kind, TiffChunkKind::Strips);
    assert_eq!(
        classic.pixels.unwrap().samples,
        TiffSampleData::U8(vec![1, 2, 3, 4, 5, 6])
    );
    assert_eq!(big.header.container, TiffContainer::BigTiff);
    assert_eq!(
        big.pixels.unwrap().samples,
        TiffSampleData::U16(vec![0x1234, 0xabcd])
    );
}

#[test]
fn signed_unsigned32_and_float32_keep_native_precision() {
    let mut signed = Cursor::new(Vec::new());
    let mut unsigned = Cursor::new(Vec::new());
    let mut float = Cursor::new(Vec::new());
    TiffEncoder::new(&mut signed)
        .unwrap()
        .write_image::<GrayI16>(3, 1, &[-32_000, -1, 32_000])
        .unwrap();
    TiffEncoder::new(&mut unsigned)
        .unwrap()
        .write_image::<Gray32>(2, 1, &[1, u32::MAX - 1])
        .unwrap();
    TiffEncoder::new(&mut float)
        .unwrap()
        .write_image::<Gray32Float>(3, 1, &[-1.25, 0.5, 42.0])
        .unwrap();

    assert_eq!(
        decode(&signed.into_inner()).pixels.unwrap().samples,
        TiffSampleData::I16(vec![-32_000, -1, 32_000])
    );
    assert_eq!(
        decode(&unsigned.into_inner()).pixels.unwrap().samples,
        TiffSampleData::U32(vec![1, u32::MAX - 1])
    );
    assert_eq!(
        decode(&float.into_inner()).pixels.unwrap().samples,
        TiffSampleData::F32(vec![-1.25, 0.5, 42.0])
    );
}

#[test]
fn supported_compressions_and_horizontal_predictor_round_trip() {
    let expected: Vec<u8> = (0..35).map(|value| value * 7).collect();
    let cases = [
        (Compression::Uncompressed, Predictor::None),
        (Compression::Packbits, Predictor::None),
        (Compression::Lzw, Predictor::None),
        (Compression::Lzw, Predictor::Horizontal),
        (
            Compression::Deflate(DeflateLevel::Balanced),
            Predictor::Horizontal,
        ),
    ];
    for (compression, predictor) in cases {
        let result = decode(&gray8(7, 5, &expected, compression, predictor));
        assert_eq!(
            result.pixels.unwrap().samples,
            TiffSampleData::U8(expected.clone())
        );
    }
}

#[test]
fn region_is_identical_to_cropping_full_decode_across_strip_edges() {
    let samples: Vec<u8> = (0..30).collect();
    let bytes = gray8(6, 5, &samples, Compression::Lzw, Predictor::Horizontal);
    let full = decode(&bytes).pixels.unwrap();
    let roi = Roi::new(2, 1, 3, 3).unwrap();
    let region = TiffDecoder::new()
        .decode_bytes(&bytes, &TiffDecodeRequest::new(limits()).region(roi))
        .unwrap();

    assert_eq!(full.samples, TiffSampleData::U8(samples));
    assert_eq!(
        region.pixels.unwrap().samples,
        TiffSampleData::U8(vec![8, 9, 10, 14, 15, 16, 20, 21, 22])
    );
    assert_eq!(region.receipt.region, Some(roi));
}

#[test]
fn largest_full_resolution_page_is_default_and_explicit_page_is_receipted() {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor).unwrap();
        let mut reduced = encoder.new_image::<Gray8>(1, 1).unwrap();
        reduced
            .encoder()
            .write_tag(Tag::NewSubfileType, 1_u32)
            .unwrap();
        reduced.write_data(&[9]).unwrap();
        encoder
            .write_image::<Gray8>(3, 2, &[1, 2, 3, 4, 5, 6])
            .unwrap();
        encoder.write_image::<Gray8>(2, 2, &[7, 8, 9, 10]).unwrap();
    }
    let bytes = cursor.into_inner();
    let default = decode(&bytes);
    let explicit = TiffDecoder::new()
        .decode_bytes(&bytes, &TiffDecodeRequest::new(limits()).page(0))
        .unwrap();

    assert_eq!(default.header.pages.len(), 3);
    assert_eq!(
        (
            default.page.dimensions.width(),
            default.page.dimensions.height()
        ),
        (3, 2)
    );
    assert_eq!(default.receipt.page_index, 1);
    assert_eq!(explicit.receipt.page_index, 0);
    assert_eq!(
        explicit.pixels.unwrap().samples,
        TiffSampleData::U8(vec![9])
    );
}

#[test]
fn orientation_alpha_and_metadata_are_inventory_not_transforms() {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor).unwrap();
        let mut image = encoder.new_image::<RGBA8>(2, 1).unwrap();
        image.encoder().write_tag(Tag::Orientation, 6_u16).unwrap();
        image
            .encoder()
            .write_tag(Tag::IccProfile, &[1_u8, 2, 3, 4][..])
            .unwrap();
        image.write_data(&[10, 20, 30, 40, 50, 60, 70, 80]).unwrap();
    }
    let bytes = cursor.into_inner();
    let result = decode(&bytes);

    assert_eq!(result.page.orientation, Orientation::Rotate90);
    assert_eq!(result.page.metadata.icc.unwrap().length, 4);
    assert_eq!(
        result.pixels.unwrap().samples,
        TiffSampleData::U8(vec![10, 20, 30, 40, 50, 60, 70, 80])
    );
    assert_eq!(result.receipt.backend, TIFF_BACKEND_ID);

    let limits = DecodeLimits::new(1_000_000, 2, 1, 2, 8).unwrap();
    let registry = ImageDecoderRegistry::standard();
    let frame = registry.decode_frame_bytes(&bytes, limits).unwrap();
    assert_eq!(
        frame.image().descriptor().orientation(),
        Orientation::Rotate90
    );
    assert_eq!(
        frame
            .image()
            .descriptor()
            .orientation()
            .output_dimensions(frame.image().descriptor().dimensions()),
        rusttable_image::ImageDimensions::new(1, 2).unwrap()
    );
    let legacy = registry.decode_bytes(&bytes, limits).unwrap();
    assert_eq!(legacy.source_orientation(), Orientation::Rotate90);
    assert_eq!(legacy.pixels(), &[10, 20, 30, 255, 50, 60, 70, 255]);
}

#[test]
fn graph_and_allocation_limits_fail_before_backend_decode() {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor).unwrap();
        encoder.write_image::<Gray8>(1, 1, &[1]).unwrap();
        encoder.write_image::<Gray8>(1, 1, &[2]).unwrap();
    }
    let mut constrained = limits();
    constrained.max_pages = 1;
    assert!(matches!(
        TiffDecoder::new().inspect_bytes(&cursor.into_inner(), constrained),
        Err(TiffDecodeError::Limit {
            kind: "page count",
            ..
        })
    ));

    let bytes = gray8(4, 4, &[0; 16], Compression::Uncompressed, Predictor::None);
    constrained = limits();
    constrained.max_decoded_bytes = 15;
    assert!(matches!(
        TiffDecoder::new().inspect_bytes(&bytes, constrained),
        Err(TiffDecodeError::Limit {
            kind: "decoded bytes",
            ..
        })
    ));
}

#[test]
fn cancellation_and_source_mutation_publish_no_result() {
    let bytes = gray8(
        2,
        2,
        &[1, 2, 3, 4],
        Compression::Uncompressed,
        Predictor::None,
    );
    let cancellation = RawCancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        TiffDecoder::new().decode_bytes(
            &bytes,
            &TiffDecodeRequest::new(limits()).with_cancellation(cancellation)
        ),
        Err(TiffDecodeError::Cancelled)
    );

    let source = ChangingSource {
        bytes,
        revisions: AtomicUsize::new(0),
    };
    assert_eq!(
        TiffDecoder::new().decode_source(&source, &TiffDecodeRequest::new(limits())),
        Err(TiffDecodeError::Source(RawSourceError::Changed))
    );
}

struct ChangingSource {
    bytes: Vec<u8>,
    revisions: AtomicUsize,
}

impl RawByteSource for ChangingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        u64::try_from(self.bytes.len()).map_err(|_| RawSourceError::LengthConversion)
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        let revision = u8::from(self.revisions.fetch_add(1, Ordering::Relaxed) != 0);
        Ok([revision; 32])
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(RawSourceError::Read {
                offset,
                requested: buffer.len(),
            })?;
        let source = self.bytes.get(start..end).ok_or(RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        buffer.copy_from_slice(source);
        Ok(())
    }
}

#[test]
fn native_scanline_output_is_four_float_channels_with_spare_alpha() {
    let native = decode_native(&rgb8(2, 1, &[255, 0, 128, 0, 64, 255]));

    assert_eq!(native.sample_format, TiffNativeSampleFormat::Unsigned8);
    assert!(!native.is_hdr());
    assert_eq!(native.pixels[3].to_bits(), 0.0_f32.to_bits());
    assert_eq!(native.pixels[7].to_bits(), 0.0_f32.to_bits());
    assert!((native.pixels[0] - 1.0).abs() < f32::EPSILON);
    assert!((native.pixels[2] - (128.0 / 255.0)).abs() < f32::EPSILON);
    assert!((native.pixels[5] - (64.0 / 255.0)).abs() < f32::EPSILON);
}

#[test]
fn native_reader_replicates_gray_and_only_inverts_8_bit_white_is_zero() {
    let mut white_is_zero = gray8(2, 1, &[0, 255], Compression::Uncompressed, Predictor::None);
    patch_short_tag(&mut white_is_zero, 262, 0);
    let native = decode_native(&white_is_zero);

    assert_eq!(native.pixels, vec![1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn native_float32_classifies_hdr_and_preserves_stored_values() {
    let native = decode_native(&gray32_float(2, 1, &[-1.25, 42.0]));

    assert_eq!(native.sample_format, TiffNativeSampleFormat::Float32);
    assert!(native.is_hdr());
    assert_eq!(
        native.pixels,
        vec![-1.25, -1.25, -1.25, 0.0, 42.0, 42.0, 42.0, 0.0]
    );
}

#[test]
fn native_uint16_scales_to_float_and_is_ldr() {
    let native = decode_native(&gray16(2, 1, &[0, u16::MAX]));

    assert_eq!(native.sample_format, TiffNativeSampleFormat::Unsigned16);
    assert!(!native.is_hdr());
    assert!(native.pixels[0].abs() < f32::EPSILON);
    assert!((native.pixels[4] - 1.0).abs() < f32::EPSILON);
    assert!(native.pixels[3].abs() < f32::EPSILON);
    assert!(native.pixels[7].abs() < f32::EPSILON);
}

#[test]
fn native_half_fallback_preserves_zero_subnormal_infinity_nan_and_sign() {
    let patterns = [
        0x0000, 0x8000, 0x0001, 0x8001, 0x3c00, 0xbc00, 0x7c00, 0xfc00, 0x7e00, 0xfe00,
    ];
    for bits in patterns {
        let actual = half_bits_to_f32(bits);
        let expected = half::f16::from_bits(bits).to_f32();
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "half pattern {bits:#06x}"
        );
    }

    let native = decode_native(&half_float_tiff(&[0x0000, 0x3c00, 0x7c00, 0x7e00]));
    assert_eq!(native.sample_format, TiffNativeSampleFormat::HalfFloat16);
    assert!(native.is_hdr());
    assert_eq!(native.pixels[0].to_bits(), 0.0_f32.to_bits());
    assert!((native.pixels[4] - 1.0).abs() < f32::EPSILON);
    assert!(native.pixels[8].is_infinite());
    assert!(native.pixels[12].is_nan());
    assert!(
        native
            .pixels
            .chunks(4)
            .all(|pixel| pixel[3].to_bits() == 0.0_f32.to_bits())
    );
}

#[test]
fn native_format_gate_matches_tiff_loader_fallback_matrix() {
    let page = decode(&gray8(
        1,
        1,
        &[17],
        Compression::Uncompressed,
        Predictor::None,
    ))
    .page;
    assert_eq!(
        page.native_sample_format()
            .expect("8-bit unsigned is native"),
        TiffNativeSampleFormat::Unsigned8
    );

    let mut tiled = page.clone();
    tiled.chunks.kind = TiffChunkKind::Tiles;
    assert!(matches!(
        tiled.native_sample_format(),
        Err(TiffDecodeError::Unsupported {
            feature: "tiled TIFF",
            value: 1
        })
    ));

    for (photometric, value) in [(TiffPhotometric::Palette, 3), (TiffPhotometric::Cmyk, 5)] {
        let mut unsupported = page.clone();
        unsupported.photometric = photometric;
        assert!(matches!(
            unsupported.native_sample_format(),
            Err(TiffDecodeError::Unsupported { feature: _, value: actual }) if actual == value
        ));
    }

    let mut planar = page.clone();
    planar.samples_per_pixel = 2;
    planar.storage = TiffStorageLayout::Planar;
    assert!(matches!(
        planar.native_sample_format(),
        Err(TiffDecodeError::Unsupported {
            feature: "non-chunky planar configuration",
            value: 2
        })
    ));

    for bits in [1, 2, 4, 32] {
        let mut unsupported = page.clone();
        unsupported.bits_per_sample[0] = bits;
        assert!(matches!(
            unsupported.native_sample_format(),
            Err(TiffDecodeError::Unsupported { feature: "native TIFF sample format", value }) if value == u64::from(bits)
        ));
    }
    let mut signed = page.clone();
    signed.sample_formats[0] = TiffSampleFormat::Signed;
    assert!(matches!(
        signed.native_sample_format(),
        Err(TiffDecodeError::Unsupported {
            feature: "native TIFF sample format",
            value: 8
        })
    ));
}

#[test]
fn sampleformat_void_is_normalized_to_unsigned_for_native_dispatch() {
    let mut bytes = gray8(1, 1, &[17], Compression::Uncompressed, Predictor::None);
    patch_short_tag(&mut bytes, 339, 4);
    let page = decode(&bytes).page;

    assert_eq!(page.sample_formats, vec![TiffSampleFormat::Unsigned]);
    assert_eq!(
        decode_native(&bytes).sample_format,
        TiffNativeSampleFormat::Unsigned8
    );
}

#[test]
fn native_lab_conversion_remains_fail_closed_until_color_owner_exists() {
    let mut bytes = gray8(1, 1, &[17], Compression::Uncompressed, Predictor::None);
    patch_short_tag(&mut bytes, 262, 8);

    assert!(matches!(
        TiffDecoder::new().decode_native_bytes(&bytes, &TiffDecodeRequest::new(limits())),
        Err(TiffDecodeError::Unsupported {
            feature: "Lab color conversion",
            value: 8
        })
    ));
}

#[test]
fn native_alpha_is_discarded_into_the_spare_channel() {
    let native = decode_native(&rgba8(1, 1, &[10, 20, 30, 255]));

    let expected = [10.0_f32 / 255.0, 20.0 / 255.0, 30.0 / 255.0];
    assert!(
        native.pixels[..3]
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < f32::EPSILON)
    );
    assert_eq!(native.pixels[3].to_bits(), 0.0_f32.to_bits());
}
