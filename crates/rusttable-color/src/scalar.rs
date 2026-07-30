use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FiniteF32(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiniteF32Error {
    NonFinite,
}

impl FiniteF32 {
    /// Creates a finite value with a canonical positive zero representation.
    pub fn new(value: f32) -> Result<Self, FiniteF32Error> {
        if !value.is_finite() {
            return Err(FiniteF32Error::NonFinite);
        }
        Ok(Self(if value == 0.0 { 0 } else { value.to_bits() }))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        f32::from_bits(self.0)
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    pub(crate) const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn zero() -> Self {
        Self(0)
    }

    #[must_use]
    pub fn is_negative(self) -> bool {
        self.get().is_sign_negative()
    }
}

impl Serialize for FiniteF32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f32(self.get())
    }
}

impl<'de> Deserialize<'de> for FiniteF32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("value must be finite"))
    }
}

impl fmt::Display for FiniteF32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

impl fmt::Display for FiniteF32Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("color scalar must be finite")
    }
}

impl std::error::Error for FiniteF32Error {}
