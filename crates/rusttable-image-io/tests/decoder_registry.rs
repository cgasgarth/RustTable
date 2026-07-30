use flate2::{Compression, write::ZlibEncoder};
use rusttable_color::{Primaries, TransferFunction};
use rusttable_image::{
    DecodeLimits, ImageInputError, InputFormat, SampleType, SourceColorEvidence,
};
use rusttable_image_io::{
    ImageDecoderRegistry, PROBE_BUDGET_BYTES, PngDecodeLimits, PngDecoder, ProbeOutcome,
};

fn limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 2, 1, 2, 8).expect("valid test limits")
}

fn png_with_chunks(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut compressed, &[0, 17, 34, 51]).expect("PNG scanline");
    let compressed = compressed.finish().expect("PNG zlib stream");
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    png_chunk(
        &mut output,
        *b"IHDR",
        &[
            1_u32.to_be_bytes().as_slice(),
            1_u32.to_be_bytes().as_slice(),
            &[8, 2, 0, 0, 0],
        ]
        .concat(),
    );
    for (kind, data) in chunks {
        png_chunk(&mut output, *kind, data);
    }
    png_chunk(&mut output, *b"IDAT", &compressed);
    png_chunk(&mut output, *b"IEND", &[]);
    output
}

fn iccp_chunk(profile: &[u8]) -> Vec<u8> {
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut compressed, profile).expect("ICC profile");
    let compressed = compressed.finish().expect("ICC zlib stream");
    let mut data = b"RustTable\0\0".to_vec();
    data.extend_from_slice(&compressed);
    data
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the synthetic ICC fixture writes bounded, validated fixed-point values"
)]
fn matrix_icc() -> Vec<u8> {
    const TABLE_END: usize = 132 + 7 * 12;
    let xyz = [
        (*b"rXYZ", xy_xyz(0.68, 0.32)),
        (*b"gXYZ", xy_xyz(0.265, 0.69)),
        (*b"bXYZ", xy_xyz(0.15, 0.06)),
        (*b"wtpt", xy_xyz(0.3127, 0.3290)),
    ];
    let curve_offset = TABLE_END + xyz.len() * 20;
    let size = curve_offset + 14;
    let mut profile = vec![0_u8; size];
    put_u32(&mut profile, 0, u32::try_from(size).expect("small profile"));
    profile[8..12].copy_from_slice(&[4, 0x40, 0, 0]);
    for (offset, value) in [(24, 2026), (26, 7), (28, 30), (30, 12), (32, 0), (34, 0)] {
        put_u16(&mut profile, offset, value);
    }
    profile[12..16].copy_from_slice(b"mntr");
    profile[16..20].copy_from_slice(b"RGB ");
    profile[20..24].copy_from_slice(b"XYZ ");
    profile[36..40].copy_from_slice(b"acsp");
    put_u32(&mut profile, 68, 0x0000_f6d6);
    put_u32(&mut profile, 72, 0x0001_0000);
    put_u32(&mut profile, 76, 0x0000_d32d);
    put_u32(&mut profile, 128, 7);
    for (index, (signature, values)) in xyz.into_iter().enumerate() {
        let offset = TABLE_END + index * 20;
        put_tag(&mut profile, index, signature, offset, 20);
        profile[offset..offset + 4].copy_from_slice(b"XYZ ");
        for (component, value) in values.into_iter().enumerate() {
            let fixed = (value * 65_536.0).round() as i32;
            profile[offset + 8 + component * 4..offset + 12 + component * 4]
                .copy_from_slice(&fixed.to_be_bytes());
        }
    }
    for (index, signature) in [*b"rTRC", *b"gTRC", *b"bTRC"].into_iter().enumerate() {
        put_tag(&mut profile, index + 4, signature, curve_offset, 14);
    }
    profile[curve_offset..curve_offset + 4].copy_from_slice(b"curv");
    put_u32(&mut profile, curve_offset + 8, 1);
    profile[curve_offset + 12..curve_offset + 14].copy_from_slice(&563_u16.to_be_bytes());
    profile
}

fn lut_icc() -> Vec<u8> {
    const TABLE_END: usize = 132 + 12;
    const LUT_BYTES: usize = 68;
    let size = TABLE_END + LUT_BYTES;
    let mut profile = vec![0_u8; size];
    put_u32(&mut profile, 0, u32::try_from(size).expect("small profile"));
    profile[8..12].copy_from_slice(&[4, 0x40, 0, 0]);
    for (offset, value) in [(24, 2026), (26, 7), (28, 30), (30, 12), (32, 0), (34, 0)] {
        put_u16(&mut profile, offset, value);
    }
    profile[12..16].copy_from_slice(b"mntr");
    profile[16..20].copy_from_slice(b"RGB ");
    profile[20..24].copy_from_slice(b"XYZ ");
    profile[36..40].copy_from_slice(b"acsp");
    put_u32(&mut profile, 68, 0x0000_f6d6);
    put_u32(&mut profile, 72, 0x0001_0000);
    put_u32(&mut profile, 76, 0x0000_d32d);
    put_u32(&mut profile, 128, 1);
    put_tag(&mut profile, 0, *b"A2B0", TABLE_END, LUT_BYTES);

    let mut lut = vec![0_u8; LUT_BYTES];
    lut[..4].copy_from_slice(b"mAB ");
    lut[8..10].copy_from_slice(&[3, 3]);
    put_u32(&mut lut, 12, 32);
    for index in 0..3 {
        let start = 32 + index * 12;
        lut[start..start + 4].copy_from_slice(b"curv");
    }
    profile[TABLE_END..].copy_from_slice(&lut);
    profile
}

fn xy_xyz(x: f32, y: f32) -> [f32; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

fn put_tag(bytes: &mut [u8], index: usize, signature: [u8; 4], offset: usize, size: usize) {
    let start = 132 + index * 12;
    bytes[start..start + 4].copy_from_slice(&signature);
    put_u32(
        bytes,
        start + 4,
        u32::try_from(offset).expect("small offset"),
    );
    put_u32(bytes, start + 8, u32::try_from(size).expect("small tag"));
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn png_chunk(output: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    output.extend_from_slice(&(u32::try_from(data.len()).expect("small PNG chunk")).to_be_bytes());
    output.extend_from_slice(&kind);
    output.extend_from_slice(data);
    output.extend_from_slice(&png_crc32(kind, data).to_be_bytes());
}

fn png_crc32(kind: [u8; 4], data: &[u8]) -> u32 {
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

fn precision_limits() -> DecodeLimits {
    DecodeLimits::new(4_000_000, 4_096, 4_096, 16_777_216, 64 * 1024 * 1024)
        .expect("valid precision limits")
}

#[test]
fn standard_registry_has_stable_unique_decoder_identities() {
    let descriptors = ImageDecoderRegistry::standard().descriptors();
    assert_eq!(descriptors.len(), 7);
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.identity().id())
            .collect::<Vec<_>>(),
        vec![
            "rusttable.decoder.jpeg.v1",
            "rusttable.decoder.jpeg-xl.v1",
            "rusttable.decoder.png.v1",
            "rusttable.decoder.openexr.v1",
            "rusttable.decoder.raw.v1",
            "rusttable.decoder.tiff.v1",
            "rusttable.decoder.webp.v1",
        ]
    );
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.identity().version() == 1)
    );
    assert_eq!(PROBE_BUDGET_BYTES, 64 * 1024);
}

fn decode_base64(encoded: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut quartet = [0_u8; 4];
    let mut count = 0;
    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        quartet[count] = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            _ => panic!("fixture contains invalid base64"),
        };
        count += 1;
        if count == 4 {
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
            if quartet[2] != 64 {
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
            }
            if quartet[3] != 64 {
                output.push((quartet[2] << 6) | quartet[3]);
            }
            count = 0;
        }
    }
    assert_eq!(count, 0, "fixture base64 must have complete quartets");
    output
}

#[test]
fn standard_registry_selects_png_without_a_path_or_extension() {
    let bytes = decode_base64(include_str!("fixtures/rgba-2x1.png.b64"));
    let registry = ImageDecoderRegistry::standard();
    let probe = registry
        .probe_bytes(&bytes, limits())
        .expect("PNG signature should select the PNG decoder");
    let decoded = registry
        .decode_bytes(&bytes, limits())
        .expect("PNG signature should decode through the PNG decoder");

    assert_eq!(probe.format(), InputFormat::Png);
    assert_eq!(decoded.dimensions(), probe.dimensions());
}

#[test]
fn png_cicp_precedes_icc_and_receipts_retain_the_selected_color() {
    let profile = matrix_icc();
    let iccp = iccp_chunk(&profile);
    let bytes = png_with_chunks(&[(*b"cICP", vec![9, 16, 0, 1]), (*b"iCCP", iccp)]);
    let registry = ImageDecoderRegistry::standard();
    let legacy = registry
        .decode_bytes(&bytes, precision_limits())
        .expect("legacy PNG cICP source color");
    let frame = registry
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("typed PNG cICP source color");
    let source = frame.source_color();

    assert_eq!(legacy.dimensions(), frame.image().descriptor().dimensions());
    assert_eq!(legacy.color_encoding(), source.encoding());
    assert_eq!(legacy.pixels(), &[17, 34, 51, 255]);
    assert_eq!(frame.image().bytes(), &[17, 34, 51]);
    assert_eq!(source.primaries(), Some(Primaries::rec2020()));
    assert_eq!(source.transfer(), Some(TransferFunction::Pq));
    assert_eq!(
        source.evidence(),
        SourceColorEvidence::EmbeddedContainerMetadata
    );
    assert_eq!(frame.receipt().source_color(), source);
    assert_eq!(
        frame.image().descriptor().color_encoding(),
        source.encoding()
    );
    assert_eq!(frame.embedded_icc(), Some(profile.as_slice()));
}

#[test]
fn valid_cicp_survives_a_structurally_valid_unsupported_icc() {
    let bytes = png_with_chunks(&[
        (*b"cICP", vec![9, 16, 0, 1]),
        (*b"iCCP", iccp_chunk(&lut_icc())),
    ]);
    let header = PngDecoder::new()
        .inspect_bytes(&bytes, PngDecodeLimits::standard())
        .expect("unsupported ICC remains structurally valid");
    assert!(header.metadata.icc_profile.is_some());

    let registry = ImageDecoderRegistry::standard();
    let legacy = registry
        .decode_bytes(&bytes, precision_limits())
        .expect("legacy decode should honor cICP");
    let frame = registry
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("typed decode should honor cICP");

    assert_eq!(legacy.color_encoding(), frame.source_color().encoding());
    assert_eq!(
        frame.source_color().evidence(),
        SourceColorEvidence::EmbeddedContainerMetadata
    );
    assert_eq!(frame.source_color().transfer(), Some(TransferFunction::Pq));
    assert_eq!(frame.embedded_icc(), Some(lut_icc().as_slice()));
}

#[test]
fn unsupported_icc_without_cicp_is_retained_as_authoritative_source() {
    let profile = lut_icc();
    let bytes = png_with_chunks(&[(*b"iCCP", iccp_chunk(&profile))]);
    let frame = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("unsupported ICC should remain a typed source");

    assert_eq!(
        frame.source_color().evidence(),
        SourceColorEvidence::EmbeddedIcc
    );
    assert_eq!(frame.source_color().transfer(), None);
    assert_eq!(frame.embedded_icc(), Some(profile.as_slice()));
}

#[test]
fn malformed_icc_with_valid_cicp_remains_fail_closed_for_both_apis() {
    let bytes = png_with_chunks(&[
        (*b"cICP", vec![9, 16, 0, 1]),
        (*b"iCCP", iccp_chunk(b"not an ICC profile")),
    ]);
    let registry = ImageDecoderRegistry::standard();

    assert!(matches!(
        registry.decode_bytes(&bytes, precision_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            ..
        })
    ));
    assert!(matches!(
        registry.decode_frame_bytes(&bytes, precision_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            ..
        })
    ));
}

#[test]
fn unsupported_png_cicp_falls_back_to_the_existing_metadata_precedence() {
    let bytes = png_with_chunks(&[(*b"cICP", vec![9, 14, 0, 0]), (*b"sRGB", vec![0])]);
    let frame = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("fallback source color");
    let source = frame.source_color();

    assert_eq!(source.encoding(), rusttable_image::ColorEncoding::SrgbD65);
    assert_eq!(source.evidence(), SourceColorEvidence::DeclaredEncoding);
}

#[test]
fn unsupported_cicp_falls_back_to_the_structurally_valid_icc_profile() {
    let profile = lut_icc();
    let bytes = png_with_chunks(&[
        (*b"cICP", vec![9, 14, 0, 0]),
        (*b"iCCP", iccp_chunk(&profile)),
    ]);
    let registry = ImageDecoderRegistry::standard();
    let legacy = registry
        .decode_bytes(&bytes, precision_limits())
        .expect("legacy ICC fallback");
    let frame = registry
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("typed ICC fallback");

    assert_eq!(legacy.color_encoding(), frame.source_color().encoding());
    assert_eq!(
        frame.source_color().evidence(),
        SourceColorEvidence::EmbeddedIcc
    );
    assert_eq!(frame.source_color().transfer(), None);
    assert_eq!(frame.embedded_icc(), Some(profile.as_slice()));
}

#[test]
fn malformed_png_cicp_is_rejected_by_the_registry_color_decision() {
    let bytes = png_with_chunks(&[(*b"cICP", vec![9, 14, 0])]);
    assert!(matches!(
        ImageDecoderRegistry::standard().decode_frame_bytes(&bytes, precision_limits()),
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message,
        }) if message.contains("cICP")
    ));
}

#[test]
fn standard_registry_decodes_jpeg_xl_and_webp_by_container_signature() {
    let registry = ImageDecoderRegistry::standard();
    for (fixture, format, sample_type) in [
        (
            include_str!("fixtures/lossless-4x3.jxl.b64"),
            InputFormat::JpegXl,
            SampleType::F32,
        ),
        (
            include_str!("fixtures/lossless-4x3-container.jxl.b64"),
            InputFormat::JpegXl,
            SampleType::F32,
        ),
        (
            include_str!("fixtures/lossless-4x3.webp.b64"),
            InputFormat::Webp,
            SampleType::U8,
        ),
    ] {
        let bytes = decode_base64(fixture);
        let probe = registry
            .probe_bytes(&bytes, precision_limits())
            .expect("modern raster signature should probe");
        let frame = registry
            .decode_frame_bytes(&bytes, precision_limits())
            .expect("modern raster signature should decode");

        assert_eq!(probe.format(), format);
        assert_eq!(probe.dimensions().width(), 4);
        assert_eq!(probe.dimensions().height(), 3);
        assert_eq!(frame.receipt().format(), format);
        assert_eq!(frame.image().descriptor().dimensions(), probe.dimensions());
        assert_eq!(frame.sample_type(), sample_type);
    }
}

#[test]
fn registry_frame_keeps_adjacent_sixteen_bit_tiff_samples_distinct() {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut encoder = tiff::encoder::TiffEncoder::new(&mut cursor).expect("TIFF encoder");
        encoder
            .write_image::<tiff::encoder::colortype::RGB16>(
                2,
                1,
                &[0x8000, 0x8000, 0x8000, 0x8001, 0x8001, 0x8001],
            )
            .expect("TIFF fixture");
    }
    let bytes = cursor.into_inner();
    let frame = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("typed TIFF frame");

    assert_eq!(frame.sample_type(), SampleType::U16);
    let pixels = frame.rgba_f32_pixels().expect("f32 bridge");
    assert!(
        pixels
            .windows(2)
            .any(|pair| (pair[0][0] - pair[1][0]).abs() > 0.0)
    );
    assert_eq!(
        frame.receipt().descriptor().format().sample_type(),
        SampleType::U16
    );
}

#[test]
fn registry_frame_keeps_float_exr_values_outside_display_range() {
    use exr::prelude::{f16, write_rgb_file};

    let path = std::env::temp_dir().join(format!(
        "rusttable-registry-precision-{}.exr",
        std::process::id()
    ));
    write_rgb_file(&path, 2, 1, |x, _| {
        let value = if x == 0 { -0.25 } else { 2.5 };
        let value = f16::from_f32(value);
        (value, value, value)
    })
    .expect("EXR fixture");
    let bytes = std::fs::read(&path).expect("EXR bytes");
    std::fs::remove_file(path).expect("remove EXR fixture");

    let frame = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&bytes, precision_limits())
        .expect("typed EXR frame");
    assert_eq!(frame.sample_type(), SampleType::F16);
    let pixels = frame.rgba_f32_pixels().expect("finite f32 bridge");
    assert!(pixels[0][0] < 0.0);
    assert!(pixels[1][0] > 1.0);
}

#[test]
fn standard_registry_verifies_jpeg_decoded_dimensions() {
    let bytes = decode_base64(include_str!("fixtures/rgb-2x1.jpg.b64"));
    let registry = ImageDecoderRegistry::standard();
    let probe = registry
        .probe_bytes(&bytes, limits())
        .expect("JPEG signature should select the JPEG decoder");
    let decoded = registry
        .decode_bytes(&bytes, limits())
        .expect("JPEG signature should decode through the JPEG decoder");

    assert_eq!(probe.format(), InputFormat::Jpeg);
    assert_eq!(decoded.dimensions(), probe.dimensions());
}

#[test]
fn recognized_png_signature_reports_malformed_input() {
    let bytes = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            ..
        })
    ));
}

#[test]
fn png_header_probe_is_structural_and_does_not_need_a_complete_payload() {
    let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    bytes.extend_from_slice(&13_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&3_u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes.extend_from_slice(&[0; 4]);
    let result = ImageDecoderRegistry::standard().probe(
        &bytes,
        DecodeLimits::new(1_000_000, 2, 3, 6, 24).expect("valid limits"),
    );

    assert!(matches!(
        result,
        ProbeOutcome::Match { decoder, probe }
            if decoder.format() == InputFormat::Png
                && probe.dimensions().width() == 2
                && probe.dimensions().height() == 3
    ));
}

#[test]
fn png_signature_with_invalid_ihdr_is_malformed_before_backend_dispatch() {
    let mut bytes = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    bytes.extend_from_slice(&12_u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&[0; 12]);

    assert!(matches!(
        ImageDecoderRegistry::standard().probe(&bytes, limits()),
        ProbeOutcome::MalformedRecognized { decoder, .. }
            if decoder.format() == InputFormat::Png
    ));
}

#[test]
fn standard_registry_selects_jpeg_and_classic_tiff_by_signature() {
    let registry = ImageDecoderRegistry::standard();
    let jpeg = decode_base64(include_str!("fixtures/rgb-2x1.jpg.b64"));
    let tiff = decode_base64(include_str!("fixtures/rgb-2x1.tiff.b64"));

    let jpeg_probe = registry
        .probe_bytes(&jpeg, limits())
        .expect("JPEG signature should select the JPEG decoder");
    let tiff_probe = registry
        .probe_bytes(&tiff, limits())
        .expect("classic TIFF signature should select the TIFF decoder");

    assert_eq!(jpeg_probe.format(), InputFormat::Jpeg);
    assert_eq!(tiff_probe.format(), InputFormat::Tiff);
}

#[test]
fn camera_raw_signature_selects_the_raw_decoder_before_decode() {
    let bytes = b"FUJIFILMCCD-RAW";
    let result = ImageDecoderRegistry::standard().probe(bytes, limits());

    assert!(matches!(
        result,
        ProbeOutcome::MalformedRecognized { decoder, error: ImageInputError::MalformedInput { format: InputFormat::Raw, .. } }
            if decoder.format() == InputFormat::Raw
    ));
}

#[test]
fn malformed_bigtiff_signature_never_falls_back() {
    let bytes = decode_base64(include_str!("fixtures/bigtiff.tiff.b64"));
    let registry = ImageDecoderRegistry::standard();

    let probe = registry.probe_bytes(&bytes, limits());
    let decode = registry.decode_bytes(&bytes, limits());

    assert!(matches!(
        probe,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            ..
        })
    ));
    assert!(matches!(
        decode,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            ..
        })
    ));
}

#[test]
fn unsupported_signature_is_rejected() {
    let bytes = *b"GIF89a";
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::UnsupportedSignature { signature }) if signature == bytes
    ));
}

#[test]
fn openexr_magic_is_registered_without_shadowing_raw() {
    use exr::prelude::{f16, write_rgb_file};

    let path = std::env::temp_dir().join(format!("rusttable-registry-{}.exr", std::process::id()));
    write_rgb_file(&path, 2, 1, |x, _| {
        let value = f16::from_f32(f32::from(u16::try_from(x).unwrap()));
        (value, value, value)
    })
    .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let registry = ImageDecoderRegistry::standard();
    let probe = registry.probe_bytes(&bytes, limits()).unwrap();
    assert_eq!(probe.format(), InputFormat::OpenExr);
    assert_eq!(
        registry.select(b"FUJIFILMCCD-RAW").unwrap().format(),
        InputFormat::Raw
    );
}

#[test]
fn matched_malformed_jpeg_never_falls_back_to_another_decoder() {
    let bytes = [0xff, 0xd8, 0xff];
    let result = ImageDecoderRegistry::standard().probe_bytes(&bytes, limits());

    assert!(matches!(
        result,
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            ..
        })
    ));
}
