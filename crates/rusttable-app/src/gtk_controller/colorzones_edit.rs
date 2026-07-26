//! Revision-safe Color Zones edit actions mapped from Darktable
//! `src/iop/colorzones.c` at the pinned migration baseline.
//!
//! This leaf only authors canonical `rusttable.colorzones` operations. Opaque
//! imported Color Zones history rows remain outside the executable operation
//! stack and are never decoded or promoted here.

use std::fmt;

use rusttable_core::{
    Edit, FiniteF64, Operation, OperationId, ParameterName, ParameterValue, Revision,
};
use rusttable_processing::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_MAX_NODES, COLORZONES_RUST_ID,
    ColorZonesChannel, ColorZonesConfig, ColorZonesCurveType, ColorZonesMode,
    ColorZonesParametersV5, ColorZonesSplinesVersion, FiniteF32, builtin_registry,
};
use rusttable_ui::iop::colorzones::{
    COLORZONES_MIN_X_DISTANCE, ColorZonesEditorError, ColorZonesEditorState,
};
use sha2::{Digest, Sha256};

use super::darkroom_edit::{DARKROOM_CANONICAL_ORDER, canonical_rank};

const MINIMUM_STRENGTH: f32 = -200.0;
const MAXIMUM_STRENGTH: f32 = 200.0;

/// An exact executable target or an explicit request for native defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesEditTarget {
    Operation(OperationId),
    Defaults,
}

/// A checked strength accepted by the native Color Zones parameter contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesStrength(f32);

impl ColorZonesStrength {
    /// # Errors
    ///
    /// Returns [`ColorZonesEditError::InvalidStrength`] when the value is
    /// non-finite or outside the native -200 through 200 range.
    pub fn new(value: f32) -> Result<Self, ColorZonesEditError> {
        if value.is_finite() && (MINIMUM_STRENGTH..=MAXIMUM_STRENGTH).contains(&value) {
            Ok(Self(value))
        } else {
            Err(ColorZonesEditError::InvalidStrength)
        }
    }

    const fn get(self) -> f32 {
        self.0
    }
}

/// One checked graph coordinate in native normalized curve space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesNodePosition {
    x: f32,
    y: f32,
}

impl ColorZonesNodePosition {
    /// # Errors
    ///
    /// Returns [`ColorZonesEditError::InvalidNodePosition`] when either
    /// coordinate is non-finite or outside normalized curve space.
    pub fn new(x: f32, y: f32) -> Result<Self, ColorZonesEditError> {
        if x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) {
            Ok(Self { x, y })
        } else {
            Err(ColorZonesEditError::InvalidNodePosition)
        }
    }
}

/// Native right-click behavior for one curve node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesNodeRemoval {
    Delete,
    ResetToNeutral,
}

/// One typed Color Zones parameter mutation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorZonesEditMutation {
    /// Changing the selection criterion resets all three curves, mode, and
    /// strength exactly as native `gui_changed()` does.
    SetSelectionChannel(ColorZonesChannel),
    SetMode(ColorZonesMode),
    SetStrength(ColorZonesStrength),
    SetSplinesVersion(ColorZonesSplinesVersion),
    SetCurveType {
        curve: ColorZonesChannel,
        curve_type: ColorZonesCurveType,
    },
    InsertNode {
        curve: ColorZonesChannel,
        position: ColorZonesNodePosition,
    },
    MoveNode {
        curve: ColorZonesChannel,
        node: usize,
        position: ColorZonesNodePosition,
    },
    RemoveNode {
        curve: ColorZonesChannel,
        node: usize,
        removal: ColorZonesNodeRemoval,
    },
    ResetCurve(ColorZonesChannel),
}

/// A mutation coupled to the exact edit revision and operation identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesEditAction {
    expected_revision: Revision,
    target: ColorZonesEditTarget,
    mutation: ColorZonesEditMutation,
}

impl ColorZonesEditAction {
    #[must_use]
    pub const fn new(
        expected_revision: Revision,
        target: ColorZonesEditTarget,
        mutation: ColorZonesEditMutation,
    ) -> Self {
        Self {
            expected_revision,
            target,
            mutation,
        }
    }
}

/// The replacement edit and the exact operation instance changed within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedColorZonesEdit {
    edit: Edit,
    operation_id: OperationId,
    changed: bool,
}

impl AppliedColorZonesEdit {
    #[must_use]
    pub const fn edit(&self) -> &Edit {
        &self.edit
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    #[must_use]
    pub fn into_edit(self) -> Edit {
        self.edit
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorZonesEditError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    MissingOperation(OperationId),
    WrongOperation(OperationId),
    ExactTargetRequired,
    InvalidStrength,
    InvalidNodePosition,
    NodeOutOfRange {
        curve: ColorZonesChannel,
        node: usize,
        count: usize,
    },
    CurveAtCapacity(ColorZonesChannel),
    NodeTooClose(ColorZonesChannel),
    InvalidCanonicalOperation(String),
    Revision(String),
}

impl fmt::Display for ColorZonesEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "Color Zones action expected edit revision {expected}, but current revision is {actual}"
            ),
            Self::MissingOperation(operation_id) => {
                write!(
                    formatter,
                    "Color Zones operation {operation_id} does not exist"
                )
            }
            Self::WrongOperation(operation_id) => write!(
                formatter,
                "operation {operation_id} is not an executable Color Zones instance"
            ),
            Self::ExactTargetRequired => formatter.write_str(
                "an exact Color Zones operation ID is required when an executable instance exists",
            ),
            Self::InvalidStrength => {
                formatter.write_str("Color Zones strength must be finite and between -200 and 200")
            }
            Self::InvalidNodePosition => formatter
                .write_str("Color Zones node coordinates must be finite and between zero and one"),
            Self::NodeOutOfRange { curve, node, count } => write!(
                formatter,
                "Color Zones {curve:?} curve node {node} is outside its {count} active nodes"
            ),
            Self::CurveAtCapacity(curve) => write!(
                formatter,
                "Color Zones {curve:?} curve already has {COLORZONES_MAX_NODES} nodes"
            ),
            Self::NodeTooClose(curve) => write!(
                formatter,
                "Color Zones {curve:?} curve nodes must remain more than {COLORZONES_MIN_X_DISTANCE} apart"
            ),
            Self::InvalidCanonicalOperation(message) | Self::Revision(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for ColorZonesEditError {}

/// Applies one action without consulting imported opaque history state.
///
/// Targetless actions materialize disabled native defaults only when no
/// executable Color Zones instance exists. Existing multi-instance edits must
/// always name the exact operation ID.
///
/// # Errors
///
/// Returns a typed revision, target, canonical-parameter, or checked curve
/// mutation error without modifying the supplied immutable edit.
pub fn apply_colorzones_edit(
    current: &Edit,
    action: &ColorZonesEditAction,
) -> Result<AppliedColorZonesEdit, ColorZonesEditError> {
    if current.revision() != action.expected_revision {
        return Err(ColorZonesEditError::StaleRevision {
            expected: action.expected_revision,
            actual: current.revision(),
        });
    }

    let mut operations = current.operations().cloned().collect::<Vec<_>>();
    let materialized_defaults = action.target == ColorZonesEditTarget::Defaults;
    let (target_index, operation_id) = match action.target {
        ColorZonesEditTarget::Operation(operation_id) => {
            let target_index = operations
                .iter()
                .position(|operation| operation.id() == operation_id)
                .ok_or(ColorZonesEditError::MissingOperation(operation_id))?;
            if !is_colorzones_operation(&operations[target_index]) {
                return Err(ColorZonesEditError::WrongOperation(operation_id));
            }
            (target_index, operation_id)
        }
        ColorZonesEditTarget::Defaults => {
            if operations.iter().any(is_colorzones_operation) {
                return Err(ColorZonesEditError::ExactTargetRequired);
            }
            let operation_id = materialized_operation_id(current);
            let defaults = builtin_registry()
                .materialize_operation(COLORZONES_RUST_ID, operation_id)
                .map_err(|error| {
                    ColorZonesEditError::InvalidCanonicalOperation(error.to_string())
                })?;
            let operation = Operation::new_with_opacity(
                defaults.id(),
                defaults.key().clone(),
                false,
                defaults.opacity(),
                defaults
                    .parameters()
                    .map(|(name, value)| (name.clone(), value.clone())),
            )
            .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))?;
            let target_index = canonical_insertion_index(&operations);
            operations.insert(target_index, operation);
            (target_index, operation_id)
        }
    };

    let (completed, mut parameters) = decode_canonical_operation(&operations[target_index])?;
    let mutation_changed = apply_mutation(&mut parameters, action.mutation)?;
    ColorZonesConfig::try_from(&parameters).map_err(|error| {
        ColorZonesEditError::InvalidCanonicalOperation(format!(
            "Color Zones action produced invalid canonical parameters: {error}"
        ))
    })?;
    if !materialized_defaults && !mutation_changed {
        return Ok(AppliedColorZonesEdit {
            edit: current.clone(),
            operation_id,
            changed: false,
        });
    }
    operations[target_index] = encode_canonical_operation(&completed, &parameters)?;

    let edit = current
        .revised(operations)
        .map_err(|error| ColorZonesEditError::Revision(error.to_string()))?;
    Ok(AppliedColorZonesEdit {
        edit,
        operation_id,
        changed: true,
    })
}

fn apply_mutation(
    parameters: &mut ColorZonesParametersV5,
    mutation: ColorZonesEditMutation,
) -> Result<bool, ColorZonesEditError> {
    let previous = *parameters;
    let output_channel = mutation_curve(mutation).unwrap_or(ColorZonesChannel::Lightness);
    let mut editor = ColorZonesEditorState::from_parameters(*parameters, output_channel)
        .map_err(|error| map_editor_error(error, output_channel, parameters))?;

    let result = match mutation {
        ColorZonesEditMutation::SetSelectionChannel(channel) => {
            editor.set_selection_channel(channel);
            Ok(())
        }
        ColorZonesEditMutation::SetMode(mode) => {
            editor.set_mode(mode);
            Ok(())
        }
        ColorZonesEditMutation::SetStrength(strength) => editor.set_strength(strength.get()),
        ColorZonesEditMutation::SetSplinesVersion(version) => {
            editor.set_splines_version(version);
            Ok(())
        }
        ColorZonesEditMutation::SetCurveType { curve, curve_type } => {
            editor.set_curve_type(curve, curve_type);
            Ok(())
        }
        ColorZonesEditMutation::InsertNode { curve, position } => editor
            .insert_node_on(curve, position.x, position.y)
            .map(|_| ()),
        ColorZonesEditMutation::MoveNode {
            curve,
            node,
            position,
        } => editor
            .move_node_on(curve, node, position.x, position.y)
            .map(|_| ()),
        ColorZonesEditMutation::RemoveNode {
            curve,
            node,
            removal: ColorZonesNodeRemoval::Delete,
        } => editor.delete_node_on(curve, node).map(|_| ()),
        ColorZonesEditMutation::RemoveNode {
            curve,
            node,
            removal: ColorZonesNodeRemoval::ResetToNeutral,
        } => editor.neutralize_node_on(curve, node),
        ColorZonesEditMutation::ResetCurve(curve) => {
            editor.reset_curve_on(curve);
            Ok(())
        }
    };
    result.map_err(|error| map_editor_error(error, output_channel, parameters))?;
    *parameters = editor.into_parameters();
    Ok(*parameters != previous)
}

const fn mutation_curve(mutation: ColorZonesEditMutation) -> Option<ColorZonesChannel> {
    match mutation {
        ColorZonesEditMutation::SetCurveType { curve, .. }
        | ColorZonesEditMutation::InsertNode { curve, .. }
        | ColorZonesEditMutation::MoveNode { curve, .. }
        | ColorZonesEditMutation::RemoveNode { curve, .. }
        | ColorZonesEditMutation::ResetCurve(curve) => Some(curve),
        ColorZonesEditMutation::SetSelectionChannel(_)
        | ColorZonesEditMutation::SetMode(_)
        | ColorZonesEditMutation::SetStrength(_)
        | ColorZonesEditMutation::SetSplinesVersion(_) => None,
    }
}

fn map_editor_error(
    error: ColorZonesEditorError,
    curve: ColorZonesChannel,
    parameters: &ColorZonesParametersV5,
) -> ColorZonesEditError {
    match error {
        ColorZonesEditorError::InvalidNodeIndex { node, active } => {
            ColorZonesEditError::NodeOutOfRange {
                curve,
                node,
                count: active,
            }
        }
        ColorZonesEditorError::NodeLimitReached => ColorZonesEditError::CurveAtCapacity(curve),
        ColorZonesEditorError::NodesTooClose => ColorZonesEditError::NodeTooClose(curve),
        error => ColorZonesEditError::InvalidCanonicalOperation(format!(
            "persisted Color Zones parameters are invalid: {error}; selection={}, spline-version={}",
            parameters.channel, parameters.splines_version
        )),
    }
}

fn decode_canonical_operation(
    operation: &Operation,
) -> Result<(Operation, ColorZonesParametersV5), ColorZonesEditError> {
    let defaults = builtin_registry()
        .materialize_operation(COLORZONES_RUST_ID, operation.id())
        .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))?;
    let mut completed_parameters = defaults
        .parameters()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (name, value) in operation.parameters() {
        let Some((_, target)) = completed_parameters
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
        else {
            return Err(ColorZonesEditError::InvalidCanonicalOperation(format!(
                "Color Zones operation contains unexpected parameter {name}"
            )));
        };
        *target = value.clone();
    }
    let completed = Operation::new_with_opacity(
        operation.id(),
        operation.key().clone(),
        operation.is_enabled(),
        operation.opacity(),
        completed_parameters,
    )
    .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))?;

    let mut parameters = ColorZonesParametersV5::defaults();
    for (name, value) in completed.parameters() {
        decode_parameter(&mut parameters, name, value)?;
    }
    ColorZonesConfig::try_from(&parameters).map_err(|error| {
        ColorZonesEditError::InvalidCanonicalOperation(format!(
            "persisted Color Zones parameters are invalid: {error}"
        ))
    })?;
    Ok((completed, parameters))
}

fn decode_parameter(
    parameters: &mut ColorZonesParametersV5,
    name: &ParameterName,
    value: &ParameterValue,
) -> Result<(), ColorZonesEditError> {
    match name.as_str() {
        "channel" => parameters.channel = integer_value(name, value)?,
        "strength" => parameters.strength = scalar_value(name, value)?,
        "mode" => parameters.mode = integer_value(name, value)?,
        "splines_version" => parameters.splines_version = integer_value(name, value)?,
        _ => {
            for curve in 0..COLORZONES_CHANNELS {
                if name.as_str() == curve_count_name(curve) {
                    parameters.curve_num_nodes[curve] = integer_value(name, value)?;
                    return Ok(());
                }
                if name.as_str() == curve_type_name(curve) {
                    parameters.curve_type[curve] = integer_value(name, value)?;
                    return Ok(());
                }
                for node in 0..COLORZONES_MAX_NODES {
                    if name.as_str() == point_name(curve, node, 'x') {
                        parameters.curves[curve][node].x = scalar_value(name, value)?;
                        return Ok(());
                    }
                    if name.as_str() == point_name(curve, node, 'y') {
                        parameters.curves[curve][node].y = scalar_value(name, value)?;
                        return Ok(());
                    }
                }
            }
            return Err(ColorZonesEditError::InvalidCanonicalOperation(format!(
                "Color Zones operation contains unexpected parameter {name}"
            )));
        }
    }
    Ok(())
}

fn integer_value(name: &ParameterName, value: &ParameterValue) -> Result<i32, ColorZonesEditError> {
    let ParameterValue::Integer(value) = value else {
        return Err(wrong_parameter_type(name));
    };
    i32::try_from(*value).map_err(|_| {
        ColorZonesEditError::InvalidCanonicalOperation(format!(
            "Color Zones parameter {name} does not fit a native 32-bit integer"
        ))
    })
}

fn scalar_value(name: &ParameterName, value: &ParameterValue) -> Result<f32, ColorZonesEditError> {
    let ParameterValue::Scalar(value) = value else {
        return Err(wrong_parameter_type(name));
    };
    FiniteF32::try_from(*value)
        .map(FiniteF32::get)
        .map_err(|_| {
            ColorZonesEditError::InvalidCanonicalOperation(format!(
                "Color Zones parameter {name} cannot be represented as a finite native float"
            ))
        })
}

fn wrong_parameter_type(name: &ParameterName) -> ColorZonesEditError {
    ColorZonesEditError::InvalidCanonicalOperation(format!(
        "Color Zones parameter {name} has the wrong value type"
    ))
}

fn encode_canonical_operation(
    operation: &Operation,
    parameters: &ColorZonesParametersV5,
) -> Result<Operation, ColorZonesEditError> {
    let encoded = operation
        .parameters()
        .map(|(name, _)| {
            encode_parameter(parameters, name)
                .map(|value| (name.clone(), value))
                .ok_or_else(|| {
                    ColorZonesEditError::InvalidCanonicalOperation(format!(
                        "Color Zones operation contains unexpected parameter {name}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Operation::new_with_opacity(
        operation.id(),
        operation.key().clone(),
        operation.is_enabled(),
        operation.opacity(),
        encoded,
    )
    .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))
}

fn encode_parameter(
    parameters: &ColorZonesParametersV5,
    name: &ParameterName,
) -> Option<ParameterValue> {
    match name.as_str() {
        "channel" => return Some(ParameterValue::Integer(i64::from(parameters.channel))),
        "strength" => return scalar_parameter(parameters.strength),
        "mode" => return Some(ParameterValue::Integer(i64::from(parameters.mode))),
        "splines_version" => {
            return Some(ParameterValue::Integer(i64::from(
                parameters.splines_version,
            )));
        }
        _ => {}
    }
    for curve in 0..COLORZONES_CHANNELS {
        if name.as_str() == curve_count_name(curve) {
            return Some(ParameterValue::Integer(i64::from(
                parameters.curve_num_nodes[curve],
            )));
        }
        if name.as_str() == curve_type_name(curve) {
            return Some(ParameterValue::Integer(i64::from(
                parameters.curve_type[curve],
            )));
        }
        for node in 0..COLORZONES_MAX_NODES {
            if name.as_str() == point_name(curve, node, 'x') {
                return scalar_parameter(parameters.curves[curve][node].x);
            }
            if name.as_str() == point_name(curve, node, 'y') {
                return scalar_parameter(parameters.curves[curve][node].y);
            }
        }
    }
    None
}

fn scalar_parameter(value: f32) -> Option<ParameterValue> {
    FiniteF64::new(f64::from(value))
        .ok()
        .map(ParameterValue::Scalar)
}

fn is_colorzones_operation(operation: &Operation) -> bool {
    builtin_registry()
        .definition(operation.key().as_str())
        .is_some_and(|definition| {
            definition.descriptor().id.compatibility_name == COLORZONES_COMPATIBILITY_ID
        })
}

fn canonical_insertion_index(operations: &[Operation]) -> usize {
    operations
        .iter()
        .position(|operation| canonical_rank(operation) > canonical_colorzones_rank())
        .unwrap_or(operations.len())
}

fn canonical_colorzones_rank() -> usize {
    DARKROOM_CANONICAL_ORDER
        .iter()
        .position(|candidate| *candidate == COLORZONES_COMPATIBILITY_ID)
        .expect("Color Zones is in the canonical darkroom order")
}

fn materialized_operation_id(edit: &Edit) -> OperationId {
    for nonce in 0_u64.. {
        let mut digest = Sha256::new();
        digest.update(b"rusttable.darkroom.colorzones.materialized-operation.v1\0");
        digest.update(edit.id().get().to_be_bytes());
        digest.update(edit.photo_id().get().to_be_bytes());
        digest.update(edit.revision().get().to_be_bytes());
        digest.update(nonce.to_be_bytes());
        let bytes = digest.finalize();
        let mut id_bytes = [0_u8; 16];
        id_bytes.copy_from_slice(&bytes[..16]);
        let raw = u128::from_be_bytes(id_bytes);
        let candidate = OperationId::new(if raw == 0 { 1 } else { raw })
            .expect("derived Color Zones operation ID is nonzero");
        if edit
            .operations()
            .all(|operation| operation.id() != candidate)
        {
            return candidate;
        }
    }
    unreachable!("the operation ID space cannot be exhausted by a finite edit")
}

fn curve_count_name(curve: usize) -> String {
    format!("curve_{curve}_num_nodes")
}

fn curve_type_name(curve: usize) -> String {
    format!("curve_{curve}_type")
}

fn point_name(curve: usize, node: usize, coordinate: char) -> String {
    format!("curve_{curve}_node_{node}_{coordinate}")
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::float_cmp,
        reason = "source-authored f32 editor coordinates are persisted as exact promoted f64 values"
    )]

    use rusttable_core::{EditId, PhotoId};

    use super::*;

    fn edit(revision: u64, operations: impl IntoIterator<Item = Operation>) -> Edit {
        Edit::from_parts(
            EditId::new(901).expect("edit ID"),
            PhotoId::new(902).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(revision),
            operations,
        )
        .expect("test edit")
    }

    fn operation(key: &str, id: u128) -> Operation {
        builtin_registry()
            .materialize_operation(key, OperationId::new(id).expect("operation ID"))
            .expect("registry operation")
    }

    fn parameter_i64(operation: &Operation, name: &str) -> i64 {
        let name = ParameterName::new(name).expect("parameter name");
        match operation.parameter(&name).expect("parameter exists") {
            ParameterValue::Integer(value) => *value,
            other => panic!("expected integer, got {other:?}"),
        }
    }

    fn parameter_f64(operation: &Operation, name: &str) -> f64 {
        let name = ParameterName::new(name).expect("parameter name");
        match operation.parameter(&name).expect("parameter exists") {
            ParameterValue::Scalar(value) => value.get(),
            other => panic!("expected scalar, got {other:?}"),
        }
    }

    fn colorzones(edit: &Edit, id: OperationId) -> &Operation {
        edit.operations()
            .find(|operation| operation.id() == id)
            .expect("Color Zones operation")
    }

    #[test]
    fn targetless_action_materializes_disabled_defaults_in_source_order() {
        let current = edit(
            7,
            [
                operation("rusttable.vibrance", 11),
                operation("rusttable.bloom", 12),
            ],
        );
        let action = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Defaults,
            ColorZonesEditMutation::SetStrength(ColorZonesStrength::new(25.0).expect("strength")),
        );

        let applied = apply_colorzones_edit(&current, &action).expect("apply targetless action");
        let operations = applied.edit().operations().collect::<Vec<_>>();
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[0].key().as_str(), "rusttable.vibrance");
        assert_eq!(operations[1].key().as_str(), COLORZONES_RUST_ID);
        assert_eq!(operations[2].key().as_str(), "rusttable.bloom");
        assert!(!operations[1].is_enabled());
        assert_eq!(parameter_f64(operations[1], "strength"), 25.0);
        assert!(applied.changed());
        assert_eq!(applied.edit().revision(), Revision::from_u64(8));
    }

    #[test]
    fn stale_revision_and_targetless_existing_instance_are_rejected() {
        let id = OperationId::new(21).expect("operation ID");
        let current = edit(3, [operation(COLORZONES_RUST_ID, id.get())]);
        let stale = ColorZonesEditAction::new(
            Revision::from_u64(2),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetMode(ColorZonesMode::Strong),
        );
        assert!(matches!(
            apply_colorzones_edit(&current, &stale),
            Err(ColorZonesEditError::StaleRevision { .. })
        ));

        let targetless = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Defaults,
            ColorZonesEditMutation::SetMode(ColorZonesMode::Strong),
        );
        assert_eq!(
            apply_colorzones_edit(&current, &targetless),
            Err(ColorZonesEditError::ExactTargetRequired)
        );
    }

    #[test]
    fn exact_instance_action_changes_only_the_named_operation() {
        let first_id = OperationId::new(31).expect("first ID");
        let second_id = OperationId::new(32).expect("second ID");
        let current = edit(
            5,
            [
                operation(COLORZONES_RUST_ID, first_id.get()),
                operation(COLORZONES_RUST_ID, second_id.get()),
            ],
        );
        let action = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(second_id),
            ColorZonesEditMutation::SetMode(ColorZonesMode::Strong),
        );

        let applied = apply_colorzones_edit(&current, &action).expect("apply exact action");
        assert_eq!(applied.operation_id(), second_id);
        assert_eq!(
            parameter_i64(colorzones(applied.edit(), first_id), "mode"),
            0
        );
        assert_eq!(
            parameter_i64(colorzones(applied.edit(), second_id), "mode"),
            1
        );
    }

    #[test]
    fn unchanged_and_rejected_movements_preserve_the_edit_revision() {
        let id = OperationId::new(36).expect("operation ID");
        let current = edit(5, [operation(COLORZONES_RUST_ID, id.get())]);
        let unchanged = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::MoveNode {
                curve: ColorZonesChannel::Lightness,
                node: 0,
                position: ColorZonesNodePosition::new(0.25, 0.5).expect("position"),
            },
        );
        let applied = apply_colorzones_edit(&current, &unchanged).expect("unchanged movement");
        assert!(!applied.changed());
        assert_eq!(applied.edit(), &current);

        let rejected = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::MoveNode {
                curve: ColorZonesChannel::Lightness,
                node: 0,
                position: ColorZonesNodePosition::new(0.75 - COLORZONES_MIN_X_DISTANCE, 0.5)
                    .expect("position"),
            },
        );
        let applied = apply_colorzones_edit(&current, &rejected).expect("rejected movement");
        assert!(!applied.changed());
        assert_eq!(applied.edit(), &current);
    }

    #[test]
    fn selection_change_resets_native_parameters_and_v1_hue_uses_default_interior_nodes() {
        let id = OperationId::new(41).expect("operation ID");
        let current = edit(9, [operation(COLORZONES_RUST_ID, id.get())]);
        let strength = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetStrength(ColorZonesStrength::new(80.0).expect("strength")),
        );
        let strengthened = apply_colorzones_edit(&current, &strength)
            .expect("set strength")
            .into_edit();
        let selection = ColorZonesEditAction::new(
            strengthened.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetSelectionChannel(ColorZonesChannel::Lightness),
        );

        let applied = apply_colorzones_edit(&strengthened, &selection).expect("change selection");
        let operation = colorzones(applied.edit(), id);
        assert_eq!(parameter_i64(operation, "channel"), 0);
        assert_eq!(parameter_i64(operation, "mode"), 0);
        assert_eq!(parameter_i64(operation, "splines_version"), 1);
        assert_eq!(parameter_f64(operation, "strength"), 0.0);
        for curve in 0..COLORZONES_CHANNELS {
            assert_eq!(parameter_i64(operation, &curve_count_name(curve)), 2);
            assert_eq!(parameter_i64(operation, &curve_type_name(curve)), 1);
            assert_eq!(parameter_f64(operation, &point_name(curve, 0, 'x')), 0.0);
            assert_eq!(parameter_f64(operation, &point_name(curve, 1, 'x')), 1.0);
        }

        let v1 = ColorZonesEditAction::new(
            applied.edit().revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetSplinesVersion(ColorZonesSplinesVersion::V1),
        );
        let v1_edit = apply_colorzones_edit(applied.edit(), &v1)
            .expect("switch to spline v1")
            .into_edit();
        let hue = ColorZonesEditAction::new(
            v1_edit.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetSelectionChannel(ColorZonesChannel::Hue),
        );
        let hue_edit = apply_colorzones_edit(&v1_edit, &hue)
            .expect("select Hue under spline v1")
            .into_edit();
        let operation = colorzones(&hue_edit, id);
        assert_eq!(parameter_i64(operation, "splines_version"), 0);
        for curve in 0..COLORZONES_CHANNELS {
            assert_eq!(parameter_f64(operation, &point_name(curve, 0, 'x')), 0.25);
            assert_eq!(parameter_f64(operation, &point_name(curve, 1, 'x')), 0.75);
        }
    }

    #[test]
    fn node_insert_move_reset_and_delete_follow_native_rules() {
        let id = OperationId::new(51).expect("operation ID");
        let mut current = edit(1, [operation(COLORZONES_RUST_ID, id.get())]);
        let target = ColorZonesEditTarget::Operation(id);

        let insert = ColorZonesEditAction::new(
            current.revision(),
            target,
            ColorZonesEditMutation::InsertNode {
                curve: ColorZonesChannel::Chroma,
                position: ColorZonesNodePosition::new(0.5, 0.8).expect("position"),
            },
        );
        current = apply_colorzones_edit(&current, &insert)
            .expect("insert node")
            .into_edit();
        let operation = colorzones(&current, id);
        assert_eq!(parameter_i64(operation, "curve_1_num_nodes"), 3);
        assert_eq!(parameter_f64(operation, "curve_1_node_1_x"), 0.5);
        assert_eq!(
            parameter_f64(operation, "curve_1_node_1_y"),
            f64::from(0.8_f32)
        );

        let move_action = ColorZonesEditAction::new(
            current.revision(),
            target,
            ColorZonesEditMutation::MoveNode {
                curve: ColorZonesChannel::Chroma,
                node: 1,
                position: ColorZonesNodePosition::new(0.6, 0.2).expect("position"),
            },
        );
        current = apply_colorzones_edit(&current, &move_action)
            .expect("move node")
            .into_edit();
        assert_eq!(
            parameter_f64(colorzones(&current, id), "curve_1_node_1_x"),
            f64::from(0.6_f32)
        );

        let neutral = ColorZonesEditAction::new(
            current.revision(),
            target,
            ColorZonesEditMutation::RemoveNode {
                curve: ColorZonesChannel::Chroma,
                node: 1,
                removal: ColorZonesNodeRemoval::ResetToNeutral,
            },
        );
        current = apply_colorzones_edit(&current, &neutral)
            .expect("neutralize node")
            .into_edit();
        assert_eq!(
            parameter_f64(colorzones(&current, id), "curve_1_node_1_y"),
            0.5
        );

        let delete = ColorZonesEditAction::new(
            current.revision(),
            target,
            ColorZonesEditMutation::RemoveNode {
                curve: ColorZonesChannel::Chroma,
                node: 1,
                removal: ColorZonesNodeRemoval::Delete,
            },
        );
        current = apply_colorzones_edit(&current, &delete)
            .expect("delete node")
            .into_edit();
        let operation = colorzones(&current, id);
        assert_eq!(parameter_i64(operation, "curve_1_num_nodes"), 2);
        assert_eq!(parameter_f64(operation, "curve_1_node_2_x"), 0.0);
        assert_eq!(parameter_f64(operation, "curve_1_node_2_y"), 0.0);
    }

    #[test]
    fn per_channel_reset_uses_selection_and_spline_boundary_rules() {
        let id = OperationId::new(61).expect("operation ID");
        let current = edit(2, [operation(COLORZONES_RUST_ID, id.get())]);
        let reset = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::ResetCurve(ColorZonesChannel::Hue),
        );

        let applied = apply_colorzones_edit(&current, &reset).expect("reset Hue curve");
        let operation = colorzones(applied.edit(), id);
        assert_eq!(parameter_i64(operation, "curve_2_num_nodes"), 2);
        assert_eq!(parameter_i64(operation, "curve_2_type"), 1);
        assert_eq!(parameter_f64(operation, "curve_2_node_0_x"), 0.25);
        assert_eq!(parameter_f64(operation, "curve_2_node_1_x"), 0.75);
    }

    #[test]
    fn spline_version_transition_resets_curves_to_native_v1_boundaries() {
        let id = OperationId::new(66).expect("operation ID");
        let current = edit(6, [operation(COLORZONES_RUST_ID, id.get())]);
        let curve_type = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetCurveType {
                curve: ColorZonesChannel::Chroma,
                curve_type: ColorZonesCurveType::Monotone,
            },
        );
        let current = apply_colorzones_edit(&current, &curve_type)
            .expect("set curve interpolation")
            .into_edit();
        let version = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(id),
            ColorZonesEditMutation::SetSplinesVersion(ColorZonesSplinesVersion::V1),
        );

        let applied = apply_colorzones_edit(&current, &version).expect("set spline version");
        let operation = colorzones(applied.edit(), id);
        assert_eq!(parameter_i64(operation, "curve_0_type"), 1);
        assert_eq!(parameter_i64(operation, "curve_1_type"), 1);
        assert_eq!(parameter_i64(operation, "curve_2_type"), 1);
        assert_eq!(parameter_i64(operation, "splines_version"), 0);
        for curve in 0..COLORZONES_CHANNELS {
            assert_eq!(parameter_i64(operation, &curve_count_name(curve)), 2);
            assert_eq!(parameter_f64(operation, &point_name(curve, 0, 'x')), 0.0);
            assert_eq!(parameter_f64(operation, &point_name(curve, 1, 'x')), 1.0);
        }
    }

    #[test]
    fn exact_target_rejects_an_unrelated_operation_id() {
        let exposure_id = OperationId::new(71).expect("exposure ID");
        let current = edit(4, [operation("rusttable.exposure", exposure_id.get())]);
        let action = ColorZonesEditAction::new(
            current.revision(),
            ColorZonesEditTarget::Operation(exposure_id),
            ColorZonesEditMutation::SetMode(ColorZonesMode::Strong),
        );

        assert_eq!(
            apply_colorzones_edit(&current, &action),
            Err(ColorZonesEditError::WrongOperation(exposure_id))
        );
    }
}
