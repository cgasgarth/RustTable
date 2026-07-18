use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use rusttable_core::FiniteF64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiniteF32Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarNarrowingError {
    Overflow,
    Underflow,
}

#[derive(Debug, Clone, Copy)]
pub struct FiniteF32(f32);

impl FiniteF32 {
    /// Creates a finite scalar and canonicalizes negative zero.
    ///
    /// # Errors
    ///
    /// Returns [`FiniteF32Error`] for NaN or either infinity.
    pub fn new(value: f32) -> Result<Self, FiniteF32Error> {
        if !value.is_finite() {
            return Err(FiniteF32Error);
        }
        Ok(Self(if value == 0.0 { 0.0 } else { value }))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }

    pub(crate) const fn from_proven_finite(value: f32) -> Self {
        Self(value)
    }
}

impl TryFrom<FiniteF64> for FiniteF32 {
    type Error = ScalarNarrowingError;

    #[allow(
        clippy::cast_possible_truncation,
        reason = "the processing boundary intentionally narrows after validating the result"
    )]
    fn try_from(value: FiniteF64) -> Result<Self, Self::Error> {
        let source = value.get();
        let narrowed = source as f32;
        if !narrowed.is_finite() {
            return Err(ScalarNarrowingError::Overflow);
        }
        if source != 0.0 && narrowed == 0.0 {
            return Err(ScalarNarrowingError::Underflow);
        }
        Self::new(narrowed).map_err(|_| ScalarNarrowingError::Overflow)
    }
}

impl PartialEq for FiniteF32 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for FiniteF32 {}

impl PartialOrd for FiniteF32 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FiniteF32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for FiniteF32 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl fmt::Display for FiniteF32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}
