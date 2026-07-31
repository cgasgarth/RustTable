//! Bounded, source-faithful CPU leaf for Darktable's diffuse/sharpen module.
//!
//! The direct native oracle is `src/iop/diffuse.c`; coupled helpers are
//! `src/common/bspline.h`, `src/common/dwt.h`, and
//! `src/develop/noise_generator.h`.  This leaf deliberately does not register
//! itself with the shared operation registry or pixelpipe.  GPU, UI, history
//! routing, and production tiling ownership remain deferred integration work.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::fmt;

mod inpaint;
mod pde;
mod wavelets;

#[allow(unused_imports)]
pub use inpaint::{gaussian_noise, splitmix32, xoshiro128plus};
#[allow(unused_imports)]
pub use wavelets::{
    B_SPLINE_SIGMA, dwt_interleave_rows, equivalent_sigma_at_step,
    num_steps_to_reach_equivalent_sigma,
};

/// Native module identity from `src/iop/diffuse.c`.
pub const DIFFUSE_COMPATIBILITY_ID: &str = "diffuse";
/// Stable Rust identity reserved for later registry integration.
pub const DIFFUSE_RUST_ID: &str = "rusttable.diffuse";
/// Native `DT_MODULE_INTROSPECTION(2, ...)` schema version.
pub const DIFFUSE_SCHEMA_VERSION: u16 = 2;
/// Current native parameter payload size on the pinned little-endian targets.
pub const DIFFUSE_PARAMETER_BYTES: usize = 60;
/// Version-one native payload size before `radius_center` was appended.
pub const DIFFUSE_V1_PARAMETER_BYTES: usize = 56;
/// Native four-channel image layout.
pub const DIFFUSE_CHANNELS: usize = 4;
/// Native scale cap from `MAX_NUM_SCALES`.
pub const DIFFUSE_MAX_SCALES: usize = 10;
/// Native PDE spatial step.
pub const DIFFUSE_H: usize = 1;
/// Native PDE time-step multiplier for `h == 1`.
pub const DIFFUSE_KAPPA: f32 = 0.25;
/// Native linear scene-referred RGB boundary.
pub const DIFFUSE_INPUT_COLOR_SPACE: &str = "linear RGB, scene-referred";
/// Native output colorspace declaration.
pub const DIFFUSE_OUTPUT_COLOR_SPACE: &str = "linear RGB";
/// Default bound for this leaf's fallible scratch allocations.
pub const DIFFUSE_DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;

/// Four float channels in native order. The fourth channel participates in the
/// CPU SIMD loop exactly as it does in `diffuse.c`; the mask itself uses RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffusePixel {
    channels: [f32; DIFFUSE_CHANNELS],
}

impl DiffusePixel {
    #[must_use]
    pub const fn from_channels(channels: [f32; DIFFUSE_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; DIFFUSE_CHANNELS] {
        self.channels
    }

    pub(crate) fn channels_mut(&mut self) -> &mut [f32; DIFFUSE_CHANNELS] {
        &mut self.channels
    }
}

/// Nonzero raster dimensions used by the native ROI loops.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffuseDimensions {
    width: u32,
    height: u32,
}

impl DiffuseDimensions {
    pub fn new(width: u32, height: u32) -> Result<Self, DiffuseExecutionError> {
        if width == 0 || height == 0 {
            return Err(DiffuseExecutionError::InvalidDimensions);
        }
        let dimensions = Self { width, height };
        let _ = dimensions.pixel_count()?;
        Ok(dimensions)
    }

    #[must_use]
    pub const fn width(self) -> usize {
        self.width as usize
    }

    #[must_use]
    pub const fn height(self) -> usize {
        self.height as usize
    }

    pub fn pixel_count(self) -> Result<usize, DiffuseExecutionError> {
        usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)
    }
}

/// Native isotropy selection encoded by the sign of each anisotropy parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IsotropyMode {
    Isotropic,
    Isophote,
    Gradient,
}

fn isotropy_mode(anisotropy: f32) -> IsotropyMode {
    if anisotropy == 0.0 {
        IsotropyMode::Isotropic
    } else if anisotropy > 0.0 {
        IsotropyMode::Isophote
    } else {
        IsotropyMode::Gradient
    }
}

/// Current native parameter payload, in exact declaration order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffuseParametersV2 {
    pub iterations: i32,
    pub sharpness: f32,
    pub radius: i32,
    pub regularization: f32,
    pub variance_threshold: f32,
    pub anisotropy_first: f32,
    pub anisotropy_second: f32,
    pub anisotropy_third: f32,
    pub anisotropy_fourth: f32,
    pub threshold: f32,
    pub first: f32,
    pub second: f32,
    pub third: f32,
    pub fourth: f32,
    pub radius_center: i32,
}

impl DiffuseParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            iterations: 1,
            sharpness: 0.0,
            radius: 8,
            regularization: 0.0,
            variance_threshold: 0.0,
            anisotropy_first: 0.0,
            anisotropy_second: 0.0,
            anisotropy_third: 0.0,
            anisotropy_fourth: 0.0,
            threshold: 0.0,
            first: 0.0,
            second: 0.0,
            third: 0.0,
            fourth: 0.0,
            radius_center: 0,
        }
    }

    #[must_use]
    pub const fn from_v1(parameters: DiffuseParametersV1) -> Self {
        Self {
            iterations: parameters.iterations,
            sharpness: parameters.sharpness,
            radius: parameters.radius,
            regularization: parameters.regularization,
            variance_threshold: parameters.variance_threshold,
            anisotropy_first: parameters.anisotropy_first,
            anisotropy_second: parameters.anisotropy_second,
            anisotropy_third: parameters.anisotropy_third,
            anisotropy_fourth: parameters.anisotropy_fourth,
            threshold: parameters.threshold,
            first: parameters.first,
            second: parameters.second,
            third: parameters.third,
            fourth: parameters.fourth,
            radius_center: 0,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; DIFFUSE_PARAMETER_BYTES] {
        let mut bytes = [0_u8; DIFFUSE_PARAMETER_BYTES];
        write_i32(&mut bytes, 0, self.iterations);
        write_f32(&mut bytes, 4, self.sharpness);
        write_i32(&mut bytes, 8, self.radius);
        write_f32(&mut bytes, 12, self.regularization);
        write_f32(&mut bytes, 16, self.variance_threshold);
        write_f32(&mut bytes, 20, self.anisotropy_first);
        write_f32(&mut bytes, 24, self.anisotropy_second);
        write_f32(&mut bytes, 28, self.anisotropy_third);
        write_f32(&mut bytes, 32, self.anisotropy_fourth);
        write_f32(&mut bytes, 36, self.threshold);
        write_f32(&mut bytes, 40, self.first);
        write_f32(&mut bytes, 44, self.second);
        write_f32(&mut bytes, 48, self.third);
        write_f32(&mut bytes, 52, self.fourth);
        write_i32(&mut bytes, 56, self.radius_center);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DiffuseCodecError> {
        if bytes.len() != DIFFUSE_PARAMETER_BYTES {
            return Err(DiffuseCodecError::InvalidLength {
                expected: DIFFUSE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            iterations: read_i32(bytes, 0),
            sharpness: read_f32(bytes, 4),
            radius: read_i32(bytes, 8),
            regularization: read_f32(bytes, 12),
            variance_threshold: read_f32(bytes, 16),
            anisotropy_first: read_f32(bytes, 20),
            anisotropy_second: read_f32(bytes, 24),
            anisotropy_third: read_f32(bytes, 28),
            anisotropy_fourth: read_f32(bytes, 32),
            threshold: read_f32(bytes, 36),
            first: read_f32(bytes, 40),
            second: read_f32(bytes, 44),
            third: read_f32(bytes, 48),
            fourth: read_f32(bytes, 52),
            radius_center: read_i32(bytes, 56),
        })
    }
}

/// Version-one native payload before `radius_center` was appended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffuseParametersV1 {
    pub iterations: i32,
    pub sharpness: f32,
    pub radius: i32,
    pub regularization: f32,
    pub variance_threshold: f32,
    pub anisotropy_first: f32,
    pub anisotropy_second: f32,
    pub anisotropy_third: f32,
    pub anisotropy_fourth: f32,
    pub threshold: f32,
    pub first: f32,
    pub second: f32,
    pub third: f32,
    pub fourth: f32,
}

impl DiffuseParametersV1 {
    #[must_use]
    pub fn to_bytes(self) -> [u8; DIFFUSE_V1_PARAMETER_BYTES] {
        let current = DiffuseParametersV2::from_v1(self).to_bytes();
        let mut bytes = [0_u8; DIFFUSE_V1_PARAMETER_BYTES];
        bytes.copy_from_slice(&current[..DIFFUSE_V1_PARAMETER_BYTES]);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DiffuseCodecError> {
        if bytes.len() != DIFFUSE_V1_PARAMETER_BYTES {
            return Err(DiffuseCodecError::InvalidLength {
                expected: DIFFUSE_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            iterations: read_i32(bytes, 0),
            sharpness: read_f32(bytes, 4),
            radius: read_i32(bytes, 8),
            regularization: read_f32(bytes, 12),
            variance_threshold: read_f32(bytes, 16),
            anisotropy_first: read_f32(bytes, 20),
            anisotropy_second: read_f32(bytes, 24),
            anisotropy_third: read_f32(bytes, 28),
            anisotropy_fourth: read_f32(bytes, 32),
            threshold: read_f32(bytes, 36),
            first: read_f32(bytes, 40),
            second: read_f32(bytes, 44),
            third: read_f32(bytes, 48),
            fourth: read_f32(bytes, 52),
        })
    }
}

/// History payload preserving unknown versions and the native v1 migration.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffuseHistory {
    V1(DiffuseParametersV1),
    V2(DiffuseParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl DiffuseHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, DiffuseCodecError> {
        match version {
            1 => Ok(Self::V1(DiffuseParametersV1::from_bytes(bytes)?)),
            DIFFUSE_SCHEMA_VERSION => Ok(Self::V2(DiffuseParametersV2::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => DIFFUSE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub fn current(&self) -> Result<DiffuseParametersV2, DiffuseCodecError> {
        match self {
            Self::V1(parameters) => Ok(DiffuseParametersV2::from_v1(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(DiffuseCodecError::UnsupportedVersion(*version)),
        }
    }
}

/// Codec and migration failures at the history boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffuseCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for DiffuseCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "diffuse payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "diffuse version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for DiffuseCodecError {}

/// Range and finite-value errors for the native parameter schema.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffuseParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for DiffuseParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "diffuse {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "diffuse {name} is outside its range"),
        }
    }
}

impl std::error::Error for DiffuseParameterError {}

/// Checked execution configuration; ABI parameters remain available separately.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffuseConfig {
    parameters: DiffuseParametersV2,
}

impl TryFrom<DiffuseParametersV2> for DiffuseConfig {
    type Error = DiffuseParameterError;

    fn try_from(parameters: DiffuseParametersV2) -> Result<Self, Self::Error> {
        check_i32("iterations", parameters.iterations, 0, 500)?;
        check_f32("sharpness", parameters.sharpness, -1.0, 1.0)?;
        check_i32("radius", parameters.radius, 0, 2048)?;
        check_f32("regularization", parameters.regularization, 0.0, 4.0)?;
        check_f32(
            "variance_threshold",
            parameters.variance_threshold,
            -2.0,
            2.0,
        )?;
        for (name, value) in [
            ("anisotropy_first", parameters.anisotropy_first),
            ("anisotropy_second", parameters.anisotropy_second),
            ("anisotropy_third", parameters.anisotropy_third),
            ("anisotropy_fourth", parameters.anisotropy_fourth),
        ] {
            check_f32(name, value, -10.0, 10.0)?;
        }
        check_f32("threshold", parameters.threshold, 0.0, 8.0)?;
        for (name, value) in [
            ("first", parameters.first),
            ("second", parameters.second),
            ("third", parameters.third),
            ("fourth", parameters.fourth),
        ] {
            check_f32(name, value, -1.0, 1.0)?;
        }
        check_i32("radius_center", parameters.radius_center, 0, 1024)?;
        Ok(Self { parameters })
    }
}

impl DiffuseConfig {
    pub fn new(parameters: DiffuseParametersV2) -> Result<Self, DiffuseParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(DiffuseParametersV2::defaults()).expect("native diffuse defaults are valid")
    }

    #[must_use]
    pub const fn parameters(self) -> DiffuseParametersV2 {
        self.parameters
    }

    #[must_use]
    pub fn anisotropy(self) -> [f32; 4] {
        [
            self.parameters.anisotropy_first * self.parameters.anisotropy_first,
            self.parameters.anisotropy_second * self.parameters.anisotropy_second,
            self.parameters.anisotropy_third * self.parameters.anisotropy_third,
            self.parameters.anisotropy_fourth * self.parameters.anisotropy_fourth,
        ]
    }

    #[must_use]
    pub fn isotropy(self) -> [IsotropyMode; 4] {
        [
            isotropy_mode(self.parameters.anisotropy_first),
            isotropy_mode(self.parameters.anisotropy_second),
            isotropy_mode(self.parameters.anisotropy_third),
            isotropy_mode(self.parameters.anisotropy_fourth),
        ]
    }

    #[must_use]
    pub fn regularization(self) -> f32 {
        10.0_f32.powf(self.parameters.regularization) - 1.0
    }

    #[must_use]
    pub fn variance_threshold(self) -> f32 {
        10.0_f32.powf(self.parameters.variance_threshold)
    }
}

/// Tiling values copied from `tiling_callback`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffuseTiling {
    pub factor: f32,
    pub factor_cl: f32,
    pub maxbuf: f32,
    pub maxbuf_cl: f32,
    pub overhead: u32,
    pub overlap: u32,
    pub align: u32,
}

/// Fallible CPU execution and publication failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffuseExecutionError {
    InvalidDimensions,
    DimensionsTooLarge,
    DimensionsMismatch { expected: usize, actual: usize },
    InvalidScale,
    AllocationFailed { required: usize },
    MemoryBudgetExceeded { required: usize, budget: usize },
    Cancelled,
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteResult { stage: &'static str },
}

impl fmt::Display for DiffuseExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("diffuse dimensions must be nonzero"),
            Self::DimensionsTooLarge => {
                formatter.write_str("diffuse dimensions exceed supported arithmetic")
            }
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "diffuse expected {expected} pixels, got {actual}"
                )
            }
            Self::InvalidScale => {
                formatter.write_str("diffuse requires finite positive ROI and piece scales")
            }
            Self::AllocationFailed { required } => {
                write!(formatter, "diffuse failed to allocate {required} bytes")
            }
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "diffuse needs {required} bytes, above {budget} byte budget"
                )
            }
            Self::Cancelled => formatter.write_str("diffuse execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "diffuse input pixel {pixel}, channel {channel} is non-finite"
                )
            }
            Self::NonFiniteResult { stage } => {
                write!(
                    formatter,
                    "diffuse produced a non-finite value during {stage}"
                )
            }
        }
    }
}

impl std::error::Error for DiffuseExecutionError {}

/// Immutable CPU plan for the native process/tiling scale pair.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiffusePlan {
    config: DiffuseConfig,
    dimensions: DiffuseDimensions,
    roi_in_scale: f32,
    piece_iscale: f32,
    scale: f32,
    final_radius: f32,
    scales: usize,
    memory_budget: usize,
}

impl DiffusePlan {
    pub fn new(
        config: DiffuseConfig,
        dimensions: DiffuseDimensions,
        roi_in_scale: f32,
        piece_iscale: f32,
    ) -> Result<Self, DiffuseExecutionError> {
        Self::new_with_budget(
            config,
            dimensions,
            roi_in_scale,
            piece_iscale,
            DIFFUSE_DEFAULT_MEMORY_BUDGET,
        )
    }

    pub fn new_with_budget(
        config: DiffuseConfig,
        dimensions: DiffuseDimensions,
        roi_in_scale: f32,
        piece_iscale: f32,
        memory_budget: usize,
    ) -> Result<Self, DiffuseExecutionError> {
        if !roi_in_scale.is_finite()
            || roi_in_scale <= 0.0
            || !piece_iscale.is_finite()
            || piece_iscale <= 0.0
        {
            return Err(DiffuseExecutionError::InvalidScale);
        }
        let scale = (piece_iscale / roi_in_scale).max(1.0);
        if !scale.is_finite() {
            return Err(DiffuseExecutionError::InvalidScale);
        }
        let final_radius =
            (config.parameters.radius + config.parameters.radius_center) as f32 * 2.0 / scale;
        let raw_scales = num_steps_to_reach_equivalent_sigma(B_SPLINE_SIGMA, final_radius);
        let scales = raw_scales.clamp(1, DIFFUSE_MAX_SCALES);
        let plan = Self {
            config,
            dimensions,
            roi_in_scale,
            piece_iscale,
            scale,
            final_radius,
            scales,
            memory_budget,
        };
        let required = plan.required_bytes()?;
        if required > memory_budget {
            return Err(DiffuseExecutionError::MemoryBudgetExceeded {
                required,
                budget: memory_budget,
            });
        }
        Ok(plan)
    }

    #[must_use]
    pub const fn dimensions(self) -> DiffuseDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn scales(self) -> usize {
        self.scales
    }

    #[must_use]
    pub const fn final_radius(self) -> f32 {
        self.final_radius
    }

    #[must_use]
    pub const fn scale(self) -> f32 {
        self.scale
    }

    #[must_use]
    pub const fn tiling(self) -> DiffuseTiling {
        let overlap = 1_u32 << self.scales;
        DiffuseTiling {
            factor: 6.25 + self.scales as f32,
            factor_cl: 6.25 + self.scales as f32,
            maxbuf: 1.0,
            maxbuf_cl: 1.0,
            overhead: 0,
            overlap,
            align: 1,
        }
    }

    fn required_bytes(self) -> Result<usize, DiffuseExecutionError> {
        let pixels = self.dimensions.pixel_count()?;
        let image_bytes = pixels
            .checked_mul(std::mem::size_of::<DiffusePixel>())
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
        let mask_bytes = pixels;
        let row_bytes = self
            .dimensions
            .width()
            .checked_mul(std::mem::size_of::<DiffusePixel>())
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
        let scratch_images = image_bytes
            .checked_mul(4 + self.scales)
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
        image_bytes
            .checked_add(scratch_images)
            .and_then(|bytes| bytes.checked_add(mask_bytes))
            .and_then(|bytes| bytes.checked_add(row_bytes))
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)
    }

    /// Executes the normal CPU path, publishing only a complete finite result.
    pub fn execute(
        &self,
        input: &[DiffusePixel],
    ) -> Result<Vec<DiffusePixel>, DiffuseExecutionError> {
        self.execute_with_cancel(input, false, || false)
    }

    /// Executes the native fast-mode disabled path, which is an input copy.
    pub fn execute_fast(
        &self,
        input: &[DiffusePixel],
    ) -> Result<Vec<DiffusePixel>, DiffuseExecutionError> {
        self.execute_with_cancel(input, true, || false)
    }

    /// Executes with row-bounded cancellation and no partial publication.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[DiffusePixel],
        fast_mode: bool,
        mut cancelled: F,
    ) -> Result<Vec<DiffusePixel>, DiffuseExecutionError> {
        let expected = self.dimensions.pixel_count()?;
        if input.len() != expected {
            return Err(DiffuseExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        validate_input(input, self.dimensions, &mut cancelled)?;
        if fast_mode {
            return try_clone(input);
        }

        let mut mask = try_bytes(expected)?;
        let mut temp1 = try_pixels(expected)?;
        let mut temp2 = try_pixels(expected)?;
        let mut low = [try_pixels(expected)?, try_pixels(expected)?];
        let mut high = Vec::new();
        high.try_reserve_exact(self.scales).map_err(|_| {
            DiffuseExecutionError::AllocationFailed {
                required: self.scales * std::mem::size_of::<Vec<DiffusePixel>>(),
            }
        })?;
        for _ in 0..self.scales {
            high.push(try_pixels(expected)?);
        }
        let mut output = try_pixels(expected)?;

        let has_mask = self.config.parameters.threshold > 0.0;
        if has_mask {
            inpaint::build_mask(
                input,
                &mut mask,
                self.config.parameters.threshold,
                self.dimensions,
            )?;
            inpaint::inpaint_mask(input, &mask, &mut temp1, self.dimensions, &mut cancelled)?;
        }
        let iterations = self.config.parameters.iterations.max(1) as usize;
        let anisotropy = self.config.anisotropy();
        let isotropy = self.config.isotropy();
        let regularization = self.config.regularization();
        let variance_threshold = self.config.variance_threshold();

        for iteration in 0..iterations {
            if cancelled() {
                return Err(DiffuseExecutionError::Cancelled);
            }
            let output_is_final = iteration + 1 == iterations;
            if output_is_final {
                let input_buffer = if iteration == 0 {
                    if has_mask { &temp1 } else { input }
                } else if iteration % 2 == 0 {
                    &temp1
                } else {
                    &temp2
                };
                self.wavelets_process(
                    input_buffer,
                    &mut output,
                    &mask,
                    has_mask,
                    &mut high,
                    &mut low,
                    &anisotropy,
                    isotropy,
                    regularization,
                    variance_threshold,
                    &mut cancelled,
                )?;
            } else if iteration % 2 == 0 {
                let input_buffer = if iteration == 0 {
                    if has_mask { &temp1 } else { input }
                } else {
                    &temp1
                };
                self.wavelets_process(
                    input_buffer,
                    &mut temp2,
                    &mask,
                    has_mask,
                    &mut high,
                    &mut low,
                    &anisotropy,
                    isotropy,
                    regularization,
                    variance_threshold,
                    &mut cancelled,
                )?;
            } else {
                self.wavelets_process(
                    &temp2,
                    &mut temp1,
                    &mask,
                    has_mask,
                    &mut high,
                    &mut low,
                    &anisotropy,
                    isotropy,
                    regularization,
                    variance_threshold,
                    &mut cancelled,
                )?;
            }
        }
        validate_result(&output, "final PDE output")?;
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn wavelets_process<F: FnMut() -> bool>(
        &self,
        input: &[DiffusePixel],
        reconstructed: &mut [DiffusePixel],
        mask: &[u8],
        has_mask: bool,
        high: &mut [Vec<DiffusePixel>],
        low: &mut [Vec<DiffusePixel>; 2],
        anisotropy: &[f32; 4],
        isotropy: [IsotropyMode; 4],
        regularization: f32,
        variance_threshold: f32,
        cancelled: &mut F,
    ) -> Result<(), DiffuseExecutionError> {
        for (scale, high_frequency) in high.iter_mut().enumerate() {
            let multiplier = 1_usize << scale;
            wavelets::decompose_2d_bspline(
                input,
                high_frequency,
                &mut low[scale % 2],
                self.dimensions,
                multiplier,
                &mut *cancelled,
            )?;
        }

        let residual_index = (self.scales - 1) % 2;
        let temporary_index = 1 - residual_index;
        for (count, scale) in (0..self.scales).rev().enumerate() {
            let multiplier = 1_usize << scale;
            let current_radius = equivalent_sigma_at_step(B_SPLINE_SIGMA, scale);
            let real_radius = current_radius * self.scale;
            let radius = self.config.parameters.radius as f32;
            let norm = (-(real_radius - self.config.parameters.radius_center as f32).powi(2)
                / radius.powi(2))
            .exp();
            if !norm.is_finite() {
                return Err(DiffuseExecutionError::NonFiniteResult {
                    stage: "scale normalization",
                });
            }
            let abcd = [
                self.config.parameters.first * DIFFUSE_KAPPA * norm,
                self.config.parameters.second * DIFFUSE_KAPPA * norm,
                self.config.parameters.third * DIFFUSE_KAPPA * norm,
                self.config.parameters.fourth * DIFFUSE_KAPPA * norm,
            ];
            let strength = self.config.parameters.sharpness * norm + 1.0;
            let input_index = if count == 0 {
                residual_index
            } else if !count.is_multiple_of(2) {
                temporary_index
            } else {
                residual_index
            };
            let output_index = if count == 0 {
                temporary_index
            } else if !count.is_multiple_of(2) {
                residual_index
            } else {
                temporary_index
            };
            let mask = has_mask.then_some(mask);
            if scale == 0 {
                pde::heat_pde_diffusion(
                    &high[scale],
                    &low[input_index],
                    mask,
                    reconstructed,
                    self.dimensions,
                    *anisotropy,
                    isotropy,
                    regularization,
                    variance_threshold,
                    current_radius.powi(2),
                    multiplier,
                    abcd,
                    strength,
                    &mut *cancelled,
                )?;
            } else {
                let (input_low, output_low) = two_low_buffers(low, input_index, output_index);
                pde::heat_pde_diffusion(
                    &high[scale],
                    input_low,
                    mask,
                    output_low,
                    self.dimensions,
                    *anisotropy,
                    isotropy,
                    regularization,
                    variance_threshold,
                    current_radius.powi(2),
                    multiplier,
                    abcd,
                    strength,
                    &mut *cancelled,
                )?;
            }
        }
        Ok(())
    }
}

fn two_low_buffers(
    low: &mut [Vec<DiffusePixel>; 2],
    input_index: usize,
    output_index: usize,
) -> (&[DiffusePixel], &mut [DiffusePixel]) {
    if input_index < output_index {
        let (left, right) = low.split_at_mut(output_index);
        (&left[input_index], &mut right[0])
    } else {
        let (left, right) = low.split_at_mut(input_index);
        (&right[0], &mut left[output_index])
    }
}

fn validate_input<F: FnMut() -> bool>(
    input: &[DiffusePixel],
    dimensions: DiffuseDimensions,
    cancelled: &mut F,
) -> Result<(), DiffuseExecutionError> {
    let width = dimensions.width();
    for (index, pixel) in input.iter().enumerate() {
        if index % width == 0 && cancelled() {
            return Err(DiffuseExecutionError::Cancelled);
        }
        for (channel, value) in pixel.channels().into_iter().enumerate() {
            if !value.is_finite() {
                return Err(DiffuseExecutionError::NonFiniteInput {
                    pixel: index,
                    channel,
                });
            }
        }
    }
    Ok(())
}

fn validate_result(
    input: &[DiffusePixel],
    stage: &'static str,
) -> Result<(), DiffuseExecutionError> {
    for pixel in input {
        if pixel.channels().into_iter().any(|value| !value.is_finite()) {
            return Err(DiffuseExecutionError::NonFiniteResult { stage });
        }
    }
    Ok(())
}

fn try_pixels(count: usize) -> Result<Vec<DiffusePixel>, DiffuseExecutionError> {
    let mut pixels = Vec::new();
    let required = count
        .checked_mul(std::mem::size_of::<DiffusePixel>())
        .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
    pixels
        .try_reserve_exact(count)
        .map_err(|_| DiffuseExecutionError::AllocationFailed { required })?;
    pixels.resize(count, DiffusePixel::from_channels([0.0; 4]));
    Ok(pixels)
}

fn try_bytes(count: usize) -> Result<Vec<u8>, DiffuseExecutionError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(count)
        .map_err(|_| DiffuseExecutionError::AllocationFailed { required: count })?;
    bytes.resize(count, 0);
    Ok(bytes)
}

fn try_clone(input: &[DiffusePixel]) -> Result<Vec<DiffusePixel>, DiffuseExecutionError> {
    let mut output = try_pixels(input.len())?;
    output.copy_from_slice(input);
    Ok(output)
}

fn check_f32(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), DiffuseParameterError> {
    if !value.is_finite() {
        return Err(DiffuseParameterError::NonFinite(name));
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(DiffuseParameterError::OutOfRange(name));
    }
    Ok(())
}

fn check_i32(
    name: &'static str,
    value: i32,
    minimum: i32,
    maximum: i32,
) -> Result<(), DiffuseParameterError> {
    if !(minimum..=maximum).contains(&value) {
        return Err(DiffuseParameterError::OutOfRange(name));
    }
    Ok(())
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated ABI range"),
    )
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated ABI range"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(parameters: DiffuseParametersV2, dimensions: DiffuseDimensions) -> DiffusePlan {
        DiffusePlan::new(
            DiffuseConfig::new(parameters).expect("parameters"),
            dimensions,
            1.0,
            1.0,
        )
        .expect("plan")
    }

    #[test]
    fn abi_and_v1_migration_append_radius_center() {
        let current = DiffuseParametersV2::defaults();
        let bytes = current.to_bytes();
        assert_eq!(bytes.len(), DIFFUSE_PARAMETER_BYTES);
        let decoded = DiffuseParametersV2::from_bytes(&bytes).expect("decode");
        assert_eq!(decoded, current);
        let legacy = DiffuseParametersV1 {
            iterations: 4,
            sharpness: -0.25,
            radius: 12,
            regularization: 2.0,
            variance_threshold: 0.25,
            anisotropy_first: 1.0,
            anisotropy_second: 0.0,
            anisotropy_third: -1.0,
            anisotropy_fourth: 2.0,
            threshold: 1.41,
            first: 0.1,
            second: 0.2,
            third: 0.3,
            fourth: 0.4,
        };
        let history = DiffuseHistory::decode(1, &legacy.to_bytes()).expect("legacy");
        assert_eq!(history.current().expect("migrate").radius_center, 0);
        assert_eq!(history.version(), 1);
    }

    #[test]
    fn zero_speed_positive_constant_reconstructs_without_blur_substitution() {
        let dimensions = DiffuseDimensions::new(5, 5).expect("dimensions");
        let input = vec![DiffusePixel::from_channels([0.4, 0.3, 0.2, 0.7]); 25];
        let plan = plan_with(DiffuseParametersV2::defaults(), dimensions);
        let output = plan.execute(&input).expect("diffuse");
        assert_eq!(output, input);
    }

    #[test]
    fn alpha_is_a_fourth_cpu_channel_and_mask_ignores_it() {
        let dimensions = DiffuseDimensions::new(3, 3).expect("dimensions");
        let mut parameters = DiffuseParametersV2::defaults();
        parameters.threshold = 0.5;
        let config = DiffuseConfig::new(parameters).expect("parameters");
        let plan = DiffusePlan::new(config, dimensions, 1.0, 1.0).expect("plan");
        let input = vec![DiffusePixel::from_channels([0.1, 0.1, 0.1, 0.8]); 9];
        let output = plan.execute(&input).expect("diffuse");
        assert_eq!(output, input);
    }

    #[test]
    fn fast_mode_and_cancellation_publish_only_complete_results() {
        let dimensions = DiffuseDimensions::new(8, 8).expect("dimensions");
        let plan = plan_with(DiffuseParametersV2::defaults(), dimensions);
        let input = vec![DiffusePixel::from_channels([0.4, 0.3, 0.2, 1.0]); 64];
        assert_eq!(plan.execute_fast(&input).expect("fast copy"), input);
        let mut checks = 0;
        let result = plan.execute_with_cancel(&input, false, || {
            checks += 1;
            checks > 2
        });
        assert_eq!(result, Err(DiffuseExecutionError::Cancelled));
    }

    #[test]
    fn nonfinite_input_is_rejected_before_allocation() {
        let dimensions = DiffuseDimensions::new(1, 1).expect("dimensions");
        let plan = plan_with(DiffuseParametersV2::defaults(), dimensions);
        let input = vec![DiffusePixel::from_channels([f32::NAN, 0.0, 0.0, 1.0])];
        assert_eq!(
            plan.execute(&input),
            Err(DiffuseExecutionError::NonFiniteInput {
                pixel: 0,
                channel: 0
            })
        );
    }

    #[test]
    fn tiling_matches_native_scale_formula() {
        let dimensions = DiffuseDimensions::new(32, 32).expect("dimensions");
        let mut parameters = DiffuseParametersV2::defaults();
        parameters.radius = 512;
        let plan = plan_with(parameters, dimensions);
        let tiling = plan.tiling();
        assert_eq!(tiling.factor, 6.25 + plan.scales() as f32);
        assert_eq!(tiling.overlap, 1_u32 << plan.scales());
        assert_eq!(tiling.align, 1);
    }
}
