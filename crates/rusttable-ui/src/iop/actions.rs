//! Typed actions and errors shared by GTK darkroom module projections.

use std::{fmt, rc::Rc};

use rusttable_core::{OperationId, Revision};

use crate::iop::colorcorrection::ColorCorrectionGridState;
use crate::presentation::darkroom_controls::{DarkroomControlError, DarkroomControlValue};

/// Error returned by a module-level action.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleError {
    NoSelection,
    MissingOperation {
        module_id: String,
    },
    Persistence {
        message: String,
    },
    Preview {
        message: String,
    },
    Unsupported {
        module_id: String,
        reason: String,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Control(DarkroomControlError),
    NotResettable,
    SnapshotRevisionRewind {
        current: Revision,
        replacement: Revision,
    },
    WrongModule {
        expected: String,
        actual: String,
    },
    WrongOperation {
        module_id: String,
        expected: Option<OperationId>,
        actual: Option<OperationId>,
    },
    InstanceActionUnavailable {
        module_id: String,
        action: &'static str,
        reason: &'static str,
    },
    UnknownPreset {
        module_id: String,
        preset_id: String,
    },
    DuplicateModule {
        id: String,
    },
    RevisionOverflow,
}

impl fmt::Display for DarkroomModuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSelection => formatter.write_str("no photo is selected"),
            Self::MissingOperation { module_id } => {
                write!(
                    formatter,
                    "selected photo has no persisted operation {module_id}"
                )
            }
            Self::Persistence { message } => {
                write!(formatter, "edit persistence failed: {message}")
            }
            Self::Preview { message } => write!(formatter, "preview refresh failed: {message}"),
            Self::Unsupported { module_id, reason } => {
                write!(formatter, "module {module_id} is unavailable: {reason}")
            }
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale module callback: expected {expected}, current {actual}"
                )
            }
            Self::Control(error) => write!(formatter, "control error: {error:?}"),
            Self::NotResettable => formatter.write_str("module does not support reset"),
            Self::SnapshotRevisionRewind {
                current,
                replacement,
            } => write!(
                formatter,
                "module snapshot revision {replacement} is older than current {current}"
            ),
            Self::WrongModule { expected, actual } => {
                write!(
                    formatter,
                    "action targets module {expected}, received {actual}"
                )
            }
            Self::WrongOperation {
                module_id,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "action targets {module_id} operation {expected:?}, received {actual:?}"
                )
            }
            Self::InstanceActionUnavailable {
                module_id,
                action,
                reason,
            } => {
                write!(
                    formatter,
                    "{action} is unavailable for module {module_id}: {reason}"
                )
            }
            Self::UnknownPreset {
                module_id,
                preset_id,
            } => {
                write!(
                    formatter,
                    "unknown preset {preset_id} for module {module_id}"
                )
            }
            Self::DuplicateModule { id } => write!(formatter, "duplicate darkroom module: {id}"),
            Self::RevisionOverflow => formatter.write_str("module revision counter overflowed"),
        }
    }
}

impl std::error::Error for DarkroomModuleError {}

/// Last-known module state exposed to a GTK status row.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleStatus {
    Ready,
    Stale {
        expected: Revision,
        actual: Revision,
    },
    Error(DarkroomModuleError),
}

/// A revision-safe action emitted by a module widget.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleAction {
    Disclosure {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        expanded: bool,
    },
    Enable {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        enabled: bool,
    },
    Reset {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    Preset {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        preset_id: String,
    },
    Control {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        id: String,
        value: DarkroomControlValue,
    },
    BloomSettled {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        parameters: rusttable_processing::operations::bloom::BloomParametersV1,
        enable_required: bool,
    },
    ColorCorrectionGrid {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
        grid: ColorCorrectionGridState,
    },
    ColorCorrectionResetParameters {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    NewInstance {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    DuplicateInstance {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    MoveInstanceUp {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    MoveInstanceDown {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    DeleteInstance {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
    Recover {
        module_id: String,
        operation_id: Option<OperationId>,
        expected_revision: Revision,
    },
}

impl DarkroomModuleAction {
    #[must_use]
    pub fn module_id(&self) -> &str {
        match self {
            Self::Disclosure { module_id, .. }
            | Self::Enable { module_id, .. }
            | Self::Reset { module_id, .. }
            | Self::Preset { module_id, .. }
            | Self::Control { module_id, .. }
            | Self::BloomSettled { module_id, .. }
            | Self::ColorCorrectionGrid { module_id, .. }
            | Self::ColorCorrectionResetParameters { module_id, .. }
            | Self::NewInstance { module_id, .. }
            | Self::DuplicateInstance { module_id, .. }
            | Self::MoveInstanceUp { module_id, .. }
            | Self::MoveInstanceDown { module_id, .. }
            | Self::DeleteInstance { module_id, .. }
            | Self::Recover { module_id, .. } => module_id,
        }
    }

    /// Persisted operation instance targeted by this action.
    ///
    /// `None` retains the compatibility path for presentation models that have
    /// not yet been projected from an edit. Persisted multi-instance panels
    /// always emit `Some`.
    #[must_use]
    pub const fn operation_id(&self) -> Option<OperationId> {
        match self {
            Self::Disclosure { operation_id, .. }
            | Self::Enable { operation_id, .. }
            | Self::Reset { operation_id, .. }
            | Self::Preset { operation_id, .. }
            | Self::Control { operation_id, .. }
            | Self::BloomSettled { operation_id, .. }
            | Self::ColorCorrectionGrid { operation_id, .. }
            | Self::ColorCorrectionResetParameters { operation_id, .. }
            | Self::NewInstance { operation_id, .. }
            | Self::DuplicateInstance { operation_id, .. }
            | Self::MoveInstanceUp { operation_id, .. }
            | Self::MoveInstanceDown { operation_id, .. }
            | Self::DeleteInstance { operation_id, .. }
            | Self::Recover { operation_id, .. } => *operation_id,
        }
    }

    /// Returns this action with an application-resolved operation target.
    ///
    /// Compatibility-only actions may be resolved only when the application
    /// has already proven that exactly one matching operation exists.
    #[must_use]
    pub fn with_operation_id(mut self, operation_id: Option<OperationId>) -> Self {
        match &mut self {
            Self::Disclosure {
                operation_id: target,
                ..
            }
            | Self::Enable {
                operation_id: target,
                ..
            }
            | Self::Reset {
                operation_id: target,
                ..
            }
            | Self::Preset {
                operation_id: target,
                ..
            }
            | Self::Control {
                operation_id: target,
                ..
            }
            | Self::BloomSettled {
                operation_id: target,
                ..
            }
            | Self::ColorCorrectionGrid {
                operation_id: target,
                ..
            }
            | Self::ColorCorrectionResetParameters {
                operation_id: target,
                ..
            }
            | Self::NewInstance {
                operation_id: target,
                ..
            }
            | Self::DuplicateInstance {
                operation_id: target,
                ..
            }
            | Self::MoveInstanceUp {
                operation_id: target,
                ..
            }
            | Self::MoveInstanceDown {
                operation_id: target,
                ..
            }
            | Self::DeleteInstance {
                operation_id: target,
                ..
            }
            | Self::Recover {
                operation_id: target,
                ..
            } => *target = operation_id,
        }
        self
    }

    #[must_use]
    pub const fn expected_revision(&self) -> Revision {
        match self {
            Self::Disclosure {
                expected_revision, ..
            }
            | Self::Enable {
                expected_revision, ..
            }
            | Self::Reset {
                expected_revision, ..
            }
            | Self::Preset {
                expected_revision, ..
            }
            | Self::Control {
                expected_revision, ..
            }
            | Self::BloomSettled {
                expected_revision, ..
            }
            | Self::ColorCorrectionGrid {
                expected_revision, ..
            }
            | Self::ColorCorrectionResetParameters {
                expected_revision, ..
            }
            | Self::NewInstance {
                expected_revision, ..
            }
            | Self::DuplicateInstance {
                expected_revision, ..
            }
            | Self::MoveInstanceUp {
                expected_revision, ..
            }
            | Self::MoveInstanceDown {
                expected_revision, ..
            }
            | Self::DeleteInstance {
                expected_revision, ..
            }
            | Self::Recover {
                expected_revision, ..
            } => *expected_revision,
        }
    }

    #[must_use]
    pub const fn is_instance_lifecycle(&self) -> bool {
        matches!(
            self,
            Self::NewInstance { .. }
                | Self::DuplicateInstance { .. }
                | Self::MoveInstanceUp { .. }
                | Self::MoveInstanceDown { .. }
                | Self::DeleteInstance { .. }
        )
    }
}

/// A registry-sourced preset with typed control values.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModulePreset {
    id: String,
    label: String,
    values: Vec<(String, DarkroomControlValue)>,
    color_correction_grid: Option<ColorCorrectionGridState>,
    enables_module: bool,
}

impl DarkroomModulePreset {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        values: Vec<(String, DarkroomControlValue)>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            values,
            color_correction_grid: None,
            enables_module: false,
        }
    }

    #[must_use]
    pub const fn with_color_correction_grid(mut self, grid: ColorCorrectionGridState) -> Self {
        self.color_correction_grid = Some(grid);
        self
    }

    #[must_use]
    pub const fn with_enabled_module(mut self, enabled: bool) -> Self {
        self.enables_module = enabled;
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub(super) fn values(&self) -> &[(String, DarkroomControlValue)] {
        &self.values
    }

    #[must_use]
    pub(super) const fn color_correction_grid(&self) -> Option<ColorCorrectionGridState> {
        self.color_correction_grid
    }

    #[must_use]
    pub const fn enables_module(&self) -> bool {
        self.enables_module
    }
}

/// Callback type used by action-aware GTK module builders.
pub type DarkroomModuleActionHandler =
    Rc<dyn Fn(DarkroomModuleAction) -> Result<Revision, DarkroomModuleError>>;
