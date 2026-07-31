//! Bounded Sigmoid CPU leaf ported from `src/iop/sigmoid.c`.
//!
//! The leaf owns the native v3 parameter ABI, v1/v2 migrations, the generalized
//! log-logistic scene-to-display transfer, RGB-ratio and per-channel paths,
//! native profile/primaries matrix arithmetic, alpha preservation, checked
//! finite execution, and the cancellation/publication boundary. Registry,
//! history materialization, pixelpipe routing, GPU/OpenCL execution, GTK,
//! presets, masks, and outer blending remain explicitly deferred rather than
//! approximated.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::if_same_then_else,
    clippy::manual_clamp,
    clippy::manual_range_contains,
    clippy::manual_saturating_arithmetic,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    dead_code,
    reason = "native ABI and scalar raster expressions retain source-shaped f32 boundaries"
)]

use std::fmt;
use std::mem::size_of;

use rusttable_processing::RasterDimensions;

pub mod source_map;

pub const SIGMOID_COMPATIBILITY_ID: &str = "sigmoid";
pub const SIGMOID_RUST_ID: &str = "rusttable.sigmoid";
pub const SIGMOID_SCHEMA_VERSION: u16 = 3;
pub const SIGMOID_PARAMETER_BYTES_V1: usize = 24;
pub const SIGMOID_PARAMETER_BYTES_V2: usize = 52;
pub const SIGMOID_PARAMETER_BYTES_V3: usize = 56;
pub const SIGMOID_DEFAULT_COLORSPACE: &str = "RGB";
pub const SIGMOID_DEFAULT_GROUPS: [&str; 2] = ["tone", "technical"];
pub const SIGMOID_SUPPORTS_BLENDING: bool = true;
pub const SIGMOID_GPU_PROGRAM: u32 = 36;
pub const SIGMOID_GPU_KERNELS: [&str; 2] = [
    "sigmoid_loglogistic_per_channel",
    "sigmoid_loglogistic_rgb_ratio",
];
pub const SIGMOID_GPU_EXECUTABLE: bool = false;
pub const SIGMOID_MIGRATION_EDGES: &[(u16, u16)] = &[(1, 3), (2, 3)];

const MIDDLE_GREY: f32 = 0.1845;
const LOGLOGISTIC_DELTA: f32 = 1e-6;
const RGB_RATIO_EPSILON: f32 = 1e-6;
const RGB_RATIO_LUMA_THRESHOLD: f32 = 1e-9;
const MATRIX_EPSILON: f32 = 1e-7;
const FLT_EPSILON: f32 = f32::EPSILON;

/// Native method enum values from `dt_iop_sigmoid_methods_type_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SigmoidColorProcessing {
    PerChannel = 0,
    RgbRatio = 1,
}

impl TryFrom<i32> for SigmoidColorProcessing {
    type Error = SigmoidCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::PerChannel),
            1 => Ok(Self::RgbRatio),
            other => Err(SigmoidCodecError::InvalidColorProcessing(other)),
        }
    }
}

/// Native base-primary enum values from `dt_iop_sigmoid_base_primaries_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SigmoidBasePrimaries {
    WorkingProfile = 0,
    Rec2020 = 1,
    DisplayP3 = 2,
    AdobeRgb = 3,
    Srgb = 4,
}

impl TryFrom<i32> for SigmoidBasePrimaries {
    type Error = SigmoidCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::WorkingProfile),
            1 => Ok(Self::Rec2020),
            2 => Ok(Self::DisplayP3),
            3 => Ok(Self::AdobeRgb),
            4 => Ok(Self::Srgb),
            other => Err(SigmoidCodecError::InvalidBasePrimaries(other)),
        }
    }
}

/// Native v1 `dt_iop_sigmoid_params_t` before custom primaries were added.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SigmoidParametersV1 {
    pub middle_grey_contrast: f32,
    pub contrast_skewness: f32,
    pub display_white_target: f32,
    pub display_black_target: f32,
    pub color_processing: SigmoidColorProcessing,
    pub hue_preservation: f32,
}

const _: () = assert!(size_of::<SigmoidParametersV1>() == SIGMOID_PARAMETER_BYTES_V1);

impl SigmoidParametersV1 {
    #[must_use]
    pub const fn new(
        middle_grey_contrast: f32,
        contrast_skewness: f32,
        display_white_target: f32,
        display_black_target: f32,
        color_processing: SigmoidColorProcessing,
        hue_preservation: f32,
    ) -> Self {
        Self {
            middle_grey_contrast,
            contrast_skewness,
            display_white_target,
            display_black_target,
            color_processing,
            hue_preservation,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            1.5,
            0.0,
            100.0,
            0.0152,
            SigmoidColorProcessing::PerChannel,
            100.0,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SIGMOID_PARAMETER_BYTES_V1] {
        let mut bytes = [0_u8; SIGMOID_PARAMETER_BYTES_V1];
        write_f32(&mut bytes, 0, self.middle_grey_contrast);
        write_f32(&mut bytes, 4, self.contrast_skewness);
        write_f32(&mut bytes, 8, self.display_white_target);
        write_f32(&mut bytes, 12, self.display_black_target);
        write_i32(&mut bytes, 16, self.color_processing as i32);
        write_f32(&mut bytes, 20, self.hue_preservation);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SigmoidCodecError> {
        if bytes.len() != SIGMOID_PARAMETER_BYTES_V1 {
            return Err(SigmoidCodecError::InvalidLength {
                expected: SIGMOID_PARAMETER_BYTES_V1,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            SigmoidColorProcessing::try_from(read_i32(bytes, 16))?,
            read_f32(bytes, 20),
        ))
    }
}

/// Native v2 parameters, including the six primary adjustments and purity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SigmoidParametersV2 {
    pub middle_grey_contrast: f32,
    pub contrast_skewness: f32,
    pub display_white_target: f32,
    pub display_black_target: f32,
    pub color_processing: SigmoidColorProcessing,
    pub hue_preservation: f32,
    pub red_inset: f32,
    pub red_rotation: f32,
    pub green_inset: f32,
    pub green_rotation: f32,
    pub blue_inset: f32,
    pub blue_rotation: f32,
    pub purity: f32,
}

const _: () = assert!(size_of::<SigmoidParametersV2>() == SIGMOID_PARAMETER_BYTES_V2);

impl SigmoidParametersV2 {
    #[must_use]
    pub const fn new(
        middle_grey_contrast: f32,
        contrast_skewness: f32,
        display_white_target: f32,
        display_black_target: f32,
        color_processing: SigmoidColorProcessing,
        hue_preservation: f32,
        red_inset: f32,
        red_rotation: f32,
        green_inset: f32,
        green_rotation: f32,
        blue_inset: f32,
        blue_rotation: f32,
        purity: f32,
    ) -> Self {
        Self {
            middle_grey_contrast,
            contrast_skewness,
            display_white_target,
            display_black_target,
            color_processing,
            hue_preservation,
            red_inset,
            red_rotation,
            green_inset,
            green_rotation,
            blue_inset,
            blue_rotation,
            purity,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            1.5,
            0.0,
            100.0,
            0.0152,
            SigmoidColorProcessing::PerChannel,
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SIGMOID_PARAMETER_BYTES_V2] {
        let mut bytes = [0_u8; SIGMOID_PARAMETER_BYTES_V2];
        write_v2_fields(&mut bytes, self);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SigmoidCodecError> {
        if bytes.len() != SIGMOID_PARAMETER_BYTES_V2 {
            return Err(SigmoidCodecError::InvalidLength {
                expected: SIGMOID_PARAMETER_BYTES_V2,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            SigmoidColorProcessing::try_from(read_i32(bytes, 16))?,
            read_f32(bytes, 20),
            read_f32(bytes, 24),
            read_f32(bytes, 28),
            read_f32(bytes, 32),
            read_f32(bytes, 36),
            read_f32(bytes, 40),
            read_f32(bytes, 44),
            read_f32(bytes, 48),
        ))
    }
}

/// Current native v3 parameters in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SigmoidParametersV3 {
    pub middle_grey_contrast: f32,
    pub contrast_skewness: f32,
    pub display_white_target: f32,
    pub display_black_target: f32,
    pub color_processing: SigmoidColorProcessing,
    pub hue_preservation: f32,
    pub red_inset: f32,
    pub red_rotation: f32,
    pub green_inset: f32,
    pub green_rotation: f32,
    pub blue_inset: f32,
    pub blue_rotation: f32,
    pub purity: f32,
    pub base_primaries: SigmoidBasePrimaries,
}

const _: () = assert!(size_of::<SigmoidParametersV3>() == SIGMOID_PARAMETER_BYTES_V3);

impl SigmoidParametersV3 {
    #[must_use]
    pub const fn new(
        middle_grey_contrast: f32,
        contrast_skewness: f32,
        display_white_target: f32,
        display_black_target: f32,
        color_processing: SigmoidColorProcessing,
        hue_preservation: f32,
        red_inset: f32,
        red_rotation: f32,
        green_inset: f32,
        green_rotation: f32,
        blue_inset: f32,
        blue_rotation: f32,
        purity: f32,
        base_primaries: SigmoidBasePrimaries,
    ) -> Self {
        Self {
            middle_grey_contrast,
            contrast_skewness,
            display_white_target,
            display_black_target,
            color_processing,
            hue_preservation,
            red_inset,
            red_rotation,
            green_inset,
            green_rotation,
            blue_inset,
            blue_rotation,
            purity,
            base_primaries,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            1.5,
            0.0,
            100.0,
            0.0152,
            SigmoidColorProcessing::PerChannel,
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            SigmoidBasePrimaries::WorkingProfile,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SIGMOID_PARAMETER_BYTES_V3] {
        let mut bytes = [0_u8; SIGMOID_PARAMETER_BYTES_V3];
        write_v2_fields(&mut bytes, SigmoidParametersV2::from(self));
        write_i32(&mut bytes, 52, self.base_primaries as i32);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SigmoidCodecError> {
        if bytes.len() != SIGMOID_PARAMETER_BYTES_V3 {
            return Err(SigmoidCodecError::InvalidLength {
                expected: SIGMOID_PARAMETER_BYTES_V3,
                actual: bytes.len(),
            });
        }
        let v2 = SigmoidParametersV2::from_bytes(&bytes[..SIGMOID_PARAMETER_BYTES_V2])?;
        Ok(Self::new(
            v2.middle_grey_contrast,
            v2.contrast_skewness,
            v2.display_white_target,
            v2.display_black_target,
            v2.color_processing,
            v2.hue_preservation,
            v2.red_inset,
            v2.red_rotation,
            v2.green_inset,
            v2.green_rotation,
            v2.blue_inset,
            v2.blue_rotation,
            v2.purity,
            SigmoidBasePrimaries::try_from(read_i32(bytes, 52))?,
        ))
    }
}

impl From<SigmoidParametersV1> for SigmoidParametersV3 {
    fn from(value: SigmoidParametersV1) -> Self {
        Self::new(
            value.middle_grey_contrast,
            value.contrast_skewness,
            value.display_white_target,
            value.display_black_target,
            value.color_processing,
            value.hue_preservation,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            SigmoidBasePrimaries::WorkingProfile,
        )
    }
}

impl From<SigmoidParametersV2> for SigmoidParametersV3 {
    fn from(value: SigmoidParametersV2) -> Self {
        Self::new(
            value.middle_grey_contrast,
            value.contrast_skewness,
            value.display_white_target,
            value.display_black_target,
            value.color_processing,
            value.hue_preservation,
            value.red_inset,
            value.red_rotation,
            value.green_inset,
            value.green_rotation,
            value.blue_inset,
            value.blue_rotation,
            value.purity,
            SigmoidBasePrimaries::WorkingProfile,
        )
    }
}

impl From<SigmoidParametersV3> for SigmoidParametersV2 {
    fn from(value: SigmoidParametersV3) -> Self {
        Self::new(
            value.middle_grey_contrast,
            value.contrast_skewness,
            value.display_white_target,
            value.display_black_target,
            value.color_processing,
            value.hue_preservation,
            value.red_inset,
            value.red_rotation,
            value.green_inset,
            value.green_rotation,
            value.blue_inset,
            value.blue_rotation,
            value.purity,
        )
    }
}

/// Codec failures for known native payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidColorProcessing(i32),
    InvalidBasePrimaries(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for SigmoidCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "sigmoid payload has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidColorProcessing(value) => {
                write!(
                    formatter,
                    "sigmoid color processing value {value} is unknown"
                )
            }
            Self::InvalidBasePrimaries(value) => {
                write!(formatter, "sigmoid base primaries value {value} is unknown")
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "sigmoid version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for SigmoidCodecError {}

/// Known history values and byte-preserved future values.
#[derive(Debug, Clone, PartialEq)]
pub enum SigmoidHistory {
    V1(SigmoidParametersV1),
    V2(SigmoidParametersV2),
    V3(SigmoidParametersV3),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl SigmoidHistory {
    /// Native v1 and v2 both migrate directly to v3; unknown versions remain opaque.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SigmoidCodecError> {
        match version {
            1 => Ok(Self::V1(SigmoidParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(SigmoidParametersV2::from_bytes(bytes)?)),
            SIGMOID_SCHEMA_VERSION => Ok(Self::V3(SigmoidParametersV3::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => SIGMOID_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::V3(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// Materializes the current v3 value, rejecting a future opaque value.
    pub fn current(&self) -> Result<SigmoidParametersV3, SigmoidCodecError> {
        match self {
            Self::V1(parameters) => Ok((*parameters).into()),
            Self::V2(parameters) => Ok((*parameters).into()),
            Self::V3(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(SigmoidCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for SigmoidParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "sigmoid {name} is non-finite"),
        }
    }
}

impl std::error::Error for SigmoidParameterError {}

/// Finite committed data. Native UI ranges are not execution clamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SigmoidConfig {
    middle_grey_contrast: rusttable_processing::FiniteF32,
    contrast_skewness: rusttable_processing::FiniteF32,
    display_white_target: rusttable_processing::FiniteF32,
    display_black_target: rusttable_processing::FiniteF32,
    color_processing: SigmoidColorProcessing,
    hue_preservation: rusttable_processing::FiniteF32,
    red_inset: rusttable_processing::FiniteF32,
    red_rotation: rusttable_processing::FiniteF32,
    green_inset: rusttable_processing::FiniteF32,
    green_rotation: rusttable_processing::FiniteF32,
    blue_inset: rusttable_processing::FiniteF32,
    blue_rotation: rusttable_processing::FiniteF32,
    purity: rusttable_processing::FiniteF32,
    base_primaries: SigmoidBasePrimaries,
}

impl TryFrom<SigmoidParametersV3> for SigmoidConfig {
    type Error = SigmoidParameterError;

    fn try_from(parameters: SigmoidParametersV3) -> Result<Self, Self::Error> {
        Ok(Self {
            middle_grey_contrast: finite_parameter(
                "middle_grey_contrast",
                parameters.middle_grey_contrast,
            )?,
            contrast_skewness: finite_parameter("contrast_skewness", parameters.contrast_skewness)?,
            display_white_target: finite_parameter(
                "display_white_target",
                parameters.display_white_target,
            )?,
            display_black_target: finite_parameter(
                "display_black_target",
                parameters.display_black_target,
            )?,
            color_processing: parameters.color_processing,
            hue_preservation: finite_parameter("hue_preservation", parameters.hue_preservation)?,
            red_inset: finite_parameter("red_inset", parameters.red_inset)?,
            red_rotation: finite_parameter("red_rotation", parameters.red_rotation)?,
            green_inset: finite_parameter("green_inset", parameters.green_inset)?,
            green_rotation: finite_parameter("green_rotation", parameters.green_rotation)?,
            blue_inset: finite_parameter("blue_inset", parameters.blue_inset)?,
            blue_rotation: finite_parameter("blue_rotation", parameters.blue_rotation)?,
            purity: finite_parameter("purity", parameters.purity)?,
            base_primaries: parameters.base_primaries,
        })
    }
}

impl SigmoidConfig {
    pub fn new(parameters: SigmoidParametersV3) -> Result<Self, SigmoidParameterError> {
        parameters.try_into()
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(SigmoidParametersV3::defaults()).expect("sigmoid defaults are finite")
    }

    #[must_use]
    pub const fn parameters(self) -> SigmoidParametersV3 {
        SigmoidParametersV3::new(
            self.middle_grey_contrast.get(),
            self.contrast_skewness.get(),
            self.display_white_target.get(),
            self.display_black_target.get(),
            self.color_processing,
            self.hue_preservation.get(),
            self.red_inset.get(),
            self.red_rotation.get(),
            self.green_inset.get(),
            self.green_rotation.get(),
            self.blue_inset.get(),
            self.blue_rotation.get(),
            self.purity.get(),
            self.base_primaries,
        )
    }

    #[must_use]
    pub const fn color_processing(self) -> SigmoidColorProcessing {
        self.color_processing
    }

    #[must_use]
    pub const fn base_primaries(self) -> SigmoidBasePrimaries {
        self.base_primaries
    }

    #[must_use]
    pub const fn middle_grey_contrast(self) -> f32 {
        self.middle_grey_contrast.get()
    }

    #[must_use]
    pub const fn contrast_skewness(self) -> f32 {
        self.contrast_skewness.get()
    }

    #[must_use]
    pub const fn display_white_target(self) -> f32 {
        self.display_white_target.get()
    }

    #[must_use]
    pub const fn display_black_target(self) -> f32 {
        self.display_black_target.get()
    }
}

fn finite_parameter(
    name: &'static str,
    value: f32,
) -> Result<rusttable_processing::FiniteF32, SigmoidParameterError> {
    rusttable_processing::FiniteF32::new(value).map_err(|_| SigmoidParameterError::NonFinite(name))
}

/// A row-major three-by-three matrix in Darktable's transposed storage order.
/// Its row index is the input channel and its column index is the output channel.
pub type SigmoidMatrix = [[f32; 3]; 3];

/// Matrix-shaper profile data needed by the native per-channel path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SigmoidProfile {
    primaries: [[f32; 2]; 3],
    whitepoint: [f32; 2],
    matrix_in_transposed: SigmoidMatrix,
    matrix_out_transposed: SigmoidMatrix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidProfileError {
    NonFinite,
    SingularMatrix,
}

impl fmt::Display for SigmoidProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("sigmoid profile contains a non-finite value"),
            Self::SingularMatrix => formatter.write_str("sigmoid profile matrix is singular"),
        }
    }
}

impl std::error::Error for SigmoidProfileError {}

impl SigmoidProfile {
    /// Creates a matrix-shaper profile using the source `dt_make_transposed...` equations.
    pub fn from_primaries(
        primaries: [[f32; 2]; 3],
        whitepoint: [f32; 2],
    ) -> Result<Self, SigmoidProfileError> {
        let matrix_in_transposed = matrix_from_primaries(primaries, whitepoint)?;
        let matrix_out_transposed =
            invert_matrix(matrix_in_transposed).ok_or(SigmoidProfileError::SingularMatrix)?;
        Self::from_matrices(
            primaries,
            whitepoint,
            matrix_in_transposed,
            matrix_out_transposed,
        )
    }

    /// Creates a profile with caller-supplied native transposed matrices.
    pub fn from_matrices(
        primaries: [[f32; 2]; 3],
        whitepoint: [f32; 2],
        matrix_in_transposed: SigmoidMatrix,
        matrix_out_transposed: SigmoidMatrix,
    ) -> Result<Self, SigmoidProfileError> {
        if !primaries
            .into_iter()
            .flatten()
            .chain(whitepoint)
            .chain(matrix_in_transposed.into_iter().flatten())
            .chain(matrix_out_transposed.into_iter().flatten())
            .all(f32::is_finite)
        {
            return Err(SigmoidProfileError::NonFinite);
        }
        Ok(Self {
            primaries,
            whitepoint,
            matrix_in_transposed,
            matrix_out_transposed,
        })
    }

    #[must_use]
    pub fn srgb() -> Self {
        Self::from_primaries(
            [[0.6400, 0.3300], [0.3000, 0.6000], [0.1500, 0.0600]],
            [0.31271, 0.32902],
        )
        .expect("sRGB profile is nonsingular")
    }

    #[must_use]
    pub fn rec2020() -> Self {
        Self::from_primaries(
            [[0.7080, 0.2920], [0.1700, 0.7970], [0.1310, 0.0460]],
            [0.31271, 0.32902],
        )
        .expect("Rec2020 profile is nonsingular")
    }

    #[must_use]
    pub fn display_p3() -> Self {
        Self::from_primaries(
            [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060]],
            [0.31271, 0.32902],
        )
        .expect("Display P3 profile is nonsingular")
    }

    #[must_use]
    pub fn adobe_rgb() -> Self {
        Self::from_primaries(
            [[0.6400, 0.3300], [0.2100, 0.7100], [0.1500, 0.0600]],
            [0.31271, 0.32902],
        )
        .expect("Adobe RGB profile is nonsingular")
    }

    #[must_use]
    pub const fn primaries(self) -> [[f32; 2]; 3] {
        self.primaries
    }

    #[must_use]
    pub const fn whitepoint(self) -> [f32; 2] {
        self.whitepoint
    }

    #[must_use]
    pub const fn matrix_in_transposed(self) -> SigmoidMatrix {
        self.matrix_in_transposed
    }

    #[must_use]
    pub const fn matrix_out_transposed(self) -> SigmoidMatrix {
        self.matrix_out_transposed
    }
}

/// Four-channel native RGB sample. The fourth channel is copied unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SigmoidPixel {
    channels: [f32; 4],
}

impl SigmoidPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn red(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn green(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn blue(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidChannel {
    Red,
    Green,
    Blue,
    Alpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidPlanError {
    NonFiniteDerived(&'static str),
    SingularMatrix,
}

impl fmt::Display for SigmoidPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDerived(name) => {
                write!(formatter, "sigmoid derived {name} is non-finite")
            }
            Self::SingularMatrix => {
                formatter.write_str("sigmoid adjusted profile matrix is singular")
            }
        }
    }
}

impl std::error::Error for SigmoidPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidExecutionError {
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: SigmoidChannel,
    },
    NonFiniteOutput {
        pixel: usize,
    },
    AllocationFailed {
        required_bytes: usize,
    },
    SizeOverflow,
    Cancelled,
}

impl fmt::Display for SigmoidExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "sigmoid expected {expected} pixels, got {actual}"
                )
            }
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "sigmoid input pixel {pixel} has non-finite {channel:?}"
                )
            }
            Self::NonFiniteOutput { pixel } => {
                write!(
                    formatter,
                    "sigmoid produced non-finite output at pixel {pixel}"
                )
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "sigmoid allocation failed for {required_bytes} bytes"
                )
            }
            Self::SizeOverflow => formatter.write_str("sigmoid execution size overflowed"),
            Self::Cancelled => formatter.write_str("sigmoid execution was cancelled"),
        }
    }
}

impl std::error::Error for SigmoidExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    ProductionRoutingDeferred,
}

impl fmt::Display for SigmoidCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => formatter.write_str("sigmoid GPU execution is unavailable"),
            Self::GtkUnavailable => formatter.write_str("sigmoid GTK controls are unavailable"),
            Self::ProductionRoutingDeferred => {
                formatter.write_str("sigmoid production routing is deferred")
            }
        }
    }
}

impl std::error::Error for SigmoidCapabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigmoidCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub profile_transforms_supported: bool,
    pub masks_consumed: bool,
    pub outer_blending_deferred: bool,
    pub production_routing_deferred: bool,
    pub alpha_preserved: bool,
}

impl SigmoidCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: SIGMOID_GPU_EXECUTABLE,
            gtk_supported: false,
            profile_transforms_supported: true,
            masks_consumed: false,
            outer_blending_deferred: true,
            production_routing_deferred: true,
            alpha_preserved: true,
        }
    }

    pub const fn require_gpu(self) -> Result<(), SigmoidCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(SigmoidCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), SigmoidCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(SigmoidCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_production_routing(self) -> Result<(), SigmoidCapabilityError> {
        if self.production_routing_deferred {
            Err(SigmoidCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> SigmoidCapabilities {
    SigmoidCapabilities::bounded_cpu_leaf()
}

#[derive(Debug, Clone, Copy)]
struct CommittedSigmoid {
    white_target: f32,
    black_target: f32,
    paper_exposure: f32,
    film_fog: f32,
    film_power: f32,
    paper_power: f32,
    hue_preservation: f32,
}

/// Immutable CPU plan. The matrix fields retain the source's transposed order.
#[derive(Debug, Clone, Copy)]
pub struct SigmoidPlan {
    config: SigmoidConfig,
    dimensions: RasterDimensions,
    committed: CommittedSigmoid,
    pipe_to_base: SigmoidMatrix,
    base_to_rendering: SigmoidMatrix,
    rendering_to_pipe: SigmoidMatrix,
}

impl SigmoidPlan {
    pub fn new(
        config: SigmoidConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, SigmoidPlanError> {
        Self::new_with_profile(config, dimensions, SigmoidProfile::srgb())
    }

    pub fn new_with_profile(
        config: SigmoidConfig,
        dimensions: RasterDimensions,
        working_profile: SigmoidProfile,
    ) -> Result<Self, SigmoidPlanError> {
        let committed = commit_parameters(config)?;
        let (pipe_to_base, base_to_rendering, rendering_to_pipe) = if matches!(
            config.color_processing(),
            SigmoidColorProcessing::PerChannel
        ) {
            let base_profile = match config.base_primaries() {
                SigmoidBasePrimaries::WorkingProfile => working_profile,
                SigmoidBasePrimaries::Rec2020 => SigmoidProfile::rec2020(),
                SigmoidBasePrimaries::DisplayP3 => SigmoidProfile::display_p3(),
                SigmoidBasePrimaries::AdobeRgb => SigmoidProfile::adobe_rgb(),
                SigmoidBasePrimaries::Srgb => SigmoidProfile::srgb(),
            };
            calculate_adjusted_primaries(config, working_profile, base_profile)?
        } else {
            (identity_matrix(), identity_matrix(), identity_matrix())
        };
        Ok(Self {
            config,
            dimensions,
            committed,
            pipe_to_base,
            base_to_rendering,
            rendering_to_pipe,
        })
    }

    #[must_use]
    pub const fn config(self) -> SigmoidConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn white_target(self) -> f32 {
        self.committed.white_target
    }

    #[must_use]
    pub const fn black_target(self) -> f32 {
        self.committed.black_target
    }

    #[must_use]
    pub const fn film_power(self) -> f32 {
        self.committed.film_power
    }

    #[must_use]
    pub const fn paper_power(self) -> f32 {
        self.committed.paper_power
    }

    #[must_use]
    pub const fn matrices(self) -> (SigmoidMatrix, SigmoidMatrix, SigmoidMatrix) {
        (
            self.pipe_to_base,
            self.base_to_rendering,
            self.rendering_to_pipe,
        )
    }

    pub fn execute(
        &self,
        input: &[SigmoidPixel],
    ) -> Result<Vec<SigmoidPixel>, SigmoidExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Polls cancellation at row boundaries and publishes only a complete output vector.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[SigmoidPixel],
        mut cancelled: F,
    ) -> Result<Vec<SigmoidPixel>, SigmoidExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| SigmoidExecutionError::SizeOverflow)?;
        if input.len() != expected {
            return Err(SigmoidExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(SigmoidExecutionError::Cancelled);
        }
        let required_bytes = expected
            .checked_mul(size_of::<SigmoidPixel>())
            .ok_or(SigmoidExecutionError::SizeOverflow)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| SigmoidExecutionError::AllocationFailed { required_bytes })?;

        let width = usize::try_from(self.dimensions.width()).expect("u32 dimensions fit usize");
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(SigmoidExecutionError::Cancelled);
            }
            validate_input(pixel, index)?;
            let result = match self.config.color_processing() {
                SigmoidColorProcessing::PerChannel => self.process_per_channel(pixel),
                SigmoidColorProcessing::RgbRatio => self.process_rgb_ratio(pixel),
            };
            if !result.channels().into_iter().all(f32::is_finite) {
                return Err(SigmoidExecutionError::NonFiniteOutput { pixel: index });
            }
            output.push(result);
        }
        if cancelled() {
            return Err(SigmoidExecutionError::Cancelled);
        }
        Ok(output)
    }

    fn process_rgb_ratio(&self, pixel: SigmoidPixel) -> SigmoidPixel {
        let input = pixel.channels();
        let strict_positive = desaturate_negative_values([input[0], input[1], input[2]]);
        let luma = (strict_positive[0] + strict_positive[1] + strict_positive[2]) / 3.0;
        let mapped_luma = generalized_loglogistic_sigmoid(
            luma,
            self.committed.white_target,
            self.committed.paper_exposure,
            self.committed.film_fog,
            self.committed.film_power,
            self.committed.paper_power,
        );
        let mut pre_out = [0.0_f32; 3];
        if luma > RGB_RATIO_LUMA_THRESHOLD {
            let scaling_factor = mapped_luma / luma;
            for channel in 0..3 {
                pre_out[channel] = scaling_factor * strict_positive[channel];
            }
        } else {
            pre_out.fill(mapped_luma);
        }

        let order = pixel_channel_order(pre_out);
        let pixel_min = pre_out[order.min];
        let pixel_max = pre_out[order.max];
        let display_border_vs_chroma_white = (self.committed.white_target - mapped_luma)
            / (pixel_max - mapped_luma + RGB_RATIO_EPSILON);
        let display_border_vs_chroma_black = (self.committed.black_target - mapped_luma)
            / (pixel_min - mapped_luma - RGB_RATIO_EPSILON);
        let display_border_vs_chroma = native_min(
            display_border_vs_chroma_white,
            display_border_vs_chroma_black,
        );
        let chroma_vs_mapping_border =
            (mapped_luma - pixel_min) / (mapped_luma + RGB_RATIO_EPSILON);
        let pixel_chroma_adjustment =
            1.0 / (chroma_vs_mapping_border * display_border_vs_chroma + RGB_RATIO_EPSILON);
        let hyperbolic_chroma = 2.0 * chroma_vs_mapping_border
            / (1.0 - chroma_vs_mapping_border * chroma_vs_mapping_border + RGB_RATIO_EPSILON)
            * pixel_chroma_adjustment;
        let hyperbolic_z = (hyperbolic_chroma * hyperbolic_chroma + 1.0).sqrt();
        let chroma_factor = hyperbolic_chroma / (1.0 + hyperbolic_z) * display_border_vs_chroma;
        let mut output = [0.0_f32; 4];
        for channel in 0..3 {
            output[channel] = mapped_luma + chroma_factor * (pre_out[channel] - mapped_luma);
        }
        output[3] = input[3];
        SigmoidPixel::from_channels(output)
    }

    fn process_per_channel(&self, pixel: SigmoidPixel) -> SigmoidPixel {
        let input = pixel.channels();
        let pipe_rgb = [input[0], input[1], input[2]];
        let base_rgb = apply_matrix(pipe_rgb, &self.pipe_to_base);
        let strict_positive = desaturate_negative_values(base_rgb);
        let rendering_rgb = apply_matrix(strict_positive, &self.base_to_rendering);
        let mut per_channel = [0.0_f32; 3];
        for channel in 0..3 {
            per_channel[channel] = generalized_loglogistic_sigmoid(
                rendering_rgb[channel],
                self.committed.white_target,
                self.committed.paper_exposure,
                self.committed.film_fog,
                self.committed.film_power,
                self.committed.paper_power,
            );
        }
        let order = pixel_channel_order(rendering_rgb);
        let per_channel_hue_corrected = preserve_hue_and_energy(
            rendering_rgb,
            per_channel,
            order,
            self.committed.hue_preservation,
        );
        let output_rgb = apply_matrix(per_channel_hue_corrected, &self.rendering_to_pipe);
        SigmoidPixel::new(output_rgb[0], output_rgb[1], output_rgb[2], input[3])
    }
}

fn validate_input(pixel: SigmoidPixel, index: usize) -> Result<(), SigmoidExecutionError> {
    let channels = pixel.channels();
    let channel_names = [
        SigmoidChannel::Red,
        SigmoidChannel::Green,
        SigmoidChannel::Blue,
        SigmoidChannel::Alpha,
    ];
    for (value, channel) in channels.into_iter().zip(channel_names) {
        if !value.is_finite() {
            return Err(SigmoidExecutionError::NonFiniteInput {
                pixel: index,
                channel,
            });
        }
    }
    Ok(())
}

fn commit_parameters(config: SigmoidConfig) -> Result<CommittedSigmoid, SigmoidPlanError> {
    // Calculate actual skew log logistic parameters to fulfill:
    // f(scene_zero) = display_black_target
    // f(scene_grey) = MIDDLE_GREY
    // f(scene_inf) = display_white_target
    // Slope at scene_grey is independent of skewness.
    let ref_film_power = config.middle_grey_contrast();
    let ref_paper_power = 1.0_f32;
    let ref_magnitude = 1.0_f32;
    let ref_film_fog = 0.0_f32;
    let ref_paper_exposure =
        (ref_film_fog + MIDDLE_GREY).powf(ref_film_power) * ((ref_magnitude / MIDDLE_GREY) - 1.0);
    let delta = LOGLOGISTIC_DELTA;
    let ref_slope = (generalized_loglogistic_sigmoid(
        MIDDLE_GREY + delta,
        ref_magnitude,
        ref_paper_exposure,
        ref_film_fog,
        ref_film_power,
        ref_paper_power,
    ) - generalized_loglogistic_sigmoid(
        MIDDLE_GREY - delta,
        ref_magnitude,
        ref_paper_exposure,
        ref_film_fog,
        ref_film_power,
        ref_paper_power,
    )) / 2.0
        / delta;

    let paper_power = 5.0_f32.powf(-config.contrast_skewness());
    let temp_film_power = 1.0_f32;
    let temp_white_target = 0.01 * config.display_white_target();
    let temp_white_grey_relation = (temp_white_target / MIDDLE_GREY).powf(1.0 / paper_power) - 1.0;
    let temp_paper_exposure = MIDDLE_GREY.powf(temp_film_power) * temp_white_grey_relation;
    let temp_slope = (generalized_loglogistic_sigmoid(
        MIDDLE_GREY + delta,
        temp_white_target,
        temp_paper_exposure,
        ref_film_fog,
        temp_film_power,
        paper_power,
    ) - generalized_loglogistic_sigmoid(
        MIDDLE_GREY - delta,
        temp_white_target,
        temp_paper_exposure,
        ref_film_fog,
        temp_film_power,
        paper_power,
    )) / 2.0
        / delta;
    let film_power = ref_slope / temp_slope;

    let white_target = 0.01 * config.display_white_target();
    let black_target = 0.01 * config.display_black_target();
    let white_grey_relation = (white_target / MIDDLE_GREY).powf(1.0 / paper_power) - 1.0;
    let white_black_relation = (black_target / white_target).powf(-1.0 / paper_power) - 1.0;
    let film_fog = MIDDLE_GREY * white_grey_relation.powf(1.0 / film_power)
        / (white_black_relation.powf(1.0 / film_power)
            - white_grey_relation.powf(1.0 / film_power));
    let paper_exposure = (film_fog + MIDDLE_GREY).powf(film_power) * white_grey_relation;
    let hue_preservation = native_min(
        native_max(0.01 * config.parameters().hue_preservation, 0.0),
        1.0,
    );
    let committed = CommittedSigmoid {
        white_target,
        black_target,
        paper_exposure,
        film_fog,
        film_power,
        paper_power,
        hue_preservation,
    };
    let values = [
        committed.white_target,
        committed.black_target,
        committed.paper_exposure,
        committed.film_fog,
        committed.film_power,
        committed.paper_power,
        committed.hue_preservation,
    ];
    for (value, name) in values.into_iter().zip([
        "white_target",
        "black_target",
        "paper_exposure",
        "film_fog",
        "film_power",
        "paper_power",
        "hue_preservation",
    ]) {
        if !value.is_finite() {
            return Err(SigmoidPlanError::NonFiniteDerived(name));
        }
    }
    Ok(committed)
}

fn native_max(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left > right {
        left
    } else if right > left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() && right.is_sign_negative() {
            left
        } else {
            0.0
        }
    } else {
        left
    }
}

fn native_min(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left < right {
        left
    } else if right < left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() || right.is_sign_negative() {
            -0.0
        } else {
            left
        }
    } else {
        left
    }
}

/// Native stable generalized log-logistic equation and NaN fallback.
#[must_use]
pub fn generalized_loglogistic_sigmoid(
    value: f32,
    magnitude: f32,
    paper_exp: f32,
    film_fog: f32,
    film_power: f32,
    paper_power: f32,
) -> f32 {
    let clamped_value = native_max(value, 0.0);
    let film_response = (film_fog + clamped_value).powf(film_power);
    let paper_response =
        magnitude * (film_response / (paper_exp + film_response)).powf(paper_power);
    if paper_response.is_nan() {
        magnitude
    } else {
        paper_response
    }
}

fn desaturate_negative_values(pixel: [f32; 3]) -> [f32; 3] {
    let pixel_average = native_max((pixel[0] + pixel[1] + pixel[2]) / 3.0, 0.0);
    let min_value = native_min(native_min(pixel[0], pixel[1]), pixel[2]);
    let saturation_factor = if min_value < 0.0 {
        -pixel_average / (min_value - pixel_average)
    } else {
        1.0
    };
    [
        pixel_average + saturation_factor * (pixel[0] - pixel_average),
        pixel_average + saturation_factor * (pixel[1] - pixel_average),
        pixel_average + saturation_factor * (pixel[2] - pixel_average),
    ]
}

#[derive(Debug, Clone, Copy)]
struct SigmoidValueOrder {
    min: usize,
    mid: usize,
    max: usize,
}

fn pixel_channel_order(pixel: [f32; 3]) -> SigmoidValueOrder {
    if pixel[0] >= pixel[1] {
        if pixel[1] > pixel[2] {
            SigmoidValueOrder {
                max: 0,
                mid: 1,
                min: 2,
            }
        } else if pixel[2] > pixel[0] {
            SigmoidValueOrder {
                max: 2,
                mid: 0,
                min: 1,
            }
        } else if pixel[2] > pixel[1] {
            SigmoidValueOrder {
                max: 0,
                mid: 2,
                min: 1,
            }
        } else {
            SigmoidValueOrder {
                max: 0,
                mid: 1,
                min: 2,
            }
        }
    } else if pixel[0] >= pixel[2] {
        SigmoidValueOrder {
            max: 1,
            mid: 0,
            min: 2,
        }
    } else if pixel[2] > pixel[1] {
        SigmoidValueOrder {
            max: 2,
            mid: 1,
            min: 0,
        }
    } else {
        SigmoidValueOrder {
            max: 1,
            mid: 2,
            min: 0,
        }
    }
}

/// Native hue interpolation, including the energy constraint and branch order.
fn preserve_hue_and_energy(
    input: [f32; 3],
    per_channel: [f32; 3],
    order: SigmoidValueOrder,
    hue_preservation: f32,
) -> [f32; 3] {
    let chroma = input[order.max] - input[order.min];
    let midscale = if chroma != 0.0 {
        (input[order.mid] - input[order.min]) / chroma
    } else {
        0.0
    };
    let full_hue_correction =
        per_channel[order.min] + (per_channel[order.max] - per_channel[order.min]) * midscale;
    let naive_hue_mid =
        (1.0 - hue_preservation) * per_channel[order.mid] + hue_preservation * full_hue_correction;
    let per_channel_energy = per_channel[0] + per_channel[1] + per_channel[2];
    let naive_hue_energy = per_channel[order.min] + naive_hue_mid + per_channel[order.max];
    let pix_in_min_plus_mid = input[order.min] + input[order.mid];
    let blend_factor = if pix_in_min_plus_mid != 0.0 {
        2.0 * input[order.min] / pix_in_min_plus_mid
    } else {
        0.0
    };
    let energy_target = blend_factor * per_channel_energy + (1.0 - blend_factor) * naive_hue_energy;
    let mut output = [0.0_f32; 3];
    if naive_hue_mid <= per_channel[order.mid] {
        let corrected_mid = ((1.0 - hue_preservation) * per_channel[order.mid]
            + hue_preservation
                * (midscale * per_channel[order.max]
                    + (1.0 - midscale) * (energy_target - per_channel[order.max])))
            / (1.0 + hue_preservation * (1.0 - midscale));
        output[order.min] = energy_target - per_channel[order.max] - corrected_mid;
        output[order.mid] = corrected_mid;
        output[order.max] = per_channel[order.max];
    } else {
        let corrected_mid = ((1.0 - hue_preservation) * per_channel[order.mid]
            + hue_preservation
                * (per_channel[order.min] * (1.0 - midscale)
                    + midscale * (energy_target - per_channel[order.min])))
            / (1.0 + hue_preservation * midscale);
        output[order.min] = per_channel[order.min];
        output[order.mid] = corrected_mid;
        output[order.max] = energy_target - per_channel[order.min] - corrected_mid;
    }
    output
}

fn calculate_adjusted_primaries(
    config: SigmoidConfig,
    pipe_work_profile: SigmoidProfile,
    base_profile: SigmoidProfile,
) -> Result<(SigmoidMatrix, SigmoidMatrix, SigmoidMatrix), SigmoidPlanError> {
    let (pipe_to_base, base_to_pipe) = if pipe_work_profile == base_profile {
        (identity_matrix(), identity_matrix())
    } else {
        let pipe_to_base = multiply_matrices(
            pipe_work_profile.matrix_in_transposed,
            base_profile.matrix_out_transposed,
        );
        let base_to_pipe = invert_matrix(pipe_to_base).ok_or(SigmoidPlanError::SingularMatrix)?;
        (pipe_to_base, base_to_pipe)
    };
    if !matrix_is_finite(pipe_to_base) || !matrix_is_finite(base_to_pipe) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }

    let parameters = config.parameters();
    let inset = [
        parameters.red_inset,
        parameters.green_inset,
        parameters.blue_inset,
    ];
    let rotation = [
        parameters.red_rotation,
        parameters.green_rotation,
        parameters.blue_rotation,
    ];
    let mut custom_primaries = [[0.0_f32; 2]; 3];
    for index in 0..3 {
        custom_primaries[index] =
            rotate_and_scale_primary(base_profile, 1.0 - inset[index], rotation[index], index);
    }
    let custom_to_xyz = matrix_from_primaries(custom_primaries, base_profile.whitepoint)
        .map_err(|_| SigmoidPlanError::SingularMatrix)?;
    if !matrix_is_finite(custom_to_xyz) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }
    let base_to_rendering = multiply_matrices(custom_to_xyz, base_profile.matrix_out_transposed);
    if !matrix_is_finite(base_to_rendering) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }

    for index in 0..3 {
        let scaling = 1.0 - parameters.purity * inset[index];
        custom_primaries[index] =
            rotate_and_scale_primary(base_profile, scaling, rotation[index], index);
    }
    let custom_to_xyz = matrix_from_primaries(custom_primaries, base_profile.whitepoint)
        .map_err(|_| SigmoidPlanError::SingularMatrix)?;
    if !matrix_is_finite(custom_to_xyz) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }
    let temporary = multiply_matrices(custom_to_xyz, base_profile.matrix_out_transposed);
    if !matrix_is_finite(temporary) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }
    let rendering_to_base = invert_matrix(temporary).ok_or(SigmoidPlanError::SingularMatrix)?;
    let rendering_to_pipe = multiply_matrices(rendering_to_base, base_to_pipe);
    if !matrix_is_finite(rendering_to_base) || !matrix_is_finite(rendering_to_pipe) {
        return Err(SigmoidPlanError::NonFiniteDerived("profile_matrix"));
    }
    Ok((pipe_to_base, base_to_rendering, rendering_to_pipe))
}

fn rotate_and_scale_primary(
    profile: SigmoidProfile,
    scaling: f32,
    rotation: f32,
    primary_index: usize,
) -> [f32; 2] {
    let dx = profile.primaries[primary_index][0] - profile.whitepoint[0];
    let dy = profile.primaries[primary_index][1] - profile.whitepoint[1];
    let angle = dy.atan2(dx) + rotation;
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    let distance_to_edge = find_distance_to_edge(profile, cos_angle, sin_angle);
    let dx_new = scaling * distance_to_edge * cos_angle;
    let dy_new = scaling * distance_to_edge * sin_angle;
    [
        dx_new + profile.whitepoint[0],
        dy_new + profile.whitepoint[1],
    ]
}

fn find_distance_to_edge(profile: SigmoidProfile, cos_angle: f32, sin_angle: f32) -> f32 {
    let x1 = profile.whitepoint[0];
    let y1 = profile.whitepoint[1];
    let x2 = x1 + cos_angle;
    let y2 = y1 + sin_angle;
    let mut distance_to_edge = f32::MAX;
    for index in 0..3 {
        let next = if index == 2 { 0 } else { index + 1 };
        let distance = intersect_line_segments(
            x1,
            y1,
            x2,
            y2,
            profile.primaries[index][0],
            profile.primaries[index][1],
            profile.primaries[next][0],
            profile.primaries[next][1],
        );
        if distance < distance_to_edge {
            distance_to_edge = distance;
        }
    }
    distance_to_edge
}

fn determinant(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a * d - b * c
}

fn intersect_line_segments(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> f32 {
    let denominator = determinant(x1 - x2, x3 - x4, y1 - y2, y3 - y4);
    if denominator == 0.0 {
        return f32::MAX;
    }
    let t = determinant(x1 - x3, x3 - x4, y1 - y3, y3 - y4) / denominator;
    if t >= 0.0 { t } else { f32::MAX }
}

fn matrix_from_primaries(
    primaries: [[f32; 2]; 3],
    whitepoint: [f32; 2],
) -> Result<SigmoidMatrix, SigmoidProfileError> {
    let mut primaries_matrix = [[0.0_f32; 3]; 3];
    for index in 0..3 {
        let y = sanitize_y(primaries[index][1]);
        primaries_matrix[index][0] = primaries[index][0] / y;
        primaries_matrix[index][1] = 1.0;
        primaries_matrix[index][2] = (1.0 - primaries[index][0] - y) / y;
    }
    let primaries_inverse =
        invert_matrix(primaries_matrix).ok_or(SigmoidProfileError::SingularMatrix)?;
    let y = sanitize_y(whitepoint[1]);
    let xyz_white = [whitepoint[0] / y, 1.0, (1.0 - whitepoint[0] - y) / y];
    let scale = apply_matrix(xyz_white, &primaries_inverse);
    let mut output = [[0.0_f32; 3]; 3];
    for index in 0..3 {
        for channel in 0..3 {
            output[index][channel] = scale[index] * primaries_matrix[index][channel];
        }
    }
    Ok(output)
}

fn sanitize_y(y: f32) -> f32 {
    if y < FLT_EPSILON && y >= 0.0 {
        FLT_EPSILON
    } else if y < 0.0 && y > -FLT_EPSILON {
        -FLT_EPSILON
    } else {
        y
    }
}

fn matrix_is_finite(matrix: SigmoidMatrix) -> bool {
    matrix.into_iter().flatten().all(f32::is_finite)
}

fn identity_matrix() -> SigmoidMatrix {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

fn apply_matrix(input: [f32; 3], matrix: &SigmoidMatrix) -> [f32; 3] {
    [
        matrix[0][0] * input[0] + matrix[1][0] * input[1] + matrix[2][0] * input[2],
        matrix[0][1] * input[0] + matrix[1][1] * input[1] + matrix[2][1] * input[2],
        matrix[0][2] * input[0] + matrix[1][2] * input[1] + matrix[2][2] * input[2],
    ]
}

fn multiply_matrices(m1: SigmoidMatrix, m2: SigmoidMatrix) -> SigmoidMatrix {
    let mut output = [[0.0_f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            let mut sum = 0.0_f32;
            for index in 0..3 {
                sum += m1[row][index] * m2[index][column];
            }
            output[row][column] = sum;
        }
    }
    output
}

fn invert_matrix(source: SigmoidMatrix) -> Option<SigmoidMatrix> {
    let det = source[0][0] * (source[2][2] * source[1][1] - source[2][1] * source[1][2])
        - source[1][0] * (source[2][2] * source[0][1] - source[2][1] * source[0][2])
        + source[2][0] * (source[1][2] * source[0][1] - source[1][1] * source[0][2]);
    if det.abs() < MATRIX_EPSILON {
        return None;
    }
    let inverse_det = 1.0 / det;
    Some([
        [
            inverse_det * (source[2][2] * source[1][1] - source[2][1] * source[1][2]),
            -inverse_det * (source[2][2] * source[0][1] - source[2][1] * source[0][2]),
            inverse_det * (source[1][2] * source[0][1] - source[1][1] * source[0][2]),
        ],
        [
            -inverse_det * (source[2][2] * source[1][0] - source[2][0] * source[1][2]),
            inverse_det * (source[2][2] * source[0][0] - source[2][0] * source[0][2]),
            -inverse_det * (source[1][2] * source[0][0] - source[1][0] * source[0][2]),
        ],
        [
            inverse_det * (source[2][1] * source[1][0] - source[2][0] * source[1][1]),
            -inverse_det * (source[2][1] * source[0][0] - source[2][0] * source[0][1]),
            inverse_det * (source[1][1] * source[0][0] - source[1][0] * source[0][1]),
        ],
    ])
}

fn write_f32<const N: usize>(bytes: &mut [u8; N], offset: usize, value: f32) {
    bytes[offset..offset + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
}

fn write_i32<const N: usize>(bytes: &mut [u8; N], offset: usize, value: i32) {
    bytes[offset..offset + size_of::<i32>()].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + size_of::<f32>()]
            .try_into()
            .expect("payload length was checked"),
    )
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + size_of::<i32>()]
            .try_into()
            .expect("payload length was checked"),
    )
}

fn write_v2_fields<const N: usize>(bytes: &mut [u8; N], value: SigmoidParametersV2) {
    write_f32(bytes, 0, value.middle_grey_contrast);
    write_f32(bytes, 4, value.contrast_skewness);
    write_f32(bytes, 8, value.display_white_target);
    write_f32(bytes, 12, value.display_black_target);
    write_i32(bytes, 16, value.color_processing as i32);
    write_f32(bytes, 20, value.hue_preservation);
    write_f32(bytes, 24, value.red_inset);
    write_f32(bytes, 28, value.red_rotation);
    write_f32(bytes, 32, value.green_inset);
    write_f32(bytes, 36, value.green_rotation);
    write_f32(bytes, 40, value.blue_inset);
    write_f32(bytes, 44, value.blue_rotation);
    write_f32(bytes, 48, value.purity);
}
