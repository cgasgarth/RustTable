use super::common::check_cancelled;
use super::{
    DiagnosticBackend, DiagnosticDescriptor, DiagnosticFinding, DiagnosticFrame,
    DiagnosticGeometry, DiagnosticPath,
};
use rusttable_image::{CancellationToken, CfaColor, CfaPattern, ImageDimensions, RawMosaic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RAW_DESCRIPTOR: DiagnosticDescriptor = DiagnosticDescriptor::new("rawoverexposed", 1);
const CFA_COLORS: [[f32; 3]; 4] = [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, 0.0],
];
const MAX_DIAGNOSTIC_PIXELS: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawOverlayMode {
    MarkCfa,
    MarkSolid,
    FalseColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawSolidColor {
    Red,
    Green,
    Blue,
    Black,
}

impl RawSolidColor {
    #[must_use]
    pub const fn rgba(self) -> [f32; 3] {
        CFA_COLORS[self as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RawOverexposedState {
    mode: RawOverlayMode,
    threshold: f32,
    solid_color: RawSolidColor,
}

impl RawOverexposedState {
    #[must_use]
    pub const fn new(mode: RawOverlayMode, threshold: f32, solid_color: RawSolidColor) -> Self {
        Self {
            mode,
            threshold,
            solid_color,
        }
    }

    #[must_use]
    pub const fn mode(self) -> RawOverlayMode {
        self.mode
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold
    }

    #[must_use]
    pub const fn solid_color(self) -> RawSolidColor {
        self.solid_color
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawOverexposedPlan {
    raw: RawMosaic,
    output_dimensions: ImageDimensions,
    state: RawOverexposedState,
    thresholds: [u32; 4],
    geometry: DiagnosticGeometry,
    source_identity: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawOverexposedReceipt {
    pub path: DiagnosticPath,
    pub mode: RawOverlayMode,
    pub threshold: f32,
    pub source_identity: [u8; 32],
    pub geometry_hash: [u8; 32],
    pub clipped: [u64; 4],
    pub out_of_bounds: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawOverexposedResult {
    frame: DiagnosticFrame,
    receipt: RawOverexposedReceipt,
    finding: Option<DiagnosticFinding>,
}

impl RawOverexposedResult {
    #[must_use]
    pub fn frame(&self) -> &DiagnosticFrame {
        &self.frame
    }

    #[must_use]
    pub const fn receipt(&self) -> RawOverexposedReceipt {
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

impl RawOverexposedPlan {
    /// Builds an immutable, source-bound plan. The RAW mosaic is retained by value
    /// so execution cannot observe a concurrently changed source buffer.
    /// Builds a source-bound immutable RAW diagnostic plan.
    ///
    /// # Errors
    ///
    /// Returns a typed finding when RAW metadata, thresholds, geometry, or
    /// resource limits are invalid.
    pub fn new(
        raw: RawMosaic,
        output_dimensions: ImageDimensions,
        state: RawOverexposedState,
        geometry: DiagnosticGeometry,
    ) -> Result<Self, DiagnosticFinding> {
        let pixels = output_dimensions
            .pixel_count()
            .map_err(|_| DiagnosticFinding::ResourceLimit)?;
        if pixels > MAX_DIAGNOSTIC_PIXELS {
            return Err(DiagnosticFinding::ResourceLimit);
        }
        if !state.threshold.is_finite() || !(0.0..=1.0).contains(&state.threshold) {
            return Err(DiagnosticFinding::InvalidThreshold);
        }
        if contains_fourth_bayer(&raw.pattern()) {
            return Err(DiagnosticFinding::UnsupportedCfa);
        }
        let levels = raw.levels();
        if levels.white() <= levels.black() {
            return Err(DiagnosticFinding::InvalidRawLevels);
        }
        let threshold = f64::from(state.threshold).mul_add(
            f64::from(levels.white() - levels.black()),
            f64::from(levels.black()),
        );
        if !threshold.is_finite() || threshold < 0.0 || threshold > f64::from(u16::MAX) {
            return Err(DiagnosticFinding::InvalidThreshold);
        }
        let threshold = checked_f64_to_u32(threshold);
        let source_identity = source_identity(&raw);
        Ok(Self {
            raw,
            output_dimensions,
            state,
            thresholds: [threshold; 4],
            geometry,
            source_identity,
        })
    }

    #[must_use]
    pub const fn descriptor() -> DiagnosticDescriptor {
        RAW_DESCRIPTOR
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> ImageDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn state(&self) -> RawOverexposedState {
        self.state
    }

    #[must_use]
    pub fn geometry_hash(&self) -> [u8; 32] {
        self.geometry.identity_hash()
    }

    #[must_use]
    pub const fn thresholds(&self) -> [u32; 4] {
        self.thresholds
    }

    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }

    /// Executes the canonical scalar path. WGPU is a bounded boundary today:
    /// the render crate has no device lease, so it retries the identical plan on
    /// CPU and records that fallback in the receipt.
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
    ) -> RawOverexposedResult {
        let path = if matches!(backend, DiagnosticBackend::Cpu) {
            DiagnosticPath::Cpu
        } else {
            DiagnosticPath::CpuFallback
        };
        let empty = [0_u64; 4];
        let base_receipt = || RawOverexposedReceipt {
            path,
            mode: self.state.mode,
            threshold: self.state.threshold,
            source_identity: self.source_identity,
            geometry_hash: self.geometry.identity_hash(),
            clipped: empty,
            out_of_bounds: 0,
        };
        if input.dimensions() != self.output_dimensions {
            return passthrough(input, base_receipt(), DiagnosticFinding::DimensionMismatch);
        }
        if let Err(finding) = check_cancelled(cancellation) {
            return passthrough(input, base_receipt(), finding);
        }
        let mut output = input.pixels().to_vec();
        let mut clipped = [0_u64; 4];
        let mut out_of_bounds = 0_u64;
        let width = self.output_dimensions.width();
        for y in 0..self.output_dimensions.height() {
            if let Err(finding) = check_cancelled(cancellation) {
                return passthrough_with_counts(
                    input,
                    base_receipt(),
                    clipped,
                    out_of_bounds,
                    finding,
                );
            }
            for x in 0..width {
                let index = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x))
                    .unwrap_or(usize::MAX);
                let Some((raw_x, raw_y)) = self.geometry.map(x, y).and_then(truncate_coordinate)
                else {
                    out_of_bounds = out_of_bounds.saturating_add(1);
                    continue;
                };
                if raw_x >= self.raw.dimensions().width() || raw_y >= self.raw.dimensions().height()
                {
                    out_of_bounds = out_of_bounds.saturating_add(1);
                    continue;
                }
                let color = cfa_index(self.raw.pattern().color_at(raw_x, raw_y, self.raw.phase()));
                let raw_index = usize::try_from(raw_y)
                    .ok()
                    .and_then(|row| row.checked_mul(self.raw.row_stride_samples()))
                    .and_then(|offset| {
                        usize::try_from(raw_x)
                            .ok()
                            .and_then(|column| offset.checked_add(column))
                    });
                let Some(raw_index) = raw_index else {
                    return passthrough_with_counts(
                        input,
                        base_receipt(),
                        clipped,
                        out_of_bounds,
                        DiagnosticFinding::InvalidCfa,
                    );
                };
                let Some(sample) = self.raw.samples().get(raw_index).copied() else {
                    return passthrough_with_counts(
                        input,
                        base_receipt(),
                        clipped,
                        out_of_bounds,
                        DiagnosticFinding::InvalidCfa,
                    );
                };
                if u32::from(sample) < self.thresholds[color] {
                    continue;
                }
                clipped[color] = clipped[color].saturating_add(1);
                let pixel = &mut output[index];
                match self.state.mode {
                    RawOverlayMode::MarkCfa => {
                        pixel[..3].copy_from_slice(&CFA_COLORS[color]);
                    }
                    RawOverlayMode::MarkSolid => {
                        pixel[..3].copy_from_slice(&self.state.solid_color.rgba());
                    }
                    RawOverlayMode::FalseColor => {
                        pixel[color.min(2)] = 0.0;
                    }
                }
            }
        }
        RawOverexposedResult {
            frame: DiagnosticFrame::new(self.output_dimensions, output)
                .expect("validated diagnostic frame"),
            receipt: RawOverexposedReceipt {
                path,
                mode: self.state.mode,
                threshold: self.state.threshold,
                source_identity: self.source_identity,
                geometry_hash: self.geometry.identity_hash(),
                clipped,
                out_of_bounds,
            },
            finding: None,
        }
    }
}

fn passthrough(
    input: &DiagnosticFrame,
    receipt: RawOverexposedReceipt,
    finding: DiagnosticFinding,
) -> RawOverexposedResult {
    passthrough_with_counts(
        input,
        receipt,
        receipt.clipped,
        receipt.out_of_bounds,
        finding,
    )
}

fn passthrough_with_counts(
    input: &DiagnosticFrame,
    mut receipt: RawOverexposedReceipt,
    clipped: [u64; 4],
    out_of_bounds: u64,
    finding: DiagnosticFinding,
) -> RawOverexposedResult {
    receipt.clipped = clipped;
    receipt.out_of_bounds = out_of_bounds;
    RawOverexposedResult {
        frame: input.clone(),
        receipt,
        finding: Some(finding),
    }
}

fn contains_fourth_bayer(pattern: &CfaPattern) -> bool {
    match pattern {
        CfaPattern::Bayer(pattern) => pattern
            .iter()
            .flatten()
            .any(|color| matches!(color, CfaColor::Clear)),
        CfaPattern::XTrans(_) => false,
    }
}

fn cfa_index(color: CfaColor) -> usize {
    match color {
        CfaColor::Red => 0,
        CfaColor::Green => 1,
        CfaColor::Blue => 2,
        CfaColor::Clear => 3,
    }
}

fn truncate_coordinate((x, y): (f64, f64)) -> Option<(u32, u32)> {
    if x < 0.0 || y < 0.0 || x > f64::from(u32::MAX) || y > f64::from(u32::MAX) {
        return None;
    }
    Some((checked_f64_to_u32(x.trunc()), checked_f64_to_u32(y.trunc())))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn checked_f64_to_u32(value: f64) -> u32 {
    value as u32
}

fn source_identity(raw: &RawMosaic) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(raw.dimensions().width().to_be_bytes());
    hasher.update(raw.dimensions().height().to_be_bytes());
    hasher.update(raw.row_stride_samples().to_be_bytes());
    hasher.update(raw.phase().x().to_be_bytes());
    hasher.update(raw.phase().y().to_be_bytes());
    hasher.update(raw.levels().black().to_be_bytes());
    hasher.update(raw.levels().white().to_be_bytes());
    let pattern = match raw.pattern() {
        CfaPattern::Bayer(pattern) => pattern
            .into_iter()
            .flatten()
            .map(|color| u8::try_from(cfa_index(color)).expect("four CFA slots"))
            .collect::<Vec<_>>(),
        CfaPattern::XTrans(pattern) => pattern
            .into_iter()
            .flatten()
            .map(|color| u8::try_from(cfa_index(color)).expect("four CFA slots"))
            .collect::<Vec<_>>(),
    };
    hasher.update(pattern);
    for sample in raw.samples() {
        hasher.update(sample.to_be_bytes());
    }
    hasher.finalize().into()
}
