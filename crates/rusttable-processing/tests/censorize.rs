#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::unreadable_literal
)]

use std::str::FromStr;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::operations::censorize::{
    CENSORIZE_PARAMETER_BYTES, CENSORIZE_RNG_VERSION, CensorizeConfig, CensorizeHistory,
    CensorizeParametersV1, CensorizePixel, CensorizePlan, CensorizeRng,
};
use rusttable_processing::{
    CompiledOperationGraph, EvaluationError, FiniteF32, LinearRgb, ProcessingOperation,
    ProcessingOperationKind, RasterDimensions, WorkingRgbImage, builtin_registry, descriptor,
    prepare_basicadj_plans_with_cancellation,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixels(width: usize, height: usize) -> Vec<CensorizePixel> {
    (0..width * height)
        .map(|index| {
            CensorizePixel::new(
                index as f32,
                index as f32,
                index as f32,
                0.25 + index as f32 / 100.0,
            )
        })
        .collect()
}

#[test]
fn v1_codec_defaults_ranges_and_future_history_are_explicit() {
    let parameters = CensorizeParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), CENSORIZE_PARAMETER_BYTES);
    assert_eq!(
        CensorizeParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert!(CensorizeConfig::new(500.0, 500.0, 500.0, 1.0).is_ok());
    assert!(CensorizeConfig::new(-0.01, 0.0, 0.0, 0.0).is_err());
    assert!(CensorizeConfig::new(0.0, 0.0, 0.0, f32::NAN).is_err());
    assert_eq!(
        CensorizeHistory::decode(9, &[1, 2, 3]).expect("opaque history"),
        CensorizeHistory::Opaque {
            version: 9,
            bytes: vec![1, 2, 3]
        }
    );
    assert!(
        !CensorizeHistory::decode(9, &[1])
            .expect("opaque")
            .executable()
    );
}

#[test]
fn descriptor_registry_and_full_frame_contract_are_registered() {
    let value = descriptor::censorize_descriptor();
    value.validate().expect("descriptor");
    assert_eq!(value.id.compatibility_name, "censorize");
    assert_eq!(value.id.parameter_version, 1);
    assert_eq!(value.io.input.channels, 4);
    assert!(
        value
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::STYLE_ELIGIBLE)
    );
    assert!(
        value
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::HISTORY_VISIBLE)
    );
    assert!(
        builtin_registry()
            .definition("rusttable.censorize")
            .is_some()
    );
}

#[test]
fn operation_key_compiles_through_the_registered_v1_parameters() {
    let operation = Operation::new(
        OperationId::new(477).expect("operation ID"),
        OperationKey::from_str("rusttable.censorize").expect("operation key"),
        true,
        [
            ("radius_1", 2.0),
            ("pixelate", 3.0),
            ("radius_2", 4.0),
            ("noise", 0.25),
        ]
        .into_iter()
        .map(|(name, value)| {
            (
                ParameterName::from_str(name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
            )
        }),
    )
    .expect("unique parameters");
    let compiled = ProcessingOperation::compile(&operation).expect("registered operation");
    assert!(matches!(
        compiled.kind(),
        ProcessingOperationKind::Censorize { config }
            if config.parameters() == CensorizeParametersV1::new(2.0, 3.0, 4.0, 0.25)
    ));
}

#[test]
fn five_point_pixelization_has_exact_half_open_partial_bounds() {
    let input = pixels(4, 4);
    let plan = CensorizePlan::new(
        CensorizeConfig::new(0.0, 1.0, 0.0, 0.0).expect("config"),
        dimensions(4, 4),
        1.0,
        1.0,
    )
    .expect("plan");
    let output = plan.execute(&input, || false).expect("pixelization");
    assert_eq!(output[0].red(), 5.0);
    assert_eq!(output[1].red(), 5.0);
    assert_eq!(output[4].red(), 5.0);
    assert_eq!(output[5].red(), 5.0);
    assert_eq!(output[3].red(), input[3].red());
    assert_eq!(output[12].red(), input[12].red());
}

#[test]
fn scale_resolution_memory_and_noise_call_order_are_frozen() {
    let config = CensorizeConfig::new(2.0, 3.0, 4.0, 0.5).expect("config");
    let plan =
        CensorizePlan::with_budget(config, dimensions(8, 4), 2.0, 1.0, usize::MAX).expect("plan");
    assert_eq!(plan.sigma_1(), 1.0);
    assert_eq!(plan.sigma_2(), 2.0);
    assert_eq!(plan.pixel_radius(), 1);
    assert_eq!(plan.effective_noise(), 0.25);
    assert_eq!(plan.noise_calls(), 2);
    assert!(
        plan.stages().pre_blur()
            && plan.stages().pixelization()
            && plan.stages().post_blur()
            && plan.stages().noise()
    );
    assert!(plan.memory_estimate() >= 8 * 4 * 16);
    assert_eq!(plan.backend().tag(), "cpu-scalar-reference");
    assert_eq!(
        CENSORIZE_RNG_VERSION,
        "splitmix32-xoshiro128plus-box-muller.v1"
    );
}

#[test]
fn deterministic_rng_and_output_are_schedule_independent_and_alpha_is_not_noised() {
    assert_eq!(rusttable_processing::splitmix32(1), 387_737_509);
    assert_eq!(rusttable_processing::splitmix32(1337), 635_086_878);
    let mut first = CensorizeRng::for_pixel(3, 2);
    let mut second = CensorizeRng::for_pixel(3, 2);
    let values = [first.next(), first.next(), first.next()];
    assert_eq!(values, [0.98807, 0.7542721, 0.3377921]);
    assert_eq!(values, [second.next(), second.next(), second.next()]);
    let input = vec![CensorizePixel::new(0.2, 0.5, 0.8, 0.125); 9];
    let plan = CensorizePlan::new(
        CensorizeConfig::new(0.0, 0.0, 0.0, 0.5).expect("config"),
        dimensions(3, 3),
        1.0,
        1.0,
    )
    .expect("plan");
    let (one, receipt) = plan
        .execute_with_receipt(&input, None, 1.0, || false)
        .expect("output");
    let two = plan.execute(&input, || false).expect("output");
    assert_eq!(one, two);
    assert!(
        one.iter()
            .all(|pixel| pixel.alpha().to_bits() == 0.125f32.to_bits())
    );
    assert_eq!(receipt.noise_calls(), 1);
    assert_ne!(one, input);
}

#[test]
fn alpha_is_preserved_across_all_stages_and_nonfinite_input_is_rejected() {
    let input = (0..16)
        .map(|index| {
            CensorizePixel::new(index as f32 / 16.0, 0.25, 0.75, 0.1 + index as f32 / 100.0)
        })
        .collect::<Vec<_>>();
    let plan = CensorizePlan::new(
        CensorizeConfig::new(1.0, 2.0, 1.0, 0.25).expect("config"),
        dimensions(4, 4),
        1.0,
        1.0,
    )
    .expect("plan");
    let output = plan.execute(&input, || false).expect("output");
    assert!(
        output
            .iter()
            .zip(&input)
            .all(|(actual, source)| { actual.alpha().to_bits() == source.alpha().to_bits() })
    );

    let mut invalid = input;
    invalid[3] = CensorizePixel::new(f32::NAN, 0.0, 0.0, 1.0);
    assert!(matches!(
        plan.execute(&invalid, || false),
        Err(
            rusttable_processing::CensorizeExecutionError::NonFiniteInput {
                pixel: 3,
                channel: 0
            }
        )
    ));
}

#[test]
fn masks_confine_blending_and_cancellation_publishes_nothing() {
    let input = pixels(4, 4);
    let plan = CensorizePlan::new(
        CensorizeConfig::new(0.0, 1.0, 0.0, 0.0).expect("config"),
        dimensions(4, 4),
        1.0,
        1.0,
    )
    .expect("plan");
    let mask = [
        0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
    ];
    let output = plan
        .execute_with_mask(&input, Some(&mask), 1.0, || false)
        .expect("masked");
    assert_eq!(output[0], input[0]);
    assert_eq!(output[1], input[1]);
    assert_ne!(output[2], input[2]);
    let cancelled = plan.execute(&input, || true);
    assert!(cancelled.is_err());
}

#[test]
fn cancellable_plan_preparation_cancels_censorize_between_pixelization_rows() {
    let operation_id = OperationId::new(478).expect("operation ID");
    let operation = Operation::new(
        operation_id,
        OperationKey::new("rusttable.censorize").expect("operation key"),
        true,
        [
            ("radius_1", 0.0),
            ("pixelate", 1.0),
            ("radius_2", 0.0),
            ("noise", 0.0),
        ]
        .into_iter()
        .map(|(name, value)| {
            (
                ParameterName::new(name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
            )
        }),
    )
    .expect("operation");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("edit");
    let graph = CompiledOperationGraph::compile(&edit).expect("graph");
    let dimensions = dimensions(6, 6);
    let input = WorkingRgbImage::new(
        dimensions,
        (0_u8..36)
            .map(|index| {
                let value = f32::from(index) / 40.0;
                let value = FiniteF32::new(value).expect("finite sample");
                LinearRgb::new(value, value, value)
            })
            .collect(),
    )
    .expect("working image");
    let polls = std::cell::Cell::new(0_usize);

    let error = prepare_basicadj_plans_with_cancellation(&graph, &input, || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 4
    })
    .expect_err("mid-filter cancellation must not publish a partial plan set");

    assert_eq!(
        error,
        EvaluationError::Cancelled {
            step_index: rusttable_processing::PipelineStepIndex::new(0),
            operation_id,
        }
    );
    assert_eq!(polls.get(), 4);
}
