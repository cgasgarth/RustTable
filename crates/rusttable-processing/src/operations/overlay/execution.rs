use super::{
    OverlayAlpha, OverlayAnchor, OverlayAsset, OverlayChannel, OverlayConfig, OverlayEdge,
    OverlayInterpolation, OverlayProfilePolicy,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image::DecodeLimits;
use sha2::{Digest, Sha256};
use std::fmt;

pub const OVERLAY_WGSL: &str = r"struct OverlayParams { output_width:u32, output_height:u32, asset_width:u32, asset_height:u32, opacity:f32, scale:f32, rotation:f32, x:f32, y:f32 }
@group(0) @binding(0) var<uniform> params:OverlayParams;
@group(0) @binding(1) var overlay_tex:texture_2d<f32>;
@group(0) @binding(2) var overlay_sampler:sampler;
@compute @workgroup_size(8,8,1) fn overlay_composite(@builtin(global_invocation_id) id:vec3<u32>) { if (id.x >= params.output_width || id.y >= params.output_height) { return; } }";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayReceipt {
    pub asset_identity: [u8; 32],
    pub output_dimensions: (u32, u32),
    pub scale: f32,
    pub rotation_degrees: f32,
    pub sampled_pixels: u64,
    pub pass_through: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayPlan {
    asset: OverlayAsset,
    config: OverlayConfig,
    dimensions: RasterDimensions,
    scale: f64,
    center: (f64, f64),
    identity: [u8; 32],
    receipt: OverlayReceipt,
}
impl OverlayPlan {
    pub fn new(
        asset: OverlayAsset,
        config: OverlayConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OverlayExecutionError> {
        if asset.identity() != config.asset_hash {
            return Err(OverlayExecutionError::AssetIdentityMismatch);
        }
        if matches!(config.profile, OverlayProfilePolicy::RequireEmbedded)
            && !asset.profile_present()
        {
            return Err(OverlayExecutionError::MissingProfile);
        }
        let (scale, center) = placement(&asset, &config, dimensions)?;
        let mut h = Sha256::new();
        h.update(b"rusttable.overlay.plan.v1");
        h.update(asset.identity());
        h.update(config.asset_hash);
        h.update(config.opacity.get().to_le_bytes());
        h.update(config.scale.get().to_le_bytes());
        h.update(config.rotation_degrees.get().to_le_bytes());
        h.update(dimensions.width().to_le_bytes());
        h.update(dimensions.height().to_le_bytes());
        let identity = h.finalize().into();
        Ok(Self {
            asset,
            config,
            dimensions,
            scale,
            center,
            identity,
            receipt: OverlayReceipt {
                asset_identity: config.asset_hash,
                output_dimensions: (dimensions.width(), dimensions.height()),
                scale: f32_from_f64(scale)?,
                rotation_degrees: config.rotation_degrees.get(),
                sampled_pixels: 0,
                pass_through: false,
            },
        })
    }
    pub const fn asset(&self) -> &OverlayAsset {
        &self.asset
    }
    pub const fn config(&self) -> OverlayConfig {
        self.config
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub const fn receipt(&self) -> OverlayReceipt {
        self.receipt
    }
    pub fn execute(&self, input: &[LinearRgb]) -> Result<OverlayExecution, OverlayExecutionError> {
        self.execute_with_cancel(input, || false)
    }
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<OverlayExecution, OverlayExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| OverlayExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(OverlayExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let mut output = input.to_vec();
        let mut sampled = 0_u64;
        let angle = self.config.rotation_degrees.get().to_radians();
        let (cos, sin) = (f64::from(angle.cos()), f64::from(angle.sin()));
        let aw = f64::from(self.asset.width());
        let ah = f64::from(self.asset.height());
        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| OverlayExecutionError::ArithmeticOverflow)?;
        for y in 0..self.dimensions.height() {
            if cancelled() {
                return Err(OverlayExecutionError::Cancelled);
            }
            for x in 0..self.dimensions.width() {
                let dx = f64::from(x) + 0.5 - self.center.0;
                let dy = f64::from(y) + 0.5 - self.center.1;
                let sx = (dx * cos + dy * sin) / self.scale + aw / 2.0 - 0.5;
                let sy = (-dx * sin + dy * cos) / self.scale + ah / 2.0 - 0.5;
                let Some(pixel) = sample(
                    &self.asset,
                    sx,
                    sy,
                    self.config.interpolation,
                    self.config.edge,
                )?
                else {
                    continue;
                };
                sampled = sampled.saturating_add(1);
                let alpha = f64::from(pixel[3]) * f64::from(self.config.opacity.get());
                let src = source_rgb(pixel, self.config.channel);
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(width))
                    .and_then(|row| row.checked_add(usize::try_from(x).ok()?))
                    .ok_or(OverlayExecutionError::ArithmeticOverflow)?;
                output[index] = blend(output[index], src, alpha, self.config.alpha)?;
            }
        }
        let mut receipt = self.receipt;
        receipt.sampled_pixels = sampled;
        receipt.pass_through = sampled == 0;
        Ok(OverlayExecution {
            pixels: output,
            dimensions: self.dimensions,
            identity: self.identity,
            receipt,
        })
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    identity: [u8; 32],
    receipt: OverlayReceipt,
}
impl OverlayExecution {
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub const fn receipt(&self) -> OverlayReceipt {
        self.receipt
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayExecutionError {
    AssetIdentityMismatch,
    MissingProfile,
    DimensionsMismatch { expected: usize, actual: usize },
    ArithmeticOverflow,
    Cancelled,
    UnsupportedSampling,
}
impl fmt::Display for OverlayExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "overlay execution error: {self:?}")
    }
}
impl std::error::Error for OverlayExecutionError {}
fn placement(
    asset: &OverlayAsset,
    config: &OverlayConfig,
    dimensions: RasterDimensions,
) -> Result<(f64, (f64, f64)), OverlayExecutionError> {
    let iw = f64::from(dimensions.width());
    let ih = f64::from(dimensions.height());
    let aw = f64::from(asset.width());
    let ah = f64::from(asset.height());
    let larger = iw.max(ih);
    let smaller = iw.min(ih);
    let target = match config.base_scale {
        super::OverlayBaseScale::Image => {
            if aw > ah {
                iw / aw
            } else {
                ih / ah
            }
        }
        super::OverlayBaseScale::LargerBorder => larger / aw.max(ah),
        super::OverlayBaseScale::SmallerBorder => smaller / aw.min(ah),
        super::OverlayBaseScale::MarkerHeight => ih / ah,
        super::OverlayBaseScale::Advanced => match config.image_scale {
            super::OverlayImageScale::Width => iw / aw,
            super::OverlayImageScale::Height => ih / ah,
            super::OverlayImageScale::Larger => larger / aw.max(ah),
            super::OverlayImageScale::Smaller => smaller / aw.min(ah),
        },
    };
    let scale = target * f64::from(config.scale.get());
    if !scale.is_finite() || scale <= 0.0 {
        return Err(OverlayExecutionError::ArithmeticOverflow);
    }
    let sw = aw * scale;
    let sh = ah * scale;
    let x = match config.anchor {
        OverlayAnchor::TopLeft | OverlayAnchor::Left | OverlayAnchor::BottomLeft => sw / 2.0,
        OverlayAnchor::Top | OverlayAnchor::Center | OverlayAnchor::Bottom => iw / 2.0,
        OverlayAnchor::TopRight | OverlayAnchor::Right | OverlayAnchor::BottomRight => {
            iw - sw / 2.0
        }
    };
    let y = match config.anchor {
        OverlayAnchor::TopLeft | OverlayAnchor::Top | OverlayAnchor::TopRight => sh / 2.0,
        OverlayAnchor::Left | OverlayAnchor::Center | OverlayAnchor::Right => ih / 2.0,
        OverlayAnchor::BottomLeft | OverlayAnchor::Bottom | OverlayAnchor::BottomRight => {
            ih - sh / 2.0
        }
    };
    Ok((
        scale,
        (
            x + f64::from(config.xoffset.get()) * iw,
            y + f64::from(config.yoffset.get()) * ih,
        ),
    ))
}
fn source_rgb(p: [f32; 4], c: OverlayChannel) -> [f32; 3] {
    match c {
        OverlayChannel::Rgb => [p[0], p[1], p[2]],
        OverlayChannel::Red => [p[0]; 3],
        OverlayChannel::Green => [p[1]; 3],
        OverlayChannel::Blue => [p[2]; 3],
        OverlayChannel::Alpha => [p[3]; 3],
    }
}
fn blend(
    old: LinearRgb,
    src: [f32; 3],
    alpha: f64,
    a: OverlayAlpha,
) -> Result<LinearRgb, OverlayExecutionError> {
    let alpha = f32_from_f64(alpha.clamp(0.0, 1.0))?;
    let old = [old.red().get(), old.green().get(), old.blue().get()];
    let mut out = [0.0; 3];
    for i in 0..3 {
        let mut s = srgb_to_linear(src[i]);
        if matches!(a, OverlayAlpha::Premultiplied) && alpha > 0.0 {
            s /= alpha;
        }
        out[i] = old[i] * (1.0 - alpha) + s * alpha;
        if !out[i].is_finite() {
            return Err(OverlayExecutionError::UnsupportedSampling);
        }
    }
    Ok(LinearRgb::new(
        FiniteF32::new(out[0]).map_err(|_| OverlayExecutionError::UnsupportedSampling)?,
        FiniteF32::new(out[1]).map_err(|_| OverlayExecutionError::UnsupportedSampling)?,
        FiniteF32::new(out[2]).map_err(|_| OverlayExecutionError::UnsupportedSampling)?,
    ))
}
fn sample(
    asset: &OverlayAsset,
    x: f64,
    y: f64,
    interpolation: OverlayInterpolation,
    edge: OverlayEdge,
) -> Result<Option<[f32; 4]>, OverlayExecutionError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(OverlayExecutionError::UnsupportedSampling);
    }
    match interpolation {
        OverlayInterpolation::Nearest => {
            pixel(asset, checked_coordinate(x)?, checked_coordinate(y)?, edge)
        }
        OverlayInterpolation::Bilinear => bilinear(asset, x, y, edge),
        OverlayInterpolation::Bicubic => bicubic(asset, x, y, edge),
    }
}
fn bilinear(
    asset: &OverlayAsset,
    x: f64,
    y: f64,
    edge: OverlayEdge,
) -> Result<Option<[f32; 4]>, OverlayExecutionError> {
    let x0 = checked_coordinate(x.floor())?;
    let y0 = checked_coordinate(y.floor())?;
    let fx = f32_from_f64(x - f64_from_i64(x0))?;
    let fy = f32_from_f64(y - f64_from_i64(y0))?;
    let mut out = [0.0; 4];
    for dy in 0..2 {
        for dx in 0..2 {
            let Some(p) = pixel(asset, x0 + dx, y0 + dy, edge)? else {
                continue;
            };
            let w = if dx == 0 { 1.0 - fx } else { fx } * if dy == 0 { 1.0 - fy } else { fy };
            for c in 0..4 {
                out[c] += p[c] * w;
            }
        }
    }
    if matches!(edge, OverlayEdge::Transparent)
        && (x < 0.0 || y < 0.0 || x >= f64::from(asset.width()) || y >= f64::from(asset.height()))
    {
        return Ok(None);
    }
    Ok(Some(out))
}
fn bicubic(
    asset: &OverlayAsset,
    x: f64,
    y: f64,
    edge: OverlayEdge,
) -> Result<Option<[f32; 4]>, OverlayExecutionError> {
    let x0 = checked_coordinate(x.floor())?;
    let y0 = checked_coordinate(y.floor())?;
    let mut out = [0.0; 4];
    for j in -1..=2 {
        for i in -1..=2 {
            let Some(p) = pixel(asset, x0 + i, y0 + j, edge)? else {
                continue;
            };
            let w = cubic(x - f64_from_i64(x0) - f64_from_i64(i))
                * cubic(y - f64_from_i64(y0) - f64_from_i64(j));
            for c in 0..4 {
                out[c] += p[c] * f32_from_f64(w)?;
            }
        }
    }
    Ok(Some(out))
}
fn cubic(v: f64) -> f64 {
    let x = v.abs();
    if x <= 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}
fn pixel(
    asset: &OverlayAsset,
    x: i64,
    y: i64,
    edge: OverlayEdge,
) -> Result<Option<[f32; 4]>, OverlayExecutionError> {
    let (x, y) = match edge {
        OverlayEdge::Transparent => {
            if x < 0 || y < 0 || x >= i64::from(asset.width()) || y >= i64::from(asset.height()) {
                return Ok(None);
            }
            (x, y)
        }
        OverlayEdge::Clamp => (
            x.clamp(0, i64::from(asset.width()) - 1),
            y.clamp(0, i64::from(asset.height()) - 1),
        ),
        OverlayEdge::Repeat => (
            x.rem_euclid(i64::from(asset.width())),
            y.rem_euclid(i64::from(asset.height())),
        ),
        OverlayEdge::Mirror => (mirror(x, asset.width()), mirror(y, asset.height())),
    };
    let index = usize::try_from(y)
        .ok()
        .and_then(|v| v.checked_mul(usize::try_from(asset.width()).ok()?))
        .and_then(|v| v.checked_add(usize::try_from(x).ok()?))
        .and_then(|v| v.checked_mul(4))
        .ok_or(OverlayExecutionError::ArithmeticOverflow)?;
    let b = &asset.pixels()[index..index + 4];
    Ok(Some([
        f32::from(b[0]) / 255.0,
        f32::from(b[1]) / 255.0,
        f32::from(b[2]) / 255.0,
        f32::from(b[3]) / 255.0,
    ]))
}
fn checked_coordinate(value: f64) -> Result<i64, OverlayExecutionError> {
    if !value.is_finite()
        || value < -4_611_686_018_427_387_904.0
        || value > 4_611_686_018_427_387_904.0
    {
        return Err(OverlayExecutionError::UnsupportedSampling);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "range is checked immediately above"
    )]
    Ok(value as i64)
}
#[expect(
    clippy::cast_precision_loss,
    reason = "sampling coordinates are bounded to the exact f64 integer range"
)]
fn f64_from_i64(value: i64) -> f64 {
    value as f64
}
#[expect(
    clippy::cast_possible_truncation,
    reason = "finite sampling values are checked against f32 bounds"
)]
fn f32_from_f64(value: f64) -> Result<f32, OverlayExecutionError> {
    if !value.is_finite() || value < -f64::from(f32::MAX) || value > f64::from(f32::MAX) {
        return Err(OverlayExecutionError::UnsupportedSampling);
    }
    Ok(value as f32)
}
fn mirror(v: i64, n: u32) -> i64 {
    let n = i64::from(n);
    let p = (n * 2).max(1);
    let v = v.rem_euclid(p);
    if v >= n { p - v - 1 } else { v }
}
fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
/// # Panics
///
/// Panics only if the fixed decoder limits are invalid.
pub fn default_asset_limits() -> DecodeLimits {
    DecodeLimits::new(
        64 * 1024 * 1024,
        16_384,
        16_384,
        16_384 * 16_384,
        256 * 1024 * 1024,
    )
    .expect("overlay limits")
}
