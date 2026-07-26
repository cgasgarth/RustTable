//! Canonical editable Color Zones operation compilation mapped from
//! Darktable `src/iop/colorzones.c` at the pinned migration baseline.
//!
//! The fixed native history bytes remain owned by `operations::colorzones::codec`.
//! This boundary flattens v5 into typed edit parameters, validates authored
//! values, and discards inactive native curve tails before CPU preparation.

use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::colorzones::{
    COLORZONES_CHANNELS, COLORZONES_MAX_NODES, ColorZonesConfig, ColorZonesNode,
    ColorZonesParametersV5, ColorZonesPlan,
};
use crate::{
    FiniteF32, OperationCompileError, ProcessingOperation, ProcessingOperationKind,
    ScalarNarrowingError,
};

const CHANNEL_PARAMETER: &str = "channel";
const MODE_PARAMETER: &str = "mode";
const SPLINES_VERSION_PARAMETER: &str = "splines_version";
const STRENGTH_PARAMETER: &str = "strength";
const CURVE_COORDINATE_MINIMUM: f32 = 0.0;
const CURVE_COORDINATE_MAXIMUM: f32 = 1.0;
const STRENGTH_MINIMUM: f32 = -200.0;
const STRENGTH_MAXIMUM: f32 = 200.0;

pub(crate) fn compile_colorzones(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation)?;
    let defaults = ColorZonesParametersV5::defaults();
    let channel = integer_parameter(operation, CHANNEL_PARAMETER, defaults.channel, 0, 2)?;
    let mode = integer_parameter(operation, MODE_PARAMETER, defaults.mode, 0, 1)?;
    let splines_version = integer_parameter(
        operation,
        SPLINES_VERSION_PARAMETER,
        defaults.splines_version,
        0,
        1,
    )?;
    let strength = scalar_parameter(operation, STRENGTH_PARAMETER, Some(defaults.strength))?;
    validate_range(
        operation,
        STRENGTH_PARAMETER,
        strength,
        STRENGTH_MINIMUM,
        STRENGTH_MAXIMUM,
    )?;

    let mut parameters = ColorZonesParametersV5::new(
        channel,
        [[ColorZonesNode::new(0.0, 0.0); COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
        [0; COLORZONES_CHANNELS],
        [0; COLORZONES_CHANNELS],
        strength,
        mode,
        splines_version,
    );

    for curve in 0..COLORZONES_CHANNELS {
        let count_name = curve_count_name(curve);
        let count = integer_parameter(
            operation,
            &count_name,
            defaults.curve_num_nodes[curve],
            1,
            i32::try_from(COLORZONES_MAX_NODES).expect("Color Zones node limit fits i32"),
        )?;
        let active = usize::try_from(count).expect("validated Color Zones count is positive");
        parameters.curve_num_nodes[curve] = count;

        let type_name = curve_type_name(curve);
        parameters.curve_type[curve] =
            integer_parameter(operation, &type_name, defaults.curve_type[curve], 0, 2)?;

        for node in 0..COLORZONES_MAX_NODES {
            let x_name = point_name(curve, node, 'x');
            let y_name = point_name(curve, node, 'y');
            let active_node = node < active;
            let x_default = default_coordinate(&defaults, curve, node, active_node, true);
            let y_default = default_coordinate(&defaults, curve, node, active_node, false);
            let x = scalar_parameter(operation, &x_name, x_default)?;
            let y = scalar_parameter(operation, &y_name, y_default)?;
            validate_range(
                operation,
                &x_name,
                x,
                CURVE_COORDINATE_MINIMUM,
                CURVE_COORDINATE_MAXIMUM,
            )?;
            validate_range(
                operation,
                &y_name,
                y,
                CURVE_COORDINATE_MINIMUM,
                CURVE_COORDINATE_MAXIMUM,
            )?;
            if active_node {
                parameters.curves[curve][node] = ColorZonesNode::new(x, y);
            }
        }
    }

    let config = ColorZonesConfig::try_from(parameters)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    let plan =
        ColorZonesPlan::new(config).map_err(|error| super::invalid_parameters(operation, error))?;

    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::ColorZones { plan },
    })
}

fn reject_unexpected(operation: &Operation) -> Result<(), OperationCompileError> {
    if let Some((parameter, _)) = operation
        .parameters()
        .find(|(name, _)| !is_colorzones_parameter(name.as_str()))
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: parameter.clone(),
        });
    }
    Ok(())
}

fn is_colorzones_parameter(name: &str) -> bool {
    if matches!(
        name,
        CHANNEL_PARAMETER | MODE_PARAMETER | SPLINES_VERSION_PARAMETER | STRENGTH_PARAMETER
    ) {
        return true;
    }
    (0..COLORZONES_CHANNELS).any(|curve| {
        name == curve_count_name(curve)
            || name == curve_type_name(curve)
            || (0..COLORZONES_MAX_NODES).any(|node| {
                name == point_name(curve, node, 'x') || name == point_name(curve, node, 'y')
            })
    })
}

fn integer_parameter(
    operation: &Operation,
    name: &str,
    default: i32,
    minimum: i32,
    maximum: i32,
) -> Result<i32, OperationCompileError> {
    let parameter = parameter_name(name);
    let value = match operation.parameter(&parameter) {
        None => default,
        Some(ParameterValue::Integer(value)) => i32::try_from(*value).map_err(|_| {
            super::invalid_parameters(operation, format!("{name} must fit a native 32-bit int"))
        })?,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must be between {minimum} and {maximum}"),
        ));
    }
    Ok(value)
}

fn scalar_parameter(
    operation: &Operation,
    name: &str,
    default: Option<f32>,
) -> Result<f32, OperationCompileError> {
    let parameter = parameter_name(name);
    let value = match operation.parameter(&parameter) {
        Some(ParameterValue::Scalar(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
        None => {
            let Some(default) = default else {
                return Err(OperationCompileError::MissingParameter {
                    operation_id: operation.id(),
                    key: operation.key().clone(),
                    parameter,
                });
            };
            return Ok(default);
        }
    };
    FiniteF32::try_from(value)
        .map(FiniteF32::get)
        .map_err(|error| match error {
            ScalarNarrowingError::Overflow => OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            },
            ScalarNarrowingError::Underflow => OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            },
        })
}

fn validate_range(
    operation: &Operation,
    name: &str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), OperationCompileError> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(super::invalid_parameters(
            operation,
            format!("{name} must be between {minimum} and {maximum}"),
        ))
    }
}

fn default_coordinate(
    defaults: &ColorZonesParametersV5,
    curve: usize,
    node: usize,
    active: bool,
    x: bool,
) -> Option<f32> {
    if !active {
        return Some(0.0);
    }
    let default_active = usize::try_from(defaults.curve_num_nodes[curve])
        .expect("native Color Zones default count is positive");
    (node < default_active).then(|| {
        if x {
            defaults.curves[curve][node].x
        } else {
            defaults.curves[curve][node].y
        }
    })
}

fn parameter_name(name: &str) -> ParameterName {
    ParameterName::new(name).expect("generated Color Zones parameter name is valid")
}

pub(crate) fn curve_count_name(curve: usize) -> String {
    format!("curve_{curve}_num_nodes")
}

pub(crate) fn curve_type_name(curve: usize) -> String {
    format!("curve_{curve}_type")
}

pub(crate) fn point_name(curve: usize, node: usize, coordinate: char) -> String {
    format!("curve_{curve}_node_{node}_{coordinate}")
}
