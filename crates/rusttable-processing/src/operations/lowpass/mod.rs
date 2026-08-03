#![expect(
    clippy::suboptimal_flops,
    reason = "Native Lowpass arithmetic order is preserved for IEEE-754 parity."
)]

//! Source-faithful CPU Lowpass leaf from Darktable's `src/iop/lowpass.c`.
//!
//! Coupled numerical sources are `src/common/gaussian.c`,
//! `src/common/gaussian.h`, `src/common/bilateral.c`, `src/common/bilateral.h`,
//! and `src/develop/imageop_math.h`.  This leaf intentionally does not register
//! the operation or provide GPU, UI, import, masks, or pixelpipe integration.
//! Those boundaries remain explicitly unavailable until their owning hubs are
//! ported.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::similar_names,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    dead_code,
    reason = "the native operation fixes f32 raster arithmetic and source-shaped helpers"
)]

#[cfg(not(test))]
use crate::common::bilateral::{BilateralError, BilateralGeometry, BilateralGrid};
#[cfg(test)]
#[path = "../../common/bilateral.rs"]
mod bilateral_oracle;
#[cfg(test)]
use bilateral_oracle::{BilateralError, BilateralGeometry, BilateralGrid};

#[cfg(not(test))]
use super::ReconstructionBudget;
#[cfg(not(test))]
use crate::RasterDimensions;
#[cfg(test)]
use rusttable_processing::{RasterDimensions, operations::ReconstructionBudget};

use std::{fmt, mem::size_of};

pub const LOWPASS_COMPATIBILITY_ID: &str = "lowpass";
pub const LOWPASS_SCHEMA_VERSION: u16 = 4;
pub const LOWPASS_PARAMETER_BYTES: usize = 28;
pub const LOWPASS_LUT_ENTRIES: usize = 0x10000;
pub const LOWPASS_RADIUS_MIN: f32 = 0.1;
pub const LOWPASS_RADIUS_MAX: f32 = 500.0;
pub const LOWPASS_CONTRAST_MIN: f32 = -3.0;
pub const LOWPASS_CONTRAST_MAX: f32 = 3.0;
pub const LOWPASS_BRIGHTNESS_MIN: f32 = -3.0;
pub const LOWPASS_BRIGHTNESS_MAX: f32 = 3.0;
pub const LOWPASS_SATURATION_MIN: f32 = -3.0;
pub const LOWPASS_SATURATION_MAX: f32 = 3.0;
pub const LOWPASS_DEFAULT_UNBOUND: i32 = 1;
pub const LOWPASS_BILATERAL_RANGE_SIGMA: f32 = 100.0;
pub const LOWPASS_BILATERAL_DETAIL: f32 = -1.0;
pub const LOWPASS_DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;
pub const LOWPASS_MIGRATION_EDGES: &[(u16, u16)] = &[(1, 4), (2, 4), (3, 4)];
pub const LOWPASS_LEGACY_PARAMETER_BYTES: [usize; 3] = [16, 20, 24];
pub const LOWPASS_METADATA_OPAQUE_BYTES: [usize; 3] = [216, 312, 424];

/// Standard processing dimensions are used at the future operation boundary.
pub type LowpassDimensions = RasterDimensions;

const LAB_MIN: [f32; 4] = [0.0, -128.0, -128.0, 0.0];
const LAB_MAX: [f32; 4] = [100.0, 128.0, 128.0, 1.0];
const UNBOUND_MIN: [f32; 4] = [-f32::MAX; 4];
const UNBOUND_MAX: [f32; 4] = [f32::MAX; 4];
const LUT_BYTES: usize = LOWPASS_LUT_ENTRIES * size_of::<f32>();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum GaussianOrder {
    Zero = 0,
    One = 1,
    Two = 2,
}

impl GaussianOrder {
    pub const fn from_raw(value: u32) -> Result<Self, LowpassParameterError> {
        match value {
            0 => Ok(Self::Zero),
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            other => Err(LowpassParameterError::InvalidOrder(other)),
        }
    }

    pub const fn raw(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum LowpassAlgorithm {
    Gaussian = 0,
    Bilateral = 1,
}

impl LowpassAlgorithm {
    pub const fn from_raw(value: u32) -> Result<Self, LowpassParameterError> {
        match value {
            0 => Ok(Self::Gaussian),
            1 => Ok(Self::Bilateral),
            other => Err(LowpassParameterError::InvalidAlgorithm(other)),
        }
    }

    pub const fn raw(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowpassParametersV1 {
    pub order: u32,
    pub radius: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl LowpassParametersV1 {
    pub const BYTE_LEN: usize = 16;

    pub const fn new(order: u32, radius: f32, contrast: f32, saturation: f32) -> Self {
        Self {
            order,
            radius,
            contrast,
            saturation,
        }
    }

    pub fn to_bytes(self) -> [u8; Self::BYTE_LEN] {
        let mut bytes = [0; Self::BYTE_LEN];
        write_u32(&mut bytes, 0, self.order);
        write_f32(&mut bytes, 4, self.radius);
        write_f32(&mut bytes, 8, self.contrast);
        write_f32(&mut bytes, 12, self.saturation);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LowpassCodecError> {
        require_length(bytes, Self::BYTE_LEN)?;
        Ok(Self::new(
            read_u32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowpassParametersV2 {
    pub order: u32,
    pub radius: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
}

impl LowpassParametersV2 {
    pub const BYTE_LEN: usize = 20;

    pub const fn new(
        order: u32,
        radius: f32,
        contrast: f32,
        brightness: f32,
        saturation: f32,
    ) -> Self {
        Self {
            order,
            radius,
            contrast,
            brightness,
            saturation,
        }
    }

    pub fn to_bytes(self) -> [u8; Self::BYTE_LEN] {
        let mut bytes = [0; Self::BYTE_LEN];
        write_u32(&mut bytes, 0, self.order);
        write_f32(&mut bytes, 4, self.radius);
        write_f32(&mut bytes, 8, self.contrast);
        write_f32(&mut bytes, 12, self.brightness);
        write_f32(&mut bytes, 16, self.saturation);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LowpassCodecError> {
        require_length(bytes, Self::BYTE_LEN)?;
        Ok(Self::new(
            read_u32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowpassParametersV3 {
    pub order: u32,
    pub radius: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub unbound: i32,
}

impl LowpassParametersV3 {
    pub const BYTE_LEN: usize = 24;

    pub const fn new(
        order: u32,
        radius: f32,
        contrast: f32,
        brightness: f32,
        saturation: f32,
        unbound: i32,
    ) -> Self {
        Self {
            order,
            radius,
            contrast,
            brightness,
            saturation,
            unbound,
        }
    }

    pub fn to_bytes(self) -> [u8; Self::BYTE_LEN] {
        let mut bytes = [0; Self::BYTE_LEN];
        write_u32(&mut bytes, 0, self.order);
        write_f32(&mut bytes, 4, self.radius);
        write_f32(&mut bytes, 8, self.contrast);
        write_f32(&mut bytes, 12, self.brightness);
        write_f32(&mut bytes, 16, self.saturation);
        write_i32(&mut bytes, 20, self.unbound);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LowpassCodecError> {
        require_length(bytes, Self::BYTE_LEN)?;
        Ok(Self::new(
            read_u32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_i32(bytes, 20),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowpassParametersV4 {
    pub order: u32,
    pub radius: f32,
    pub contrast: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub lowpass_algo: u32,
    pub unbound: i32,
}

impl LowpassParametersV4 {
    pub const BYTE_LEN: usize = LOWPASS_PARAMETER_BYTES;

    pub const fn new(
        order: u32,
        radius: f32,
        contrast: f32,
        brightness: f32,
        saturation: f32,
        lowpass_algo: u32,
        unbound: i32,
    ) -> Self {
        Self {
            order,
            radius,
            contrast,
            brightness,
            saturation,
            lowpass_algo,
            unbound,
        }
    }

    pub const fn defaults() -> Self {
        Self::new(0, 10.0, 1.0, 0.0, 1.0, 0, LOWPASS_DEFAULT_UNBOUND)
    }

    pub fn to_bytes(self) -> [u8; Self::BYTE_LEN] {
        let mut bytes = [0; Self::BYTE_LEN];
        write_u32(&mut bytes, 0, self.order);
        write_f32(&mut bytes, 4, self.radius);
        write_f32(&mut bytes, 8, self.contrast);
        write_f32(&mut bytes, 12, self.brightness);
        write_f32(&mut bytes, 16, self.saturation);
        write_u32(&mut bytes, 20, self.lowpass_algo);
        write_i32(&mut bytes, 24, self.unbound);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LowpassCodecError> {
        require_length(bytes, Self::BYTE_LEN)?;
        Ok(Self::new(
            read_u32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_u32(bytes, 20),
            read_i32(bytes, 24),
        ))
    }
}

/// Exact C-compatible payloads migrate directly to v4.  The larger v1/v2/v3
/// lengths currently asserted by shared compatibility metadata have no
/// authoritative bytes or proven prefix relationship, so they remain opaque.
#[derive(Debug, Clone, PartialEq)]
pub enum LowpassHistory {
    V1(LowpassParametersV1),
    V2(LowpassParametersV2),
    V3(LowpassParametersV3),
    V4(LowpassParametersV4),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl LowpassHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, LowpassCodecError> {
        match version {
            1 if bytes.len() == LowpassParametersV1::BYTE_LEN => {
                Ok(Self::V1(LowpassParametersV1::from_bytes(bytes)?))
            }
            2 if bytes.len() == LowpassParametersV2::BYTE_LEN => {
                Ok(Self::V2(LowpassParametersV2::from_bytes(bytes)?))
            }
            3 if bytes.len() == LowpassParametersV3::BYTE_LEN => {
                Ok(Self::V3(LowpassParametersV3::from_bytes(bytes)?))
            }
            4 => Ok(Self::V4(LowpassParametersV4::from_bytes(bytes)?)),
            1..=3 if metadata_opaque_length(version, bytes.len()) => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
            1 => Err(invalid_length(LowpassParametersV1::BYTE_LEN, bytes.len())),
            2 => Err(invalid_length(LowpassParametersV2::BYTE_LEN, bytes.len())),
            3 => Err(invalid_length(LowpassParametersV3::BYTE_LEN, bytes.len())),
            other => Ok(Self::Opaque {
                version: other,
                bytes: bytes.to_vec(),
            }),
        }
    }

    pub fn migrate_to_v4(&self) -> Result<LowpassParametersV4, LowpassMigrationError> {
        match self {
            Self::V1(old) => Ok(LowpassParametersV4::new(
                old.order,
                f64::from(old.radius).abs() as f32,
                old.contrast,
                0.0,
                old.saturation,
                legacy_algorithm(old.radius),
                0,
            )),
            Self::V2(old) => Ok(LowpassParametersV4::new(
                old.order,
                f64::from(old.radius).abs() as f32,
                old.contrast,
                old.brightness,
                old.saturation,
                legacy_algorithm(old.radius),
                0,
            )),
            Self::V3(old) => Ok(LowpassParametersV4::new(
                old.order,
                f64::from(old.radius).abs() as f32,
                old.contrast,
                old.brightness,
                old.saturation,
                legacy_algorithm(old.radius),
                old.unbound,
            )),
            Self::V4(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(LowpassMigrationError::OpaqueVersion(*version)),
        }
    }

    pub fn config(&self) -> Result<LowpassConfig, LowpassMigrationError> {
        LowpassConfig::try_from(self.migrate_to_v4()?).map_err(LowpassMigrationError::Parameters)
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => value.to_bytes().to_vec(),
            Self::V2(value) => value.to_bytes().to_vec(),
            Self::V3(value) => value.to_bytes().to_vec(),
            Self::V4(value) => value.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => 3,
            Self::V4(_) => 4,
            Self::Opaque { version, .. } => *version,
        }
    }
}

fn metadata_opaque_length(version: u16, actual: usize) -> bool {
    let index = usize::from(version - 1);
    actual == LOWPASS_METADATA_OPAQUE_BYTES[index]
}

fn legacy_algorithm(radius: f32) -> u32 {
    if radius < 0.0 {
        LowpassAlgorithm::Bilateral.raw()
    } else {
        LowpassAlgorithm::Gaussian.raw()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowpassCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for LowpassCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "lowpass payload has {actual} bytes; expected {expected}"
            ),
        }
    }
}
impl std::error::Error for LowpassCodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowpassParameterError {
    NonFinite(&'static str),
    InvalidOrder(u32),
    InvalidAlgorithm(u32),
}

impl fmt::Display for LowpassParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "lowpass {name} is non-finite"),
            Self::InvalidOrder(value) => write!(formatter, "lowpass order {value} is invalid"),
            Self::InvalidAlgorithm(value) => {
                write!(formatter, "lowpass algorithm {value} is invalid")
            }
        }
    }
}
impl std::error::Error for LowpassParameterError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowpassMigrationError {
    OpaqueVersion(u16),
    Parameters(LowpassParameterError),
}
impl fmt::Display for LowpassMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpaqueVersion(version) => {
                write!(formatter, "lowpass history version {version} is opaque")
            }
            Self::Parameters(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for LowpassMigrationError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LowpassConfig {
    order: GaussianOrder,
    radius: f32,
    contrast: f32,
    brightness: f32,
    saturation: f32,
    algorithm: LowpassAlgorithm,
    unbound: i32,
}

impl TryFrom<LowpassParametersV4> for LowpassConfig {
    type Error = LowpassParameterError;

    fn try_from(parameters: LowpassParametersV4) -> Result<Self, Self::Error> {
        let order = GaussianOrder::from_raw(parameters.order)?;
        let algorithm = LowpassAlgorithm::from_raw(parameters.lowpass_algo)?;
        for (name, value) in [
            ("radius", parameters.radius),
            ("contrast", parameters.contrast),
            ("brightness", parameters.brightness),
            ("saturation", parameters.saturation),
        ] {
            if !value.is_finite() {
                return Err(LowpassParameterError::NonFinite(name));
            }
        }
        Ok(Self {
            order,
            radius: parameters.radius,
            contrast: parameters.contrast,
            brightness: parameters.brightness,
            saturation: parameters.saturation,
            algorithm,
            unbound: parameters.unbound,
        })
    }
}

impl LowpassConfig {
    pub fn new(
        order: GaussianOrder,
        radius: f32,
        contrast: f32,
        brightness: f32,
        saturation: f32,
        algorithm: LowpassAlgorithm,
        unbound: i32,
    ) -> Result<Self, LowpassParameterError> {
        Self::try_from(LowpassParametersV4::new(
            order.raw(),
            radius,
            contrast,
            brightness,
            saturation,
            algorithm.raw(),
            unbound,
        ))
    }

    pub fn defaults() -> Self {
        Self::try_from(LowpassParametersV4::defaults()).expect("native lowpass defaults are valid")
    }

    pub const fn order(self) -> GaussianOrder {
        self.order
    }
    pub const fn radius(self) -> f32 {
        self.radius
    }
    pub const fn contrast(self) -> f32 {
        self.contrast
    }
    pub const fn brightness(self) -> f32 {
        self.brightness
    }
    pub const fn saturation(self) -> f32 {
        self.saturation
    }
    pub const fn algorithm(self) -> LowpassAlgorithm {
        self.algorithm
    }
    pub const fn unbound(self) -> i32 {
        self.unbound
    }
    pub const fn parameters(self) -> LowpassParametersV4 {
        LowpassParametersV4::new(
            self.order.raw(),
            self.radius,
            self.contrast,
            self.brightness,
            self.saturation,
            self.algorithm.raw(),
            self.unbound,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LowpassError {
    InvalidDimensions,
    SizeOverflow,
    InvalidParameter(&'static str),
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteOutput { pixel: usize, channel: usize },
    MemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { required: usize },
    Cancelled,
    OpaqueHistory(u16),
    Bilateral(BilateralError),
}

impl fmt::Display for LowpassError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("lowpass dimensions must be nonzero"),
            Self::SizeOverflow => formatter.write_str("lowpass size overflowed"),
            Self::InvalidParameter(name) => {
                write!(formatter, "lowpass parameter {name} is invalid")
            }
            Self::DimensionsMismatch { expected, actual } => write!(
                formatter,
                "lowpass expected {expected} pixels, got {actual}"
            ),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "lowpass input channel {channel} at pixel {pixel} is non-finite"
            ),
            Self::NonFiniteOutput { pixel, channel } => write!(
                formatter,
                "lowpass output channel {channel} at pixel {pixel} is non-finite"
            ),
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "lowpass requires {required} bytes; budget is {budget}"
            ),
            Self::AllocationFailed { required } => {
                write!(formatter, "lowpass could not allocate {required} bytes")
            }
            Self::Cancelled => formatter.write_str("lowpass execution was cancelled"),
            Self::OpaqueHistory(version) => write!(
                formatter,
                "lowpass history version {version} cannot execute"
            ),
            Self::Bilateral(error) => error.fmt(formatter),
        }
    }
}
impl std::error::Error for LowpassError {}

/// Controls the operation-local filter-initialization fault seam.
///
/// Darktable's native `process` receives a caller-owned destination. If
/// `dt_gaussian_init` or `dt_bilateral_init` returns null, it copies the input
/// ROI to that destination and returns successfully. The destination API uses
/// these modes to make that boundary deterministic in tests and in backend
/// fault-injection checks. The `Vec` API intentionally has no copy-through
/// destination and remains fail-closed on allocation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowpassAllocationMode {
    Normal,
    FailGaussianInitialization,
    FailBilateralInitialization,
}

impl LowpassAllocationMode {
    const fn fails_for(self, algorithm: LowpassAlgorithm) -> bool {
        matches!(
            (self, algorithm),
            (Self::FailGaussianInitialization, LowpassAlgorithm::Gaussian)
                | (
                    Self::FailBilateralInitialization,
                    LowpassAlgorithm::Bilateral
                )
        )
    }
}

/// CPU capabilities are deliberately fail-closed. The native tiling formula is
/// retained as `overlap_pixels`, but the pixelpipe tiling contract is unavailable
/// until a shared owner supplies factors, buffers, and frame/tile publication.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Each capability flag is an independent fail-closed integration surface."
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LowpassCapabilities {
    pub cpu: bool,
    pub gpu: bool,
    pub lab_d50: bool,
    pub tiling: bool,
    pub masks: bool,
    pub analysis: bool,
    pub ui: bool,
    pub presets: bool,
}
impl LowpassCapabilities {
    pub const fn cpu_only() -> Self {
        Self {
            cpu: true,
            gpu: false,
            lab_d50: true,
            tiling: false,
            masks: false,
            analysis: false,
            ui: false,
            presets: false,
        }
    }
}
pub const fn capabilities() -> LowpassCapabilities {
    LowpassCapabilities::cpu_only()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LowpassPlan {
    config: LowpassConfig,
    dimensions: RasterDimensions,
    sigma: f32,
    overlap: u32,
    required_memory_bytes: usize,
    bilateral_geometry: Option<([usize; 3], f32, f32)>,
    curves: LowpassCurves,
}

impl LowpassPlan {
    pub fn from_history(
        history: &LowpassHistory,
        dimensions: RasterDimensions,
    ) -> Result<Self, LowpassError> {
        let config = history.config().map_err(|error| match error {
            LowpassMigrationError::OpaqueVersion(version) => LowpassError::OpaqueHistory(version),
            LowpassMigrationError::Parameters(_) => LowpassError::InvalidParameter("history"),
        })?;
        Self::new(config, dimensions)
    }

    pub fn new(config: LowpassConfig, dimensions: RasterDimensions) -> Result<Self, LowpassError> {
        Self::new_with_scale_and_budget(
            config,
            dimensions,
            1.0,
            1.0,
            ReconstructionBudget::default(),
        )
    }

    pub fn new_with_scale(
        config: LowpassConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<Self, LowpassError> {
        Self::new_with_scale_and_budget(
            config,
            dimensions,
            roi_scale,
            piece_scale,
            ReconstructionBudget::default(),
        )
    }

    pub fn new_with_scale_and_budget(
        config: LowpassConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_scale: f32,
        budget: ReconstructionBudget,
    ) -> Result<Self, LowpassError> {
        let width = dimension_width(dimensions)?;
        let height = dimension_height(dimensions)?;
        if !roi_scale.is_finite() || roi_scale <= 0.0 {
            return Err(LowpassError::InvalidParameter("roi_scale"));
        }
        if !piece_scale.is_finite() || piece_scale <= 0.0 {
            return Err(LowpassError::InvalidParameter("piece_scale"));
        }
        let radius = (0.1_f64.max(f64::from(config.radius))) as f32;
        let sigma = radius * roi_scale / piece_scale;
        if !sigma.is_finite() || sigma <= 0.0 {
            return Err(LowpassError::InvalidParameter("sigma"));
        }
        let overlap_float = (4.0_f32 * sigma).ceil();
        let overlap = u32::try_from(overlap_float as u64)
            .map_err(|_| LowpassError::InvalidParameter("overlap"))?;
        let bilateral_geometry = match config.algorithm {
            LowpassAlgorithm::Gaussian => None,
            LowpassAlgorithm::Bilateral => {
                let geometry =
                    BilateralGeometry::new(width, height, sigma, LOWPASS_BILATERAL_RANGE_SIGMA)
                        .map_err(LowpassError::Bilateral)?;
                Some((
                    geometry.grid_dimensions(),
                    geometry.effective_sigma_s(),
                    geometry.effective_sigma_r(),
                ))
            }
        };
        let required_memory_bytes = required_memory(config, dimensions, sigma)?;
        if required_memory_bytes > budget.maximum_bytes() {
            return Err(LowpassError::MemoryBudgetExceeded {
                required: required_memory_bytes,
                budget: budget.maximum_bytes(),
            });
        }
        let curves = LowpassCurves::new(config)?;
        Ok(Self {
            config,
            dimensions,
            sigma,
            overlap,
            required_memory_bytes,
            bilateral_geometry,
            curves,
        })
    }

    pub const fn config(&self) -> LowpassConfig {
        self.config
    }
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    pub const fn sigma(&self) -> f32 {
        self.sigma
    }
    pub const fn overlap_pixels(&self) -> u32 {
        self.overlap
    }
    pub const fn required_memory_bytes(&self) -> usize {
        self.required_memory_bytes
    }
    pub const fn bilateral_geometry(&self) -> Option<([usize; 3], f32, f32)> {
        self.bilateral_geometry
    }

    pub fn execute(&self, input: &[[f32; 4]]) -> Result<Vec<[f32; 4]>, LowpassError> {
        self.execute_with_cancel(input, || false)
    }

    /// Execute into a caller-owned destination, matching native copy-through
    /// publication when a filter cannot be initialized.
    pub fn execute_into(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
    ) -> Result<(), LowpassError> {
        self.execute_into_with_cancel(input, output, || false)
    }

    /// Execute into a caller-owned destination without publishing partial
    /// filter or curve results when cancellation or another error occurs.
    pub fn execute_into_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        cancelled: F,
    ) -> Result<(), LowpassError> {
        self.execute_into_with_cancel_and_allocation_mode(
            input,
            output,
            LowpassAllocationMode::Normal,
            cancelled,
        )
    }

    /// Execute into a caller-owned destination with a deterministic
    /// filter-initialization failure seam.
    ///
    /// `FailGaussianInitialization` and `FailBilateralInitialization` model
    /// native `dt_gaussian_init` and `dt_bilateral_init` returning null. In
    /// either matching case the input is copied byte-for-byte to `output` and
    /// the call succeeds. This seam is operation-local so tests do not need to
    /// alter global allocation state.
    pub fn execute_into_with_cancel_and_allocation_mode<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        allocation_mode: LowpassAllocationMode,
        mut cancelled: F,
    ) -> Result<(), LowpassError> {
        let expected = pixel_count(self.dimensions)?;
        if output.len() != expected {
            return Err(LowpassError::DimensionsMismatch {
                expected,
                actual: output.len(),
            });
        }
        self.validate_input(input, &mut cancelled)?;
        match self.execute_validated(input, &mut cancelled, allocation_mode, true)? {
            LowpassValidatedOutput::Filtered(filtered) => output.copy_from_slice(&filtered),
            LowpassValidatedOutput::CopyThrough => output.copy_from_slice(input),
        }
        Ok(())
    }

    /// Allocation failures are fail-closed: unlike native's caller-owned
    /// copy-through fallback, this `Vec` API has no destination to publish to.
    /// No partial result is returned on any error. Use [`Self::execute_into`]
    /// when the caller owns the destination buffer and needs native
    /// copy-through and publication semantics.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        mut cancelled: F,
    ) -> Result<Vec<[f32; 4]>, LowpassError> {
        self.validate_input(input, &mut cancelled)?;
        match self.execute_validated(input, &mut cancelled, LowpassAllocationMode::Normal, false)? {
            LowpassValidatedOutput::Filtered(filtered) => Ok(filtered),
            LowpassValidatedOutput::CopyThrough => {
                unreachable!("Vec execution never enables native copy-through")
            }
        }
    }

    fn validate_input<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        cancelled: &mut F,
    ) -> Result<(), LowpassError> {
        let expected = pixel_count(self.dimensions)?;
        let width = dimension_width(self.dimensions)?;
        if input.len() != expected {
            return Err(LowpassError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        for (pixel, sample) in input.iter().enumerate() {
            if pixel % width == 0 && cancelled() {
                return Err(LowpassError::Cancelled);
            }
            for (channel, value) in sample.iter().enumerate() {
                if !value.is_finite() {
                    return Err(LowpassError::NonFiniteInput { pixel, channel });
                }
            }
        }
        if cancelled() {
            return Err(LowpassError::Cancelled);
        }
        Ok(())
    }

    fn execute_validated<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        cancelled: &mut F,
        allocation_mode: LowpassAllocationMode,
        allow_copy_through: bool,
    ) -> Result<LowpassValidatedOutput, LowpassError> {
        let expected = pixel_count(self.dimensions)?;
        let width = dimension_width(self.dimensions)?;
        let bounds = if self.config.unbound != 0 {
            (UNBOUND_MIN, UNBOUND_MAX)
        } else {
            (LAB_MIN, LAB_MAX)
        };
        if allocation_mode.fails_for(self.config.algorithm) {
            return Ok(LowpassValidatedOutput::CopyThrough);
        }
        let filtered = match self.config.algorithm {
            LowpassAlgorithm::Gaussian => match gaussian_blur(
                input,
                self.dimensions,
                self.sigma,
                bounds.0,
                bounds.1,
                self.config.order,
                cancelled,
            ) {
                Ok(filtered) => filtered,
                Err(error)
                    if allow_copy_through
                        && matches!(error, LowpassError::AllocationFailed { .. }) =>
                {
                    return Ok(LowpassValidatedOutput::CopyThrough);
                }
                Err(error) => return Err(error),
            },
            LowpassAlgorithm::Bilateral => {
                match bilateral_blur_with_copy_through(
                    input,
                    self.dimensions,
                    self.sigma,
                    cancelled,
                    allow_copy_through,
                )? {
                    LowpassValidatedOutput::Filtered(filtered) => filtered,
                    LowpassValidatedOutput::CopyThrough => {
                        return Ok(LowpassValidatedOutput::CopyThrough);
                    }
                }
            }
        };
        let mut output = Vec::new();
        reserve_exact(&mut output, expected, size_of::<[f32; 4]>())?;
        for (pixel, (source, mut result)) in input.iter().zip(filtered).enumerate() {
            if pixel % width == 0 && cancelled() {
                return Err(LowpassError::Cancelled);
            }
            result[0] = self.curves.apply_l(result[0]);
            result[1] = scale_chroma(result[1], self.config.saturation, bounds.0[1], bounds.1[1]);
            result[2] = scale_chroma(result[2], self.config.saturation, bounds.0[2], bounds.1[2]);
            result[3] = source[3];
            for (channel, value) in result.iter().enumerate() {
                if !value.is_finite() {
                    return Err(LowpassError::NonFiniteOutput { pixel, channel });
                }
            }
            output.push(result);
        }
        Ok(LowpassValidatedOutput::Filtered(output))
    }
}

#[derive(Debug, PartialEq)]
enum LowpassValidatedOutput {
    Filtered(Vec<[f32; 4]>),
    CopyThrough,
}

fn dimension_width(dimensions: RasterDimensions) -> Result<usize, LowpassError> {
    usize::try_from(dimensions.width()).map_err(|_| LowpassError::SizeOverflow)
}
fn dimension_height(dimensions: RasterDimensions) -> Result<usize, LowpassError> {
    usize::try_from(dimensions.height()).map_err(|_| LowpassError::SizeOverflow)
}
fn pixel_count(dimensions: RasterDimensions) -> Result<usize, LowpassError> {
    usize::try_from(dimensions.pixel_count()).map_err(|_| LowpassError::SizeOverflow)
}

fn required_memory(
    config: LowpassConfig,
    dimensions: RasterDimensions,
    sigma: f32,
) -> Result<usize, LowpassError> {
    let pixels = pixel_count(dimensions)?;
    let base = checked_mul(pixels, size_of::<[f32; 4]>())?;
    let filter = match config.algorithm {
        LowpassAlgorithm::Gaussian => base,
        LowpassAlgorithm::Bilateral => BilateralGrid::required_memory_bytes(
            dimension_width(dimensions)?,
            dimension_height(dimensions)?,
            sigma,
            LOWPASS_BILATERAL_RANGE_SIGMA,
        )
        .map_err(LowpassError::Bilateral)?,
    };
    checked_add(
        checked_add(checked_mul(base, 2)?, base.max(filter))?,
        checked_mul(LUT_BYTES, 2)?,
    )
}

fn bilateral_blur_with_copy_through<F: FnMut() -> bool>(
    input: &[[f32; 4]],
    dimensions: RasterDimensions,
    sigma: f32,
    cancelled: &mut F,
    allow_copy_through: bool,
) -> Result<LowpassValidatedOutput, LowpassError> {
    let width = dimension_width(dimensions)?;
    let height = dimension_height(dimensions)?;
    let mut grid = match BilateralGrid::new(width, height, sigma, LOWPASS_BILATERAL_RANGE_SIGMA)
        .map_err(LowpassError::Bilateral)
    {
        Ok(grid) => grid,
        Err(error)
            if allow_copy_through
                && matches!(
                    error,
                    LowpassError::Bilateral(BilateralError::AllocationFailed { .. })
                ) =>
        {
            return Ok(LowpassValidatedOutput::CopyThrough);
        }
        Err(error) => return Err(error),
    };
    // The operation-level validation above is row-cancellable before entering
    // the shared helper. Its second scalar-lightness check is finite and bounded
    // by the committed memory budget; all expensive grid passes poll by row.
    grid.splat_with_cancel(input, cancelled)
        .map_err(map_bilateral_error)?;
    grid.blur_with_cancel(cancelled)
        .map_err(map_bilateral_error)?;
    let filtered = grid
        .slice_with_cancel(input, LOWPASS_BILATERAL_DETAIL, cancelled)
        .map_err(map_bilateral_error)?;
    Ok(LowpassValidatedOutput::Filtered(filtered))
}
const fn map_bilateral_error(error: BilateralError) -> LowpassError {
    match error {
        BilateralError::Cancelled => LowpassError::Cancelled,
        other => LowpassError::Bilateral(other),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LowpassCurves {
    contrast: Vec<f32>,
    contrast_coefficients: [f32; 3],
    brightness: Vec<f32>,
    brightness_coefficients: [f32; 3],
}
impl LowpassCurves {
    fn new(config: LowpassConfig) -> Result<Self, LowpassError> {
        let mut contrast = Vec::new();
        reserve_exact(&mut contrast, LOWPASS_LUT_ENTRIES, size_of::<f32>())?;
        contrast.resize(LOWPASS_LUT_ENTRIES, 0.0);
        if f64::from(config.contrast).abs() <= 1.0 {
            for (k, value) in contrast.iter_mut().enumerate() {
                *value = config.contrast * (100.0_f32 * k as f32 / 0x10000 as f32 - 50.0) + 50.0;
            }
        } else {
            let contrastm1sq = (5.0_f64
                * (f64::from(config.contrast).abs() - 1.0)
                * (f64::from(config.contrast).abs() - 1.0)) as f32;
            let contrastscale = config.contrast.signum() * (1.0_f32 + contrastm1sq).sqrt();
            for (k, value) in contrast.iter_mut().enumerate() {
                let kx2m1 = 2.0_f32 * k as f32 / 0x10000 as f32 - 1.0;
                *value = 50.0
                    * (contrastscale * kx2m1 / (1.0 + contrastm1sq * kx2m1 * kx2m1).sqrt() + 1.0);
            }
        }
        let contrast_coefficients = estimate_exp(&contrast);
        let mut brightness = Vec::new();
        reserve_exact(&mut brightness, LOWPASS_LUT_ENTRIES, size_of::<f32>())?;
        brightness.resize(LOWPASS_LUT_ENTRIES, 0.0);
        let gamma = if config.brightness >= 0.0 {
            1.0 / (1.0 + config.brightness)
        } else {
            1.0 - config.brightness
        };
        for (k, value) in brightness.iter_mut().enumerate() {
            *value = 100.0 * (k as f32 / 0x10000 as f32).powf(gamma);
        }
        let brightness_coefficients = estimate_exp(&brightness);
        Ok(Self {
            contrast,
            contrast_coefficients,
            brightness,
            brightness_coefficients,
        })
    }
    fn apply_l(&self, lightness: f32) -> f32 {
        let contrast = lookup_curve(lightness, &self.contrast, self.contrast_coefficients);
        lookup_curve(contrast, &self.brightness, self.brightness_coefficients)
    }
}
fn estimate_exp(table: &[f32]) -> [f32; 3] {
    let x = [0.7_f32, 0.8, 0.9, 1.0];
    let y = [
        table[clamp_lut_index((x[0] * 0x10000 as f32) as i32)],
        table[clamp_lut_index((x[1] * 0x10000 as f32) as i32)],
        table[clamp_lut_index((x[2] * 0x10000 as f32) as i32)],
        table[clamp_lut_index((x[3] * 0x10000 as f32) as i32)],
    ];
    let x0 = x[3];
    let y0 = y[3];
    let mut exponent = 0.0;
    let mut count = 0;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = x[index] / x0;
        if yy > 0.0 && xx > 0.0 {
            exponent += (y[index] / y0).ln() / (x[index] / x0).ln();
            count += 1;
        }
    }
    if count != 0 {
        exponent *= 1.0 / count as f32;
    } else {
        exponent = 1.0;
    }
    [1.0 / x0, y0, exponent]
}
fn lookup_curve(lightness: f32, table: &[f32], coefficients: [f32; 3]) -> f32 {
    if lightness < 100.0 {
        table[clamp_lut_index((lightness / 100.0 * 0x10000 as f32) as i32)]
    } else {
        coefficients[1] * (lightness / 100.0 * coefficients[0]).powf(coefficients[2])
    }
}
fn clamp_lut_index(value: i32) -> usize {
    value.clamp(0, 0xffff) as usize
}
fn scale_chroma(value: f32, saturation: f32, minimum: f32, maximum: f32) -> f32 {
    clamp_native(value * saturation, minimum, maximum)
}
fn clamp_native(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value >= minimum {
        if value <= maximum { value } else { maximum }
    } else {
        minimum
    }
}

fn gaussian_blur<F: FnMut() -> bool>(
    input: &[[f32; 4]],
    dimensions: RasterDimensions,
    sigma: f32,
    minimum: [f32; 4],
    maximum: [f32; 4],
    order: GaussianOrder,
    cancelled: &mut F,
) -> Result<Vec<[f32; 4]>, LowpassError> {
    let expected = pixel_count(dimensions)?;
    let width = dimension_width(dimensions)?;
    let height = dimension_height(dimensions)?;
    let mut temp = Vec::new();
    reserve_exact(&mut temp, expected, size_of::<[f32; 4]>())?;
    temp.resize(expected, [0.0; 4]);
    let mut output = Vec::new();
    reserve_exact(&mut output, expected, size_of::<[f32; 4]>())?;
    output.resize(expected, [0.0; 4]);
    let (a0, a1, a2, a3, b1, b2, coefp, coefn) = gaussian_parameters(sigma, order);

    for x in 0..width {
        if cancelled() {
            return Err(LowpassError::Cancelled);
        }
        let first = clamp_channels(input[x], minimum, maximum);
        let mut xp = first;
        let mut yb = first.map(|value| value * coefp);
        let mut yp = yb;
        for y in 0..height {
            if cancelled() {
                return Err(LowpassError::Cancelled);
            }
            let index = y * width + x;
            let current = clamp_channels(input[index], minimum, maximum);
            let mut value = [0.0; 4];
            for channel in 0..4 {
                value[channel] =
                    a0 * current[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = current[channel];
                yb[channel] = yp[channel];
                yp[channel] = value[channel];
            }
            temp[index] = value;
        }
        let last = clamp_channels(input[(height - 1) * width + x], minimum, maximum);
        let mut xn = last;
        let mut xa = last;
        let mut yn = last.map(|value| value * coefn);
        let mut ya = yn;
        for y in (0..height).rev() {
            if cancelled() {
                return Err(LowpassError::Cancelled);
            }
            let index = y * width + x;
            let current = clamp_channels(input[index], minimum, maximum);
            let mut value = [0.0; 4];
            for channel in 0..4 {
                value[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = current[channel];
                ya[channel] = yn[channel];
                yn[channel] = value[channel];
                temp[index][channel] += value[channel];
            }
        }
    }
    for y in 0..height {
        if cancelled() {
            return Err(LowpassError::Cancelled);
        }
        let row = y * width;
        let first = clamp_channels(temp[row], minimum, maximum);
        let mut xp = first;
        let mut yb = first.map(|value| value * coefp);
        let mut yp = yb;
        for x in 0..width {
            if cancelled() {
                return Err(LowpassError::Cancelled);
            }
            let index = row + x;
            let current = clamp_channels(temp[index], minimum, maximum);
            let mut value = [0.0; 4];
            for channel in 0..4 {
                value[channel] =
                    a0 * current[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = current[channel];
                yb[channel] = yp[channel];
                yp[channel] = value[channel];
            }
            output[index] = value;
        }
        let last = clamp_channels(temp[row + width - 1], minimum, maximum);
        let mut xn = last;
        let mut xa = last;
        let mut yn = last.map(|value| value * coefn);
        let mut ya = yn;
        for x in (0..width).rev() {
            if cancelled() {
                return Err(LowpassError::Cancelled);
            }
            let index = row + x;
            let current = clamp_channels(temp[index], minimum, maximum);
            let mut value = [0.0; 4];
            for channel in 0..4 {
                value[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = current[channel];
                ya[channel] = yn[channel];
                yn[channel] = value[channel];
                output[index][channel] += value[channel];
            }
        }
    }
    Ok(output)
}

fn gaussian_parameters(
    sigma: f32,
    order: GaussianOrder,
) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let alpha = 1.695 / sigma;
    let ema = (-alpha).exp();
    let ema2 = (-2.0 * alpha).exp();
    let b1 = -2.0 * ema;
    let b2 = ema2;
    let (a0, a1, a2, a3) = match order {
        GaussianOrder::Zero => {
            let k = (1.0 - ema) * (1.0 - ema) / (1.0 + 2.0 * alpha * ema - ema2);
            (
                k,
                k * (alpha - 1.0) * ema,
                k * (alpha + 1.0) * ema,
                -k * ema2,
            )
        }
        GaussianOrder::One => {
            let a0 = (1.0 - ema) * (1.0 - ema);
            (a0, 0.0, -a0, 0.0)
        }
        GaussianOrder::Two => {
            let k = -(ema2 - 1.0) / (2.0 * alpha * ema);
            let mut kn = -2.0 * (-1.0 + 3.0 * ema - 3.0 * ema * ema + ema * ema * ema);
            kn /= 3.0 * ema + 1.0 + 3.0 * ema * ema + ema * ema * ema;
            (
                kn,
                -kn * (1.0 + k * alpha) * ema,
                kn * (1.0 - k * alpha) * ema,
                -kn * ema2,
            )
        }
    };
    let denominator = 1.0 + b1 + b2;
    (
        a0,
        a1,
        a2,
        a3,
        b1,
        b2,
        (a0 + a1) / denominator,
        (a2 + a3) / denominator,
    )
}
fn clamp_channels(value: [f32; 4], minimum: [f32; 4], maximum: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|channel| clamp_native(value[channel], minimum[channel], maximum[channel]))
}

fn reserve_exact<T>(
    vector: &mut Vec<T>,
    length: usize,
    element_bytes: usize,
) -> Result<(), LowpassError> {
    let required = checked_mul(length, element_bytes)?;
    vector
        .try_reserve_exact(length)
        .map_err(|_| LowpassError::AllocationFailed { required })?;
    Ok(())
}
fn checked_mul(left: usize, right: usize) -> Result<usize, LowpassError> {
    left.checked_mul(right).ok_or(LowpassError::SizeOverflow)
}
fn checked_add(left: usize, right: usize) -> Result<usize, LowpassError> {
    left.checked_add(right).ok_or(LowpassError::SizeOverflow)
}
const fn invalid_length(expected: usize, actual: usize) -> LowpassCodecError {
    LowpassCodecError::InvalidLength { expected, actual }
}
const fn require_length(bytes: &[u8], expected: usize) -> Result<(), LowpassCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(invalid_length(expected, bytes.len()))
    }
}
fn read_u32(bytes: &[u8], start: usize) -> u32 {
    u32::from_le_bytes(
        bytes[start..start + 4]
            .try_into()
            .expect("validated scalar range"),
    )
}
fn read_i32(bytes: &[u8], start: usize) -> i32 {
    i32::from_le_bytes(
        bytes[start..start + 4]
            .try_into()
            .expect("validated scalar range"),
    )
}
fn read_f32(bytes: &[u8], start: usize) -> f32 {
    f32::from_le_bytes(
        bytes[start..start + 4]
            .try_into()
            .expect("validated scalar range"),
    )
}
fn write_u32(bytes: &mut [u8], start: usize, value: u32) {
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
}
fn write_i32(bytes: &mut [u8], start: usize, value: i32) {
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
}
fn write_f32(bytes: &mut [u8], start: usize, value: f32) {
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
}
