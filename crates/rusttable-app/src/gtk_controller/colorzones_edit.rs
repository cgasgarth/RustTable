//! Revision-safe Color Zones edit replacement mapped from Darktable
//! `src/iop/colorzones.c` at the pinned migration baseline.
//!
//! The GTK leaf owns gesture and curve interaction semantics. This adapter only
//! snapshots canonical v5 parameters and atomically replaces one exact operation.

use std::fmt;

use rusttable_core::{
    Edit, FiniteF64, Operation, OperationId, ParameterName, ParameterValue, Revision,
};
use rusttable_processing::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_MAX_NODES, COLORZONES_RUST_ID,
    ColorZonesChannel, ColorZonesConfig, ColorZonesParametersV5, FiniteF32, builtin_registry,
};
use rusttable_ui::iop::colorzones::{
    ColorZonesEditorState, ColorZonesGraphHeight, ColorZonesGtkPreferences, ColorZonesGtkState,
    ColorZonesSettledAction,
};
use sha2::{Digest, Sha256};

/// Durable global Color Zones GUI preferences mirroring native
/// `plugins/darkroom/colorzones/gui_channel` and
/// `plugins/darkroom/colorzones/graphheight`.
///
/// These are presentation state, never image-operation parameters. Graph height
/// is measured in logical pixels, not percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorZonesGuiPreferences {
    output_channel: ColorZonesChannel,
    graph_height: ColorZonesGraphHeight,
}

impl ColorZonesGuiPreferences {
    #[must_use]
    pub const fn new(
        output_channel: ColorZonesChannel,
        graph_height: ColorZonesGraphHeight,
    ) -> Self {
        Self {
            output_channel,
            graph_height,
        }
    }

    #[must_use]
    pub const fn output_channel(self) -> ColorZonesChannel {
        self.output_channel
    }

    #[must_use]
    pub const fn graph_height(self) -> ColorZonesGraphHeight {
        self.graph_height
    }

    #[must_use]
    pub const fn with_output_channel(mut self, output_channel: ColorZonesChannel) -> Self {
        self.output_channel = output_channel;
        self
    }

    #[must_use]
    pub const fn with_graph_height(mut self, graph_height: ColorZonesGraphHeight) -> Self {
        self.graph_height = graph_height;
        self
    }
}

impl From<ColorZonesGtkPreferences> for ColorZonesGuiPreferences {
    fn from(preferences: ColorZonesGtkPreferences) -> Self {
        Self::new(preferences.output_channel(), preferences.graph_height())
    }
}

impl Default for ColorZonesGuiPreferences {
    fn default() -> Self {
        Self {
            output_channel: ColorZonesChannel::Lightness,
            graph_height: ColorZonesGraphHeight::default(),
        }
    }
}

use super::darkroom_edit::{DARKROOM_CANONICAL_ORDER, canonical_rank};

/// The replacement edit and exact Color Zones instance changed within it.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorZonesEditError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    MissingOperation(OperationId),
    WrongOperation(OperationId),
    MaterializationRequired(OperationId),
    UnexpectedMaterialization(OperationId),
    MaterializationTargetMismatch {
        expected: OperationId,
        actual: OperationId,
    },
    EnableRequirementMismatch {
        operation_id: OperationId,
        expected: bool,
        actual: bool,
    },
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
            Self::MaterializationRequired(operation_id) => write!(
                formatter,
                "Color Zones operation {operation_id} is a default snapshot and must be materialized"
            ),
            Self::UnexpectedMaterialization(operation_id) => write!(
                formatter,
                "Color Zones operation {operation_id} already exists and cannot be materialized again"
            ),
            Self::MaterializationTargetMismatch { expected, actual } => write!(
                formatter,
                "Color Zones default snapshot targets {expected}, not {actual}"
            ),
            Self::EnableRequirementMismatch {
                operation_id,
                expected,
                actual,
            } => write!(
                formatter,
                "Color Zones operation {operation_id} enable requirement is {actual}, but the current snapshot requires {expected}"
            ),
            Self::InvalidCanonicalOperation(message) | Self::Revision(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for ColorZonesEditError {}

/// Projects every persisted Color Zones instance, or one exact unmaterialized
/// default when the edit has no executable instance.
///
/// Both UI-owned durable preferences are supplied by the caller so rebuilds and
/// stale reconciliation preserve the selected output tab and graph height.
///
/// # Errors
///
/// Returns an error when a persisted operation is not a valid canonical v5
/// Color Zones operation.
pub fn colorzones_snapshots(
    edit: &Edit,
    preferences: ColorZonesGuiPreferences,
) -> Result<Vec<ColorZonesGtkState>, ColorZonesEditError> {
    let operations = edit
        .operations()
        .filter(|operation| is_colorzones_operation(operation))
        .collect::<Vec<_>>();
    if operations.is_empty() {
        return Ok(vec![default_snapshot(edit, preferences)?]);
    }
    operations
        .into_iter()
        .map(|operation| operation_snapshot(edit, operation, preferences))
        .collect()
}

/// Resolves controller truth for a mounted leaf after a stale action.
///
/// The same exact instance is preferred. If another transaction removed it,
/// the first current instance (or the current default snapshot) replaces the
/// stale target without replaying the rejected action.
///
/// # Errors
///
/// Returns an error when current persisted Color Zones parameters are invalid.
pub fn reconcile_colorzones_snapshot(
    edit: &Edit,
    preferred_target: OperationId,
    preferences: ColorZonesGuiPreferences,
) -> Result<ColorZonesGtkState, ColorZonesEditError> {
    let snapshots = colorzones_snapshots(edit, preferences)?;
    Ok(snapshots
        .iter()
        .find(|snapshot| snapshot.operation_id() == preferred_target)
        .cloned()
        .unwrap_or_else(|| snapshots[0].clone()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesEditAction {
    target: OperationId,
    expected_revision: Revision,
    parameters: ColorZonesParametersV5,
    enable_required: bool,
    materialization_required: bool,
}

impl ColorZonesEditAction {
    #[must_use]
    pub const fn new(
        target: OperationId,
        expected_revision: Revision,
        parameters: ColorZonesParametersV5,
        enable_required: bool,
        materialization_required: bool,
    ) -> Self {
        Self {
            target,
            expected_revision,
            parameters,
            enable_required,
            materialization_required,
        }
    }

    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn parameters(self) -> ColorZonesParametersV5 {
        self.parameters
    }

    #[must_use]
    pub const fn enable_required(self) -> bool {
        self.enable_required
    }

    #[must_use]
    pub const fn materialization_required(self) -> bool {
        self.materialization_required
    }
}

impl From<ColorZonesSettledAction> for ColorZonesEditAction {
    fn from(action: ColorZonesSettledAction) -> Self {
        Self {
            target: action.target(),
            expected_revision: action.expected_revision(),
            parameters: action.parameters(),
            enable_required: action.enable_required(),
            materialization_required: action.materialization_required(),
        }
    }
}

/// Atomically applies one settled full-parameter replacement.
///
/// Exact target, expected revision, enable state, and materialization state are
/// all checked against the immutable edit. A disabled or default instance is
/// enabled/materialized in the same single replacement. Existing operation ID,
/// key, opacity, and stack position are preserved.
///
/// # Errors
///
/// Returns a typed stale, target, requirement, canonical-parameter, or revision
/// error without modifying the supplied edit.
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
    validate_parameters(&action.parameters)?;

    let mut operations = current.operations().cloned().collect::<Vec<_>>();
    let existing_index = operations
        .iter()
        .position(|operation| operation.id() == action.target());
    let target_index = if let Some(index) = existing_index {
        if !is_colorzones_operation(&operations[index]) {
            return Err(ColorZonesEditError::WrongOperation(action.target()));
        }
        if action.materialization_required() {
            return Err(ColorZonesEditError::UnexpectedMaterialization(
                action.target(),
            ));
        }
        let expected_enable = !operations[index].is_enabled();
        if action.enable_required() != expected_enable {
            return Err(ColorZonesEditError::EnableRequirementMismatch {
                operation_id: action.target(),
                expected: expected_enable,
                actual: action.enable_required(),
            });
        }
        index
    } else {
        if !action.materialization_required() {
            return Err(ColorZonesEditError::MissingOperation(action.target()));
        }
        if operations.iter().any(is_colorzones_operation) {
            return Err(ColorZonesEditError::MaterializationRequired(
                action.target(),
            ));
        }
        let expected_target = materialized_operation_id(current);
        if action.target() != expected_target {
            return Err(ColorZonesEditError::MaterializationTargetMismatch {
                expected: expected_target,
                actual: action.target(),
            });
        }
        if !action.enable_required() {
            return Err(ColorZonesEditError::EnableRequirementMismatch {
                operation_id: action.target(),
                expected: true,
                actual: false,
            });
        }
        let defaults = builtin_registry()
            .materialize_operation(COLORZONES_RUST_ID, action.target())
            .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))?;
        let insertion = canonical_insertion_index(&operations);
        operations.insert(insertion, defaults);
        insertion
    };

    let operation = &operations[target_index];
    let (_, current_parameters) = decode_canonical_operation(operation)?;
    let next_enabled = operation.is_enabled() || action.enable_required();
    if !action.materialization_required()
        && current_parameters == action.parameters()
        && next_enabled == operation.is_enabled()
    {
        return Ok(AppliedColorZonesEdit {
            edit: current.clone(),
            operation_id: action.target(),
            changed: false,
        });
    }

    let completed = complete_canonical_operation(operation)?;
    operations[target_index] =
        encode_canonical_operation(&completed, &action.parameters(), next_enabled)?;
    let edit = current
        .revised(operations)
        .map_err(|error| ColorZonesEditError::Revision(error.to_string()))?;
    Ok(AppliedColorZonesEdit {
        edit,
        operation_id: action.target(),
        changed: true,
    })
}

fn operation_snapshot(
    edit: &Edit,
    operation: &Operation,
    preferences: ColorZonesGuiPreferences,
) -> Result<ColorZonesGtkState, ColorZonesEditError> {
    let (_, parameters) = decode_canonical_operation(operation)?;
    let editor = ColorZonesEditorState::from_parameters(parameters, preferences.output_channel())
        .map_err(|error| {
        ColorZonesEditError::InvalidCanonicalOperation(format!(
            "persisted Color Zones parameters are invalid for the editor: {error}"
        ))
    })?;
    Ok(ColorZonesGtkState::new(
        operation.id(),
        edit.revision(),
        editor,
        operation.is_enabled(),
        operation.opacity(),
        true,
        false,
    )
    .with_graph_height(preferences.graph_height()))
}

fn default_snapshot(
    edit: &Edit,
    preferences: ColorZonesGuiPreferences,
) -> Result<ColorZonesGtkState, ColorZonesEditError> {
    let operation_id = materialized_operation_id(edit);
    let operation = builtin_registry()
        .materialize_operation(COLORZONES_RUST_ID, operation_id)
        .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))?;
    let (_, parameters) = decode_canonical_operation(&operation)?;
    let editor = ColorZonesEditorState::from_parameters(parameters, preferences.output_channel())
        .map_err(|error| {
        ColorZonesEditError::InvalidCanonicalOperation(format!(
            "native Color Zones defaults are invalid for the editor: {error}"
        ))
    })?;
    Ok(ColorZonesGtkState::new(
        operation_id,
        edit.revision(),
        editor,
        false,
        operation.opacity(),
        true,
        true,
    )
    .with_graph_height(preferences.graph_height()))
}

fn validate_parameters(parameters: &ColorZonesParametersV5) -> Result<(), ColorZonesEditError> {
    ColorZonesConfig::try_from(parameters)
        .map(|_| ())
        .map_err(|error| {
            ColorZonesEditError::InvalidCanonicalOperation(format!(
                "Color Zones action contains invalid canonical v5 parameters: {error}"
            ))
        })
}

fn decode_canonical_operation(
    operation: &Operation,
) -> Result<(Operation, ColorZonesParametersV5), ColorZonesEditError> {
    let completed = complete_canonical_operation(operation)?;
    let mut parameters = ColorZonesParametersV5::defaults();
    for (name, value) in completed.parameters() {
        decode_parameter(&mut parameters, name, value)?;
    }
    validate_parameters(&parameters)?;
    Ok((completed, parameters))
}

fn complete_canonical_operation(operation: &Operation) -> Result<Operation, ColorZonesEditError> {
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
    Operation::new_with_opacity(
        operation.id(),
        operation.key().clone(),
        operation.is_enabled(),
        operation.opacity(),
        completed_parameters,
    )
    .map_err(|error| ColorZonesEditError::InvalidCanonicalOperation(error.to_string()))
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
    enabled: bool,
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
        enabled,
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
        digest.update(b"rusttable.darkroom.colorzones.materialized-operation.v2\0");
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
    use rusttable_core::{EditId, OperationOpacity, PhotoId};
    use rusttable_processing::{ColorZonesCurveType, ColorZonesMode};

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

    fn operation(key: &str, id: u128, enabled: bool) -> Operation {
        let defaults = builtin_registry()
            .materialize_operation(key, OperationId::new(id).expect("operation ID"))
            .expect("registry operation");
        Operation::new_with_opacity(
            defaults.id(),
            defaults.key().clone(),
            enabled,
            defaults.opacity(),
            defaults
                .parameters()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .expect("operation")
    }

    fn settled(
        state: &ColorZonesGtkState,
        parameters: &ColorZonesParametersV5,
    ) -> ColorZonesEditAction {
        ColorZonesEditAction {
            target: state.operation_id(),
            expected_revision: state.revision(),
            parameters: *parameters,
            enable_required: !state.enabled(),
            materialization_required: state.materialization_required(),
        }
    }

    #[test]
    fn default_snapshot_materializes_enables_and_replaces_all_parameters_once() {
        let current = edit(
            7,
            [
                operation("rusttable.vibrance", 11, true),
                operation("rusttable.bloom", 12, true),
            ],
        );
        let state = colorzones_snapshots(
            &current,
            ColorZonesGuiPreferences::default().with_output_channel(ColorZonesChannel::Hue),
        )
        .expect("default snapshot")
        .remove(0);
        assert!(state.materialization_required());
        assert!(!state.enabled());
        assert_eq!(state.editor().output_channel(), ColorZonesChannel::Hue);
        let unchanged_defaults = apply_colorzones_edit(
            &current,
            &settled(&state, &state.editor().parameters_value()),
        )
        .expect("materialize unchanged defaults");
        assert!(unchanged_defaults.changed());
        assert_eq!(unchanged_defaults.edit().revision(), Revision::from_u64(8));

        let mut parameters = state.editor().parameters_value();
        parameters.mode = ColorZonesMode::Strong.raw();
        parameters.curve_num_nodes[1] = 3;
        parameters.curve_type[1] = ColorZonesCurveType::Monotone.raw();
        parameters.curves[1][1].x = 0.5;
        parameters.curves[1][1].y = 0.8;

        let applied = apply_colorzones_edit(&current, &settled(&state, &parameters))
            .expect("materialize settled action");
        let operations = applied.edit().operations().collect::<Vec<_>>();
        assert!(applied.changed());
        assert_eq!(applied.edit().revision(), Revision::from_u64(8));
        assert_eq!(operations[1].id(), state.operation_id());
        assert!(operations[1].is_enabled());
        assert_eq!(operations[1].key().as_str(), COLORZONES_RUST_ID);
        assert_eq!(operations[0].key().as_str(), "rusttable.vibrance");
        assert_eq!(operations[2].key().as_str(), "rusttable.bloom");
    }

    #[test]
    fn exact_disabled_instance_preserves_identity_opacity_order_and_enables_atomically() {
        let first = operation(COLORZONES_RUST_ID, 21, true);
        let defaults = operation(COLORZONES_RUST_ID, 22, false);
        let opacity = OperationOpacity::new(0.375).expect("opacity");
        let second = Operation::new_with_opacity(
            defaults.id(),
            defaults.key().clone(),
            false,
            opacity,
            defaults
                .parameters()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .expect("disabled operation");
        let current = edit(3, [first.clone(), second]);
        let states = colorzones_snapshots(
            &current,
            ColorZonesGuiPreferences::default().with_output_channel(ColorZonesChannel::Chroma),
        )
        .expect("instance snapshots");
        let state = states
            .iter()
            .find(|state| state.operation_id().get() == 22)
            .expect("second state");
        let mut parameters = state.editor().parameters_value();
        parameters.strength = 42.0;
        parameters.curve_num_nodes[0] = 3;
        parameters.curves[0][1].x = 0.5;
        parameters.curves[0][1].y = 0.7;

        let applied = apply_colorzones_edit(&current, &settled(state, &parameters))
            .expect("replace exact instance");
        let operations = applied.edit().operations().collect::<Vec<_>>();
        assert_eq!(operations[0], &first);
        assert_eq!(operations[1].id(), state.operation_id());
        assert_eq!(operations[1].opacity(), opacity);
        assert!(operations[1].is_enabled());
        assert_eq!(applied.edit().revision(), Revision::from_u64(4));
    }

    #[test]
    fn no_op_and_stale_actions_do_not_advance_history() {
        let current = edit(5, [operation(COLORZONES_RUST_ID, 31, true)]);
        let state = colorzones_snapshots(&current, ColorZonesGuiPreferences::default())
            .expect("snapshot")
            .remove(0);
        let no_op = settled(&state, &state.editor().parameters_value());
        let applied = apply_colorzones_edit(&current, &no_op).expect("no-op");
        assert!(!applied.changed());
        assert_eq!(applied.edit(), &current);

        let stale_edit = current
            .revised(current.operations().cloned())
            .expect("unrelated revision");
        assert!(matches!(
            apply_colorzones_edit(&stale_edit, &no_op),
            Err(ColorZonesEditError::StaleRevision { .. })
        ));
        let preferences = ColorZonesGuiPreferences::new(
            ColorZonesChannel::Hue,
            ColorZonesGraphHeight::new(219).expect("graph height"),
        );
        let reconciled =
            reconcile_colorzones_snapshot(&stale_edit, state.operation_id(), preferences)
                .expect("controller truth");
        assert_eq!(reconciled.revision(), stale_edit.revision());
        assert_eq!(reconciled.operation_id(), state.operation_id());
        assert_eq!(reconciled.editor().output_channel(), ColorZonesChannel::Hue);
        assert_eq!(reconciled.graph_height().logical_pixels(), 219);
    }

    #[test]
    fn gui_preferences_change_projection_without_revising_image_edit() {
        let current = edit(9, [operation(COLORZONES_RUST_ID, 35, true)]);
        let before_revision = current.revision();
        let preferences = ColorZonesGuiPreferences::default()
            .with_output_channel(ColorZonesChannel::Hue)
            .with_graph_height(ColorZonesGraphHeight::new(241).expect("graph height"));
        let rebuilt =
            colorzones_snapshots(&current, preferences).expect("preference-shaped snapshots");
        assert_eq!(rebuilt[0].editor().output_channel(), ColorZonesChannel::Hue);
        assert_eq!(rebuilt[0].graph_height().logical_pixels(), 241);
        assert_eq!(current.revision(), before_revision);
    }

    #[test]
    fn invalid_full_parameters_are_rejected_without_revising() {
        let current = edit(2, [operation(COLORZONES_RUST_ID, 41, true)]);
        let state = colorzones_snapshots(&current, ColorZonesGuiPreferences::default())
            .expect("snapshot")
            .remove(0);
        let mut invalid = state.editor().parameters_value();
        invalid.mode = 99;
        assert!(matches!(
            apply_colorzones_edit(&current, &settled(&state, &invalid)),
            Err(ColorZonesEditError::InvalidCanonicalOperation(_))
        ));
        assert_eq!(current.revision(), Revision::from_u64(2));
    }
}
