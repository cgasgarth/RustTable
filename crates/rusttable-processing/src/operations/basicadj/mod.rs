#![expect(
    clippy::suboptimal_flops,
    reason = "Native Basic Adjust arithmetic order is preserved for IEEE-754 parity."
)]

//! Darktable's legacy `basicadj` composite adjustment operation.
//!
//! Source lineage: `src/iop/basicadj.c` and `src/common/rgb_norms.h`.
//!
//! The operation stays atomic because Darktable applies these stages in a
//! compatibility-sensitive order.  The current `RustTable` operation boundary
//! supplies deterministic point execution, so the auto-levels controls are
//! retained in the persisted configuration while automatic analysis resolves
//! one immutable plan before execution.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::large_stack_arrays,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::struct_field_names
)]

pub mod analysis;

use self::analysis::{BasicAdjAnalysisError, BasicAdjAnalysisRaster, BasicAdjAnalysisResult};
use super::common::OperationExecutionError;
use crate::{FiniteF32, LinearRgb, RgbChannel, WorkingFrameDescriptor};
use rusttable_color::rgb_to_xyz_matrix;
use sha2::{Digest, Sha256};
use std::fmt;

pub const BASICADJ_COMPATIBILITY_ID: &str = "basicadj";
pub const BASICADJ_SCHEMA_VERSION: u16 = 2;
const DEFAULT_MIDDLE_GREY: f32 = 18.42;
const BASICADJ_LUT_SIZE: usize = 0x10000;

/// Legacy controls that may be resolved by one deterministic full-image pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BasicAdjAutoControls(u8);

impl BasicAdjAutoControls {
    const BLACK_POINT: u8 = 1 << 0;
    const EXPOSURE: u8 = 1 << 1;
    const BRIGHTNESS: u8 = 1 << 2;
    const CONTRAST: u8 = 1 << 3;
    const HLCOMPR: u8 = 1 << 4;
    const HLCOMPRTHRESH: u8 = 1 << 5;

    #[must_use]
    pub const fn none() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn all() -> Self {
        Self(
            Self::BLACK_POINT
                | Self::EXPOSURE
                | Self::BRIGHTNESS
                | Self::CONTRAST
                | Self::HLCOMPR
                | Self::HLCOMPRTHRESH,
        )
    }

    #[must_use]
    pub const fn with_black_point(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::BLACK_POINT, enabled))
    }

    #[must_use]
    pub const fn with_exposure(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::EXPOSURE, enabled))
    }

    #[must_use]
    pub const fn with_brightness(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::BRIGHTNESS, enabled))
    }

    #[must_use]
    pub const fn with_contrast(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::CONTRAST, enabled))
    }

    #[must_use]
    pub const fn with_highlight_compression(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::HLCOMPR, enabled))
    }

    #[must_use]
    pub const fn with_highlight_threshold(self, enabled: bool) -> Self {
        Self(set_bit(self.0, Self::HLCOMPRTHRESH, enabled))
    }

    #[must_use]
    pub const fn black_point(self) -> bool {
        self.0 & Self::BLACK_POINT != 0
    }
    #[must_use]
    pub const fn exposure(self) -> bool {
        self.0 & Self::EXPOSURE != 0
    }
    #[must_use]
    pub const fn brightness(self) -> bool {
        self.0 & Self::BRIGHTNESS != 0
    }
    #[must_use]
    pub const fn contrast(self) -> bool {
        self.0 & Self::CONTRAST != 0
    }
    #[must_use]
    pub const fn highlight_compression(self) -> bool {
        self.0 & Self::HLCOMPR != 0
    }
    #[must_use]
    pub const fn highlight_threshold(self) -> bool {
        self.0 & Self::HLCOMPRTHRESH != 0
    }
    #[must_use]
    pub const fn is_active(self) -> bool {
        self.0 != 0
    }
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

const fn set_bit(bits: u8, bit: u8, enabled: bool) -> u8 {
    if enabled { bits | bit } else { bits & !bit }
}

/// Darktable's stable RGB norm IDs used by the color-preserving contrast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreserveColors {
    None,
    Luminance,
    Max,
    Average,
    Sum,
    Norm,
    Power,
}

impl PreserveColors {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Luminance => 1,
            Self::Max => 2,
            Self::Average => 3,
            Self::Sum => 4,
            Self::Norm => 5,
            Self::Power => 6,
        }
    }

    pub const fn from_id(id: i32) -> Result<Self, BasicAdjConfigError> {
        match id {
            0 => Ok(Self::None),
            1 => Ok(Self::Luminance),
            2 => Ok(Self::Max),
            3 => Ok(Self::Average),
            4 => Ok(Self::Sum),
            5 => Ok(Self::Norm),
            6 => Ok(Self::Power),
            value => Err(BasicAdjConfigError::UnknownPreserveColors(value)),
        }
    }
}

/// Working-profile evidence required by the native luminance norm.
///
/// Darktable supplies the current working profile's input matrix to
/// `dt_rgb_norm`. A `basicadj` execution must receive equivalent evidence rather
/// than silently falling back to camera coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicAdjNormEvidence {
    luminance_coefficients: [FiniteF32; 3],
    profile_identity: [u8; 32],
}

impl BasicAdjNormEvidence {
    /// Creates evidence from the Y row of the authoritative working RGB to XYZ
    /// matrix and its frame identity.
    pub fn new(
        luminance_coefficients: [f32; 3],
        profile_identity: [u8; 32],
    ) -> Result<Self, BasicAdjNormEvidenceError> {
        let luminance_coefficients = luminance_coefficients
            .map(|value| FiniteF32::new(value).map_err(|_| BasicAdjNormEvidenceError::NonFinite));
        let luminance_coefficients: [FiniteF32; 3] = luminance_coefficients
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| BasicAdjNormEvidenceError::InvalidCoefficients)?;
        if luminance_coefficients
            .iter()
            .all(|coefficient| coefficient.get() == 0.0)
        {
            return Err(BasicAdjNormEvidenceError::InvalidCoefficients);
        }
        Ok(Self {
            luminance_coefficients,
            profile_identity,
        })
    }

    /// Derives the matrix evidence from a typed working frame.
    pub fn from_working_frame(
        frame: WorkingFrameDescriptor,
    ) -> Result<Self, BasicAdjNormEvidenceError> {
        let primaries = frame.primaries();
        let matrix = rgb_to_xyz_matrix(
            [
                (primaries.red().0.get(), primaries.red().1.get()),
                (primaries.green().0.get(), primaries.green().1.get()),
                (primaries.blue().0.get(), primaries.blue().1.get()),
            ],
            primaries.white(),
        )
        .map_err(|_| BasicAdjNormEvidenceError::InvalidCoefficients)?;
        let rows = matrix.rows();
        Self::new([rows[3], rows[4], rows[5]], frame.identity())
    }

    #[must_use]
    pub const fn luminance_coefficients(self) -> [FiniteF32; 3] {
        self.luminance_coefficients
    }

    #[must_use]
    pub const fn profile_identity(self) -> [u8; 32] {
        self.profile_identity
    }

    #[must_use]
    pub fn luminance(self, values: [f32; 3]) -> f32 {
        values[0] * self.luminance_coefficients[0].get()
            + values[1] * self.luminance_coefficients[1].get()
            + values[2] * self.luminance_coefficients[2].get()
    }

    #[must_use]
    pub fn luminance_for(self, values: [f32; 3], mode: PreserveColors) -> f32 {
        match mode {
            PreserveColors::Luminance => self.luminance(values),
            PreserveColors::None | PreserveColors::Average => {
                (values[0] + values[1] + values[2]) / 3.0
            }
            PreserveColors::Max => values[0].max(values[1]).max(values[2]),
            PreserveColors::Sum => values[0] + values[1] + values[2],
            PreserveColors::Norm => {
                (values[0] * values[0] + values[1] * values[1] + values[2] * values[2]).sqrt()
            }
            PreserveColors::Power => {
                let squares = values.map(|value| value * value);
                (values[0] * squares[0] + values[1] * squares[1] + values[2] * squares[2])
                    / (squares[0] + squares[1] + squares[2])
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjNormEvidenceError {
    NonFinite,
    InvalidCoefficients,
}

impl fmt::Display for BasicAdjNormEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "basicadj working-profile norm coefficients are non-finite",
            Self::InvalidCoefficients => "basicadj working-profile norm coefficients are invalid",
        })
    }
}

impl std::error::Error for BasicAdjNormEvidenceError {}

/// RGBA sample used by the operation-local execution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicAdjRgba {
    rgb: LinearRgb,
    alpha: FiniteF32,
}

impl BasicAdjRgba {
    #[must_use]
    pub const fn new(rgb: LinearRgb, alpha: FiniteF32) -> Self {
        Self { rgb, alpha }
    }

    #[must_use]
    pub const fn rgb(self) -> LinearRgb {
        self.rgb
    }

    #[must_use]
    pub const fn alpha(self) -> FiniteF32 {
        self.alpha
    }
}

/// Version 1 of Darktable's persisted `basicadj` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV1 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: i32,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub clip: f32,
}

/// Version 2 added the independent vibrance control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV2 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: i32,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub clip: f32,
}

impl BasicAdjParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            black_point: 0.0,
            exposure: 0.0,
            hlcompr: 0.0,
            hlcomprthresh: 0.0,
            contrast: 0.0,
            preserve_colors: 1,
            middle_grey: DEFAULT_MIDDLE_GREY,
            brightness: 0.0,
            saturation: 0.0,
            clip: 0.0,
        }
    }
}

impl BasicAdjParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        BasicAdjParametersV1::defaults_v2()
    }
}

impl BasicAdjParametersV1 {
    const fn defaults_v2() -> BasicAdjParametersV2 {
        BasicAdjParametersV2 {
            black_point: 0.0,
            exposure: 0.0,
            hlcompr: 0.0,
            hlcomprthresh: 0.0,
            contrast: 0.0,
            preserve_colors: 1,
            middle_grey: DEFAULT_MIDDLE_GREY,
            brightness: 0.0,
            saturation: 0.0,
            vibrance: 0.0,
            clip: 0.0,
        }
    }
}

#[must_use]
pub const fn migrate_v1_to_v2(value: BasicAdjParametersV1) -> BasicAdjParametersV2 {
    BasicAdjParametersV2 {
        black_point: value.black_point,
        exposure: value.exposure,
        hlcompr: value.hlcompr,
        hlcomprthresh: value.hlcomprthresh,
        contrast: value.contrast,
        preserve_colors: value.preserve_colors,
        middle_grey: value.middle_grey,
        brightness: value.brightness,
        saturation: value.saturation,
        vibrance: 0.0,
        clip: value.clip,
    }
}

/// Checked, immutable configuration for one legacy `basicadj` history node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicAdjConfig {
    black_point: FiniteF32,
    exposure: FiniteF32,
    hlcompr: FiniteF32,
    hlcomprthresh: FiniteF32,
    contrast: FiniteF32,
    preserve_colors: PreserveColors,
    middle_grey: FiniteF32,
    brightness: FiniteF32,
    saturation: FiniteF32,
    vibrance: FiniteF32,
    clip: FiniteF32,
    auto_controls: BasicAdjAutoControls,
}

impl BasicAdjConfig {
    pub fn new(value: BasicAdjParametersV2) -> Result<Self, BasicAdjConfigError> {
        Ok(Self {
            black_point: bounded("black_point", value.black_point, -1.0, 1.0)?,
            exposure: bounded("exposure", value.exposure, -18.0, 18.0)?,
            hlcompr: bounded("hlcompr", value.hlcompr, 0.0, 500.0)?,
            hlcomprthresh: bounded("hlcomprthresh", value.hlcomprthresh, 0.0, 100.0)?,
            contrast: bounded("contrast", value.contrast, -1.0, 5.0)?,
            preserve_colors: PreserveColors::from_id(value.preserve_colors)?,
            middle_grey: bounded("middle_grey", value.middle_grey, 0.05, 100.0)?,
            brightness: bounded("brightness", value.brightness, -4.0, 4.0)?,
            saturation: bounded("saturation", value.saturation, -1.0, 1.0)?,
            vibrance: bounded("vibrance", value.vibrance, -1.0, 1.0)?,
            clip: bounded("clip", value.clip, -1.0, 1.0)?,
            auto_controls: BasicAdjAutoControls::none(),
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(BasicAdjParametersV2::defaults()).expect("basicadj defaults are valid")
    }

    #[must_use]
    pub const fn black_point(self) -> f32 {
        self.black_point.get()
    }
    #[must_use]
    pub const fn exposure(self) -> f32 {
        self.exposure.get()
    }
    #[must_use]
    pub const fn hlcompr(self) -> f32 {
        self.hlcompr.get()
    }
    #[must_use]
    pub const fn hlcomprthresh(self) -> f32 {
        self.hlcomprthresh.get()
    }
    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast.get()
    }
    #[must_use]
    pub const fn preserve_colors(self) -> PreserveColors {
        self.preserve_colors
    }
    #[must_use]
    pub const fn middle_grey(self) -> f32 {
        self.middle_grey.get()
    }
    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.brightness.get()
    }
    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }
    #[must_use]
    pub const fn vibrance(self) -> f32 {
        self.vibrance.get()
    }
    #[must_use]
    pub const fn clip(self) -> f32 {
        self.clip.get()
    }

    /// Enables selected legacy automatic controls for the next immutable plan.
    #[must_use]
    pub const fn with_auto_controls(mut self, controls: BasicAdjAutoControls) -> Self {
        self.auto_controls = controls;
        self
    }

    #[must_use]
    pub const fn auto_controls(self) -> BasicAdjAutoControls {
        self.auto_controls
    }

    pub(crate) const fn with_resolved_analysis(self, result: &BasicAdjAnalysisResult) -> Self {
        let controls = self.auto_controls;
        let values = result.resolved_values();
        Self {
            black_point: if controls.black_point() {
                FiniteF32::from_proven_finite(values.black_point())
            } else {
                self.black_point
            },
            exposure: if controls.exposure() {
                FiniteF32::from_proven_finite(values.exposure())
            } else {
                self.exposure
            },
            hlcompr: if controls.highlight_compression() {
                FiniteF32::from_proven_finite(values.hlcompr())
            } else {
                self.hlcompr
            },
            hlcomprthresh: if controls.highlight_threshold() {
                FiniteF32::from_proven_finite(values.hlcomprthresh())
            } else {
                self.hlcomprthresh
            },
            contrast: if controls.contrast() {
                FiniteF32::from_proven_finite(values.contrast())
            } else {
                self.contrast
            },
            middle_grey: self.middle_grey,
            brightness: if controls.brightness() {
                FiniteF32::from_proven_finite(values.brightness())
            } else {
                self.brightness
            },
            saturation: self.saturation,
            vibrance: self.vibrance,
            clip: self.clip,
            preserve_colors: self.preserve_colors,
            auto_controls: BasicAdjAutoControls::none(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjConfigError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
    UnknownPreserveColors(i32),
}

impl fmt::Display for BasicAdjConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "basicadj {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "basicadj {name} is outside its range"),
            Self::UnknownPreserveColors(value) => {
                write!(
                    formatter,
                    "basicadj preserve-colors mode {value} is unknown"
                )
            }
        }
    }
}

impl std::error::Error for BasicAdjConfigError {}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, BasicAdjConfigError> {
    let value = FiniteF32::new(value).map_err(|_| BasicAdjConfigError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(BasicAdjConfigError::OutOfRange(name));
    }
    Ok(value)
}

/// Immutable derived point-operation state.  The stage sequence is frozen in
/// `apply_pixel` and the identity covers both controls and derived constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicAdjPlan {
    config: BasicAdjConfig,
    scale: FiniteF32,
    gamma: FiniteF32,
    middle_grey: FiniteF32,
    contrast: FiniteF32,
    hlcomp: FiniteF32,
    hlrange: FiniteF32,
    lut_gamma: Box<[FiniteF32; BASICADJ_LUT_SIZE]>,
    lut_contrast: Box<[FiniteF32; BASICADJ_LUT_SIZE]>,
    analysis_identity: [u8; 32],
    identity: [u8; 32],
}

impl BasicAdjPlan {
    pub fn new(config: BasicAdjConfig) -> Result<Self, BasicAdjPlanError> {
        Self::new_with_analysis(config, [0; 32])
    }

    fn new_with_analysis(
        config: BasicAdjConfig,
        analysis_identity: [u8; 32],
    ) -> Result<Self, BasicAdjPlanError> {
        let white = (-config.exposure()).exp2();
        let denominator = white - config.black_point();
        let scale = FiniteF32::new(1.0 / denominator)
            .map_err(|_| BasicAdjPlanError::InvalidExposureScale)?;
        let middle_grey = if config.middle_grey() > 0.0 {
            config.middle_grey() / 100.0
        } else {
            0.1842
        };
        let middle_grey = FiniteF32::new(middle_grey)
            .map_err(|_| BasicAdjPlanError::InvalidDerivedValue("middle_grey"))?;
        let brightness = config.brightness() * 2.0;
        let gamma = if brightness >= 0.0 {
            1.0 / (1.0 + brightness)
        } else {
            1.0 - brightness
        };
        let gamma =
            FiniteF32::new(gamma).map_err(|_| BasicAdjPlanError::InvalidDerivedValue("gamma"))?;
        let hlcomp = FiniteF32::from_proven_finite(config.hlcompr() / 100.0);
        let shoulder = config.hlcomprthresh() / 800.0 + 0.1;
        let hlrange = FiniteF32::new(1.0 - shoulder)
            .map_err(|_| BasicAdjPlanError::InvalidDerivedValue("highlight range"))?;
        let contrast = FiniteF32::from_proven_finite(config.contrast() + 1.0);
        let mut lut_gamma: Box<[FiniteF32; BASICADJ_LUT_SIZE]> =
            vec![FiniteF32::from_proven_finite(0.0); BASICADJ_LUT_SIZE]
                .into_boxed_slice()
                .try_into()
                .expect("basicadj gamma LUT has its fixed native size");
        let mut lut_contrast: Box<[FiniteF32; BASICADJ_LUT_SIZE]> =
            vec![FiniteF32::from_proven_finite(0.0); BASICADJ_LUT_SIZE]
                .into_boxed_slice()
                .try_into()
                .expect("basicadj contrast LUT has its fixed native size");
        let process_gamma = config.brightness() != 0.0;
        let plain_contrast =
            config.preserve_colors() == PreserveColors::None && config.contrast() != 0.0;
        if process_gamma || plain_contrast {
            for index in 0..BASICADJ_LUT_SIZE {
                let percentage = index as f32 / BASICADJ_LUT_SIZE as f32;
                if process_gamma {
                    lut_gamma[index] = FiniteF32::from_proven_finite(percentage.powf(gamma.get()));
                }
                if plain_contrast {
                    lut_contrast[index] = FiniteF32::from_proven_finite(
                        (percentage / middle_grey.get()).powf(contrast.get()) * middle_grey.get(),
                    );
                }
            }
        }
        let identity = plan_identity(
            &config,
            scale,
            gamma,
            middle_grey,
            contrast,
            hlcomp,
            hlrange,
            config.auto_controls(),
            analysis_identity,
        );
        Ok(Self {
            config,
            scale,
            gamma,
            middle_grey,
            contrast,
            hlcomp,
            hlrange,
            lut_gamma,
            lut_contrast,
            analysis_identity,
            identity,
        })
    }

    /// Resolves automatic controls once against a full analysis raster.
    pub fn resolve(
        config: BasicAdjConfig,
        raster: BasicAdjAnalysisRaster<'_>,
    ) -> Result<Self, BasicAdjAnalysisError> {
        if !config.auto_controls().is_active() {
            return Self::new(config).map_err(BasicAdjAnalysisError::Plan);
        }
        let result = analysis::BasicAdjAnalysisPlan::analyze(config, raster)?;
        let resolved = config.with_resolved_analysis(&result);
        Self::new_with_analysis(resolved, result.identity()).map_err(BasicAdjAnalysisError::Plan)
    }

    /// Resolves automatic controls and checks the supplied cancellation hook
    /// at deterministic row boundaries.
    pub fn resolve_with_cancellation(
        config: BasicAdjConfig,
        raster: BasicAdjAnalysisRaster<'_>,
        should_cancel: impl Fn() -> bool,
    ) -> Result<Self, BasicAdjAnalysisError> {
        if !config.auto_controls().is_active() {
            return Self::new(config).map_err(BasicAdjAnalysisError::Plan);
        }
        let result = analysis::BasicAdjAnalysisPlan::analyze_with_cancellation(
            config,
            raster,
            should_cancel,
        )?;
        let resolved = config.with_resolved_analysis(&result);
        Self::new_with_analysis(resolved, result.identity()).map_err(BasicAdjAnalysisError::Plan)
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn analysis_identity(&self) -> [u8; 32] {
        self.analysis_identity
    }

    #[must_use]
    pub const fn gpu_parameters(&self) -> BasicAdjGpuParameters {
        BasicAdjGpuParameters {
            black_point: self.config.black_point(),
            scale: self.scale.get(),
            gamma: self.gamma.get(),
            middle_grey: self.middle_grey.get(),
            contrast: self.contrast.get(),
            hlcomp: self.hlcomp.get(),
            hlrange: self.hlrange.get(),
            preserve_colors: self.config.preserve_colors().id(),
            saturation: self.config.saturation(),
            vibrance: self.config.vibrance(),
        }
    }

    /// Produces immutable execution evidence for cache/publication owners.
    #[must_use]
    pub const fn receipt(&self) -> BasicAdjExecutionReceipt {
        BasicAdjExecutionReceipt {
            plan_identity: self.identity,
            analysis_identity: self.analysis_identity,
            profile_identity: [0; 32],
        }
    }

    /// Produces a profile-qualified execution receipt. A plan may only be
    /// reused across frames when this evidence is also unchanged.
    #[must_use]
    pub const fn receipt_with_norm_evidence(
        &self,
        evidence: BasicAdjNormEvidence,
    ) -> BasicAdjExecutionReceipt {
        BasicAdjExecutionReceipt {
            plan_identity: self.identity,
            analysis_identity: self.analysis_identity,
            profile_identity: evidence.profile_identity(),
        }
    }

    #[must_use]
    pub const fn config(&self) -> BasicAdjConfig {
        self.config
    }

    /// Executes against the default sRGB linear working-frame contract.
    ///
    /// Callers processing another working frame must use
    /// [`Self::execute_with_working_frame`] so its profile-derived luminance is
    /// carried explicitly. This default keeps the legacy evaluator boundary
    /// source-faithful without falling back to camera coefficients.
    pub fn execute(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let evidence = if self.requires_luminance_norm() {
            Some(
                BasicAdjNormEvidence::from_working_frame(WorkingFrameDescriptor::srgb()).map_err(
                    |_| {
                        OperationExecutionError::UnsupportedCapability(
                            "basicadj requires valid default working-frame norm evidence",
                        )
                    },
                )?,
            )
        } else {
            None
        };
        self.execute_inner(input, pixel_index_offset, evidence.as_ref(), || false)
    }

    /// Executes a profile-independent plan with bounded cancellation polling.
    pub fn execute_with_cancellation<C: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        should_cancel: C,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_inner(input, pixel_index_offset, None, should_cancel)
    }

    /// Executes with working-frame norm evidence and bounded cancellation polling.
    pub fn execute_with_working_frame_and_cancellation<C: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        frame: WorkingFrameDescriptor,
        should_cancel: C,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let evidence = BasicAdjNormEvidence::from_working_frame(frame).map_err(|_| {
            OperationExecutionError::UnsupportedCapability(
                "basicadj requires valid working-profile norm evidence",
            )
        })?;
        self.execute_inner(input, pixel_index_offset, Some(&evidence), should_cancel)
    }

    /// Executes with the authoritative working-frame norm/profile evidence.
    pub fn execute_with_norm_evidence(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        evidence: &BasicAdjNormEvidence,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_inner(input, pixel_index_offset, Some(evidence), || false)
    }

    /// Derives operation-local norm evidence from the typed working frame.
    pub fn execute_with_working_frame(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        frame: WorkingFrameDescriptor,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let evidence = BasicAdjNormEvidence::from_working_frame(frame).map_err(|_| {
            OperationExecutionError::UnsupportedCapability(
                "basicadj requires valid working-profile norm evidence",
            )
        })?;
        self.execute_with_norm_evidence(input, pixel_index_offset, &evidence)
    }

    /// Executes with bounded cancellation polling. A candidate is published
    /// only after the complete input has been checked for finite output.
    pub fn execute_with_norm_evidence_and_cancellation<C: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        evidence: &BasicAdjNormEvidence,
        should_cancel: C,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_inner(input, pixel_index_offset, Some(evidence), should_cancel)
    }

    /// Executes RGBA samples while preserving alpha bit-for-bit through the
    /// operation-local candidate/reconstruction boundary.
    pub fn execute_rgba_with_working_frame(
        &self,
        input: &[BasicAdjRgba],
        pixel_index_offset: usize,
        frame: WorkingFrameDescriptor,
    ) -> Result<Vec<BasicAdjRgba>, OperationExecutionError> {
        let evidence = BasicAdjNormEvidence::from_working_frame(frame).map_err(|_| {
            OperationExecutionError::UnsupportedCapability(
                "basicadj requires valid working-profile norm evidence",
            )
        })?;
        self.validate_norm_evidence(Some(&evidence), input.is_empty())?;
        let mut output = Vec::with_capacity(input.len());
        for (index, sample) in input.iter().copied().enumerate() {
            let values = self.apply_pixel(sample.rgb, Some(&evidence))?;
            output.push(BasicAdjRgba::new(
                LinearRgb::new(
                    checked(values[0], pixel_index_offset + index, RgbChannel::Red)?,
                    checked(values[1], pixel_index_offset + index, RgbChannel::Green)?,
                    checked(values[2], pixel_index_offset + index, RgbChannel::Blue)?,
                ),
                sample.alpha,
            ));
        }
        Ok(output)
    }

    fn execute_inner<C: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        evidence: Option<&BasicAdjNormEvidence>,
        should_cancel: C,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.validate_norm_evidence(evidence, input.is_empty())?;
        if should_cancel() {
            return Err(OperationExecutionError::Cancelled);
        }
        let mut output = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if index != 0 && index.is_multiple_of(1024) && should_cancel() {
                return Err(OperationExecutionError::Cancelled);
            }
            let values = self.apply_pixel(pixel, evidence)?;
            output.push(LinearRgb::new(
                checked(values[0], pixel_index_offset + index, RgbChannel::Red)?,
                checked(values[1], pixel_index_offset + index, RgbChannel::Green)?,
                checked(values[2], pixel_index_offset + index, RgbChannel::Blue)?,
            ));
        }
        if should_cancel() {
            return Err(OperationExecutionError::Cancelled);
        }
        Ok(output)
    }

    fn validate_norm_evidence(
        &self,
        evidence: Option<&BasicAdjNormEvidence>,
        empty_input: bool,
    ) -> Result<(), OperationExecutionError> {
        if empty_input || !self.requires_luminance_norm() || evidence.is_some() {
            Ok(())
        } else {
            Err(OperationExecutionError::UnsupportedCapability(
                "basicadj requires working-frame norm evidence",
            ))
        }
    }

    fn requires_luminance_norm(&self) -> bool {
        self.config.hlcompr() > 0.0
            || (self.config.contrast() != 0.0
                && self.config.preserve_colors() == PreserveColors::Luminance)
    }

    fn apply_pixel(
        &self,
        pixel: LinearRgb,
        evidence: Option<&BasicAdjNormEvidence>,
    ) -> Result<[f32; 3], OperationExecutionError> {
        let mut values = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
        let black = self.config.black_point();
        for value in &mut values {
            *value = (*value - black) * self.scale.get();
        }

        if self.config.hlcompr() > 0.0 {
            let luminance = evidence
                .ok_or(OperationExecutionError::UnsupportedCapability(
                    "basicadj requires working-frame norm evidence",
                ))?
                .luminance(values);
            if luminance > 0.0 {
                let ratio = hlcurve(luminance, self.hlcomp.get(), self.hlrange.get());
                for value in &mut values {
                    *value *= ratio;
                }
            }
        }

        if self.config.brightness() != 0.0 {
            for value in &mut values {
                if *value > 0.0 {
                    *value = get_lut_gamma(*value, self.gamma.get(), &self.lut_gamma);
                }
            }
        }

        if self.config.preserve_colors() == PreserveColors::None && self.config.contrast() != 0.0 {
            for value in &mut values {
                if *value > 0.0 {
                    *value = get_lut_contrast(
                        *value,
                        self.contrast.get(),
                        self.middle_grey.get(),
                        &self.lut_contrast,
                    );
                }
            }
        } else if self.config.preserve_colors() != PreserveColors::None
            && self.config.contrast() != 0.0
        {
            let luminance = if self.config.preserve_colors() == PreserveColors::Luminance {
                evidence
                    .ok_or(OperationExecutionError::UnsupportedCapability(
                        "basicadj requires working-frame norm evidence",
                    ))?
                    .luminance(values)
            } else {
                norm_without_profile(values, self.config.preserve_colors())
            };
            if luminance > 0.0 {
                let contrast_luminance = (luminance / self.middle_grey.get())
                    .powf(self.contrast.get())
                    * self.middle_grey.get();
                let ratio = contrast_luminance / luminance;
                for value in &mut values {
                    *value *= ratio;
                }
            }
        }

        if self.config.saturation() != 0.0 || self.config.vibrance() != 0.0 {
            let average = (values[0] + values[1] + values[2]) / 3.0;
            let delta = ((average - values[0]) * (average - values[0])
                + (average - values[1]) * (average - values[1])
                + (average - values[2]) * (average - values[2]))
                .sqrt();
            let vibrance = self.config.vibrance() / 1.4;
            let boost = vibrance * (1.0 - delta.powf(vibrance.abs()));
            let factor = self.config.saturation() + 1.0 + boost;
            for value in &mut values {
                *value = average + factor * (*value - average);
            }
        }
        Ok(values)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjPlanError {
    InvalidExposureScale,
    InvalidDerivedValue(&'static str),
}

impl fmt::Display for BasicAdjPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExposureScale => {
                formatter.write_str("basicadj exposure scale is non-finite")
            }
            Self::InvalidDerivedValue(name) => {
                write!(formatter, "basicadj derived {name} is invalid")
            }
        }
    }
}
impl std::error::Error for BasicAdjPlanError {}

/// Scalar parameters consumed by the atomic basicadj WGPU point stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjGpuParameters {
    pub black_point: f32,
    pub scale: f32,
    pub gamma: f32,
    pub middle_grey: f32,
    pub contrast: f32,
    pub hlcomp: f32,
    pub hlrange: f32,
    pub preserve_colors: i32,
    pub saturation: f32,
    pub vibrance: f32,
}

/// Stable execution evidence for one resolved basicadj plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicAdjExecutionReceipt {
    plan_identity: [u8; 32],
    analysis_identity: [u8; 32],
    profile_identity: [u8; 32],
}

impl BasicAdjExecutionReceipt {
    #[must_use]
    pub const fn plan_identity(self) -> [u8; 32] {
        self.plan_identity
    }

    #[must_use]
    pub const fn analysis_identity(self) -> [u8; 32] {
        self.analysis_identity
    }

    #[must_use]
    pub const fn profile_identity(self) -> [u8; 32] {
        self.profile_identity
    }
}

/// Descriptive alias used by cache and pixelpipe callers.
pub type BasicAdjustmentsPlan = BasicAdjPlan;

fn norm_without_profile(values: [f32; 3], mode: PreserveColors) -> f32 {
    match mode {
        PreserveColors::None | PreserveColors::Average => (values[0] + values[1] + values[2]) / 3.0,
        PreserveColors::Max => values[0].max(values[1]).max(values[2]),
        PreserveColors::Sum => values[0] + values[1] + values[2],
        PreserveColors::Norm => {
            (values[0] * values[0] + values[1] * values[1] + values[2] * values[2]).sqrt()
        }
        PreserveColors::Power => {
            let squares = values.map(|value| value * value);
            (values[0] * squares[0] + values[1] * squares[1] + values[2] * squares[2])
                / (squares[0] + squares[1] + squares[2])
        }
        PreserveColors::Luminance => unreachable!("profile luminance is handled by evidence"),
    }
}

fn checked(
    value: f32,
    pixel: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

fn get_lut_gamma(value: f32, gamma: f32, lut: &[FiniteF32; BASICADJ_LUT_SIZE]) -> f32 {
    if value > 1.0 {
        value.powf(gamma)
    } else {
        let index = ((value * BASICADJ_LUT_SIZE as f32) as isize).clamp(0, 0xffff) as usize;
        lut[index].get()
    }
}

fn get_lut_contrast(
    value: f32,
    contrast: f32,
    middle_grey: f32,
    lut: &[FiniteF32; BASICADJ_LUT_SIZE],
) -> f32 {
    if value > 1.0 {
        (value / middle_grey).powf(contrast) * middle_grey
    } else {
        let index = ((value * BASICADJ_LUT_SIZE as f32) as isize).clamp(0, 0xffff) as usize;
        lut[index].get()
    }
}

fn hlcurve(level: f32, hlcomp: f32, hlrange: f32) -> f32 {
    if hlcomp <= 0.0 {
        return 1.0;
    }
    let mut value = level + (hlrange - 1.0);
    if value == 0.0 {
        value = 0.000_001;
    }
    let mut y = value / hlrange * hlcomp;
    if y <= -1.0 {
        y = -0.999_999;
    }
    let ratio = hlrange / (value * hlcomp);
    y.ln_1p() * ratio
}

#[expect(
    clippy::too_many_arguments,
    reason = "the cache identity preserves the native parameter and derived-plan field order"
)]
fn plan_identity(
    config: &BasicAdjConfig,
    scale: FiniteF32,
    gamma: FiniteF32,
    middle_grey: FiniteF32,
    contrast: FiniteF32,
    hlcomp: FiniteF32,
    hlrange: FiniteF32,
    auto_controls: BasicAdjAutoControls,
    analysis_identity: [u8; 32],
) -> [u8; 32] {
    let fields = [
        config.black_point(),
        config.exposure(),
        config.hlcompr(),
        config.hlcomprthresh(),
        config.contrast(),
        config.middle_grey(),
        config.brightness(),
        config.saturation(),
        config.vibrance(),
        config.clip(),
        scale.get(),
        gamma.get(),
        middle_grey.get(),
        contrast.get(),
        hlcomp.get(),
        hlrange.get(),
    ];
    let mut hasher = Sha256::new();
    hasher.update(BASICADJ_SCHEMA_VERSION.to_le_bytes());
    hasher.update(config.preserve_colors().id().to_le_bytes());
    hasher.update([auto_controls.bits()]);
    hasher.update(analysis_identity);
    for field in fields {
        hasher.update(field.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_migration_adds_neutral_vibrance() {
        assert_eq!(
            migrate_v1_to_v2(BasicAdjParametersV1::defaults())
                .vibrance
                .to_bits(),
            0.0_f32.to_bits()
        );
    }

    #[test]
    fn default_plan_is_identity_and_deterministic() {
        let plan = BasicAdjPlan::new(BasicAdjConfig::defaults()).expect("defaults");
        let pixel = LinearRgb::new(
            FiniteF32::new(0.2).expect("finite"),
            FiniteF32::new(0.4).expect("finite"),
            FiniteF32::new(0.6).expect("finite"),
        );
        assert_eq!(plan.execute(&[pixel], 0).expect("execution"), vec![pixel]);
        assert_eq!(
            plan.identity(),
            BasicAdjPlan::new(BasicAdjConfig::defaults())
                .expect("defaults")
                .identity()
        );
    }

    #[test]
    fn preserve_colors_changes_only_contrast_luminance() {
        let mut parameters = BasicAdjParametersV2::defaults();
        parameters.contrast = 1.0;
        let plan =
            BasicAdjPlan::new(BasicAdjConfig::new(parameters).expect("parameters")).expect("plan");
        let pixel = LinearRgb::new(
            FiniteF32::new(0.1).expect("finite"),
            FiniteF32::new(0.3).expect("finite"),
            FiniteF32::new(0.6).expect("finite"),
        );
        let output = plan
            .execute_with_working_frame(&[pixel], 0, WorkingFrameDescriptor::srgb())
            .expect("execution")[0];
        let ratio = output.red().get() / pixel.red().get();
        assert!((output.green().get() / pixel.green().get() - ratio).abs() < 0.000_01);
        assert!((output.blue().get() / pixel.blue().get() - ratio).abs() < 0.000_01);
    }
}
