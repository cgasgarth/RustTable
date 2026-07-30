//! Display-safe mask-manager state and its application-service port.

#![allow(clippy::missing_errors_doc)]

use std::fmt;

pub const MASK_MANAGER_FOCUS_ORDER: [&str; 9] = [
    "mask-manager-group",
    "mask-manager-create-group",
    "mask-manager-invert",
    "mask-manager-feather",
    "mask-manager-opacity",
    "mask-manager-combination",
    "mask-manager-consumption",
    "mask-manager-refresh",
    "mask-manager-status",
];

pub const MASK_MANAGER_MAX_FEATHER: f64 = 100.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskGroupOption {
    id: String,
    label: String,
}

impl MaskGroupOption {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaskCombination {
    #[default]
    Union,
    Intersection,
    Difference,
    Exclusion,
}

impl MaskCombination {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Union => "Union",
            Self::Intersection => "Intersection",
            Self::Difference => "Difference",
            Self::Exclusion => "Exclusion",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Union,
            Self::Intersection,
            Self::Difference,
            Self::Exclusion,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskConsumptionState {
    NotConsumed,
    ConsumedBy(String),
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskManagerCapability {
    Available,
    Unavailable { reason: String },
}

impl MaskManagerCapability {
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskManagerSnapshot {
    generation: u64,
    capability: MaskManagerCapability,
    groups: Vec<MaskGroupOption>,
    selected_group: Option<String>,
    inverted: bool,
    feather: f64,
    opacity: f64,
    combination: MaskCombination,
    consumption: MaskConsumptionState,
}

impl MaskManagerSnapshot {
    #[must_use]
    pub fn unavailable(generation: u64, reason: impl Into<String>) -> Self {
        Self {
            generation,
            capability: MaskManagerCapability::Unavailable {
                reason: reason.into(),
            },
            groups: Vec::new(),
            selected_group: None,
            inverted: false,
            feather: 0.0,
            opacity: 1.0,
            combination: MaskCombination::Union,
            consumption: MaskConsumptionState::Unavailable {
                reason: "mask consumers are not connected".to_owned(),
            },
        }
    }

    #[must_use]
    pub fn available(
        generation: u64,
        groups: Vec<MaskGroupOption>,
        selected_group: Option<String>,
    ) -> Self {
        Self {
            generation,
            capability: MaskManagerCapability::Available,
            groups,
            selected_group,
            inverted: false,
            feather: 0.0,
            opacity: 1.0,
            combination: MaskCombination::Union,
            consumption: MaskConsumptionState::NotConsumed,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn capability(&self) -> &MaskManagerCapability {
        &self.capability
    }

    #[must_use]
    pub fn groups(&self) -> &[MaskGroupOption] {
        &self.groups
    }

    #[must_use]
    pub fn selected_group(&self) -> Option<&str> {
        self.selected_group.as_deref()
    }

    #[must_use]
    pub const fn inverted(&self) -> bool {
        self.inverted
    }

    #[must_use]
    pub const fn feather(&self) -> f64 {
        self.feather
    }

    #[must_use]
    pub const fn opacity(&self) -> f64 {
        self.opacity
    }

    #[must_use]
    pub const fn combination(&self) -> MaskCombination {
        self.combination
    }

    #[must_use]
    pub const fn consumption(&self) -> &MaskConsumptionState {
        &self.consumption
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MaskManagerAction {
    SelectGroup(Option<String>),
    CreateGroup,
    SetInverted(bool),
    SetFeather(f64),
    SetOpacity(f64),
    SetCombination(MaskCombination),
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskManagerServiceError {
    BackendUnavailable,
    InvalidControl,
    StaleGeneration { expected: u64, actual: u64 },
    Failed(String),
}

impl fmt::Display for MaskManagerServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable => formatter.write_str(
                "mask-manager service is unavailable; no mask graph or catalog change was performed",
            ),
            Self::InvalidControl => formatter.write_str("mask-manager control is invalid"),
            Self::StaleGeneration { expected, actual } => {
                write!(formatter, "stale mask-manager generation: expected {expected}, got {actual}")
            }
            Self::Failed(message) => write!(formatter, "mask-manager service failed: {message}"),
        }
    }
}

impl std::error::Error for MaskManagerServiceError {}

pub trait MaskManagerServicePort {
    fn snapshot(&mut self, generation: u64)
    -> Result<MaskManagerSnapshot, MaskManagerServiceError>;

    fn dispatch(
        &mut self,
        generation: u64,
        action: &MaskManagerAction,
    ) -> Result<MaskManagerSnapshot, MaskManagerServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_snapshot_is_explicit_and_neutral() {
        let snapshot = MaskManagerSnapshot::unavailable(4, "backend not installed");
        assert_eq!(snapshot.generation(), 4);
        assert!(!snapshot.capability().is_available());
        assert!(snapshot.groups().is_empty());
        assert!((snapshot.opacity() - 1.0).abs() < f64::EPSILON);
        assert!(matches!(
            snapshot.consumption(),
            MaskConsumptionState::Unavailable { .. }
        ));
    }

    #[test]
    fn combination_labels_are_stable() {
        assert_eq!(
            MaskCombination::all()
                .into_iter()
                .map(MaskCombination::label)
                .collect::<Vec<_>>(),
            ["Union", "Intersection", "Difference", "Exclusion"]
        );
    }
}
