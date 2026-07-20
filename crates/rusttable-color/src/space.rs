use crate::{FiniteF32, FiniteF32Error, Matrix3};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ColorEncoding {
    Unspecified,
    SrgbD65,
    DisplayP3D65,
    LinearSrgbD65,
    LinearDisplayP3D65,
    Rec2020D65,
    LinearRec2020D65,
    AcesCgD60,
    XyzD50,
    XyzD65,
    LabD50,
    LchD50,
    External(crate::ProfileId),
}

#[allow(non_upper_case_globals)]
impl ColorEncoding {
    pub const Srgb: Self = Self::SrgbD65;
    pub const DisplayP3: Self = Self::DisplayP3D65;
    pub const LinearSrgb: Self = Self::LinearSrgbD65;

    #[must_use]
    pub const fn is_explicit(self) -> bool {
        !matches!(self, Self::Unspecified)
    }

    #[must_use]
    pub const fn builtin(self) -> Option<BuiltinSpace> {
        match self {
            Self::SrgbD65 | Self::LinearSrgbD65 => Some(BuiltinSpace::SrgbD65),
            Self::DisplayP3D65 | Self::LinearDisplayP3D65 => Some(BuiltinSpace::DisplayP3D65),
            Self::Rec2020D65 | Self::LinearRec2020D65 => Some(BuiltinSpace::Rec2020D65),
            Self::AcesCgD60 => Some(BuiltinSpace::AcesCgD60),
            Self::XyzD50 => Some(BuiltinSpace::XyzD50),
            Self::XyzD65 => Some(BuiltinSpace::XyzD65),
            Self::LabD50 => Some(BuiltinSpace::LabD50),
            Self::LchD50 => Some(BuiltinSpace::LchD50),
            Self::Unspecified | Self::External(_) => None,
        }
    }

    #[must_use]
    pub const fn transfer(self) -> Option<TransferFunction> {
        match self {
            Self::SrgbD65 | Self::DisplayP3D65 => Some(TransferFunction::Srgb),
            Self::Rec2020D65 => Some(TransferFunction::Rec2020),
            Self::LinearSrgbD65
            | Self::LinearDisplayP3D65
            | Self::LinearRec2020D65
            | Self::AcesCgD60
            | Self::XyzD50
            | Self::XyzD65
            | Self::LabD50
            | Self::LchD50 => Some(TransferFunction::Linear),
            Self::Unspecified | Self::External(_) => None,
        }
    }

    #[must_use]
    pub const fn white_point(self) -> Option<WhitePoint> {
        match self {
            Self::SrgbD65
            | Self::DisplayP3D65
            | Self::LinearSrgbD65
            | Self::LinearDisplayP3D65
            | Self::Rec2020D65
            | Self::LinearRec2020D65
            | Self::XyzD65 => Some(WhitePoint::D65),
            Self::AcesCgD60 => Some(WhitePoint::D60),
            Self::XyzD50 | Self::LabD50 | Self::LchD50 => Some(WhitePoint::D50),
            Self::Unspecified | Self::External(_) => None,
        }
    }

    #[must_use]
    pub const fn is_linear(self) -> bool {
        matches!(
            self,
            Self::LinearSrgbD65
                | Self::LinearDisplayP3D65
                | Self::LinearRec2020D65
                | Self::AcesCgD60
                | Self::XyzD50
                | Self::XyzD65
                | Self::LabD50
                | Self::LchD50
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BuiltinSpace {
    SrgbD65,
    DisplayP3D65,
    Rec2020D65,
    AcesCgD60,
    XyzD50,
    XyzD65,
    LabD50,
    LchD50,
}

impl BuiltinSpace {
    #[must_use]
    pub const fn encoding(self, linear: bool) -> ColorEncoding {
        match (self, linear) {
            (Self::SrgbD65, false) => ColorEncoding::SrgbD65,
            (Self::SrgbD65, true) => ColorEncoding::LinearSrgbD65,
            (Self::DisplayP3D65, false) => ColorEncoding::DisplayP3D65,
            (Self::DisplayP3D65, true) => ColorEncoding::LinearDisplayP3D65,
            (Self::Rec2020D65, false) => ColorEncoding::Rec2020D65,
            (Self::Rec2020D65, true) => ColorEncoding::LinearRec2020D65,
            (Self::AcesCgD60, _) => ColorEncoding::AcesCgD60,
            (Self::XyzD50, _) => ColorEncoding::XyzD50,
            (Self::XyzD65, _) => ColorEncoding::XyzD65,
            (Self::LabD50, _) => ColorEncoding::LabD50,
            (Self::LchD50, _) => ColorEncoding::LchD50,
        }
    }

    #[must_use]
    pub const fn white_point(self) -> WhitePoint {
        match self {
            Self::SrgbD65 | Self::DisplayP3D65 | Self::Rec2020D65 | Self::XyzD65 => WhitePoint::D65,
            Self::AcesCgD60 => WhitePoint::D60,
            Self::XyzD50 | Self::LabD50 | Self::LchD50 => WhitePoint::D50,
        }
    }

    #[must_use]
    pub const fn primaries(self) -> Option<Primaries> {
        match self {
            Self::SrgbD65 => Some(Primaries::srgb()),
            Self::DisplayP3D65 => Some(Primaries::display_p3()),
            Self::Rec2020D65 => Some(Primaries::rec2020()),
            Self::AcesCgD60 => Some(Primaries::aces_cg()),
            Self::XyzD50 | Self::XyzD65 | Self::LabD50 | Self::LchD50 => None,
        }
    }

    #[must_use]
    #[allow(clippy::excessive_precision)]
    pub fn to_xyz_matrix(self) -> Option<Matrix3> {
        let values = match self {
            Self::SrgbD65 => [
                0.412_456_4,
                0.357_576_1,
                0.180_437_5,
                0.212_672_9,
                0.715_152_2,
                0.072_175,
                0.019_333_9,
                0.119_192,
                0.950_304_1,
            ],
            Self::DisplayP3D65 => [
                0.486_570_95,
                0.265_667_69,
                0.198_217_29,
                0.228_974_56,
                0.691_738_52,
                0.079_286_91,
                0.0,
                0.045_113_38,
                1.043_944_37,
            ],
            Self::Rec2020D65 => [
                0.636_958_05,
                0.144_616_9,
                0.168_880_98,
                0.262_700_21,
                0.677_998_07,
                0.059_301_72,
                0.0,
                0.028_072_69,
                1.060_985_06,
            ],
            Self::AcesCgD60 => [
                0.662_454_18,
                0.134_004_2,
                0.156_187_69,
                0.272_228_72,
                0.674_081_74,
                0.053_689_52,
                -0.005_574_65,
                0.004_060_73,
                1.010_339_1,
            ],
            Self::XyzD50 | Self::XyzD65 => return Some(Matrix3::identity()),
            Self::LabD50 | Self::LchD50 => return None,
        };
        Matrix3::new(values).ok()
    }

    #[must_use]
    pub const fn transfer(self) -> TransferFunction {
        match self {
            Self::SrgbD65 | Self::DisplayP3D65 => TransferFunction::Srgb,
            Self::Rec2020D65 => TransferFunction::Rec2020,
            Self::AcesCgD60 | Self::XyzD50 | Self::XyzD65 | Self::LabD50 | Self::LchD50 => {
                TransferFunction::Linear
            }
        }
    }

    #[must_use]
    pub fn is_matrix_space(self) -> bool {
        self.to_xyz_matrix().is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WhitePoint {
    D50,
    D60,
    D65,
    Custom { x: FiniteF32, y: FiniteF32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhitePointError {
    NonFinite,
    OutOfRange,
}

impl WhitePoint {
    pub fn custom(x: f32, y: f32) -> Result<Self, WhitePointError> {
        let x = FiniteF32::new(x).map_err(|_: FiniteF32Error| WhitePointError::NonFinite)?;
        let y = FiniteF32::new(y).map_err(|_: FiniteF32Error| WhitePointError::NonFinite)?;
        if x.get() <= 0.0 || y.get() <= 0.0 || x.get() >= 1.0 || y.get() >= 1.0 {
            return Err(WhitePointError::OutOfRange);
        }
        Ok(Self::Custom { x, y })
    }

    #[must_use]
    pub const fn xy(self) -> (f32, f32) {
        match self {
            Self::D50 => (0.345_7, 0.358_5),
            Self::D60 => (0.321_68, 0.337_67),
            Self::D65 => (0.312_7, 0.329_0),
            Self::Custom { x, y } => (x.get(), y.get()),
        }
    }

    #[must_use]
    pub const fn xyz(self) -> [f32; 3] {
        let (x, y) = self.xy();
        [x / y, 1.0, (1.0 - x - y) / y]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Primaries {
    red: (FiniteF32, FiniteF32),
    green: (FiniteF32, FiniteF32),
    blue: (FiniteF32, FiniteF32),
    white: WhitePoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimariesError {
    NonFinite,
    OutOfRange,
}

impl Primaries {
    pub fn new(
        red: (f32, f32),
        green: (f32, f32),
        blue: (f32, f32),
        white: WhitePoint,
    ) -> Result<Self, PrimariesError> {
        let convert = |(x, y): (f32, f32)| {
            let x = FiniteF32::new(x).map_err(|_: FiniteF32Error| PrimariesError::NonFinite)?;
            let y = FiniteF32::new(y).map_err(|_: FiniteF32Error| PrimariesError::NonFinite)?;
            if x.get() <= 0.0 || y.get() <= 0.0 || x.get() >= 1.0 || y.get() >= 1.0 {
                return Err(PrimariesError::OutOfRange);
            }
            Ok((x, y))
        };
        Ok(Self {
            red: convert(red)?,
            green: convert(green)?,
            blue: convert(blue)?,
            white,
        })
    }

    #[must_use]
    pub const fn srgb() -> Self {
        Self::published((0.64, 0.33), (0.3, 0.6), (0.15, 0.06), WhitePoint::D65)
    }

    #[must_use]
    pub const fn display_p3() -> Self {
        Self::published((0.68, 0.32), (0.265, 0.69), (0.15, 0.06), WhitePoint::D65)
    }

    #[must_use]
    pub const fn rec2020() -> Self {
        Self::published(
            (0.708, 0.292),
            (0.17, 0.797),
            (0.131, 0.046),
            WhitePoint::D65,
        )
    }

    #[must_use]
    pub const fn aces_cg() -> Self {
        Self::published(
            (0.713, 0.293),
            (0.165, 0.83),
            (0.128, 0.044),
            WhitePoint::D60,
        )
    }

    #[must_use]
    pub const fn red(self) -> (FiniteF32, FiniteF32) {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> (FiniteF32, FiniteF32) {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> (FiniteF32, FiniteF32) {
        self.blue
    }

    #[must_use]
    pub const fn white(self) -> WhitePoint {
        self.white
    }

    const fn published(
        red: (f32, f32),
        green: (f32, f32),
        blue: (f32, f32),
        white: WhitePoint,
    ) -> Self {
        Self {
            red: (published_finite(red.0), published_finite(red.1)),
            green: (published_finite(green.0), published_finite(green.1)),
            blue: (published_finite(blue.0), published_finite(blue.1)),
            white,
        }
    }
}

const fn published_finite(value: f32) -> FiniteF32 {
    FiniteF32::from_bits(value.to_bits())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum TransferFunction {
    Linear,
    Srgb,
    Rec709,
    Rec2020,
    Gamma(FiniteF32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferFunctionError {
    NonFinite,
    InvalidGamma,
    Overflow,
}

impl TransferFunction {
    pub fn gamma(value: f32) -> Result<Self, TransferFunctionError> {
        let value = FiniteF32::new(value).map_err(|_| TransferFunctionError::NonFinite)?;
        if value.get() <= 0.0 {
            return Err(TransferFunctionError::InvalidGamma);
        }
        Ok(Self::Gamma(value))
    }

    /// Decodes a transfer-coded scalar without clamping negative or HDR values.
    pub fn decode(self, value: f32) -> Result<f32, TransferFunctionError> {
        self.evaluate(value, false)
    }

    /// Encodes a linear scalar without clamping negative or HDR values.
    pub fn encode(self, value: f32) -> Result<f32, TransferFunctionError> {
        self.evaluate(value, true)
    }

    fn evaluate(self, value: f32, encode: bool) -> Result<f32, TransferFunctionError> {
        if !value.is_finite() {
            return Err(TransferFunctionError::NonFinite);
        }
        let sign = value.signum();
        let magnitude = value.abs();
        let result = match self {
            Self::Linear => magnitude,
            Self::Srgb => {
                if encode {
                    if magnitude <= 0.003_130_8 {
                        12.92 * magnitude
                    } else {
                        1.055 * magnitude.powf(1.0 / 2.4) - 0.055
                    }
                } else if magnitude <= 0.040_45 {
                    magnitude / 12.92
                } else {
                    ((magnitude + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::Rec709 | Self::Rec2020 => {
                if encode {
                    if magnitude < 0.018 {
                        4.5 * magnitude
                    } else {
                        1.099 * magnitude.powf(0.45) - 0.099
                    }
                } else if magnitude < 0.081 {
                    magnitude / 4.5
                } else {
                    ((magnitude + 0.099) / 1.099).powf(1.0 / 0.45)
                }
            }
            Self::Gamma(gamma) => {
                if encode {
                    magnitude.powf(1.0 / gamma.get())
                } else {
                    magnitude.powf(gamma.get())
                }
            }
        };
        let result = sign * result;
        result
            .is_finite()
            .then_some(result)
            .ok_or(TransferFunctionError::Overflow)
    }
}

impl fmt::Display for WhitePointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "white point contains a non-finite value",
            Self::OutOfRange => "white point must be inside the chromaticity domain",
        })
    }
}

impl std::error::Error for WhitePointError {}

impl fmt::Display for PrimariesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "primaries contain a non-finite value",
            Self::OutOfRange => "primary chromaticities must be inside the domain",
        })
    }
}

impl std::error::Error for PrimariesError {}

impl fmt::Display for TransferFunctionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "transfer value is non-finite",
            Self::InvalidGamma => "gamma must be positive",
            Self::Overflow => "transfer evaluation overflowed",
        })
    }
}

impl std::error::Error for TransferFunctionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AdaptationMethod {
    Bradford,
    Identity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ColorRole {
    Input,
    Working,
    Display,
    Export,
    Proof,
    Analysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AlphaMode {
    Opaque,
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExtendedRange {
    Normalized,
    Extended,
}

impl fmt::Display for TransferFunction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}
