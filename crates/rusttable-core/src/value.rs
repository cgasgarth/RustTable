use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevisionOverflow;

impl Revision {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advances this revision without wrapping.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionOverflow`] when the revision is already at `u64::MAX`.
    pub const fn checked_increment(self) -> Result<Self, RevisionOverflow> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(RevisionOverflow),
        }
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiniteF64Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOpacityError {
    NonFinite,
    BelowZero,
    AboveOne,
}

#[derive(Debug, Clone, Copy)]
pub struct OperationOpacity(FiniteF64);

impl OperationOpacity {
    pub const ZERO: Self = Self(FiniteF64(0.0));
    pub const ONE: Self = Self(FiniteF64(1.0));

    /// Creates opacity from a finite value in the inclusive unit interval.
    ///
    /// # Errors
    ///
    /// Returns [`OperationOpacityError::NonFinite`] for NaN or infinity,
    /// [`OperationOpacityError::BelowZero`] for negative values, or
    /// [`OperationOpacityError::AboveOne`] for values greater than one.
    pub fn new(value: f64) -> Result<Self, OperationOpacityError> {
        let value = FiniteF64::new(value).map_err(|_| OperationOpacityError::NonFinite)?;
        if value.get() < 0.0 {
            return Err(OperationOpacityError::BelowZero);
        }
        if value.get() > 1.0 {
            return Err(OperationOpacityError::AboveOne);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0.get()
    }
}

impl PartialEq for OperationOpacity {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for OperationOpacity {}

impl PartialOrd for OperationOpacity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OperationOpacity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl Hash for OperationOpacity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for OperationOpacity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FiniteF64(f64);

impl FiniteF64 {
    /// Creates a finite value and canonicalizes negative zero.
    ///
    /// # Errors
    ///
    /// Returns [`FiniteF64Error`] for NaN or either infinity.
    pub fn new(value: f64) -> Result<Self, FiniteF64Error> {
        if !value.is_finite() {
            return Err(FiniteF64Error);
        }
        Ok(Self(if value == 0.0 { 0.0 } else { value }))
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for FiniteF64 {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for FiniteF64 {}

impl PartialOrd for FiniteF64 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FiniteF64 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for FiniteF64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl fmt::Display for OperationOpacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NonFinite => "operation opacity must be finite",
            Self::BelowZero => "operation opacity must not be below zero",
            Self::AboveOne => "operation opacity must not be above one",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for OperationOpacityError {}
