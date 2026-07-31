#![allow(clippy::float_cmp)]

#[path = "../src/operations/diffuse/mod.rs"]
pub mod diffuse;

use diffuse::{
    DiffuseConfig, DiffuseDimensions, DiffuseExecutionError, DiffuseParametersV2, DiffusePixel,
    DiffusePlan, IsotropyMode,
};

#[test]
fn native_sign_encoding_selects_each_direction_without_a_blur_alias() {
    let mut parameters = DiffuseParametersV2::defaults();
    parameters.anisotropy_first = 1.0;
    parameters.anisotropy_second = -1.0;
    parameters.anisotropy_third = 0.0;
    parameters.anisotropy_fourth = -0.0;
    let config = DiffuseConfig::new(parameters).expect("parameters");
    assert_eq!(
        config.isotropy(),
        [
            IsotropyMode::Isophote,
            IsotropyMode::Gradient,
            IsotropyMode::Isotropic,
            IsotropyMode::Isotropic,
        ]
    );
    assert_eq!(config.anisotropy(), [1.0, 1.0, 0.0, 0.0]);
}

#[test]
fn memory_admission_happens_before_cpu_scratch_allocation() {
    let dimensions = DiffuseDimensions::new(64, 64).expect("dimensions");
    let result = DiffusePlan::new_with_budget(DiffuseConfig::defaults(), dimensions, 1.0, 1.0, 1);
    assert!(matches!(
        result,
        Err(DiffuseExecutionError::MemoryBudgetExceeded { .. })
    ));
}

#[test]
fn multiscale_pde_uses_chained_low_frequency_residuals() {
    let dimensions = DiffuseDimensions::new(7, 5).expect("dimensions");
    let mut parameters = DiffuseParametersV2::defaults();
    parameters.radius = 32;
    parameters.sharpness = 0.35;
    parameters.regularization = 0.25;
    parameters.anisotropy_first = 0.6;
    parameters.anisotropy_second = -0.3;
    parameters.first = 0.45;
    parameters.second = -0.35;
    parameters.third = 0.2;
    parameters.fourth = 0.1;
    let plan = DiffusePlan::new(
        DiffuseConfig::new(parameters).expect("parameters"),
        dimensions,
        1.0,
        1.0,
    )
    .expect("plan");
    assert!(plan.scales() > 1);
    let input = (0..dimensions.pixel_count().expect("pixels"))
        .map(|index| {
            let x = f32::from(
                u16::try_from(index % dimensions.width()).expect("test width fits in u16"),
            );
            let y = f32::from(
                u16::try_from(index / dimensions.width()).expect("test height fits in u16"),
            );
            let detail =
                f32::from(u16::try_from((index * 3 + 1) % 7).expect("test detail fits in u16"));
            DiffusePixel::from_channels([
                0.1 + 0.07 * x + 0.03 * y,
                0.8 - 0.05 * x + 0.02 * y,
                0.2 + 0.04 * detail,
                0.5 + 0.01 * x,
            ])
        })
        .collect::<Vec<_>>();
    let output = plan.execute(&input).expect("diffuse");
    let expected = [
        [0.047_031_537, 0.821_005, 0.228_841_87, 0.494_434_12],
        [0.130_212, 0.761_891_84, 0.360_213_28, 0.506_216_05],
        [0.214_579_05, 0.701_928_9, 0.184_081_36, 0.518_154_6],
    ];
    for (actual, expected) in output.iter().take(expected.len()).zip(expected) {
        for (actual, expected) in actual.channels().into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-5, "{actual} != {expected}");
        }
    }
}

#[test]
fn masked_inpainting_is_deterministic_and_finite() {
    let dimensions = DiffuseDimensions::new(2, 1).expect("dimensions");
    let mut parameters = DiffuseParametersV2::defaults();
    parameters.threshold = 0.5;
    let plan = DiffusePlan::new(
        DiffuseConfig::new(parameters).expect("parameters"),
        dimensions,
        1.0,
        1.0,
    )
    .expect("plan");
    let input = vec![
        DiffusePixel::from_channels([0.1, 0.1, 0.1, 0.25]),
        DiffusePixel::from_channels([0.8, 0.1, 0.1, 0.75]),
    ];
    let first = plan.execute(&input).expect("first");
    let second = plan.execute(&input).expect("second");
    assert_eq!(first, second);
    assert!(
        first
            .iter()
            .flat_map(|pixel| pixel.channels())
            .all(f32::is_finite)
    );
}
