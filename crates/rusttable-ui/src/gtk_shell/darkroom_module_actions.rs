//! Typed actions and errors shared by GTK darkroom module projections.

use std::{fmt, rc::Rc};

use rusttable_core::Revision;

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
        expected_revision: Revision,
        expanded: bool,
    },
    Enable {
        module_id: String,
        expected_revision: Revision,
        enabled: bool,
    },
    Reset {
        module_id: String,
        expected_revision: Revision,
    },
    Preset {
        module_id: String,
        expected_revision: Revision,
        preset_id: String,
    },
    Control {
        module_id: String,
        expected_revision: Revision,
        id: String,
        value: DarkroomControlValue,
    },
    Recover {
        module_id: String,
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
            | Self::Recover { module_id, .. } => module_id,
        }
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
            | Self::Recover {
                expected_revision, ..
            } => *expected_revision,
        }
    }
}

/// A registry-sourced preset with typed control values.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModulePreset {
    id: String,
    label: String,
    values: Vec<(String, DarkroomControlValue)>,
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
        }
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
}

/// Callback type used by action-aware GTK module builders.
pub type DarkroomModuleActionHandler =
    Rc<dyn Fn(DarkroomModuleAction) -> Result<Revision, DarkroomModuleError>>;
