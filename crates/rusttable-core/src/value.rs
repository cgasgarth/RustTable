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
