use std::fmt;

use rusttable_core::{Edit, FiniteF64};

use super::parse::checked_value;
use super::{
    BasicEditDraft, BasicEditDraftError, BasicEditDraftReplacementError, BasicEditValue,
    BasicEditValueError,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicEditValues {
    exposure_stops: FiniteF64,
    rgb_red: FiniteF64,
    rgb_green: FiniteF64,
    rgb_blue: FiniteF64,
}

impl BasicEditValues {
    /// Validates and stores the complete typed edit value set.
    ///
    /// All four values are checked before a command can build a replacement edit.
    ///
    /// # Errors
    ///
    /// Returns an error if any value is non-finite or outside its operation range.
    pub fn new(
        exposure_stops: f64,
        rgb_red: f64,
        rgb_green: f64,
        rgb_blue: f64,
    ) -> Result<Self, BasicEditValueError> {
        Ok(Self {
            exposure_stops: checked_value(BasicEditValue::ExposureStops, exposure_stops)?,
            rgb_red: checked_value(BasicEditValue::RgbRed, rgb_red)?,
            rgb_green: checked_value(BasicEditValue::RgbGreen, rgb_green)?,
            rgb_blue: checked_value(BasicEditValue::RgbBlue, rgb_blue)?,
        })
    }

    #[must_use]
    pub const fn exposure_stops(self) -> f64 {
        self.exposure_stops.get()
    }

    #[must_use]
    pub const fn rgb_red(self) -> f64 {
        self.rgb_red.get()
    }

    #[must_use]
    pub const fn rgb_green(self) -> f64 {
        self.rgb_green.get()
    }

    #[must_use]
    pub const fn rgb_blue(self) -> f64 {
        self.rgb_blue.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicEditCommandError {
    InvalidDraft(BasicEditDraftError),
    InvalidValue(BasicEditValueError),
    Replacement(BasicEditDraftReplacementError),
}

impl fmt::Display for BasicEditCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDraft(source) => source.fmt(formatter),
            Self::InvalidValue(source) => source.fmt(formatter),
            Self::Replacement(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for BasicEditCommandError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidDraft(source) => Some(source),
            Self::InvalidValue(source) => Some(source),
            Self::Replacement(source) => Some(source),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicEditCommand {
    draft: BasicEditDraft,
}

impl BasicEditCommand {
    /// Creates a pure in-memory command from the exact persisted edit snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the persisted edit does not contain one valid exposure
    /// operation and one valid RGB-gain operation.
    pub fn from_edit(edit: &Edit) -> Result<Self, BasicEditCommandError> {
        Ok(Self {
            draft: BasicEditDraft::from_edit(edit).map_err(BasicEditCommandError::InvalidDraft)?,
        })
    }

    #[must_use]
    pub const fn draft(&self) -> &BasicEditDraft {
        &self.draft
    }

    #[must_use]
    pub const fn values(&self) -> BasicEditValues {
        BasicEditValues {
            exposure_stops: self.draft.exposure_stops,
            rgb_red: self.draft.rgb_red,
            rgb_green: self.draft.rgb_green,
            rgb_blue: self.draft.rgb_blue,
        }
    }

    /// Validates all values, then builds exactly one canonical in-memory replacement.
    ///
    /// The source draft is immutable and remains unchanged on every error. No partial edit is
    /// returned, so callers can hand the result to a later persistence transaction atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if a supplied value is invalid or the next edit revision cannot build.
    pub fn build_replacement(
        &self,
        exposure_stops: f64,
        rgb_red: f64,
        rgb_green: f64,
        rgb_blue: f64,
    ) -> Result<Edit, BasicEditCommandError> {
        let values = BasicEditValues::new(exposure_stops, rgb_red, rgb_green, rgb_blue)
            .map_err(BasicEditCommandError::InvalidValue)?;
        self.build_replacement_from_values(values)
    }

    /// Builds a replacement from an already validated complete value set.
    ///
    /// # Errors
    ///
    /// Returns an error if the canonical replacement edit cannot be formed.
    pub fn build_replacement_from_values(
        &self,
        values: BasicEditValues,
    ) -> Result<Edit, BasicEditCommandError> {
        let mut draft = self.draft.clone();
        draft
            .set_exposure_stops(values.exposure_stops())
            .map_err(BasicEditCommandError::InvalidValue)?;
        draft
            .set_rgb_red(values.rgb_red())
            .map_err(BasicEditCommandError::InvalidValue)?;
        draft
            .set_rgb_green(values.rgb_green())
            .map_err(BasicEditCommandError::InvalidValue)?;
        draft
            .set_rgb_blue(values.rgb_blue())
            .map_err(BasicEditCommandError::InvalidValue)?;
        draft
            .replacement_edit()
            .map_err(BasicEditCommandError::Replacement)
    }
}
