use std::io::Cursor;

use rusttable_image::{
    ChannelLayout, ColorEncoding, DecodeLimits, DecodeStage, Orientation, SampleType,
};
use rusttable_image_io::{
    ImageDecoderRegistry, RawCapabilityKind, RawContainerKind, RawDecodeLimits, RawDecodeRequest,
    RawPlaneLayout, RawlerRawDecoder,
};
use tiff::encoder::colortype::{Gray8, Gray16};
use tiff::encoder::{Rational, TiffEncoder, TiffKind};
use tiff::tags::Tag;

fn limits() -> RawDecodeLimits {
    RawDecodeLimits::new(1_000_000, 64, 64, 4_096, 8 * 1024).expect("RAW limits")
}

fn image_limits() -> DecodeLimits {
    DecodeLimits::new(1_000_000, 64, 64, 4_096, 16 * 1024).expect("image limits")
}

fn dng_fixture(big: bool, compression: Option<u16>) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    if big {
        let mut encoder = TiffEncoder::new_big(&mut cursor).expect("BigTIFF encoder");
        write_pages(&mut encoder, compression);
    } else {
        let mut encoder = TiffEncoder::new(&mut cursor).expect("TIFF encoder");
        write_pages(&mut encoder, compression);
    }
    cursor.into_inner()
}

fn linear_tiff_fixture(big: bool) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    if big {
        let mut encoder = TiffEncoder::new_big(&mut cursor).expect("BigTIFF encoder");
        write_linear_page(&mut encoder);
    } else {
        let mut encoder = TiffEncoder::new(&mut cursor).expect("TIFF encoder");
        write_linear_page(&mut encoder);
    }
    cursor.into_inner()
}

fn developable_dng_fixture() -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor).expect("TIFF encoder");
        let mut raw = encoder.new_image::<Gray16>(8, 6).expect("raw image");
        let writer = raw.encoder();
        writer
            .write_tag(Tag::Unknown(50_706), &[1_u8, 4, 0, 0][..])
            .expect("DNG version");
        writer.write_tag(Tag::Make, "RustTable").expect("make");
        writer
            .write_tag(Tag::Model, "DNG Orientation")
            .expect("model");
        writer
            .write_tag(Tag::PhotometricInterpretation, 32_803_u16)
            .expect("CFA photometric");
        writer
            .write_tag(Tag::Orientation, 6_u16)
            .expect("orientation");
        writer
            .write_tag(Tag::Unknown(33_421), &[2_u16, 2][..])
            .expect("CFA repeat");
        writer
            .write_tag(Tag::Unknown(33_422), &[0_u8, 1, 1, 2][..])
            .expect("CFA pattern");
        writer
            .write_tag(Tag::Unknown(50_829), &[0_u32, 0, 6, 8][..])
            .expect("active area");
        writer
            .write_tag(Tag::Unknown(50_719), &[0_u32, 0][..])
            .expect("crop origin");
        writer
            .write_tag(Tag::Unknown(50_720), &[8_u32, 6][..])
            .expect("crop size");
        writer
            .write_tag(Tag::Unknown(50_714), &[64_u32][..])
            .expect("black level");
        writer
            .write_tag(Tag::Unknown(50_717), &[4_095_u32][..])
            .expect("white level");
        writer
            .write_tag(
                Tag::Unknown(50_728),
                &[
                    Rational { n: 1, d: 1 },
                    Rational { n: 1, d: 1 },
                    Rational { n: 1, d: 1 },
                ][..],
            )
            .expect("white balance");
        writer
            .write_tag(Tag::Unknown(50_778), 21_u16)
            .expect("illuminant");
        let matrix = [
            Rational { n: 1, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 1, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 1, d: 1 },
        ];
        writer
            .write_tag(Tag::Unknown(50_721), &matrix[..])
            .expect("color matrix");
        let samples = (0_u16..48)
            .map(|value| 512 + value * 32)
            .collect::<Vec<_>>();
        raw.write_data(&samples).expect("raw samples");
    }
    cursor.into_inner()
}

fn scene_invariance_dng(background: u16) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut cursor).expect("TIFF encoder");
        let mut raw = encoder.new_image::<Gray16>(8, 6).expect("raw image");
        let writer = raw.encoder();
        writer
            .write_tag(Tag::Unknown(50_706), &[1_u8, 4, 0, 0][..])
            .expect("DNG version");
        writer.write_tag(Tag::Make, "RustTable").expect("make");
        writer
            .write_tag(Tag::Model, "RAW scene invariance")
            .expect("model");
        writer
            .write_tag(Tag::PhotometricInterpretation, 32_803_u16)
            .expect("CFA photometric");
        writer
            .write_tag(Tag::Unknown(33_421), &[2_u16, 2][..])
            .expect("CFA repeat");
        writer
            .write_tag(Tag::Unknown(33_422), &[0_u8, 1, 1, 2][..])
            .expect("CFA pattern");
        writer
            .write_tag(Tag::Unknown(50_829), &[0_u32, 0, 6, 8][..])
            .expect("active area");
        writer
            .write_tag(Tag::Unknown(50_719), &[0_u32, 0][..])
            .expect("crop origin");
        writer
            .write_tag(Tag::Unknown(50_720), &[8_u32, 6][..])
            .expect("crop size");
        writer
            .write_tag(Tag::Unknown(50_714), &[64_u32][..])
            .expect("black level");
        writer
            .write_tag(Tag::Unknown(50_717), &[4_095_u32][..])
            .expect("white level");
        writer
            .write_tag(
                Tag::Unknown(50_728),
                &[
                    Rational { n: 1, d: 1 },
                    Rational { n: 1, d: 1 },
                    Rational { n: 1, d: 1 },
                ][..],
            )
            .expect("white balance");
        writer
            .write_tag(Tag::Unknown(50_778), 21_u16)
            .expect("illuminant");
        let matrix = [
            Rational { n: 1, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 1, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 0, d: 1 },
            Rational { n: 1, d: 1 },
        ];
        writer
            .write_tag(Tag::Unknown(50_721), &matrix[..])
            .expect("color matrix");
        let samples = (0..6)
            .flat_map(|y| {
                (0..8).map(move |x| {
                    if (1..=6).contains(&x) && (1..=4).contains(&y) {
                        1_500
                    } else {
                        background
                    }
                })
            })
            .collect::<Vec<_>>();
        raw.write_data(&samples).expect("raw samples");
    }
    cursor.into_inner()
}

fn write_linear_page<E, K>(encoder: &mut TiffEncoder<E, K>)
where
    E: std::io::Write + std::io::Seek,
    K: TiffKind,
{
    let mut raw = encoder.new_image::<Gray16>(3, 1).expect("linear RAW image");
    raw.encoder()
        .write_tag(Tag::PhotometricInterpretation, 34_892_u16)
        .expect("LinearRaw photometric");
    raw.write_data(&[11, 22, 33]).expect("linear RAW samples");
}

fn write_pages<E, K>(encoder: &mut TiffEncoder<E, K>, compression: Option<u16>)
where
    E: std::io::Write + std::io::Seek,
    K: TiffKind,
{
    let mut preview = encoder.new_image::<Gray8>(2, 1).expect("preview");
    preview
        .encoder()
        .write_tag(Tag::NewSubfileType, 1_u32)
        .expect("preview kind");
    preview.write_data(&[7, 8]).expect("preview samples");

    let mut raw = encoder.new_image::<Gray16>(4, 2).expect("raw image");
    let writer = raw.encoder();
    writer
        .write_tag(Tag::Unknown(50_706), &[1_u8, 4, 0, 0][..])
        .expect("DNG version");
    writer
        .write_tag(Tag::Unknown(50_707), &[1_u8, 2, 0, 0][..])
        .expect("DNG backward version");
    writer.write_tag(Tag::Make, "RustTable").expect("make");
    writer.write_tag(Tag::Model, "DNG Contract").expect("model");
    writer
        .write_tag(Tag::PhotometricInterpretation, 32_803_u16)
        .expect("CFA photometric");
    writer
        .write_tag(Tag::Orientation, 6_u16)
        .expect("orientation");
    writer
        .write_tag(Tag::Unknown(33_421), &[2_u16, 2][..])
        .expect("CFA repeat");
    writer
        .write_tag(Tag::Unknown(33_422), &[0_u8, 1, 1, 2][..])
        .expect("CFA pattern");
    writer
        .write_tag(Tag::Unknown(50_829), &[0_u32, 1, 2, 3][..])
        .expect("active area");
    writer
        .write_tag(Tag::Unknown(50_719), &[1_u32, 0][..])
        .expect("crop origin");
    writer
        .write_tag(Tag::Unknown(50_720), &[2_u32, 1][..])
        .expect("crop size");
    writer
        .write_tag(Tag::Unknown(50_713), &[2_u16, 2][..])
        .expect("black repeat");
    writer
        .write_tag(
            Tag::Unknown(50_714),
            &[
                Rational { n: 64, d: 1 },
                Rational { n: 64, d: 1 },
                Rational { n: 64, d: 1 },
                Rational { n: 64, d: 1 },
            ][..],
        )
        .expect("black level");
    writer
        .write_tag(Tag::Unknown(50_717), &[4_095_u32][..])
        .expect("white level");
    writer
        .write_tag(
            Tag::Unknown(50_728),
            &[
                Rational { n: 1, d: 2 },
                Rational { n: 1, d: 1 },
                Rational { n: 1, d: 4 },
            ][..],
        )
        .expect("white balance");
    writer
        .write_tag(Tag::Unknown(50_778), 21_u16)
        .expect("illuminant");
    let matrix: Vec<_> = (0..9).map(|_| Rational { n: 1, d: 1 }).collect();
    writer
        .write_tag(Tag::Unknown(50_721), &matrix[..])
        .expect("color matrix");
    if let Some(compression) = compression {
        writer
            .write_tag(Tag::Compression, compression)
            .expect("compression");
    }
    raw.write_data(&[100, 200, 300, 400, 500, 600, 700, 800])
        .expect("raw samples");
}

#[test]
fn classic_and_bigtiff_dng_preserve_samples_metadata_and_preview_inventory() {
    for bytes in [dng_fixture(false, None), dng_fixture(true, None)] {
        let result = RawlerRawDecoder::new()
            .decode_bytes(&bytes, &RawDecodeRequest::new(limits()))
            .expect("DNG decode");
        assert_eq!(result.receipt.container, RawContainerKind::Dng);
        assert_eq!(
            result
                .receipt
                .dng
                .as_ref()
                .expect("DNG receipt")
                .preview_count,
            1
        );
        assert_eq!(
            result
                .receipt
                .dng
                .as_ref()
                .expect("DNG receipt")
                .opcode_count,
            0
        );
        assert_eq!(
            result.frame.parts().planes[0].samples,
            [100, 200, 300, 400, 500, 600, 700, 800]
        );
        assert_eq!(
            result.frame.parts().active_area,
            rusttable_image_io::RawRect::new(1, 0, 2, 2).unwrap()
        );
        assert_eq!(
            result.frame.parts().crop_area,
            rusttable_image_io::RawRect::new(2, 0, 2, 1).unwrap()
        );
        assert_eq!(
            result.frame.parts().orientation,
            rusttable_image_io::RawOrientation::Rotate90
        );
        match &result.frame.parts().planes[0].layout {
            RawPlaneLayout::Mosaic(cfa) => {
                assert_eq!(
                    (cfa.width, cfa.height, cfa.phase_x, cfa.phase_y),
                    (2, 2, 1, 0)
                );
            }
            RawPlaneLayout::Linear { .. } => panic!("DNG fixture is a CFA mosaic"),
        }
        assert_eq!(
            result.frame.parts().white_balance[..3],
            [Some(2.0), Some(1.0), Some(4.0)]
        );
    }
}

#[test]
fn developed_raw_preview_defers_orientation_to_the_shared_geometry_boundary() {
    let image = ImageDecoderRegistry::standard()
        .decode_bytes(&developable_dng_fixture(), image_limits())
        .expect("developed DNG preview");

    assert_eq!(
        (image.dimensions().width(), image.dimensions().height()),
        (8, 6)
    );
    assert_eq!(image.source_orientation(), Orientation::Rotate90);
    assert_eq!(
        (
            image.oriented_dimensions().width(),
            image.oriented_dimensions().height()
        ),
        (6, 8)
    );
    assert_eq!(image.pixels().len(), 8 * 6 * 4);
}

#[test]
fn raw_frame_is_linear_and_receipt_exposes_every_development_stage() {
    let frame = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&developable_dng_fixture(), image_limits())
        .expect("linear RAW frame");
    assert_eq!(
        frame.image().descriptor().format().sample_type(),
        SampleType::F32
    );
    assert_eq!(
        frame.image().descriptor().format().channels(),
        ChannelLayout::Rgba
    );
    assert_eq!(
        frame.image().descriptor().color_encoding(),
        ColorEncoding::LinearSrgb
    );
    assert_eq!(
        frame.receipt().processing_stages(),
        &[
            DecodeStage::RawRescale,
            DecodeStage::RawActiveAreaCrop,
            DecodeStage::RawCfa,
            DecodeStage::RawDemosaic,
            DecodeStage::RawColorCalibration,
            DecodeStage::RawDefaultCrop,
        ]
    );
    assert!(frame.raw_source().is_some());
    assert!(
        frame
            .receipt()
            .processing_stages()
            .iter()
            .all(|stage| !stage.name().contains("tone"))
    );
    assert!(
        frame
            .rgba_f32_pixels()
            .expect("finite linear pixels")
            .iter()
            .flatten()
            .all(|value| value.is_finite())
    );
}

#[test]
fn shared_patch_is_scene_invariant_and_redecode_is_cache_stable() {
    let frames = [128_u16, 1_200, 3_800]
        .into_iter()
        .map(|background| {
            ImageDecoderRegistry::standard()
                .decode_frame_bytes(&scene_invariance_dng(background), image_limits())
                .expect("scene-invariance RAW frame")
        })
        .collect::<Vec<_>>();
    let patch_index = 2 * 8 + 3;
    let patch_values = frames
        .iter()
        .map(|frame| {
            frame
                .rgba_f32_pixels()
                .expect("finite scene-invariance pixels")[patch_index]
        })
        .collect::<Vec<_>>();
    for value in &patch_values[1..] {
        for (actual, expected) in value.iter().zip(patch_values[0]) {
            assert!((actual - expected).abs() <= 1.0e-6);
        }
    }
    assert!(patch_values[0][0] > 0.2);

    let source = scene_invariance_dng(1_200);
    let first = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&source, image_limits())
        .expect("first RAW frame");
    drop(first);
    let second = ImageDecoderRegistry::standard()
        .decode_frame_bytes(&source, image_limits())
        .expect("redecoded RAW frame");
    assert_eq!(
        second
            .rgba_f32_pixels()
            .expect("redecoded finite linear pixels"),
        frames[1]
            .rgba_f32_pixels()
            .expect("cached finite linear pixels")
    );
    assert_eq!(second.receipt(), frames[1].receipt());
}

#[test]
fn proprietary_compression_is_rejected_before_raw_frame_publication() {
    let error = RawlerRawDecoder::new()
        .decode_bytes(
            &dng_fixture(false, Some(52_546)),
            &RawDecodeRequest::new(limits()),
        )
        .expect_err("JPEG XL is not part of the pinned TIFF backend");
    match error {
        rusttable_image_io::RawDecodeError::Capability(error) => {
            assert_eq!(error.missing, RawCapabilityKind::Compression);
            assert!(error.detail.contains("JPEG XL"));
        }
        other => panic!("expected compression capability error, got {other:?}"),
    }
}

#[test]
fn standardized_linear_tiff_raw_supports_classic_and_bigtiff() {
    for bytes in [linear_tiff_fixture(false), linear_tiff_fixture(true)] {
        let result = RawlerRawDecoder::new()
            .decode_bytes(&bytes, &RawDecodeRequest::new(limits()))
            .expect("linear TIFF RAW decode");
        assert_eq!(result.receipt.container, RawContainerKind::TiffRaw);
        assert_eq!(result.frame.parts().planes[0].samples, [11, 22, 33]);
        assert!(matches!(
            result.frame.parts().planes[0].layout,
            RawPlaneLayout::Linear { .. }
        ));
        assert_eq!(result.receipt.dng.expect("RAW receipt").preview_count, 0);
    }
}
