//! Generation-safe application-boundary commands for darkroom controls.

use std::fmt;

use rusttable_core::{PhotoId, Revision};

use super::DarkroomControlValue;
use crate::viewport_presentation::ViewportGeneration;

/// Identity captured by a GTK control before it emits an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DarkroomEditTarget {
    photo_id: PhotoId,
    generation: ViewportGeneration,
}

impl DarkroomEditTarget {
    #[must_use]
    pub const fn new(photo_id: PhotoId, generation: ViewportGeneration) -> Self {
        Self {
            photo_id,
            generation,
        }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }
}

/// A typed operation edit independent of GTK and catalog mutation.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomEditCommand {
    target: DarkroomEditTarget,
    module_id: String,
    parameter_id: String,
    expected_revision: Revision,
    value: DarkroomControlValue,
}

impl DarkroomEditCommand {
    #[must_use]
    pub fn new(
        target: DarkroomEditTarget,
        module_id: impl Into<String>,
        parameter_id: impl Into<String>,
        expected_revision: Revision,
        value: DarkroomControlValue,
    ) -> Self {
        Self {
            target,
            module_id: module_id.into(),
            parameter_id: parameter_id.into(),
            expected_revision,
            value,
        }
    }

    #[must_use]
    pub const fn target(&self) -> DarkroomEditTarget {
        self.target
    }

    #[must_use]
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    #[must_use]
    pub fn parameter_id(&self) -> &str {
        &self.parameter_id
    }

    #[must_use]
    pub const fn expected_revision(&self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn value(&self) -> DarkroomControlValue {
        self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomEditRouteError {
    StaleGeneration {
        expected: DarkroomEditTarget,
        actual: Option<DarkroomEditTarget>,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    RevisionOverflow,
}

impl fmt::Display for DarkroomEditRouteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleGeneration { .. } => {
                formatter.write_str("darkroom edit is from a stale photo generation")
            }
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale darkroom edit revision: expected {expected}, current {actual}"
                )
            }
            Self::RevisionOverflow => formatter.write_str("darkroom edit revision overflowed"),
        }
    }
}

impl std::error::Error for DarkroomEditRouteError {}

/// Typed router used by the application controller before it calls an edit service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomEditRouter {
    target: Option<DarkroomEditTarget>,
    revision: Revision,
}

impl Default for DarkroomEditRouter {
    fn default() -> Self {
        Self {
            target: None,
            revision: Revision::ZERO,
        }
    }
}

impl DarkroomEditRouter {
    #[must_use]
    pub const fn target(&self) -> Option<DarkroomEditTarget> {
        self.target
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Replaces the application-owned current target and revision.
    pub fn reconcile(&mut self, target: DarkroomEditTarget, revision: Revision) {
        self.target = Some(target);
        self.revision = revision;
    }

    /// Accepts only the selected photo generation and current module revision.
    ///
    /// # Errors
    ///
    /// Returns a stale-generation, stale-revision, or overflow error without accepting the edit.
    pub fn route(
        &mut self,
        command: &DarkroomEditCommand,
    ) -> Result<Revision, DarkroomEditRouteError> {
        if self.target != Some(command.target()) {
            return Err(DarkroomEditRouteError::StaleGeneration {
                expected: command.target(),
                actual: self.target,
            });
        }
        if self.revision != command.expected_revision() {
            return Err(DarkroomEditRouteError::StaleRevision {
                expected: command.expected_revision(),
                actual: self.revision,
            });
        }
        self.revision = self
            .revision
            .checked_increment()
            .map_err(|_| DarkroomEditRouteError::RevisionOverflow)?;
        Ok(self.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(generation: u64) -> DarkroomEditTarget {
        DarkroomEditTarget::new(
            PhotoId::new(42).expect("photo"),
            ViewportGeneration::new(generation),
        )
    }

    #[test]
    fn stale_generation_is_rejected_before_parameter_routing() {
        let current = target(2);
        let mut router = DarkroomEditRouter::default();
        router.reconcile(current, Revision::from_u64(8));
        let command = DarkroomEditCommand::new(
            target(1),
            "vignette",
            "scale",
            Revision::from_u64(8),
            DarkroomControlValue::Slider(40.0),
        );
        assert!(matches!(
            router.route(&command),
            Err(DarkroomEditRouteError::StaleGeneration { .. })
        ));
    }

    #[test]
    fn current_generation_and_revision_advance_once() {
        let current = target(2);
        let mut router = DarkroomEditRouter::default();
        router.reconcile(current, Revision::from_u64(8));
        let command = DarkroomEditCommand::new(
            current,
            "graduatednd",
            "density",
            Revision::from_u64(8),
            DarkroomControlValue::Slider(2.0),
        );
        assert_eq!(router.route(&command), Ok(Revision::from_u64(9)));
        assert!(matches!(
            router.route(&command),
            Err(DarkroomEditRouteError::StaleRevision { .. })
        ));
    }
}
