#![allow(clippy::cast_precision_loss)]

use rusttable_masks::MaskRaster;
use rusttable_processing::{
    RasterDimensions, RetouchAlgorithm, RetouchConfig, RetouchExecutionError, RetouchForm,
    RetouchParameters, RetouchPixel, RetouchPlan, RetouchScale,
};

fn dimensions() -> RasterDimensions {
    RasterDimensions::new(8, 8).expect("dimensions")
}
fn input() -> Vec<RetouchPixel> {
    (0..64)
        .map(|index| RetouchPixel::new(index as f32, (index * 2) as f32, 1.0))
        .collect()
}
fn full_mask() -> MaskRaster {
    MaskRaster::new(8, 8, vec![1.0; 64]).expect("mask")
}

#[test]
fn neutral_multiscale_reconstruction_is_exact_and_deterministic() {
    let plan =
        RetouchPlan::new(RetouchConfig::new().with_num_scales(2), dimensions()).expect("plan");
    let (first, receipt) = plan.execute(&input(), || false, |_| {}).expect("execute");
    let (second, second_receipt) = plan.execute(&input(), || false, |_| {}).expect("execute");
    assert_eq!(first, input());
    assert_eq!(first, second);
    assert_eq!(receipt.identity(), second_receipt.identity());
    assert_eq!(receipt.scales(), 2);
    assert_eq!(receipt.tile_count(), 1);
}

#[test]
fn clone_uses_source_offset_and_strength_without_touching_unmasked_pixels() {
    let mask = MaskRaster::new(
        8,
        8,
        (0..64)
            .map(|index| if index == 0 { 1.0 } else { 0.0 })
            .collect(),
    )
    .expect("mask");
    let form = RetouchForm::new(2, RetouchScale::Residual, RetouchAlgorithm::Clone, mask)
        .expect("form")
        .with_source_offset(1, 0)
        .with_strength(0.5)
        .expect("strength");
    let plan = RetouchPlan::new(
        RetouchConfig::new().with_num_scales(1).with_form(form),
        dimensions(),
    )
    .expect("plan");
    let source = input();
    let (output, _) = plan.execute(&source, || false, |_| {}).expect("execute");
    assert!(output[0].channels()[0] > source[0].channels()[0]);
    assert_eq!(output[10], source[10]);
}

#[test]
fn blur_fill_and_cancel_are_distinct_typed_behaviors() {
    let blur = RetouchForm::new(
        1,
        RetouchScale::Residual,
        RetouchAlgorithm::Blur,
        full_mask(),
    )
    .expect("form")
    .with_blur(rusttable_processing::RetouchBlurType::Gaussian, 1.0)
    .expect("blur");
    let fill = RetouchForm::new(
        2,
        RetouchScale::Residual,
        rusttable_processing::RetouchAlgorithm::Fill,
        full_mask(),
    )
    .expect("form")
    .with_fill(
        rusttable_processing::RetouchFillMode::Color,
        RetouchPixel::new(2.0, 3.0, 4.0),
        1.0,
    )
    .expect("fill");
    let blur_plan = RetouchPlan::new(
        RetouchConfig::new().with_num_scales(1).with_form(blur),
        dimensions(),
    )
    .expect("plan");
    let fill_plan = RetouchPlan::new(
        RetouchConfig::new().with_num_scales(1).with_form(fill),
        dimensions(),
    )
    .expect("plan");
    let (blurred, _) = blur_plan.execute(&input(), || false, |_| {}).expect("blur");
    let (filled, _) = fill_plan.execute(&input(), || false, |_| {}).expect("fill");
    assert_ne!(blurred, filled);
    assert_eq!(
        blur_plan.execute(&input(), || true, |_| {}),
        Err(RetouchExecutionError::Cancelled)
    );
}

#[test]
fn resource_planning_rejects_oversized_work_before_execution() {
    let parameters = RetouchParameters::new(2).with_memory_budget(1);
    assert!(matches!(
        RetouchPlan::new(parameters.config(), dimensions()),
        Err(RetouchExecutionError::MemoryBudgetExceeded { .. })
    ));
}
