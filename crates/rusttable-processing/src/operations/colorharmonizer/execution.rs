//! Full-frame CPU execution ported from `src/iop/colorharmonizer.c`.
//!
//! The leaf accepts only an explicit working-profile RGB↔XYZ D50 matrix pair.
//! Profile acquisition, operation registration, shared evaluation, blending,
//! GPU dispatch, and UI routing remain intentionally outside this module.

#![allow(
    clippy::approx_constant,
    clippy::cast_precision_loss,
    clippy::excessive_precision,
    clippy::similar_names,
    clippy::unreadable_literal
)]

use std::fmt;

use super::codec::{
    COLORHARMONIZER_MAX_NODES, ColorHarmonizerCodecError, ColorHarmonizerParametersV1,
    ColorHarmonizerRule,
};
use super::ucs::{
    self, HarmonyTables, clampf, harmony_nodes, jch_to_xyy, xyy_to_jch, xyy_to_xyz, xyz_d65_to_xyy,
    y_to_l_star,
};
use crate::operations::ReconstructionBudget;

const CAT16_D50_TO_D65_TRANSPOSED: [[f32; 4]; 3] = [
    [
        0.989466254_f32,
        -0.00540518733_f32,
        -0.000403920992_f32,
        0.0_f32,
    ],
    [-0.0400304626_f32, 1.00666069_f32, 0.0150768030_f32, 0.0_f32],
    [
        0.0440530317_f32,
        -0.00175551955_f32,
        1.30210211_f32,
        0.0_f32,
    ],
];

const CAT16_D65_TO_D50_TRANSPOSED: [[f32; 4]; 3] = [
    [
        1.01085433_f32,
        0.00542814201_f32,
        0.000250722468_f32,
        0.0_f32,
    ],
    [
        0.0407086103_f32,
        0.993581926_f32,
        -0.0114918759_f32,
        0.0_f32,
    ],
    [
        -0.0341445825_f32,
        0.00115592039_f32,
        0.767964947_f32,
        0.0_f32,
    ],
];

const GAUSSIAN_RANGE: f32 = 1.0e9_f32;
const PI_F: f32 = 3.14159265358979323846_f32;
const TWO_PI_F: f32 = 6.28318530717958647693_f32;

/// Exact f32 working profile matrices required by the native operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkingProfileMatrices {
    /// Native `work_profile->matrix_in_transposed` (RGB working to XYZ D50).
    pub matrix_in_transposed: [[f32; 4]; 3],
    /// Native `work_profile->matrix_out_transposed` (XYZ D50 to RGB working).
    pub matrix_out_transposed: [[f32; 4]; 3],
}

impl WorkingProfileMatrices {
    #[must_use]
    pub const fn new(
        matrix_in_transposed: [[f32; 4]; 3],
        matrix_out_transposed: [[f32; 4]; 3],
    ) -> Self {
        Self {
            matrix_in_transposed,
            matrix_out_transposed,
        }
    }

    /// Rejects non-finite or malformed matrix context before execution.
    pub fn validate(self) -> Result<(), ColorHarmonizerExecutionError> {
        for matrix in [self.matrix_in_transposed, self.matrix_out_transposed] {
            if matrix
                .iter()
                .flat_map(|row| row.iter())
                .any(|value| !value.is_finite())
            {
                return Err(ColorHarmonizerExecutionError::InvalidProfileMatrix);
            }
        }
        Ok(())
    }
}

/// Nonzero full-frame dimensions required by the native identity-ROI path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDimensions {
    pub width: usize,
    pub height: usize,
}

impl FrameDimensions {
    pub fn new(width: usize, height: usize) -> Result<Self, ColorHarmonizerExecutionError> {
        if width == 0 || height == 0 || width.checked_mul(height).is_none() {
            return Err(ColorHarmonizerExecutionError::InvalidDimensions { width, height });
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn pixels(self) -> usize {
        self.width * self.height
    }

    fn validated_pixels(self) -> Result<usize, ColorHarmonizerExecutionError> {
        if self.width == 0 || self.height == 0 {
            return Err(ColorHarmonizerExecutionError::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }
        self.width.checked_mul(self.height).ok_or(
            ColorHarmonizerExecutionError::InvalidDimensions {
                width: self.width,
                height: self.height,
            },
        )
    }
}

/// Finite execution state after source parameter validation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHarmonizerConfig {
    parameters: ColorHarmonizerParametersV1,
}

impl ColorHarmonizerConfig {
    pub fn new(parameters: ColorHarmonizerParametersV1) -> Result<Self, ColorHarmonizerCodecError> {
        validate_finite("anchor_hue", parameters.anchor_hue)?;
        validate_finite("pull_strength", parameters.pull_strength)?;
        validate_finite("neutral_protection", parameters.neutral_protection)?;
        validate_finite("pull_width", parameters.pull_width)?;
        validate_finite("smoothing", parameters.smoothing)?;
        for (index, value) in parameters.custom_hue.iter().enumerate() {
            validate_finite_indexed("custom_hue", index, *value)?;
            validate_hue("custom_hue", *value)?;
        }
        for (index, value) in parameters.node_saturation.iter().enumerate() {
            validate_finite_indexed("node_saturation", index, *value)?;
            validate_parameter_range("node_saturation", *value, 0.0, 2.0)?;
        }
        validate_hue("anchor_hue", parameters.anchor_hue)?;
        validate_parameter_range("pull_strength", parameters.pull_strength, 0.0, 1.0)?;
        validate_parameter_range(
            "neutral_protection",
            parameters.neutral_protection,
            0.0,
            1.0,
        )?;
        if parameters.pull_width <= 0.0_f32 {
            return Err(ColorHarmonizerCodecError::NonPositivePullWidth);
        }
        validate_parameter_range("pull_width", parameters.pull_width, 0.25, 4.0)?;
        if !(2..=4).contains(&parameters.num_custom_nodes) {
            return Err(ColorHarmonizerCodecError::NodeCountOutOfRange {
                value: parameters.num_custom_nodes,
                minimum: 2,
                maximum: 4,
            });
        }
        validate_parameter_range("smoothing", parameters.smoothing, 0.0, 2.0)?;
        Ok(Self { parameters })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self {
            parameters: ColorHarmonizerParametersV1::defaults(),
        }
    }

    #[must_use]
    pub const fn parameters(self) -> ColorHarmonizerParametersV1 {
        self.parameters
    }

    #[must_use]
    pub const fn rule(self) -> ColorHarmonizerRule {
        self.parameters.rule
    }

    #[must_use]
    pub const fn anchor_hue(self) -> f32 {
        self.parameters.anchor_hue
    }

    #[must_use]
    pub const fn pull_strength(self) -> f32 {
        self.parameters.pull_strength
    }

    #[must_use]
    pub const fn neutral_protection(self) -> f32 {
        self.parameters.neutral_protection
    }

    #[must_use]
    pub const fn pull_width(self) -> f32 {
        self.parameters.pull_width
    }

    #[must_use]
    pub const fn custom_hue(self) -> [f32; COLORHARMONIZER_MAX_NODES] {
        self.parameters.custom_hue
    }

    #[must_use]
    pub const fn num_custom_nodes(self) -> i32 {
        self.parameters.num_custom_nodes
    }

    #[must_use]
    pub const fn node_saturation(self) -> [f32; COLORHARMONIZER_MAX_NODES] {
        self.parameters.node_saturation
    }

    #[must_use]
    pub const fn smoothing(self) -> f32 {
        self.parameters.smoothing
    }
}

impl TryFrom<ColorHarmonizerParametersV1> for ColorHarmonizerConfig {
    type Error = ColorHarmonizerCodecError;

    fn try_from(parameters: ColorHarmonizerParametersV1) -> Result<Self, Self::Error> {
        Self::new(parameters)
    }
}

fn validate_finite(name: &'static str, value: f32) -> Result<(), ColorHarmonizerCodecError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ColorHarmonizerCodecError::NonFinite(name))
    }
}

fn validate_finite_indexed(
    name: &'static str,
    index: usize,
    value: f32,
) -> Result<(), ColorHarmonizerCodecError> {
    if value.is_finite() {
        Ok(())
    } else if name == "custom_hue" {
        Err(ColorHarmonizerCodecError::NonFinite(match index {
            0 => "custom_hue[0]",
            1 => "custom_hue[1]",
            2 => "custom_hue[2]",
            _ => "custom_hue[3]",
        }))
    } else {
        Err(ColorHarmonizerCodecError::NonFinite(match index {
            0 => "node_saturation[0]",
            1 => "node_saturation[1]",
            2 => "node_saturation[2]",
            _ => "node_saturation[3]",
        }))
    }
}

fn validate_hue(name: &'static str, value: f32) -> Result<(), ColorHarmonizerCodecError> {
    if (0.0_f32..=1.0_f32).contains(&value) {
        Ok(())
    } else {
        Err(ColorHarmonizerCodecError::HueOutOfRange(name))
    }
}

fn validate_parameter_range(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<(), ColorHarmonizerCodecError> {
    if (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name,
            minimum,
            maximum,
        })
    }
}

/// Immutable operation state with native precomputed harmony nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorHarmonizerPlan {
    config: ColorHarmonizerConfig,
    nodes: [f32; COLORHARMONIZER_MAX_NODES],
    num_nodes: usize,
    tables: &'static HarmonyTables,
}

impl ColorHarmonizerPlan {
    #[must_use]
    pub fn new(config: ColorHarmonizerConfig) -> Self {
        let tables = ucs::harmony_tables();
        let (nodes, num_nodes) = harmony_nodes(
            config.rule(),
            config.anchor_hue(),
            &config.custom_hue(),
            config.num_custom_nodes(),
            tables,
        );
        Self {
            config,
            nodes,
            num_nodes,
            tables,
        }
    }

    #[must_use]
    pub const fn config(&self) -> ColorHarmonizerConfig {
        self.config
    }

    #[must_use]
    pub fn nodes(&self) -> &[f32] {
        &self.nodes[..self.num_nodes]
    }

    #[must_use]
    pub const fn num_nodes(&self) -> usize {
        self.num_nodes
    }

    #[must_use]
    pub const fn tables(&self) -> &'static HarmonyTables {
        self.tables
    }

    /// Executes the native full-frame CPU path with no publication until all
    /// requested pixels and smoothing passes succeed.
    pub fn execute(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: WorkingProfileMatrices,
        roi_scale: f32,
        piece_iscale: f32,
    ) -> Result<Vec<[f32; 4]>, ColorHarmonizerExecutionError> {
        self.execute_with_budget(
            input,
            dimensions,
            profile,
            roi_scale,
            piece_iscale,
            ReconstructionBudget::default(),
        )
    }

    /// Full-frame execution with an explicit project reconstruction budget.
    pub fn execute_with_budget(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: WorkingProfileMatrices,
        roi_scale: f32,
        piece_iscale: f32,
        budget: ReconstructionBudget,
    ) -> Result<Vec<[f32; 4]>, ColorHarmonizerExecutionError> {
        self.execute_with_cancellation_and_budget(
            input,
            dimensions,
            profile,
            roi_scale,
            piece_iscale,
            budget,
            || false,
        )
    }

    /// Cancellation-aware full-frame execution.  A cancellation or allocation
    /// error returns before any output is published to the caller.
    pub fn execute_with_cancellation<F>(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: WorkingProfileMatrices,
        roi_scale: f32,
        piece_iscale: f32,
        cancelled: F,
    ) -> Result<Vec<[f32; 4]>, ColorHarmonizerExecutionError>
    where
        F: FnMut() -> bool,
    {
        self.execute_with_cancellation_and_budget(
            input,
            dimensions,
            profile,
            roi_scale,
            piece_iscale,
            ReconstructionBudget::default(),
            cancelled,
        )
    }

    /// Cancellation-aware execution with an explicit project reconstruction budget.
    pub fn execute_with_cancellation_and_budget<F>(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: WorkingProfileMatrices,
        roi_scale: f32,
        piece_iscale: f32,
        budget: ReconstructionBudget,
        mut cancelled: F,
    ) -> Result<Vec<[f32; 4]>, ColorHarmonizerExecutionError>
    where
        F: FnMut() -> bool,
    {
        let pixel_count = dimensions.validated_pixels()?;
        check_memory_budget(pixel_count, self.config.smoothing() > 0.0_f32, budget)?;
        if input.len() != pixel_count {
            return Err(ColorHarmonizerExecutionError::InputLength {
                expected: pixel_count,
                actual: input.len(),
            });
        }
        profile.validate()?;
        validate_pixels(input)?;
        if cancelled() {
            return Err(ColorHarmonizerExecutionError::Cancelled);
        }

        if self.config.smoothing() <= 0.0_f32 {
            let mut output = reserve_values(pixel_count, "output")?;
            for (index, pixel) in input.iter().enumerate() {
                if index % dimensions.width == 0 && cancelled() {
                    return Err(ColorHarmonizerExecutionError::Cancelled);
                }
                output.push(self.process_fused(*pixel, profile));
            }
            return Ok(output);
        }

        let sigma = smoothing_sigma(self.config, roi_scale, piece_iscale)?;
        let mut jch_cache = reserve_values(pixel_count, "jch_cache")?;
        let mut corrections = reserve_values(pixel_count, "corrections")?;
        for (index, pixel) in input.iter().enumerate() {
            if index % dimensions.width == 0 && cancelled() {
                return Err(ColorHarmonizerExecutionError::Cancelled);
            }
            let jch = Self::forward_jch(*pixel, profile);
            let hue = (jch[2] + PI_F) / TWO_PI_F;
            let correction = self.correction(hue);
            jch_cache.push([jch[0], jch[1], hue]);
            corrections.push(correction);
        }

        blur_corrections(
            &mut corrections,
            dimensions.width,
            dimensions.height,
            sigma,
            &mut cancelled,
        )?;
        if cancelled() {
            return Err(ColorHarmonizerExecutionError::Cancelled);
        }

        let mut output = reserve_values(pixel_count, "output")?;
        for (index, (cached, correction)) in jch_cache.iter().zip(&corrections).enumerate() {
            if index % dimensions.width == 0 && cancelled() {
                return Err(ColorHarmonizerExecutionError::Cancelled);
            }
            output.push(self.process_cached(*cached, *correction, input[index], profile));
        }
        Ok(output)
    }

    fn correction(&self, hue: f32) -> [f32; 2] {
        let weighted =
            weighted_hue_shift(hue, &self.nodes, self.num_nodes, self.config.pull_width());
        let saturation = self.config.node_saturation()[weighted.winning_index];
        [
            weighted.hue_shift,
            (saturation - 1.0_f32) * weighted.maximum_weight,
        ]
    }

    fn forward_jch(pixel: [f32; 4], profile: WorkingProfileMatrices) -> [f32; 3] {
        let rgb = [
            nonnegative(pixel[0]),
            nonnegative(pixel[1]),
            nonnegative(pixel[2]),
        ];
        let xyz_d50 = apply_transposed(rgb, profile.matrix_in_transposed);
        let xyz_d65 = apply_transposed(xyz_d50, CAT16_D50_TO_D65_TRANSPOSED);
        let xyy = xyz_d65_to_xyy(xyz_d65);
        xyy_to_jch(xyy, y_to_l_star(1.0_f32))
    }

    fn process_fused(&self, pixel: [f32; 4], profile: WorkingProfileMatrices) -> [f32; 4] {
        let jch = Self::forward_jch(pixel, profile);
        let hue = (jch[2] + PI_F) / TWO_PI_F;
        let correction = self.correction(hue);
        self.process_cached([jch[0], jch[1], hue], correction, pixel, profile)
    }

    fn process_cached(
        &self,
        cached: [f32; 3],
        correction: [f32; 2],
        pixel: [f32; 4],
        profile: WorkingProfileMatrices,
    ) -> [f32; 4] {
        let chroma = cached[1];
        let chroma_weight = chroma
            / (chroma
                + self.config.neutral_protection()
                    * self.config.neutral_protection()
                    * self.config.neutral_protection()
                    * 0.03_f32
                + 1.0e-5_f32);
        let new_hue =
            wrap_hue(cached[2] + correction[0] * self.config.pull_strength() * chroma_weight);
        let new_chroma = nonnegative(chroma * (1.0_f32 + correction[1] * chroma_weight));
        let xyz_d65 = xyy_to_xyz(jch_to_xyy(
            [cached[0], new_chroma, new_hue * TWO_PI_F - PI_F],
            y_to_l_star(1.0_f32),
        ));
        let xyz_d50 = apply_transposed(xyz_d65, CAT16_D65_TO_D50_TRANSPOSED);
        let rgb = apply_transposed(xyz_d50, profile.matrix_out_transposed);
        [rgb[0], rgb[1], rgb[2], pixel[3]]
    }
}

fn validate_pixels(input: &[[f32; 4]]) -> Result<(), ColorHarmonizerExecutionError> {
    for (index, pixel) in input.iter().enumerate() {
        for (channel, value) in pixel.iter().enumerate() {
            if !value.is_finite() {
                return Err(ColorHarmonizerExecutionError::NonFiniteInput { index, channel });
            }
        }
    }
    Ok(())
}

fn check_memory_budget(
    pixel_count: usize,
    smoothing: bool,
    budget: ReconstructionBudget,
) -> Result<(), ColorHarmonizerExecutionError> {
    let output_bytes = pixel_count.checked_mul(std::mem::size_of::<[f32; 4]>());
    let required = if smoothing {
        output_bytes
            .and_then(|bytes| {
                pixel_count
                    .checked_mul(std::mem::size_of::<[f32; 3]>())
                    .and_then(|jch_bytes| bytes.checked_add(jch_bytes))
            })
            .and_then(|bytes| {
                pixel_count
                    .checked_mul(std::mem::size_of::<[f32; 2]>())
                    .and_then(|correction_bytes| bytes.checked_add(correction_bytes))
            })
            .and_then(|bytes| {
                pixel_count
                    .checked_mul(std::mem::size_of::<[f32; 2]>())
                    .and_then(|gaussian_bytes| bytes.checked_add(gaussian_bytes))
            })
    } else {
        output_bytes
    }
    .ok_or(ColorHarmonizerExecutionError::MemoryBudgetExceeded {
        required: usize::MAX,
        budget: budget.maximum_bytes(),
    })?;

    if required <= budget.maximum_bytes() {
        Ok(())
    } else {
        Err(ColorHarmonizerExecutionError::MemoryBudgetExceeded {
            required,
            budget: budget.maximum_bytes(),
        })
    }
}

fn reserve_values<T>(
    count: usize,
    buffer: &'static str,
) -> Result<Vec<T>, ColorHarmonizerExecutionError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| ColorHarmonizerExecutionError::AllocationFailed { buffer })?;
    Ok(values)
}

fn nonnegative(value: f32) -> f32 {
    if value > 0.0_f32 { value } else { 0.0_f32 }
}

fn apply_transposed(rgb: [f32; 3], matrix: [[f32; 4]; 3]) -> [f32; 3] {
    [
        matrix[0][0] * rgb[0] + matrix[1][0] * rgb[1] + matrix[2][0] * rgb[2],
        matrix[0][1] * rgb[0] + matrix[1][1] * rgb[1] + matrix[2][1] * rgb[2],
        matrix[0][2] * rgb[0] + matrix[1][2] * rgb[1] + matrix[2][2] * rgb[2],
    ]
}

/// Source Gaussian selection result, including first-winner tie behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeightedHueShift {
    pub winning_index: usize,
    pub maximum_weight: f32,
    pub hue_shift: f32,
}

#[must_use]
pub fn weighted_hue_shift(
    pixel_hue: f32,
    nodes: &[f32],
    num_nodes: usize,
    pull_width: f32,
) -> WeightedHueShift {
    if num_nodes == 0 {
        return WeightedHueShift {
            winning_index: 0,
            maximum_weight: 0.0_f32,
            hue_shift: 0.0_f32,
        };
    }
    let sigma = pull_width * 0.5_f32 / num_nodes as f32;
    let inverse_two_sigma_squared = 1.0_f32 / (2.0_f32 * sigma * sigma);
    let mut maximum_weight = 0.0_f32;
    let mut winning_index = 0;
    let mut difference_winning = 0.0_f32;
    for (index, node) in nodes.iter().take(num_nodes).enumerate() {
        let mut distance = (pixel_hue - *node).abs();
        if distance > 0.5_f32 {
            distance = 1.0_f32 - distance;
        }
        let weight = (-distance * distance * inverse_two_sigma_squared).exp();
        let mut difference = *node - pixel_hue;
        if difference > 0.5_f32 {
            difference -= 1.0_f32;
        } else if difference < -0.5_f32 {
            difference += 1.0_f32;
        }
        if weight > maximum_weight {
            maximum_weight = weight;
            winning_index = index;
            difference_winning = difference;
        }
    }
    WeightedHueShift {
        winning_index,
        maximum_weight,
        hue_shift: difference_winning * maximum_weight,
    }
}

#[must_use]
pub fn wrap_hue(mut hue: f32) -> f32 {
    hue %= 1.0_f32;
    if hue < 0.0_f32 {
        hue += 1.0_f32;
    }
    hue
}

/// Source smoothing sigma formula, exposed for source-derived tests.
pub fn smoothing_sigma(
    config: ColorHarmonizerConfig,
    roi_scale: f32,
    piece_iscale: f32,
) -> Result<f32, ColorHarmonizerExecutionError> {
    if !roi_scale.is_finite()
        || !piece_iscale.is_finite()
        || roi_scale <= 0.0_f32
        || piece_iscale <= 0.0_f32
    {
        return Err(ColorHarmonizerExecutionError::InvalidScale {
            roi_scale,
            piece_iscale,
        });
    }
    let sigma = config.smoothing()
        * (1.5_f32.max(8.0_f32 * roi_scale / piece_iscale))
        * 1.0_f32.max(config.pull_width());
    if sigma.is_finite() && sigma > 0.0_f32 {
        Ok(sigma)
    } else {
        Err(ColorHarmonizerExecutionError::InvalidScale {
            roi_scale,
            piece_iscale,
        })
    }
}

fn blur_corrections<F>(
    corrections: &mut [[f32; 2]],
    width: usize,
    height: usize,
    sigma: f32,
    cancelled: &mut F,
) -> Result<(), ColorHarmonizerExecutionError>
where
    F: FnMut() -> bool,
{
    let (a0, a1, a2, a3, b1, b2, coefp, coefn) = gaussian_params(sigma);
    let mut temp = Vec::new();
    temp.try_reserve_exact(corrections.len()).map_err(|_| {
        ColorHarmonizerExecutionError::AllocationFailed {
            buffer: "gaussian_temp",
        }
    })?;
    temp.resize(corrections.len(), [0.0_f32; 2]);

    for column in 0..width {
        if cancelled() {
            return Err(ColorHarmonizerExecutionError::Cancelled);
        }
        let mut xp = [0.0_f32; 2];
        let mut yb = [0.0_f32; 2];
        let mut yp = [0.0_f32; 2];
        for channel in 0..2 {
            xp[channel] = clampf(
                corrections[column][channel],
                -GAUSSIAN_RANGE,
                GAUSSIAN_RANGE,
            );
            yb[channel] = xp[channel] * coefp;
            yp[channel] = yb[channel];
        }
        for row in 0..height {
            let index = row * width + column;
            let mut xc = [0.0_f32; 2];
            let mut yc = [0.0_f32; 2];
            for channel in 0..2 {
                xc[channel] = clampf(corrections[index][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
                yc[channel] =
                    a0 * xc[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = xc[channel];
                yb[channel] = yp[channel];
                yp[channel] = yc[channel];
            }
            temp[index] = yc;
        }

        let last = (height - 1) * width + column;
        let mut xn = [0.0_f32; 2];
        let mut xa = [0.0_f32; 2];
        let mut yn = [0.0_f32; 2];
        let mut ya = [0.0_f32; 2];
        for channel in 0..2 {
            xn[channel] = clampf(corrections[last][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
            xa[channel] = xn[channel];
            yn[channel] = xn[channel] * coefn;
            ya[channel] = yn[channel];
        }
        for row in (0..height).rev() {
            let index = row * width + column;
            let mut xc = [0.0_f32; 2];
            let mut yc = [0.0_f32; 2];
            for channel in 0..2 {
                xc[channel] = clampf(corrections[index][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
                yc[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = xc[channel];
                ya[channel] = yn[channel];
                yn[channel] = yc[channel];
                temp[index][channel] += yc[channel];
            }
        }
    }

    for row in 0..height {
        if cancelled() {
            return Err(ColorHarmonizerExecutionError::Cancelled);
        }
        let first = row * width;
        let mut xp = [0.0_f32; 2];
        let mut yb = [0.0_f32; 2];
        let mut yp = [0.0_f32; 2];
        for channel in 0..2 {
            xp[channel] = clampf(temp[first][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
            yb[channel] = xp[channel] * coefp;
            yp[channel] = yb[channel];
        }
        for column in 0..width {
            let index = first + column;
            let mut xc = [0.0_f32; 2];
            let mut yc = [0.0_f32; 2];
            for channel in 0..2 {
                xc[channel] = clampf(temp[index][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
                yc[channel] =
                    a0 * xc[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = xc[channel];
                yb[channel] = yp[channel];
                yp[channel] = yc[channel];
                corrections[index][channel] = yc[channel];
            }
        }

        let last = first + width - 1;
        let mut xn = [0.0_f32; 2];
        let mut xa = [0.0_f32; 2];
        let mut yn = [0.0_f32; 2];
        let mut ya = [0.0_f32; 2];
        for channel in 0..2 {
            xn[channel] = clampf(temp[last][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
            xa[channel] = xn[channel];
            yn[channel] = xn[channel] * coefn;
            ya[channel] = yn[channel];
        }
        for column in (0..width).rev() {
            let index = first + column;
            let mut xc = [0.0_f32; 2];
            let mut yc = [0.0_f32; 2];
            for channel in 0..2 {
                xc[channel] = clampf(temp[index][channel], -GAUSSIAN_RANGE, GAUSSIAN_RANGE);
                yc[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = xc[channel];
                ya[channel] = yn[channel];
                yn[channel] = yc[channel];
                corrections[index][channel] += yc[channel];
            }
        }
    }
    Ok(())
}

fn gaussian_params(sigma: f32) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let alpha = 1.695_f32 / sigma;
    let ema = (-alpha).exp();
    let ema2 = (-2.0_f32 * alpha).exp();
    let b1 = -2.0_f32 * ema;
    let b2 = ema2;
    let k = (1.0_f32 - ema) * (1.0_f32 - ema) / (1.0_f32 + 2.0_f32 * alpha * ema - ema2);
    let a0 = k;
    let a1 = k * (alpha - 1.0_f32) * ema;
    let a2 = k * (alpha + 1.0_f32) * ema;
    let a3 = -k * ema2;
    let divisor = 1.0_f32 + b1 + b2;
    let coefp = (a0 + a1) / divisor;
    let coefn = (a2 + a3) / divisor;
    (a0, a1, a2, a3, b1, b2, coefp, coefn)
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorHarmonizerExecutionError {
    InvalidDimensions { width: usize, height: usize },
    InputLength { expected: usize, actual: usize },
    InvalidProfileMatrix,
    InvalidScale { roi_scale: f32, piece_iscale: f32 },
    NonFiniteInput { index: usize, channel: usize },
    MemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { buffer: &'static str },
    Cancelled,
}

impl fmt::Display for ColorHarmonizerExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(
                    formatter,
                    "Color Harmonizer frame dimensions {width}x{height} are invalid"
                )
            }
            Self::InputLength { expected, actual } => {
                write!(
                    formatter,
                    "Color Harmonizer input has {actual} pixels; expected {expected}"
                )
            }
            Self::InvalidProfileMatrix => {
                formatter.write_str("Color Harmonizer profile matrices contain non-finite values")
            }
            Self::InvalidScale {
                roi_scale,
                piece_iscale,
            } => write!(
                formatter,
                "Color Harmonizer scales {roi_scale} and {piece_iscale} cannot resolve smoothing"
            ),
            Self::NonFiniteInput { index, channel } => {
                write!(
                    formatter,
                    "Color Harmonizer input pixel {index} channel {channel} is non-finite"
                )
            }
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "Color Harmonizer requires {required} bytes, budget is {budget}"
            ),
            Self::AllocationFailed { buffer } => {
                write!(formatter, "Color Harmonizer could not allocate {buffer}")
            }
            Self::Cancelled => formatter.write_str("Color Harmonizer execution was cancelled"),
        }
    }
}

impl std::error::Error for ColorHarmonizerExecutionError {}
