use crate::{FiniteF32, FiniteF32Error};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Matrix3 {
    values: [FiniteF32; 9],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixError {
    NonFinite,
    Singular,
}

impl Matrix3 {
    /// Constructs an invertible 3×3 matrix from row-major values.
    pub fn new(values: [f32; 9]) -> Result<Self, MatrixError> {
        let mut finite = [FiniteF32::zero(); 9];
        for (slot, value) in finite.iter_mut().zip(values) {
            *slot = FiniteF32::new(value).map_err(|_: FiniteF32Error| MatrixError::NonFinite)?;
        }
        let matrix = Self { values: finite };
        matrix.validate()?;
        Ok(matrix)
    }

    pub(crate) fn from_finite(values: [FiniteF32; 9]) -> Result<Self, MatrixError> {
        let matrix = Self { values };
        matrix.validate()?;
        Ok(matrix)
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self {
            values: [
                FiniteF32::from_bits(0x3f80_0000),
                FiniteF32::zero(),
                FiniteF32::zero(),
                FiniteF32::zero(),
                FiniteF32::from_bits(0x3f80_0000),
                FiniteF32::zero(),
                FiniteF32::zero(),
                FiniteF32::zero(),
                FiniteF32::from_bits(0x3f80_0000),
            ],
        }
    }

    #[must_use]
    pub fn rows(self) -> [f32; 9] {
        self.values.map(FiniteF32::get)
    }

    #[must_use]
    pub const fn finite_rows(self) -> [FiniteF32; 9] {
        self.values
    }

    #[must_use]
    pub fn determinant(self) -> f32 {
        let m = self.rows();
        m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
            + m[2] * (m[3] * m[7] - m[4] * m[6])
    }

    /// Returns a checked inverse with deterministic singularity rejection.
    pub fn inverse(self) -> Result<Self, MatrixError> {
        let m = self.rows();
        let determinant = self.determinant();
        if !determinant.is_finite() {
            return Err(MatrixError::NonFinite);
        }
        if determinant.abs() <= 1.0e-7 {
            return Err(MatrixError::Singular);
        }
        let inverse = [
            (m[4] * m[8] - m[5] * m[7]) / determinant,
            (m[2] * m[7] - m[1] * m[8]) / determinant,
            (m[1] * m[5] - m[2] * m[4]) / determinant,
            (m[5] * m[6] - m[3] * m[8]) / determinant,
            (m[0] * m[8] - m[2] * m[6]) / determinant,
            (m[2] * m[3] - m[0] * m[5]) / determinant,
            (m[3] * m[7] - m[4] * m[6]) / determinant,
            (m[1] * m[6] - m[0] * m[7]) / determinant,
            (m[0] * m[4] - m[1] * m[3]) / determinant,
        ];
        Self::new(inverse)
    }

    pub fn multiply(self, rhs: Self) -> Result<Self, MatrixError> {
        let left = self.rows();
        let right = rhs.rows();
        let mut result = [0.0; 9];
        for row in 0..3 {
            for column in 0..3 {
                result[row * 3 + column] = (0..3)
                    .map(|index| left[row * 3 + index] * right[index * 3 + column])
                    .sum();
            }
        }
        Self::new(result)
    }

    #[must_use]
    pub fn apply(self, vector: [f32; 3]) -> [f32; 3] {
        let m = self.rows();
        [
            m[0] * vector[0] + m[1] * vector[1] + m[2] * vector[2],
            m[3] * vector[0] + m[4] * vector[1] + m[5] * vector[2],
            m[6] * vector[0] + m[7] * vector[1] + m[8] * vector[2],
        ]
    }

    fn validate(self) -> Result<Self, MatrixError> {
        if !self.determinant().is_finite() {
            return Err(MatrixError::NonFinite);
        }
        if self.determinant().abs() <= 1.0e-7 {
            return Err(MatrixError::Singular);
        }
        Ok(self)
    }
}

impl Serialize for Matrix3 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Matrix3 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = <[FiniteF32; 9]>::deserialize(deserializer)?;
        Self::from_finite(values).map_err(|error| serde::de::Error::custom(error.to_string()))
    }
}

impl fmt::Display for MatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "matrix contains a non-finite value",
            Self::Singular => "matrix is singular or numerically unstable",
        })
    }
}

impl std::error::Error for MatrixError {}
