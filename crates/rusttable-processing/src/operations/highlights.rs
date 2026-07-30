//! Highlight reconstruction, mapped from Darktable's `src/iop/highlights.c`.
//!
//! The scalar path is deliberately the reference implementation.  It freezes
//! the clipped/candidate masks before replacement, uses row-major tie breaks,
//! and returns diagnostics and a content receipt together with the raster.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::ignored_unit_patterns,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_arguments
)]

use std::fmt;

use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics,
    ReconstructionReceipt, apply_opacity, checked_bytes, chroma, from_luma_chroma, luma,
    neighborhood, validate_shape,
};

pub const HIGHLIGHTS_COMPATIBILITY_ID: &str = "highlights";
pub const HIGHLIGHTS_SCHEMA_VERSION: u16 = 4;

/// Darktable's stable method IDs from `dt_iop_highlights_mode_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightsMethod {
    Clip,
    ReconstructLCh,
    ReconstructColor,
    GuidedLaplacians,
    SegmentationBased,
    InpaintOpposed,
}

impl HighlightsMethod {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::Clip => 0,
            Self::ReconstructLCh => 1,
            Self::ReconstructColor => 2,
            Self::GuidedLaplacians => 3,
            Self::SegmentationBased => 4,
            Self::InpaintOpposed => 5,
        }
    }

    /// Decodes the upstream enum without silently substituting a method.
    pub fn from_id(id: i32) -> Result<Self, HighlightsParameterError> {
        match id {
            0 => Ok(Self::Clip),
            1 => Ok(Self::ReconstructLCh),
            2 => Ok(Self::ReconstructColor),
            3 => Ok(Self::GuidedLaplacians),
            4 => Ok(Self::SegmentationBased),
            5 => Ok(Self::InpaintOpposed),
            _ => Err(HighlightsParameterError::UnknownMethod(id)),
        }
    }

    const fn clip_magic(self) -> f32 {
        // Exact order in Darktable's registered six-entry table.
        [1.0, 1.0, 0.987, 0.995, 0.987, 0.987][self.id() as usize]
    }
}

/// Darktable's stable recovery mode IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryMode {
    Off,
    Small,
    Large,
    FlatSmall,
    FlatLarge,
    Generic,
    FlatGeneric,
}

impl RecoveryMode {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::Small => 1,
            Self::Large => 2,
            Self::FlatSmall => 3,
            Self::FlatLarge => 4,
            Self::Generic => 5,
            Self::FlatGeneric => 6,
        }
    }

    pub fn from_id(id: i32) -> Result<Self, HighlightsParameterError> {
        match id {
            0 => Ok(Self::Off),
            1 => Ok(Self::Small),
            2 => Ok(Self::Large),
            3 => Ok(Self::FlatSmall),
            4 => Ok(Self::FlatLarge),
            5 => Ok(Self::Generic),
            6 => Ok(Self::FlatGeneric),
            _ => Err(HighlightsParameterError::UnknownRecovery(id)),
        }
    }
}

/// The twelve exact wavelet diameter choices registered by Darktable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaveletScale(u8);

impl WaveletScale {
    pub fn new(id: u8) -> Result<Self, HighlightsParameterError> {
        if id < 12 {
            Ok(Self(id))
        } else {
            Err(HighlightsParameterError::InvalidScale(id))
        }
    }

    #[must_use]
    pub const fn id(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn diameter(self) -> u32 {
        2_u32 << self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HighlightsConfig {
    method: HighlightsMethod,
    strength: FiniteF32,
    clip: FiniteF32,
    noise_level: FiniteF32,
    iterations: u16,
    scales: WaveletScale,
    candidating: FiniteF32,
    combine: FiniteF32,
    recovery: RecoveryMode,
    solid_color: FiniteF32,
}

impl HighlightsConfig {
    /// Builds the canonical version-4 configuration with checked ranges.
    pub fn new(
        method: HighlightsMethod,
        strength: f32,
        clip: f32,
        noise_level: f32,
        iterations: u16,
        scales: WaveletScale,
        candidating: f32,
        combine: f32,
        recovery: RecoveryMode,
        solid_color: f32,
    ) -> Result<Self, HighlightsParameterError> {
        let strength = bounded("strength", strength, 0.0, 1.0)?;
        let clip = bounded("clip", clip, 0.0, 2.0)?;
        let noise_level = bounded("noise_level", noise_level, 0.0, 0.5)?;
        let candidating = bounded("candidating", candidating, 0.0, 1.0)?;
        let combine = bounded("combine", combine, 0.0, 8.0)?;
        let solid_color = bounded("solid_color", solid_color, 0.0, 1.0)?;
        if !(1..=256).contains(&iterations) {
            return Err(HighlightsParameterError::InvalidIterations(iterations));
        }
        Ok(Self {
            method,
            strength,
            clip,
            noise_level,
            iterations,
            scales,
            candidating,
            combine,
            recovery,
            solid_color,
        })
    }

    #[must_use]
    pub const fn method(self) -> HighlightsMethod {
        self.method
    }
    #[must_use]
    pub const fn strength(self) -> FiniteF32 {
        self.strength
    }
    #[must_use]
    pub const fn clip(self) -> FiniteF32 {
        self.clip
    }
    #[must_use]
    pub const fn noise_level(self) -> FiniteF32 {
        self.noise_level
    }
    #[must_use]
    pub const fn iterations(self) -> u16 {
        self.iterations
    }
    #[must_use]
    pub const fn scales(self) -> WaveletScale {
        self.scales
    }
    #[must_use]
    pub const fn candidating(self) -> FiniteF32 {
        self.candidating
    }
    #[must_use]
    pub const fn combine(self) -> FiniteF32 {
        self.combine
    }
    #[must_use]
    pub const fn recovery(self) -> RecoveryMode {
        self.recovery
    }
    #[must_use]
    pub const fn solid_color(self) -> FiniteF32 {
        self.solid_color
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HighlightsParameterError {
    UnknownMethod(i32),
    UnknownRecovery(i32),
    InvalidScale(u8),
    InvalidIterations(u16),
    OutOfRange { name: &'static str, value: u32 },
    NonFinite(&'static str),
}

impl fmt::Display for HighlightsParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMethod(id) => write!(formatter, "unknown highlights method {id}"),
            Self::UnknownRecovery(id) => write!(formatter, "unknown recovery mode {id}"),
            Self::InvalidScale(id) => write!(formatter, "invalid wavelet scale {id}"),
            Self::InvalidIterations(value) => write!(formatter, "invalid iteration count {value}"),
            Self::OutOfRange { name, value } => {
                write!(formatter, "{name} is out of range ({value})")
            }
            Self::NonFinite(name) => write!(formatter, "{name} is non-finite"),
        }
    }
}
impl std::error::Error for HighlightsParameterError {}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, HighlightsParameterError> {
    let value = FiniteF32::new(value).map_err(|_| HighlightsParameterError::NonFinite(name))?;
    if (minimum..=maximum).contains(&value.get()) {
        Ok(value)
    } else {
        Err(HighlightsParameterError::OutOfRange {
            name,
            value: value.get().to_bits(),
        })
    }
}

/// Version-1 payload as retained by imported Darktable edits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightsV1 {
    pub method: i32,
    pub blend_l: f32,
    pub blend_c: f32,
    pub strength: f32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightsV2 {
    pub method: i32,
    pub blend_l: f32,
    pub blend_c: f32,
    pub strength: f32,
    pub clip: f32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightsV3 {
    pub method: i32,
    pub blend_l: f32,
    pub blend_c: f32,
    pub strength: f32,
    pub clip: f32,
    pub noise_level: f32,
    pub iterations: u16,
    pub scales: u8,
    pub candidating: f32,
    pub combine: f32,
    pub recovery: i32,
}

/// The version-4 payload retains the historical fields even when Darktable
/// marks them unused, so edit identity and round-trip evidence stay intact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightsV4 {
    pub method: i32,
    pub blend_l: f32,
    pub blend_c: f32,
    pub strength: f32,
    pub clip: f32,
    pub noise_level: f32,
    pub iterations: u16,
    pub scales: u8,
    pub candidating: f32,
    pub combine: f32,
    pub recovery: i32,
    pub solid_color: f32,
}

pub fn migrate_v1(value: HighlightsV1) -> Result<HighlightsV4, HighlightsParameterError> {
    Ok(HighlightsV4 {
        method: value.method,
        blend_l: value.blend_l,
        blend_c: value.blend_c,
        strength: 0.0,
        clip: 1.0,
        noise_level: 0.0,
        iterations: 1,
        scales: 5,
        candidating: 0.4,
        combine: 2.0,
        recovery: 0,
        solid_color: 0.0,
    })
}
pub fn migrate_v2(value: HighlightsV2) -> Result<HighlightsV4, HighlightsParameterError> {
    Ok(HighlightsV4 {
        method: value.method,
        blend_l: value.blend_l,
        blend_c: value.blend_c,
        strength: 0.0,
        clip: value.clip,
        noise_level: 0.0,
        iterations: 1,
        scales: 5,
        candidating: 0.4,
        combine: 2.0,
        recovery: 0,
        solid_color: 0.0,
    })
}
pub fn migrate_v3(value: HighlightsV3) -> Result<HighlightsV4, HighlightsParameterError> {
    Ok(HighlightsV4 {
        method: value.method,
        blend_l: value.blend_l,
        blend_c: value.blend_c,
        strength: 0.0,
        clip: value.clip,
        noise_level: value.noise_level,
        iterations: value.iterations,
        scales: value.scales,
        candidating: value.candidating,
        combine: value.combine,
        recovery: value.recovery,
        solid_color: 0.0,
    })
}

impl HighlightsV4 {
    pub fn config(self) -> Result<HighlightsConfig, HighlightsParameterError> {
        HighlightsConfig::new(
            HighlightsMethod::from_id(self.method)?,
            self.strength,
            self.clip,
            self.noise_level,
            self.iterations,
            WaveletScale::new(self.scales)?,
            self.candidating,
            self.combine,
            RecoveryMode::from_id(self.recovery)?,
            self.solid_color,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightsInputClass {
    Rgb,
    Bayer,
    XTrans,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightsPlan {
    config: HighlightsConfig,
    dimensions: RasterDimensions,
    input_class: HighlightsInputClass,
    budget: ReconstructionBudget,
}

impl HighlightsPlan {
    pub fn new(
        config: HighlightsConfig,
        dimensions: RasterDimensions,
        input_class: HighlightsInputClass,
        budget: ReconstructionBudget,
    ) -> Result<Self, OperationExecutionError> {
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX),
            10,
            budget,
        )?;
        Ok(Self {
            config,
            dimensions,
            input_class,
            budget,
        })
    }

    #[must_use]
    pub const fn config(self) -> HighlightsConfig {
        self.config
    }
    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn input_class(self) -> HighlightsInputClass {
        self.input_class
    }
    #[must_use]
    pub const fn full_image_analysis(self) -> bool {
        !matches!(self.config.method(), HighlightsMethod::Clip)
    }
    #[must_use]
    pub const fn support_radius(self) -> u32 {
        self.config.scales().diameter() / 2
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<HighlightsExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<HighlightsExecution, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        let count = input.len();
        checked_bytes(count, 10, self.budget)?;
        let clip = self.config.clip().get() * self.config.method().clip_magic();
        let mut diagnostics = ReconstructionDiagnostics::new(count);
        let mut clipped = vec![false; count];
        let mut candidates = vec![false; count];
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            clipped[index] = pixel.red().get() >= clip
                || pixel.green().get() >= clip
                || pixel.blue().get() >= clip;
            diagnostics.affected[index] = clipped[index];
        }
        for index in 0..count {
            if !clipped[index] {
                continue;
            }
            candidates[index] = neighborhood(self.dimensions, index, self.support_radius())
                .any(|neighbor| neighbor != index && !clipped[neighbor]);
            diagnostics.candidate[index] = candidates[index];
        }
        let mut output = input.to_vec();
        for index in 0..count {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            if !clipped[index] {
                continue;
            }
            let candidate = match self.config.method() {
                HighlightsMethod::Clip => clip_pixel(input[index], clip),
                HighlightsMethod::ReconstructLCh => reconstruct_lch(
                    input,
                    &clipped,
                    index,
                    self.dimensions,
                    self.support_radius(),
                ),
                HighlightsMethod::ReconstructColor => reconstruct_color(
                    input,
                    &clipped,
                    index,
                    self.dimensions,
                    self.support_radius(),
                ),
                HighlightsMethod::GuidedLaplacians => guided(
                    input,
                    &clipped,
                    index,
                    self.dimensions,
                    self.support_radius(),
                    self.config.iterations(),
                ),
                HighlightsMethod::SegmentationBased => {
                    nearest_segment(input, &clipped, index, self.dimensions)
                }
                HighlightsMethod::InpaintOpposed => opposed(
                    input,
                    &clipped,
                    index,
                    self.dimensions,
                    self.support_radius(),
                ),
            };
            let candidate = apply_solid_color(candidate, self.config.solid_color().get());
            let weight = self.config.strength().get()
                * if candidates[index] {
                    1.0
                } else {
                    self.config.candidating().get()
                };
            let blended = blend_pixel(input[index], candidate, weight)?;
            diagnostics.confidence[index] = weight.clamp(0.0, 1.0);
            diagnostics.contribution[index] = difference(input[index], blended)?;
            output[index] = blended;
        }
        let receipt = ReconstructionReceipt::new(
            HIGHLIGHTS_COMPATIBILITY_ID,
            HIGHLIGHTS_SCHEMA_VERSION,
            input,
            &output,
            &diagnostics,
        );
        Ok(HighlightsExecution {
            pixels: output,
            diagnostics,
            receipt,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HighlightsExecution {
    pixels: Vec<LinearRgb>,
    diagnostics: ReconstructionDiagnostics,
    receipt: ReconstructionReceipt,
}
impl HighlightsExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn diagnostics(&self) -> &ReconstructionDiagnostics {
        &self.diagnostics
    }
    #[must_use]
    pub const fn receipt(&self) -> &ReconstructionReceipt {
        &self.receipt
    }
}

fn clip_pixel(pixel: LinearRgb, threshold: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(pixel.red().get().min(threshold)).expect("finite input and threshold"),
        FiniteF32::new(pixel.green().get().min(threshold)).expect("finite input and threshold"),
        FiniteF32::new(pixel.blue().get().min(threshold)).expect("finite input and threshold"),
    )
}

fn reconstruct_lch(
    input: &[LinearRgb],
    clipped: &[bool],
    index: usize,
    dimensions: RasterDimensions,
    radius: u32,
) -> LinearRgb {
    let source = input[index];
    let mut sum = (0.0, 0.0, 0.0);
    for neighbor in neighborhood(dimensions, index, radius) {
        if neighbor == index || clipped[neighbor] {
            continue;
        }
        let (a, b) = chroma(input[neighbor]);
        let distance = (neighbor as isize - index as isize).unsigned_abs() as f32 + 1.0;
        sum.0 += a / distance;
        sum.1 += b / distance;
        sum.2 += 1.0 / distance;
    }
    if sum.2 == 0.0 {
        return source;
    }
    from_luma_chroma(luma(source), (sum.0 / sum.2, sum.1 / sum.2)).unwrap_or(source)
}

fn reconstruct_color(
    input: &[LinearRgb],
    clipped: &[bool],
    index: usize,
    dimensions: RasterDimensions,
    radius: u32,
) -> LinearRgb {
    let mut sum = [0.0; 3];
    let mut weight = 0.0;
    for neighbor in neighborhood(dimensions, index, radius) {
        if neighbor == index || clipped[neighbor] {
            continue;
        }
        let distance = (neighbor as isize - index as isize).unsigned_abs() as f32 + 1.0;
        let p = input[neighbor];
        let w = 1.0 / distance;
        sum[0] += p.red().get() * w;
        sum[1] += p.green().get() * w;
        sum[2] += p.blue().get() * w;
        weight += w;
    }
    if weight == 0.0 {
        input[index]
    } else {
        LinearRgb::new(
            FiniteF32::new(sum[0] / weight).expect("finite"),
            FiniteF32::new(sum[1] / weight).expect("finite"),
            FiniteF32::new(sum[2] / weight).expect("finite"),
        )
    }
}

fn guided(
    input: &[LinearRgb],
    clipped: &[bool],
    index: usize,
    dimensions: RasterDimensions,
    radius: u32,
    iterations: u16,
) -> LinearRgb {
    let mut result = reconstruct_color(input, clipped, index, dimensions, radius);
    let passes = iterations.min(8);
    for _ in 1..passes {
        result = reconstruct_color(input, clipped, index, dimensions, radius);
    }
    result
}

fn nearest_segment(
    input: &[LinearRgb],
    clipped: &[bool],
    index: usize,
    dimensions: RasterDimensions,
) -> LinearRgb {
    let width = usize::try_from(dimensions.width()).expect("width fits");
    let x = index % width;
    let y = index / width;
    let mut best = None;
    for (candidate, is_clipped) in clipped.iter().copied().enumerate() {
        if is_clipped {
            continue;
        }
        let cx = candidate % width;
        let cy = candidate / width;
        let distance = x.abs_diff(cx).saturating_add(y.abs_diff(cy));
        let key = (distance, candidate);
        if best.is_none_or(|(old, _)| key < old) {
            best = Some((key, candidate));
        }
    }
    best.map_or(input[index], |(_, candidate)| input[candidate])
}

fn opposed(
    input: &[LinearRgb],
    clipped: &[bool],
    index: usize,
    dimensions: RasterDimensions,
    radius: u32,
) -> LinearRgb {
    let source = input[index];
    let source_chroma = chroma(source);
    let mut selected = Vec::new();
    for neighbor in neighborhood(dimensions, index, radius) {
        if neighbor == index || clipped[neighbor] {
            continue;
        }
        let c = chroma(input[neighbor]);
        if c.0 * source_chroma.0 + c.1 * source_chroma.1 <= 0.0 {
            selected.push(input[neighbor]);
        }
    }
    if selected.is_empty() {
        reconstruct_lch(input, clipped, index, dimensions, radius)
    } else {
        let average = selected
            .iter()
            .fold([0.0; 3], |mut sum, pixel| {
                sum[0] += pixel.red().get();
                sum[1] += pixel.green().get();
                sum[2] += pixel.blue().get();
                sum
            })
            .map(|value| value / selected.len() as f32);
        LinearRgb::new(
            FiniteF32::new(average[0]).expect("finite"),
            FiniteF32::new(average[1]).expect("finite"),
            FiniteF32::new(average[2]).expect("finite"),
        )
    }
}

fn apply_solid_color(pixel: LinearRgb, amount: f32) -> LinearRgb {
    let lightness = luma(pixel);
    from_luma_chroma(
        lightness,
        (
            chroma(pixel).0 * (1.0 - amount),
            chroma(pixel).1 * (1.0 - amount),
        ),
    )
    .unwrap_or(pixel)
}

fn difference(source: LinearRgb, output: LinearRgb) -> Result<LinearRgb, OperationExecutionError> {
    let values = [
        output.red().get() - source.red().get(),
        output.green().get() - source.green().get(),
        output.blue().get() - source.blue().get(),
    ];
    Ok(LinearRgb::new(
        FiniteF32::new(values[0]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Red,
        })?,
        FiniteF32::new(values[1]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Green,
        })?,
        FiniteF32::new(values[2]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Blue,
        })?,
    ))
}

fn blend_pixel(
    source: LinearRgb,
    candidate: LinearRgb,
    weight: f32,
) -> Result<LinearRgb, OperationExecutionError> {
    apply_opacity(source, candidate, weight).map_err(|_| OperationExecutionError::NonFiniteResult {
        pixel: 0,
        channel: crate::RgbChannel::Red,
    })
}

/// A canonical list of GPU passes required to mirror the scalar plan.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 8] {
    [
        "highlights.clip",
        "highlights.mask",
        "highlights.remosaic",
        "highlights.distance",
        "highlights.wavelet",
        "highlights.segment",
        "highlights.inpaint",
        "highlights.replace",
    ]
}
