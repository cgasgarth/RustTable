//! Typed v1 compatibility values for the imported-history-only CLAHE seam.
//!
//! This module deliberately contains no executor. Issue #473 owns the CPU
//! implementation. Its exact integration point is `registry_clahe.rs`: replace
//! the `None` CPU binding and remove `DefinitionAvailability::Unavailable`
//! only when that backend is qualified. GTK must continue to use the canonical
//! `Operation`/edit-history path rather than evaluating these values locally.

use std::fmt;

pub const CLAHE_COMPATIBILITY_ID: &str = "clahe";
pub const CLAHE_OPERATION_KEY: &str = "rusttable.clahe";
pub const CLAHE_SCHEMA_VERSION: u16 = 1;
pub const CLAHE_PARAMETER_VERSION: u16 = 1;
pub const CLAHE_RADIUS_MINIMUM: f64 = 0.0;
pub const CLAHE_RADIUS_MAXIMUM: f64 = 256.0;
pub const CLAHE_RADIUS_DEFAULT: f64 = 64.0;
pub const CLAHE_SLOPE_MINIMUM: f64 = 1.0;
pub const CLAHE_SLOPE_MAXIMUM: f64 = 3.0;
pub const CLAHE_SLOPE_DEFAULT: f64 = 1.25;

/// The exact numeric parameter contract stored by darktable v1 history.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClaheParametersV1 {
    pub radius: f64,
    pub slope: f64,
}

impl ClaheParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            radius: CLAHE_RADIUS_DEFAULT,
            slope: CLAHE_SLOPE_DEFAULT,
        }
    }

    /// Validates persisted values before the future #473 executor consumes them.
    ///
    /// # Errors
    ///
    /// Returns the first non-finite or out-of-range parameter.
    pub fn validate(self) -> Result<(), ClaheParameterError> {
        if !self.radius.is_finite()
            || !(CLAHE_RADIUS_MINIMUM..=CLAHE_RADIUS_MAXIMUM).contains(&self.radius)
        {
            return Err(ClaheParameterError::Radius(self.radius));
        }
        if !self.slope.is_finite()
            || !(CLAHE_SLOPE_MINIMUM..=CLAHE_SLOPE_MAXIMUM).contains(&self.slope)
        {
            return Err(ClaheParameterError::Slope(self.slope));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClaheParameterError {
    Radius(f64),
    Slope(f64),
}

impl fmt::Display for ClaheParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Radius(value) => write!(formatter, "CLAHE radius {value} is outside 0..=256"),
            Self::Slope(value) => write!(formatter, "CLAHE slope {value} is outside 1..=3"),
        }
    }
}

impl std::error::Error for ClaheParameterError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_defaults_and_bounds_are_exact() {
        let defaults = ClaheParametersV1::defaults();
        assert_eq!(defaults.radius.to_bits(), 64.0_f64.to_bits());
        assert_eq!(defaults.slope.to_bits(), 1.25_f64.to_bits());
        assert!(defaults.validate().is_ok());
        assert!(
            ClaheParametersV1 {
                radius: -0.01,
                ..defaults
            }
            .validate()
            .is_err()
        );
        assert!(
            ClaheParametersV1 {
                slope: 3.01,
                ..defaults
            }
            .validate()
            .is_err()
        );
        assert!(
            ClaheParametersV1 {
                radius: f64::NAN,
                ..defaults
            }
            .validate()
            .is_err()
        );
    }
}
