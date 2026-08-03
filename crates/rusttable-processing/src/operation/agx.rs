use rusttable_core::Operation;

use crate::operations::agx::{AgxBasePrimaries, AgxConfig, AgxParametersV7};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const PARAMETERS: [&str; 36] = [
    "look_lift",
    "look_slope",
    "look_brightness",
    "look_saturation",
    "look_original_hue_mix_ratio",
    "range_black_relative_ev",
    "range_white_relative_ev",
    "dynamic_range_scaling",
    "curve_pivot_x",
    "curve_pivot_y_linear_output",
    "curve_contrast_around_pivot",
    "curve_linear_ratio_below_pivot",
    "curve_linear_ratio_above_pivot",
    "curve_toe_power",
    "curve_shoulder_power",
    "curve_gamma",
    "auto_gamma",
    "curve_target_display_black_ratio",
    "curve_target_display_white_ratio",
    "base_primaries",
    "disable_primaries_adjustments",
    "red_inset",
    "red_rotation",
    "green_inset",
    "green_rotation",
    "blue_inset",
    "blue_rotation",
    "master_outset_ratio",
    "master_unrotation_ratio",
    "red_outset",
    "red_unrotation",
    "green_outset",
    "green_unrotation",
    "blue_outset",
    "blue_unrotation",
    "completely_reverse_primaries",
];

#[expect(
    clippy::too_many_lines,
    reason = "Native AgX parameter compilation keeps the complete fixed-order contract together."
)]
pub fn compile_agx(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &PARAMETERS)?;
    let defaults = AgxParametersV7::defaults();
    let parameters = AgxParametersV7 {
        look_lift: scalar(operation, "look_lift", defaults.look_lift)?,
        look_slope: scalar(operation, "look_slope", defaults.look_slope)?,
        look_brightness: scalar(operation, "look_brightness", defaults.look_brightness)?,
        look_saturation: scalar(operation, "look_saturation", defaults.look_saturation)?,
        look_original_hue_mix_ratio: scalar(
            operation,
            "look_original_hue_mix_ratio",
            defaults.look_original_hue_mix_ratio,
        )?,
        range_black_relative_ev: scalar(
            operation,
            "range_black_relative_ev",
            defaults.range_black_relative_ev,
        )?,
        range_white_relative_ev: scalar(
            operation,
            "range_white_relative_ev",
            defaults.range_white_relative_ev,
        )?,
        dynamic_range_scaling: scalar(
            operation,
            "dynamic_range_scaling",
            defaults.dynamic_range_scaling,
        )?,
        curve_pivot_x: scalar(operation, "curve_pivot_x", defaults.curve_pivot_x)?,
        curve_pivot_y_linear_output: scalar(
            operation,
            "curve_pivot_y_linear_output",
            defaults.curve_pivot_y_linear_output,
        )?,
        curve_contrast_around_pivot: scalar(
            operation,
            "curve_contrast_around_pivot",
            defaults.curve_contrast_around_pivot,
        )?,
        curve_linear_ratio_below_pivot: scalar(
            operation,
            "curve_linear_ratio_below_pivot",
            defaults.curve_linear_ratio_below_pivot,
        )?,
        curve_linear_ratio_above_pivot: scalar(
            operation,
            "curve_linear_ratio_above_pivot",
            defaults.curve_linear_ratio_above_pivot,
        )?,
        curve_toe_power: scalar(operation, "curve_toe_power", defaults.curve_toe_power)?,
        curve_shoulder_power: scalar(
            operation,
            "curve_shoulder_power",
            defaults.curve_shoulder_power,
        )?,
        curve_gamma: scalar(operation, "curve_gamma", defaults.curve_gamma)?,
        auto_gamma: i32::from(super::parameter_bool_default(
            operation,
            "auto_gamma",
            defaults.auto_gamma != 0,
        )?),
        curve_target_display_black_ratio: scalar(
            operation,
            "curve_target_display_black_ratio",
            defaults.curve_target_display_black_ratio,
        )?,
        curve_target_display_white_ratio: scalar(
            operation,
            "curve_target_display_white_ratio",
            defaults.curve_target_display_white_ratio,
        )?,
        base_primaries: AgxBasePrimaries::try_from(super::parameter_integer(
            operation,
            "base_primaries",
            f64::from(defaults.base_primaries as i32),
        )?)
        .map_err(|error| super::invalid_parameters(operation, error))?,
        disable_primaries_adjustments: i32::from(super::parameter_bool_default(
            operation,
            "disable_primaries_adjustments",
            defaults.disable_primaries_adjustments != 0,
        )?),
        red_inset: scalar(operation, "red_inset", defaults.red_inset)?,
        red_rotation: scalar(operation, "red_rotation", defaults.red_rotation)?,
        green_inset: scalar(operation, "green_inset", defaults.green_inset)?,
        green_rotation: scalar(operation, "green_rotation", defaults.green_rotation)?,
        blue_inset: scalar(operation, "blue_inset", defaults.blue_inset)?,
        blue_rotation: scalar(operation, "blue_rotation", defaults.blue_rotation)?,
        master_outset_ratio: scalar(
            operation,
            "master_outset_ratio",
            defaults.master_outset_ratio,
        )?,
        master_unrotation_ratio: scalar(
            operation,
            "master_unrotation_ratio",
            defaults.master_unrotation_ratio,
        )?,
        red_outset: scalar(operation, "red_outset", defaults.red_outset)?,
        red_unrotation: scalar(operation, "red_unrotation", defaults.red_unrotation)?,
        green_outset: scalar(operation, "green_outset", defaults.green_outset)?,
        green_unrotation: scalar(operation, "green_unrotation", defaults.green_unrotation)?,
        blue_outset: scalar(operation, "blue_outset", defaults.blue_outset)?,
        blue_unrotation: scalar(operation, "blue_unrotation", defaults.blue_unrotation)?,
        completely_reverse_primaries: i32::from(super::parameter_bool_default(
            operation,
            "completely_reverse_primaries",
            defaults.completely_reverse_primaries != 0,
        )?),
    };
    let config =
        AgxConfig::new(parameters).map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Agx { config },
    })
}

fn scalar(
    operation: &Operation,
    name: &'static str,
    default: f32,
) -> Result<f32, OperationCompileError> {
    super::parameter_f32(operation, name, f64::from(default))
}
