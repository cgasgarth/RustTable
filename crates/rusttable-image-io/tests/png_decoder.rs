use std::sync::atomic::{AtomicU8, Ordering};

use flate2::{Compression, write::ZlibEncoder};
use rusttable_image_io::png::{PngEncodeOptions, PngEncoder};
use rusttable_image_io::{
    PngBitDepth, PngColorType, PngDecodeError, PngDecodeLimits, PngDecodeRequest, PngDecoder,
    PngPixelData, RawByteSource, RawCancellationToken, RawSourceError,
};

fn limits(bytes: u64) -> PngDecodeLimits {
    let mut limits = PngDecodeLimits::new(bytes, 64, 64, 4096, 16 * 1024).expect("limits");
    limits.max_chunk_bytes = 1024 * 1024;
    limits.max_compressed_bytes = bytes;
    limits.max_decompressed_bytes = 32 * 1024;
    limits
}

fn png(
    width: u32,
    height: u32,
    color: PngColorType,
    depth: PngBitDepth,
    rows: &[Vec<u8>],
) -> Vec<u8> {
    png_with_chunks(width, height, color, depth, &[], rows, false)
}

fn png_with_chunks(
    width: u32,
    height: u32,
    color: PngColorType,
    depth: PngBitDepth,
    chunks: &[([u8; 4], Vec<u8>)],
    rows: &[Vec<u8>],
    adam7: bool,
) -> Vec<u8> {
    let color_code = match color {
        PngColorType::Grayscale => 0,
        PngColorType::Rgb => 2,
        PngColorType::Indexed => 3,
        PngColorType::GrayscaleAlpha => 4,
        PngColorType::Rgba => 6,
    };
    let raw = if adam7 {
        adam7_rows(width, height, color, depth, rows)
    } else {
        rows.iter()
            .flat_map(|row| std::iter::once(0).chain(row.iter().copied()))
            .collect()
    };
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut compressed, &raw).expect("zlib");
    let compressed = compressed.finish().expect("zlib finish");
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[depth.bits(), color_code, 0, 0, u8::from(adam7)]);
    chunk(&mut output, *b"IHDR", &ihdr);
    for (kind, data) in chunks {
        chunk(&mut output, *kind, data);
    }
    chunk(&mut output, *b"IDAT", &compressed);
    chunk(&mut output, *b"IEND", &[]);
    output
}

fn chunk(output: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    output.extend_from_slice(&(u32::try_from(data.len()).expect("chunk length")).to_be_bytes());
    output.extend_from_slice(&kind);
    output.extend_from_slice(data);
    output.extend_from_slice(&crc32(kind, data).to_be_bytes());
}

fn crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in kind.into_iter().chain(data.iter().copied()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adam7_rows(
    width: u32,
    height: u32,
    color: PngColorType,
    depth: PngBitDepth,
    rows: &[Vec<u8>],
) -> Vec<u8> {
    let channels = usize::from(color.channels());
    let bits = usize::from(depth.bits());
    let x_starts = [0_u32, 4, 0, 2, 0, 1, 0];
    let y_starts = [0_u32, 0, 4, 0, 2, 0, 1];
    let x_steps = [8_u32, 8, 4, 4, 2, 2, 1];
    let y_steps = [8_u32, 8, 8, 4, 4, 2, 2];
    let mut output = Vec::new();
    for pass in 0..7 {
        let pw = width.saturating_sub(x_starts[pass]).div_ceil(x_steps[pass]);
        let ph = height
            .saturating_sub(y_starts[pass])
            .div_ceil(y_steps[pass]);
        if pw == 0 || ph == 0 {
            continue;
        }
        for pass_y in 0..ph {
            output.push(0);
            let y = y_starts[pass] + pass_y * y_steps[pass];
            let mut packed =
                vec![0; (usize::try_from(pw).expect("width") * channels * bits).div_ceil(8)];
            for pass_x in 0..pw {
                let x = x_starts[pass] + pass_x * x_steps[pass];
                let source = &rows[usize::try_from(y).expect("row")];
                let source_bit = usize::try_from(x).expect("x") * channels * bits;
                let target_bit = usize::try_from(pass_x).expect("pass x") * channels * bits;
                for bit in 0..channels * bits {
                    let value = (source[source_bit / 8] >> (7 - (source_bit + bit) % 8)) & 1;
                    packed[target_bit / 8] |= value << (7 - (target_bit + bit) % 8);
                }
            }
            output.extend_from_slice(&packed);
        }
    }
    output
}

#[test]
fn decodes_gray_subbyte_and_expands_samples() {
    let source = png(
        4,
        1,
        PngColorType::Grayscale,
        PngBitDepth::Two,
        &[vec![0b00_01_10_11]],
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)))
        .expect("gray");
    assert_eq!(result.header.bit_depth, PngBitDepth::Two);
    assert_eq!(
        result.pixels,
        Some(PngPixelData::GrayU8 {
            dimensions: rusttable_image::ImageDimensions::new(4, 1).unwrap(),
            samples: vec![0, 85, 170, 255]
        })
    );
}

#[test]
fn decodes_palette_transparency_and_trns() {
    let source = png_with_chunks(
        2,
        1,
        PngColorType::Indexed,
        PngBitDepth::Eight,
        &[(*b"PLTE", vec![255, 0, 0, 0, 255, 0]), (*b"tRNS", vec![17])],
        &[vec![0, 1]],
        false,
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)))
        .expect("palette");
    assert_eq!(
        result.pixels,
        Some(PngPixelData::RgbaU8 {
            dimensions: rusttable_image::ImageDimensions::new(2, 1).unwrap(),
            samples: vec![255, 0, 0, 17, 0, 255, 0, 255]
        })
    );
}

#[test]
fn decodes_16_bit_samples_as_host_order_values() {
    let source = png(
        1,
        1,
        PngColorType::Rgba,
        PngBitDepth::Sixteen,
        &[vec![0x12, 0x34, 0xab, 0xcd, 0x00, 0x01, 0xff, 0xfe]],
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)))
        .expect("16-bit");
    assert_eq!(
        result.pixels,
        Some(PngPixelData::RgbaU16 {
            dimensions: rusttable_image::ImageDimensions::new(1, 1).unwrap(),
            samples: vec![0x1234, 0xabcd, 1, 0xfffe]
        })
    );
    assert_eq!(
        result
            .image
            .expect("typed image")
            .descriptor()
            .format()
            .byte_order(),
        rusttable_image::ByteOrder::Native
    );
}

#[test]
fn decodes_adam7_without_changing_row_major_order() {
    let rows = vec![vec![0, 64, 128], vec![192, 255, 32]];
    let source = png_with_chunks(
        3,
        2,
        PngColorType::Grayscale,
        PngBitDepth::Eight,
        &[],
        &rows,
        true,
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)))
        .expect("adam7");
    assert_eq!(
        result.pixels,
        Some(PngPixelData::GrayU8 {
            dimensions: rusttable_image::ImageDimensions::new(3, 2).unwrap(),
            samples: vec![0, 64, 128, 192, 255, 32]
        })
    );
}

#[test]
fn rejects_crc_order_critical_chunk_and_invalid_palette() {
    let valid = png(
        1,
        1,
        PngColorType::Grayscale,
        PngBitDepth::Eight,
        &[vec![7]],
    );
    let mut crc = valid.clone();
    let last = crc.len() - 5;
    crc[last] ^= 1;
    assert!(
        matches!(PngDecoder::new().inspect_bytes(&crc, limits(4096)), Err(PngDecodeError::Malformed(message)) if message.contains("CRC"))
    );

    let after_idat = {
        let mut value = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(
            &mut value,
            *b"IHDR",
            &[
                1_u32.to_be_bytes().as_slice(),
                1_u32.to_be_bytes().as_slice(),
                &[8, 0, 0, 0, 0],
            ]
            .concat(),
        );
        chunk(&mut value, *b"IDAT", &[1, 2, 3]);
        chunk(&mut value, *b"PLTE", &[0, 0, 0]);
        chunk(&mut value, *b"IEND", &[]);
        value
    };
    assert!(
        matches!(PngDecoder::new().inspect_bytes(&after_idat, limits(4096)), Err(PngDecodeError::Malformed(message)) if message.contains("after IDAT"))
    );

    let unknown_critical = {
        let mut value = valid[..33].to_vec();
        chunk(&mut value, *b"ABCD", &[]);
        value
    };
    assert!(
        matches!(PngDecoder::new().inspect_bytes(&unknown_critical, limits(4096)), Err(PngDecodeError::Malformed(message)) if message.contains("critical"))
    );

    let invalid_palette = png_with_chunks(
        1,
        1,
        PngColorType::Indexed,
        PngBitDepth::One,
        &[(*b"PLTE", vec![0, 0, 0, 1, 1, 1, 2, 2, 2])],
        &[vec![0]],
        false,
    );
    assert!(
        matches!(PngDecoder::new().inspect_bytes(&invalid_palette, limits(4096)), Err(PngDecodeError::Malformed(message)) if message.contains("palette"))
    );
}

#[test]
fn enforces_output_and_chunk_limits_before_publication() {
    let source = png(2, 1, PngColorType::Rgba, PngBitDepth::Eight, &[vec![0; 8]]);
    let mut constrained = limits(4096);
    constrained.max_decoded_bytes = 4;
    assert!(matches!(
        PngDecoder::new().decode_bytes(&source, &PngDecodeRequest::new(constrained)),
        Err(PngDecodeError::Limit {
            kind: "decoded bytes",
            ..
        })
    ));
    let mut chunk_limited = limits(4096);
    chunk_limited.max_chunk_bytes = 4;
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&source, chunk_limited),
        Err(PngDecodeError::Limit {
            kind: "chunk bytes",
            ..
        })
    ));
}

#[test]
fn preserves_cicp_precedence_and_bounded_metadata_inventory() {
    let source = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[
            (*b"cICP", vec![1, 13, 0, 1]),
            (*b"gAMA", 45_455_u32.to_be_bytes().to_vec()),
            (*b"pHYs", vec![0, 0, 1, 44, 0, 0, 2, 88, 1]),
            (*b"tEXt", b"Comment\0PNG metadata".to_vec()),
            (*b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<xmp/>".to_vec()),
        ],
        &[vec![9, 8, 7]],
        false,
    );
    let header = PngDecoder::new()
        .inspect_bytes(&source, limits(4096))
        .expect("metadata PNG");
    assert_eq!(
        header.color_encoding,
        rusttable_image::ColorEncoding::SrgbD65
    );
    assert_eq!(
        header.metadata.cicp,
        Some(rusttable_image_io::png::PngCicp {
            color_primaries: 1,
            transfer_characteristics: 13,
            matrix_coefficients: 0,
            full_range: 1,
        })
    );
    assert_eq!(header.metadata.gamma, Some(45_455));
    assert_eq!(
        header.metadata.physical_resolution,
        Some(rusttable_image_io::PngPhysicalResolution {
            x_pixels_per_unit: 300,
            y_pixels_per_unit: 600,
            unit_is_meter: true,
        })
    );
    assert_eq!(header.metadata.xmp_chunks, 1);
    assert_eq!(header.metadata.text.len(), 2);
}

#[test]
fn validates_cicp_supported_unsupported_malformed_and_duplicate_forms() {
    let unsupported = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"cICP", vec![9, 14, 0, 1])],
        &[vec![1, 2, 3]],
        false,
    );
    let header = PngDecoder::new()
        .inspect_bytes(&unsupported, limits(4096))
        .expect("structurally valid unsupported cICP");
    assert_eq!(
        header.metadata.cicp,
        Some(rusttable_image_io::PngCicp {
            color_primaries: 9,
            transfer_characteristics: 14,
            matrix_coefficients: 0,
            full_range: 1,
        })
    );
    assert_eq!(
        header.color_encoding,
        rusttable_image::ColorEncoding::Unspecified
    );

    let malformed = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"cICP", vec![9, 16, 0])],
        &[vec![1, 2, 3]],
        false,
    );
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&malformed, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("cICP")
    ));

    let duplicate = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"cICP", vec![1, 13, 0, 1]), (*b"cICP", vec![1, 13, 0, 1])],
        &[vec![1, 2, 3]],
        false,
    );
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&duplicate, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("duplicate cICP")
    ));
}

#[test]
fn rejects_truncation_duplicate_iend_and_invalid_compressed_text() {
    let source = png(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[vec![1, 2, 3]],
    );
    let mut truncated = source.clone();
    truncated.pop();
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&truncated, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("truncated")
    ));

    let mut duplicate = source;
    chunk(&mut duplicate, *b"IEND", &[]);
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&duplicate, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("IEND")
    ));

    let invalid_itxt = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"iTXt", b"keyword\0\0\x01\0\0text".to_vec())],
        &[vec![1, 2, 3]],
        false,
    );
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&invalid_itxt, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("iTXt compression")
    ));

    let malformed_compressed_text = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"zTXt", b"keyword\0\0not-zlib".to_vec())],
        &[vec![1, 2, 3]],
        false,
    );
    assert!(matches!(
        PngDecoder::new().inspect_bytes(&malformed_compressed_text, limits(4096)),
        Err(PngDecodeError::Malformed(message)) if message.contains("decompression")
    ));
}

#[test]
fn deterministic_encoder_round_trips_channel_order_and_16_bit_endianness() {
    let pixels = PngPixelData::RgbaU16 {
        dimensions: rusttable_image::ImageDimensions::new(2, 1).expect("dimensions"),
        samples: vec![0x1234, 0xabcd, 1, 0xfffe, 0, 0x8001, 0x7fff, 0x0102],
    };
    let options = PngEncodeOptions::new(5, 4096);
    let first = PngEncoder::new().encode(&pixels, options).expect("encode");
    let second = PngEncoder::new().encode(&pixels, options).expect("encode");
    assert_eq!(first, second);
    let decoded = PngDecoder::new()
        .decode_bytes(&first, &PngDecodeRequest::new(limits(4096)))
        .expect("round trip");
    assert_eq!(decoded.header.color_type, PngColorType::Rgba);
    assert_eq!(decoded.header.bit_depth, PngBitDepth::Sixteen);
    assert_eq!(decoded.pixels, Some(pixels));
}

#[test]
fn encoder_rejects_sample_mismatch_and_bounded_output() {
    let pixels = PngPixelData::RgbU8 {
        dimensions: rusttable_image::ImageDimensions::new(2, 1).expect("dimensions"),
        samples: vec![1, 2, 3],
    };
    assert!(matches!(
        PngEncoder::new().encode(&pixels, PngEncodeOptions::default()),
        Err(rusttable_image_io::png::PngEncodeError::SampleCount { .. })
    ));
    let valid = PngPixelData::RgbU8 {
        dimensions: rusttable_image::ImageDimensions::new(1, 1).expect("dimensions"),
        samples: vec![1, 2, 3],
    };
    assert!(matches!(
        PngEncoder::new().encode(&valid, PngEncodeOptions::new(5, 20)),
        Err(rusttable_image_io::png::PngEncodeError::OutputTooLarge { .. })
    ));
}

#[test]
fn encoded_limit_bounds_the_completed_png_not_raw_scanlines() {
    let pixels = PngPixelData::RgbaU8 {
        dimensions: rusttable_image::ImageDimensions::new(64, 64).expect("dimensions"),
        samples: vec![0; 64 * 64 * 4],
    };
    let encoded = PngEncoder::new()
        .encode(&pixels, PngEncodeOptions::default())
        .expect("compressible PNG");
    let raw_scanlines = 64 * (1 + 64 * 4);
    let final_size = u64::try_from(encoded.len()).expect("encoded size");
    assert!(u64::try_from(raw_scanlines).expect("raw size") > final_size);

    let exact = PngEncoder::new()
        .encode(&pixels, PngEncodeOptions::new(5, final_size))
        .expect("exact final-size limit");
    assert_eq!(exact, encoded);
    assert!(matches!(
        PngEncoder::new().encode(
            &pixels,
            PngEncodeOptions::new(5, final_size - 1),
        ),
        Err(rusttable_image_io::png::PngEncodeError::OutputTooLarge {
            actual,
            limit,
        }) if actual == final_size && limit == final_size - 1
    ));
}

#[test]
fn thumbnail_mode_has_a_truthful_full_source_fallback() {
    let source = png(
        2,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[vec![1, 2, 3, 4, 5, 6]],
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)).thumbnail())
        .expect("thumbnail fallback");
    assert_eq!(
        result.receipt.mode,
        rusttable_image_io::PngDecodeMode::Thumbnail
    );
    assert_eq!(result.header.dimensions.width(), 2);
    assert!(result.pixels.is_some());
}

#[test]
fn apng_default_image_is_decoded_and_animation_is_receipted() {
    let source = png_with_chunks(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[(*b"acTL", vec![0, 0, 0, 1, 0, 0, 0, 0])],
        &[vec![9, 8, 7]],
        false,
    );
    let result = PngDecoder::new()
        .decode_bytes(&source, &PngDecodeRequest::new(limits(4096)))
        .expect("default APNG");
    assert!(
        result
            .receipt
            .animation
            .expect("animation")
            .has_default_image
    );

    let no_default = {
        let mut value = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(
            &mut value,
            *b"IHDR",
            &[
                1_u32.to_be_bytes().as_slice(),
                1_u32.to_be_bytes().as_slice(),
                &[8, 2, 0, 0, 0],
            ]
            .concat(),
        );
        chunk(&mut value, *b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
        chunk(&mut value, *b"IEND", &[]);
        value
    };
    assert!(matches!(
        PngDecoder::new().decode_bytes(&no_default, &PngDecodeRequest::new(limits(4096))),
        Err(PngDecodeError::UnsupportedAnimation)
    ));
}

#[test]
fn cancellation_and_source_mutation_are_rejected() {
    let source = png(
        1,
        1,
        PngColorType::Rgb,
        PngBitDepth::Eight,
        &[vec![1, 2, 3]],
    );
    let cancellation = RawCancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        PngDecoder::new().decode_bytes(
            &source,
            &PngDecodeRequest::new(limits(4096)).with_cancellation(cancellation)
        ),
        Err(PngDecodeError::Cancelled)
    );

    let changing = ChangingSource {
        bytes: source,
        reads: AtomicU8::new(0),
    };
    let result = PngDecoder::new().decode_source(&changing, &PngDecodeRequest::new(limits(4096)));
    assert!(matches!(
        result,
        Err(PngDecodeError::Source(RawSourceError::Changed))
    ));
}

struct ChangingSource {
    bytes: Vec<u8>,
    reads: AtomicU8,
}
impl RawByteSource for ChangingSource {
    fn len(&self) -> Result<u64, RawSourceError> {
        Ok(self.bytes.len() as u64)
    }
    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        let mut revision = [0; 32];
        revision[0] = self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(revision)
    }
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        let end = start + buffer.len();
        buffer.copy_from_slice(&self.bytes[start..end]);
        Ok(())
    }
}
