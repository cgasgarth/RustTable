//! Typed v1 compatibility values shared by the registry and imported-history seam.
//!
//! This module intentionally contains no executor.  Issue #475 owns the Lab CPU
//! implementation; until it lands, the registry publishes the descriptor as an
//! unavailable deprecated operation so GTK can inspect imported histories without
//! pretending that processing is qualified.

use std::fmt;

pub const DEFRINGE_COMPATIBILITY_ID: &str = "defringe";
pub const DEFRINGE_OPERATION_KEY: &str = "rusttable.defringe";
pub const DEFRINGE_SCHEMA_VERSION: u16 = 1;
pub const DEFRINGE_PARAMETER_VERSION: u16 = 1;
pub const DEFRINGE_RADIUS_MINIMUM: f32 = 0.5;
pub const DEFRINGE_RADIUS_MAXIMUM: f32 = 20.0;
pub const DEFRINGE_RADIUS_DEFAULT: f32 = 4.0;
pub const DEFRINGE_THRESHOLD_MINIMUM: f32 = 0.5;
pub const DEFRINGE_THRESHOLD_MAXIMUM: f32 = 128.0;
pub const DEFRINGE_THRESHOLD_DEFAULT: f32 = 20.0;

/// The numeric mode values stored by darktable v1 history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefringeMode {
    GlobalAverage,
    LocalAverage,
    Static,
}

impl DefringeMode {
    #[must_use]
    pub const fn numeric(self) -> i64 {
        match self {
            Self::GlobalAverage => 0,
            Self::LocalAverage => 1,
            Self::Static => 2,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::GlobalAverage => "global_average",
            Self::LocalAverage => "local_average",
            Self::Static => "static",
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::GlobalAverage => 0,
            Self::LocalAverage => 1,
            Self::Static => 2,
        }
    }

    #[must_use]
    pub const fn from_numeric(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::GlobalAverage),
            1 => Some(Self::LocalAverage),
            2 => Some(Self::Static),
            _ => None,
        }
    }
}

/// The typed v1 parameter seam that #475's executor will consume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefringeParametersV1 {
    pub radius: f32,
    pub threshold: f32,
    pub mode: DefringeMode,
}

impl DefringeParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            radius: DEFRINGE_RADIUS_DEFAULT,
            threshold: DEFRINGE_THRESHOLD_DEFAULT,
            mode: DefringeMode::GlobalAverage,
        }
    }

    /// Validates the exact persisted v1 bounds before executor integration.
    ///
    /// # Errors
    ///
    /// Returns the first radius or threshold value outside its v1 bounds.
    pub fn validate(self) -> Result<(), DefringeParameterError> {
        if !self.radius.is_finite()
            || !(DEFRINGE_RADIUS_MINIMUM..=DEFRINGE_RADIUS_MAXIMUM).contains(&self.radius)
        {
            return Err(DefringeParameterError::Radius(self.radius));
        }
        if !self.threshold.is_finite()
            || !(DEFRINGE_THRESHOLD_MINIMUM..=DEFRINGE_THRESHOLD_MAXIMUM).contains(&self.threshold)
        {
            return Err(DefringeParameterError::Threshold(self.threshold));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefringeParameterError {
    Radius(f32),
    Threshold(f32),
}

impl fmt::Display for DefringeParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Radius(value) => write!(formatter, "defringe radius {value} is outside 0.5..=20"),
            Self::Threshold(value) => {
                write!(formatter, "defringe threshold {value} is outside 0.5..=128")
            }
        }
    }
}

impl std::error::Error for DefringeParameterError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_modes_keep_the_imported_numeric_contract() {
        for mode in [
            DefringeMode::GlobalAverage,
            DefringeMode::LocalAverage,
            DefringeMode::Static,
        ] {
            assert_eq!(DefringeMode::from_numeric(mode.numeric()), Some(mode));
        }
        assert_eq!(DefringeMode::from_numeric(-1), None);
        assert_eq!(DefringeMode::from_numeric(3), None);
    }

    #[test]
    fn v1_defaults_and_bounds_are_exact() {
        let defaults = DefringeParametersV1::defaults();
        assert_eq!(defaults.radius.to_bits(), 4.0_f32.to_bits());
        assert_eq!(defaults.threshold.to_bits(), 20.0_f32.to_bits());
        assert_eq!(defaults.mode, DefringeMode::GlobalAverage);
        assert!(defaults.validate().is_ok());
        assert!(
            DefringeParametersV1 {
                radius: 0.49,
                ..defaults
            }
            .validate()
            .is_err()
        );
        assert!(
            DefringeParametersV1 {
                threshold: 128.01,
                ..defaults
            }
            .validate()
            .is_err()
        );
    }
}
