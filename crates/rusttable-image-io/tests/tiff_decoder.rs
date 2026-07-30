use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_image::{DecodeLimits, Orientation, Roi};
use rusttable_image_io::{
    ImageDecoderRegistry, RawByteSource, RawCancellationToken, RawSourceError, TIFF_BACKEND_ID,
    TiffChunkKind, TiffContainer, TiffDecodeError, TiffDecodeLimits, TiffDecodeRequest,
    TiffDecoder, TiffSampleData,
};
use tiff::encoder::colortype::{Gray8, Gray16, Gray32, Gray32Float, GrayI16, RGBA8};
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
    assert_eq!(legacy.pixels(), &[10, 20, 30, 40, 50, 60, 70, 80]);
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
