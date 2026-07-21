#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "tests assert the compatibility float values exactly"
)]

use rusttable_processing::descriptor::{OperationFlags, dither_descriptor};
use rusttable_processing::operations::dither::{
    DitherBitDepth, DitherConfig, DitherHistory, DitherMethod, DitherParametersV1,
    DitherParametersV2, DitherPlan, DitherRenderContext, quantize_round_half_strictly_greater,
    rgb_to_gray,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry};

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("finite red"),
        FiniteF32::new(value).expect("finite green"),
        FiniteF32::new(value).expect("finite blue"),
    )
}

#[test]
fn dither_codecs_preserve_fixed_fields_migrations_and_unknown_payloads() {
    let v1 = DitherParametersV1::defaults();
    assert_eq!(v1.to_bytes().len(), 168);
    let v2 = DitherHistory::V1(v1.clone())
        .migrate_v1()
        .expect("migration");
    assert_eq!(v2.to_bytes().len(), 36);
    assert_eq!(v2.method_id, DitherMethod::FsAuto.id());
    assert_eq!(v2.opaque_source().expect("source").len(), 168);
    let unknown = DitherHistory::decode(44, &[3, 1, 4]).expect("opaque");
    assert_eq!(unknown.payload(), [3, 1, 4]);
    let mut unknown_method_bytes = DitherParametersV2::defaults().to_bytes();
    unknown_method_bytes[0..4].copy_from_slice(&999_u32.to_le_bytes());
    let unknown_method = DitherHistory::decode(2, &unknown_method_bytes).expect("opaque");
    assert_eq!(unknown_method.payload(), unknown_method_bytes);
    assert_eq!(DitherParametersV2::defaults().to_bytes().len(), 36);
}

#[test]
fn strict_half_quantization_and_compatibility_luma_are_exact() {
    assert_eq!(quantize_round_half_strictly_greater(0.5, 2), 0.0);
    assert_eq!(quantize_round_half_strictly_greater(0.500_001, 2), 1.0);
    assert_eq!(quantize_round_half_strictly_greater(f32::NAN, 2), 0.0);
    assert_eq!(rgb_to_gray([1.0, 0.0, 0.0]), 0.30);
    assert!(DitherConfig::new(DitherMethod::Posterize(1), -100.0).is_err());
}

#[test]
fn posterize_and_random_are_deterministic_and_bounded() {
    let dimensions = RasterDimensions::new(3, 2).unwrap();
    let input = vec![
        pixel(0.13),
        pixel(0.5),
        pixel(0.91),
        pixel(0.2),
        pixel(0.7),
        pixel(1.0),
    ];
    let poster = DitherPlan::new(
        DitherConfig::new(DitherMethod::Posterize(4), -100.0).unwrap(),
        dimensions,
    );
    let output = poster.execute(&input).unwrap();
    assert!(output.iter().all(|pixel| {
        [pixel.red(), pixel.green(), pixel.blue()]
            .into_iter()
            .all(|channel| (0.0..=1.0).contains(&channel.get()))
    }));
    let random = DitherConfig::new(DitherMethod::Random, -100.0)
        .unwrap()
        .with_seed(42);
    let first = DitherPlan::new(random, dimensions).execute(&input).unwrap();
    let second = DitherPlan::new(random, dimensions).execute(&input).unwrap();
    assert_eq!(first, second);
}

#[test]
fn floyd_steinberg_is_full_image_cpu_and_preserves_rgb_level_bounds() {
    let dimensions = RasterDimensions::new(4, 4).unwrap();
    let input = (0..16)
        .map(|index| pixel(index as f32 / 15.0))
        .collect::<Vec<_>>();
    let plan = DitherPlan::with_context(
        DitherConfig::new(DitherMethod::Fs2BitRgb, -100.0).unwrap(),
        dimensions,
        DitherRenderContext::new(1.0, DitherBitDepth::Int8, true, false, false).unwrap(),
    );
    let output = plan.execute(&input).unwrap();
    assert_eq!(output.len(), input.len());
    assert!(output.iter().all(|pixel| pixel.red().get().is_finite()));
    assert!(plan.execute(&input[..8]).is_err());
}

#[test]
fn descriptor_and_registry_record_deterministic_cpu_full_image_identity() {
    let descriptor = dither_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    assert!(builtin_registry().definition("rusttable.dither").is_some());
}
