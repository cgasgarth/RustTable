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
