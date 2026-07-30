use super::common::{apply_profile, check_cancelled, profile_luminance, profile_plan};
use super::{
    DiagnosticBackend, DiagnosticDescriptor, DiagnosticFinding, DiagnosticFrame, DiagnosticPath,
};
use rusttable_color::ColorEncoding;
use rusttable_image::{CancellationToken, ImageDimensions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const OVEREXPOSED_DESCRIPTOR: DiagnosticDescriptor = DiagnosticDescriptor::new("overexposed", 3);
const MAX_DIAGNOSTIC_PIXELS: u64 = 64 * 1024 * 1024;
const COLORS: [[[f32; 3]; 2]; 3] = [
    [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
    [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.371, 0.434, 0.934], [0.512, 0.934, 0.371]],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverexposedMode {
    AnyRgb,
    Gamut,
    Luminance,
    Saturation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverexposedColorScheme {
    BlackWhite,
    RedBlue,
    PurpleGreen,
}

impl OverexposedColorScheme {
    #[must_use]
    pub const fn colors(self) -> [[f32; 3]; 2] {
        COLORS[self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OverexposedState {
    mode: OverexposedMode,
    lower_ev: f32,
    upper_percent: f32,
    color_scheme: OverexposedColorScheme,
    histogram_profile: ColorEncoding,
}

impl OverexposedState {
    #[must_use]
    pub const fn new(
        mode: OverexposedMode,
        lower_ev: f32,
        upper_percent: f32,
        color_scheme: OverexposedColorScheme,
        histogram_profile: ColorEncoding,
    ) -> Self {
        Self {
            mode,
            lower_ev,
            upper_percent,
            color_scheme,
            histogram_profile,
        }
    }

    #[must_use]
    pub const fn mode(self) -> OverexposedMode {
        self.mode
    }
    #[must_use]
    pub const fn lower_ev(self) -> f32 {
        self.lower_ev
    }
    #[must_use]
    pub const fn upper_percent(self) -> f32 {
        self.upper_percent
    }
    #[must_use]
    pub const fn color_scheme(self) -> OverexposedColorScheme {
        self.color_scheme
    }
    #[must_use]
    pub const fn histogram_profile(self) -> ColorEncoding {
        self.histogram_profile
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverexposedPlan {
    output_dimensions: ImageDimensions,
    state: OverexposedState,
    lower: f32,
    upper: f32,
    transform: rusttable_color::TransformPlan,
    transform_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverexposedReceipt {
    pub path: DiagnosticPath,
    pub mode: OverexposedMode,
    pub lower: f32,
    pub upper: f32,
    pub current_profile: ColorEncoding,
    pub histogram_profile: ColorEncoding,
    pub transform_hash: [u8; 32],
    pub upper_count: u64,
    pub lower_count: u64,
    pub non_finite_count: u64,
    pub zero_denominator_count: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverexposedResult {
    frame: DiagnosticFrame,
    receipt: OverexposedReceipt,
    finding: Option<DiagnosticFinding>,
}

impl OverexposedResult {
    #[must_use]
    pub fn frame(&self) -> &DiagnosticFrame {
        &self.frame
    }
    #[must_use]
    pub const fn receipt(&self) -> OverexposedReceipt {
        self.receipt
    }
    #[must_use]
    pub const fn finding(&self) -> Option<DiagnosticFinding> {
        self.finding
    }
    #[must_use]
    pub fn applied(&self) -> bool {
        self.finding.is_none()
    }
}

impl OverexposedPlan {
    /// Builds a profile-bound, immutable overexposure plan.
    ///
    /// # Errors
    ///
    /// Returns a typed finding when thresholds, profiles, transforms, or
    /// resource bounds are invalid.
    pub fn new(
        current_profile: ColorEncoding,
        output_dimensions: ImageDimensions,
        state: OverexposedState,
    ) -> Result<Self, DiagnosticFinding> {
        let pixels = output_dimensions
            .pixel_count()
            .map_err(|_| DiagnosticFinding::ResourceLimit)?;
        if pixels > MAX_DIAGNOSTIC_PIXELS {
            return Err(DiagnosticFinding::ResourceLimit);
        }
        if !state.lower_ev.is_finite()
            || !state.upper_percent.is_finite()
            || !(0.0..=100.0).contains(&state.upper_percent)
        {
            return Err(DiagnosticFinding::InvalidState);
        }
        let lower = 2.0_f32.powf(state.lower_ev.min(-4.0));
        let upper = state.upper_percent / 100.0;
        if !lower.is_finite() || !upper.is_finite() || lower >= upper {
            return Err(DiagnosticFinding::InvalidThreshold);
        }
        let transform = profile_plan(current_profile, state.histogram_profile)?;
        let transform_hash = transform
            .identity()
            .map_err(|_| DiagnosticFinding::InvalidTransform)?;
        if state
            .histogram_profile
            .builtin()
            .and_then(rusttable_color::BuiltinSpace::to_xyz_matrix)
            .is_none()
        {
            return Err(DiagnosticFinding::UnsupportedProfile);
        }
        Ok(Self {
            output_dimensions,
            state,
            lower,
            upper,
            transform,
            transform_hash,
        })
    }

    #[must_use]
    pub const fn descriptor() -> DiagnosticDescriptor {
        OVEREXPOSED_DESCRIPTOR
    }

    #[must_use]
    pub const fn thresholds(&self) -> (f32, f32) {
        (self.lower, self.upper)
    }

    #[must_use]
    pub const fn state(&self) -> OverexposedState {
        self.state
    }

    #[must_use]
    pub const fn transform_hash(&self) -> [u8; 32] {
        self.transform_hash
    }

    #[must_use]
    #[allow(clippy::too_many_lines)]
    /// Executes the deterministic CPU plan, or the identical CPU fallback when
    /// the caller requests a GPU path that has no device lease at this boundary.
    ///
    /// # Panics
    ///
    /// Panics only if an internally constructed frame violates its validated
    /// dimension invariant.
    pub fn execute(
        &self,
        input: &DiagnosticFrame,
        backend: DiagnosticBackend,
        cancellation: &CancellationToken,
    ) -> OverexposedResult {
        let path = if matches!(backend, DiagnosticBackend::Cpu) {
            DiagnosticPath::Cpu
        } else {
            DiagnosticPath::CpuFallback
        };
        let receipt = || OverexposedReceipt {
            path,
            mode: self.state.mode,
            lower: self.lower,
            upper: self.upper,
            current_profile: self.transform.request().source(),
            histogram_profile: self.state.histogram_profile,
            transform_hash: self.transform_hash,
            upper_count: 0,
            lower_count: 0,
            non_finite_count: 0,
            zero_denominator_count: 0,
        };
        if input.dimensions() != self.output_dimensions {
            return passthrough(input, receipt(), DiagnosticFinding::DimensionMismatch);
        }
        if let Err(finding) = check_cancelled(cancellation) {
            return passthrough(input, receipt(), finding);
        }
        let mut output = input.pixels().to_vec();
        let mut upper_count = 0_u64;
        let mut lower_count = 0_u64;
        let mut non_finite_count = 0_u64;
        let mut zero_denominator_count = 0_u64;
        let width = self.output_dimensions.width();
        for y in 0..self.output_dimensions.height() {
            if let Err(finding) = check_cancelled(cancellation) {
                return passthrough_counts(
                    input,
                    receipt(),
                    upper_count,
                    lower_count,
                    non_finite_count,
                    zero_denominator_count,
                    finding,
                );
            }
            for x in 0..width {
                let index = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x))
                    .unwrap_or(usize::MAX);
                let source = input.pixels()[index];
                if !source[..3].iter().all(|value| value.is_finite()) {
                    non_finite_count = non_finite_count.saturating_add(1);
                    continue;
                }
                let converted =
                    match apply_profile(&self.transform, [source[0], source[1], source[2]]) {
                        Ok(value) => value,
                        Err(finding) => {
                            return passthrough_counts(
                                input,
                                receipt(),
                                upper_count,
                                lower_count,
                                non_finite_count,
                                zero_denominator_count,
                                finding,
                            );
                        }
                    };
                let luminance = match profile_luminance(self.state.histogram_profile, converted) {
                    Ok(value) => value,
                    Err(finding) => {
                        return passthrough_counts(
                            input,
                            receipt(),
                            upper_count,
                            lower_count,
                            non_finite_count,
                            zero_denominator_count,
                            finding,
                        );
                    }
                };
                let mut mark = None;
                match self.state.mode {
                    OverexposedMode::AnyRgb => {
                        if any_at_or_above(converted, self.upper) {
                            mark = Some(true);
                        } else if all_at_or_below(converted, self.lower) {
                            mark = Some(false);
                        }
                    }
                    OverexposedMode::Gamut => {
                        if luminance >= self.upper {
                            mark = Some(true);
                        } else if luminance <= self.lower {
                            mark = Some(false);
                        } else {
                            let saturation = saturation_values(
                                converted,
                                luminance,
                                &mut zero_denominator_count,
                            );
                            if saturation.iter().any(|value| *value > self.upper)
                                || any_at_or_above(converted, self.upper)
                            {
                                mark = Some(true);
                            } else if all_at_or_below(converted, self.lower) {
                                mark = Some(false);
                            }
                        }
                    }
                    OverexposedMode::Luminance => {
                        if luminance >= self.upper {
                            mark = Some(true);
                        } else if luminance <= self.lower {
                            mark = Some(false);
                        }
                    }
                    OverexposedMode::Saturation => {
                        if luminance < self.upper && luminance > self.lower {
                            let saturation = saturation_values(
                                converted,
                                luminance,
                                &mut zero_denominator_count,
                            );
                            if saturation.iter().any(|value| *value > self.upper)
                                || any_at_or_above(converted, self.upper)
                            {
                                mark = Some(true);
                            } else if all_at_or_below(converted, self.lower) {
                                mark = Some(false);
                            }
                        }
                    }
                }
                if let Some(upper) = mark {
                    if upper {
                        upper_count = upper_count.saturating_add(1);
                    } else {
                        lower_count = lower_count.saturating_add(1);
                    }
                    let color = self.state.color_scheme.colors()[usize::from(!upper)];
                    output[index][..3].copy_from_slice(&color);
                }
            }
        }
        OverexposedResult {
            frame: DiagnosticFrame::new(self.output_dimensions, output)
                .expect("validated diagnostic frame"),
            receipt: OverexposedReceipt {
                path,
                mode: self.state.mode,
                lower: self.lower,
                upper: self.upper,
                current_profile: self.transform.request().source(),
                histogram_profile: self.state.histogram_profile,
                transform_hash: self.transform_hash,
                upper_count,
                lower_count,
                non_finite_count,
                zero_denominator_count,
            },
            finding: None,
        }
    }
}

fn passthrough(
    input: &DiagnosticFrame,
    receipt: OverexposedReceipt,
    finding: DiagnosticFinding,
) -> OverexposedResult {
    OverexposedResult {
        frame: input.clone(),
        receipt,
        finding: Some(finding),
    }
}

fn passthrough_counts(
    input: &DiagnosticFrame,
    mut receipt: OverexposedReceipt,
    upper: u64,
    lower: u64,
    non_finite: u64,
    zero_denominator: u64,
    finding: DiagnosticFinding,
) -> OverexposedResult {
    receipt.upper_count = upper;
    receipt.lower_count = lower;
    receipt.non_finite_count = non_finite;
    receipt.zero_denominator_count = zero_denominator;
    passthrough(input, receipt, finding)
}

fn any_at_or_above(rgb: [f32; 3], threshold: f32) -> bool {
    rgb.iter().any(|value| *value >= threshold)
}
fn all_at_or_below(rgb: [f32; 3], threshold: f32) -> bool {
    rgb.iter().all(|value| *value <= threshold)
}

fn saturation_values(rgb: [f32; 3], luminance: f32, zero_denominator_count: &mut u64) -> [f32; 3] {
    rgb.map(|channel| {
        let denominator = luminance.mul_add(luminance, channel * channel);
        if denominator == 0.0 {
            *zero_denominator_count = zero_denominator_count.saturating_add(1);
            0.0
        } else {
            ((channel - luminance).powi(2) / denominator).sqrt()
        }
    })
}

#[allow(dead_code)]
fn _state_hash(state: OverexposedState) -> [u8; 32] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&state.lower_ev.to_bits().to_be_bytes());
    bytes.extend_from_slice(&state.upper_percent.to_bits().to_be_bytes());
    Sha256::digest(bytes).into()
}
