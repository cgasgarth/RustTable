//! Explicit LUT profile evidence for the bounded CPU leaf.
//!
//! Native profile construction and ICC/LCMS acquisition remain a shared seam.
//! This type accepts only matrices supplied by that future seam and never
//! silently assumes that the current working RGB space is the LUT space.

use std::fmt;

use super::codec::Lut3dColorspace;

pub type Matrix3 = [[f32; 3]; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lut3dProfileEvidence {
    BuiltIn(Lut3dColorspace),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lut3dProfileContext {
    evidence: Lut3dProfileEvidence,
    /// Row-major working RGB → selected LUT RGB transform.
    working_to_lut: Matrix3,
    /// Row-major selected LUT RGB → working RGB transform.
    lut_to_working: Matrix3,
}

impl Lut3dProfileContext {
    /// Constructs context only when explicit profile evidence and finite
    /// transforms have been provided by the integration boundary.
    pub fn from_builtin(
        colorspace: Lut3dColorspace,
        working_to_lut: Matrix3,
        lut_to_working: Matrix3,
    ) -> Result<Self, Lut3dProfileError> {
        let context = Self {
            evidence: Lut3dProfileEvidence::BuiltIn(colorspace),
            working_to_lut,
            lut_to_working,
        };
        context.validate()?;
        Ok(context)
    }

    pub fn validate(self) -> Result<(), Lut3dProfileError> {
        if self
            .working_to_lut
            .into_iter()
            .flatten()
            .chain(self.lut_to_working.into_iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err(Lut3dProfileError::NonFiniteMatrix);
        }
        Ok(())
    }

    #[must_use]
    pub const fn evidence(self) -> Lut3dProfileEvidence {
        self.evidence
    }

    #[must_use]
    pub const fn colorspace(self) -> Lut3dColorspace {
        match self.evidence {
            Lut3dProfileEvidence::BuiltIn(colorspace) => colorspace,
        }
    }

    #[must_use]
    pub const fn working_to_lut(self) -> Matrix3 {
        self.working_to_lut
    }

    #[must_use]
    pub const fn lut_to_working(self) -> Matrix3 {
        self.lut_to_working
    }

    #[must_use]
    pub fn transform_working_to_lut(self, rgb: [f32; 3]) -> [f32; 3] {
        transform(self.working_to_lut, rgb)
    }

    #[must_use]
    pub fn transform_lut_to_working(self, rgb: [f32; 3]) -> [f32; 3] {
        transform(self.lut_to_working, rgb)
    }
}

fn transform(matrix: Matrix3, rgb: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * rgb[0] + matrix[0][1] * rgb[1] + matrix[0][2] * rgb[2],
        matrix[1][0] * rgb[0] + matrix[1][1] * rgb[1] + matrix[1][2] * rgb[2],
        matrix[2][0] * rgb[0] + matrix[2][1] * rgb[1] + matrix[2][2] * rgb[2],
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lut3dProfileError {
    NonFiniteMatrix,
}

impl fmt::Display for Lut3dProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteMatrix => {
                formatter.write_str("LUT3D profile matrix contains a non-finite value")
            }
        }
    }
}

impl std::error::Error for Lut3dProfileError {}
