use std::fmt;

/// Common profile families understood by the RGB AI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileKind {
    Srgb,
    DisplayP3,
    Rec2020,
    Matrix,
}

/// A row-major 3×3 linear RGB transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbMatrix {
    values: [f64; 9],
}

impl RgbMatrix {
    #[must_use]
    pub const fn new(values: [f64; 9]) -> Self {
        Self { values }
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self::new([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
    }

    #[must_use]
    pub const fn values(self) -> [f64; 9] {
        self.values
    }

    #[must_use]
    pub fn apply(self, rgb: [f32; 3]) -> [f32; 3] {
        [
            (self.values[0] * f64::from(rgb[0])
                + self.values[1] * f64::from(rgb[1])
                + self.values[2] * f64::from(rgb[2])) as f32,
            (self.values[3] * f64::from(rgb[0])
                + self.values[4] * f64::from(rgb[1])
                + self.values[5] * f64::from(rgb[2])) as f32,
            (self.values[6] * f64::from(rgb[0])
                + self.values[7] * f64::from(rgb[1])
                + self.values[8] * f64::from(rgb[2])) as f32,
        ]
    }

    /// Returns the inverse after rejecting singular or non-finite matrices.
    pub fn inverse(self) -> Result<Self, ColorProfileError> {
        if self.values.iter().any(|value| !value.is_finite()) {
            return Err(ColorProfileError::NonFiniteMatrix);
        }
        let a = self.values;
        let determinant = a[0] * (a[4] * a[8] - a[5] * a[7]) - a[1] * (a[3] * a[8] - a[5] * a[6])
            + a[2] * (a[3] * a[7] - a[4] * a[6]);
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return Err(ColorProfileError::SingularMatrix);
        }
        let inverse = [
            (a[4] * a[8] - a[5] * a[7]) / determinant,
            (a[2] * a[7] - a[1] * a[8]) / determinant,
            (a[1] * a[5] - a[2] * a[4]) / determinant,
            (a[5] * a[6] - a[3] * a[8]) / determinant,
            (a[0] * a[8] - a[2] * a[6]) / determinant,
            (a[2] * a[3] - a[0] * a[5]) / determinant,
            (a[3] * a[7] - a[4] * a[6]) / determinant,
            (a[1] * a[6] - a[0] * a[7]) / determinant,
            (a[0] * a[4] - a[1] * a[3]) / determinant,
        ];
        if inverse.iter().all(|value| value.is_finite()) {
            Ok(Self::new(inverse))
        } else {
            Err(ColorProfileError::NonFiniteMatrix)
        }
    }

    #[must_use]
    pub fn then(self, next: Self) -> Self {
        let a = self.values;
        let b = next.values;
        Self::new([
            b[0] * a[0] + b[1] * a[3] + b[2] * a[6],
            b[0] * a[1] + b[1] * a[4] + b[2] * a[7],
            b[0] * a[2] + b[1] * a[5] + b[2] * a[8],
            b[3] * a[0] + b[4] * a[3] + b[5] * a[6],
            b[3] * a[1] + b[4] * a[4] + b[5] * a[7],
            b[3] * a[2] + b[4] * a[5] + b[5] * a[8],
            b[6] * a[0] + b[7] * a[3] + b[8] * a[6],
            b[6] * a[1] + b[7] * a[4] + b[8] * a[7],
            b[6] * a[2] + b[7] * a[5] + b[8] * a[8],
        ])
    }
}

/// Immutable profile evidence. The ICC payload is embedded byte-for-byte.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorProfile {
    name: String,
    kind: ProfileKind,
    icc: Vec<u8>,
    to_xyz: RgbMatrix,
}

impl ColorProfile {
    /// Creates a profile with explicit ICC evidence and linear primaries.
    pub fn new(
        name: impl Into<String>,
        kind: ProfileKind,
        icc: Vec<u8>,
        to_xyz: RgbMatrix,
    ) -> Result<Self, ColorProfileError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ColorProfileError::EmptyName);
        }
        if icc.is_empty() {
            return Err(ColorProfileError::MissingIcc);
        }
        to_xyz.inverse()?;
        Ok(Self {
            name,
            kind,
            icc,
            to_xyz,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> ProfileKind {
        self.kind
    }

    #[must_use]
    pub fn icc(&self) -> &[u8] {
        &self.icc
    }

    #[must_use]
    pub fn to_model_srgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.to_xyz
            .then(Self::srgb_to_xyz().inverse().expect("fixed sRGB matrix"))
            .apply(rgb)
    }

    #[must_use]
    pub fn from_model_srgb(&self, rgb: [f32; 3]) -> [f32; 3] {
        Self::srgb_to_xyz()
            .then(self.to_xyz.inverse().expect("validated profile matrix"))
            .apply(rgb)
    }

    #[must_use]
    pub fn srgb_to_xyz() -> RgbMatrix {
        RgbMatrix::new([
            0.4124564, 0.3575761, 0.1804375, 0.2126729, 0.7151522, 0.0721750, 0.0193339, 0.1191920,
            0.9503041,
        ])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorProfileError {
    EmptyName,
    MissingIcc,
    NonFiniteMatrix,
    SingularMatrix,
}

impl fmt::Display for ColorProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyName => "color profile name is empty",
            Self::MissingIcc => "color profile has no ICC payload",
            Self::NonFiniteMatrix => "color profile matrix is non-finite",
            Self::SingularMatrix => "color profile matrix is singular",
        })
    }
}

impl std::error::Error for ColorProfileError {}
