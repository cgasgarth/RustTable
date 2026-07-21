#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::needless_range_loop,
    clippy::unnecessary_wraps
)]

use super::{WatermarkAnchor, WatermarkParametersV7, WatermarkScaleMode};
use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image_io::{ManagedSvgAsset, SvgError, SvgLimits};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatermarkReceipt {
    template_hash: [u8; 32],
    expanded_tree_hash: [u8; 32],
    context_hash: [u8; 32],
    raster_dimensions: (u32, u32),
    findings: Vec<String>,
}

impl WatermarkReceipt {
    #[must_use]
    pub const fn template_hash(&self) -> [u8; 32] {
        self.template_hash
    }
    #[must_use]
    pub const fn expanded_tree_hash(&self) -> [u8; 32] {
        self.expanded_tree_hash
    }
    #[must_use]
    pub const fn context_hash(&self) -> [u8; 32] {
        self.context_hash
    }
    #[must_use]
    pub const fn raster_dimensions(&self) -> (u32, u32) {
        self.raster_dimensions
    }
    #[must_use]
    pub fn findings(&self) -> &[String] {
        &self.findings
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatermarkExecutionError {
    Svg(SvgError),
    TemplateMismatch,
    DimensionMismatch,
    PixelBufferMismatch,
    NonFiniteOutput,
}

impl std::fmt::Display for WatermarkExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Svg(error) => error.fmt(f),
            Self::TemplateMismatch => {
                f.write_str("watermark template identity does not match the managed asset")
            }
            Self::DimensionMismatch => {
                f.write_str("watermark dimensions do not match the working image")
            }
            Self::PixelBufferMismatch => {
                f.write_str("watermark pixel buffer does not match the image dimensions")
            }
            Self::NonFiniteOutput => {
                f.write_str("watermark compositing produced a non-finite pixel")
            }
        }
    }
}
impl std::error::Error for WatermarkExecutionError {}
impl From<SvgError> for WatermarkExecutionError {
    fn from(error: SvgError) -> Self {
        Self::Svg(error)
    }
}

#[derive(Debug, Clone)]
pub struct WatermarkPlan {
    parameters: WatermarkParametersV7,
    raster: rusttable_image_io::SvgRaster,
    receipt: WatermarkReceipt,
    position: (f32, f32),
    rotation_radians: f32,
}

impl WatermarkPlan {
    pub fn new(
        asset: &ManagedSvgAsset,
        parameters: WatermarkParametersV7,
        context: &super::WatermarkContext,
        dimensions: RasterDimensions,
    ) -> Result<Self, WatermarkExecutionError> {
        if parameters.template_hash() != asset.source_hash() {
            return Err(WatermarkExecutionError::TemplateMismatch);
        }
        let expanded = if parameters.expand_variables() {
            context
                .expand(asset.source_bytes())
                .map_err(|error| WatermarkExecutionError::Svg(SvgError::Parse(error.to_string())))?
        } else {
            super::ExpandedWatermark {
                bytes: asset.source_bytes().to_vec(),
                hash: asset.source_hash(),
                findings: Vec::new(),
            }
        };
        let expanded_asset =
            ManagedSvgAsset::parse(expanded.bytes().to_vec(), SvgLimits::default())?;
        let (width, height) = raster_dimensions(
            expanded_asset.size().width(),
            expanded_asset.size().height(),
            parameters.scale(),
            parameters.scale_mode(),
            dimensions,
        )?;
        let raster = expanded_asset.rasterize(width, height)?;
        let position = anchor_position(
            parameters.anchor(),
            dimensions,
            (width, height),
            parameters.x_offset(),
            parameters.y_offset(),
        );
        let rotation_radians = parameters.rotation().to_radians();
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.watermark.plan.v1");
        hasher.update(parameters.cache_identity());
        hasher.update(expanded_asset.tree_hash());
        hasher.update(context.rendered_hash());
        let _plan_identity: [u8; 32] = hasher.finalize().into();
        Ok(Self {
            parameters,
            raster,
            receipt: WatermarkReceipt {
                template_hash: asset.source_hash(),
                expanded_tree_hash: expanded_asset.tree_hash(),
                context_hash: context.rendered_hash(),
                raster_dimensions: (width, height),
                findings: expanded.findings().to_vec(),
            },
            position,
            rotation_radians,
        })
    }

    #[must_use]
    pub const fn receipt(&self) -> &WatermarkReceipt {
        &self.receipt
    }

    pub fn execute(
        &self,
        pixels: &mut [LinearRgb],
        dimensions: RasterDimensions,
    ) -> Result<(), WatermarkExecutionError> {
        if pixels.len() as u64 != dimensions.pixel_count() {
            return Err(WatermarkExecutionError::PixelBufferMismatch);
        }
        if self.parameters.opacity().to_bits() == 0
            || self.parameters.scale().to_bits() == 0
            || self.raster.width() == 0
            || self.raster.height() == 0
        {
            return Ok(());
        }
        let width = usize::try_from(dimensions.width())
            .map_err(|_| WatermarkExecutionError::DimensionMismatch)?;
        let sin = self.rotation_radians.sin();
        let cos = self.rotation_radians.cos();
        for (index, pixel) in pixels.iter_mut().enumerate() {
            let x = (index % width) as f32;
            let y = (index / width) as f32;
            let sampled = self.sample(x, y, sin, cos);
            let Some((red, green, blue, source_alpha)) = sampled else {
                continue;
            };
            let alpha = source_alpha * self.parameters.opacity() * self.parameters.color()[3];
            if alpha <= 0.0 {
                continue;
            }
            let color = self.parameters.color();
            let source = [
                decode_premultiplied(red, source_alpha) * color[0],
                decode_premultiplied(green, source_alpha) * color[1],
                decode_premultiplied(blue, source_alpha) * color[2],
            ];
            let output = [
                pixel.red().get() * (1.0 - alpha) + source[0] * alpha,
                pixel.green().get() * (1.0 - alpha) + source[1] * alpha,
                pixel.blue().get() * (1.0 - alpha) + source[2] * alpha,
            ];
            if output.iter().any(|value| !value.is_finite()) {
                return Err(WatermarkExecutionError::NonFiniteOutput);
            }
            *pixel = LinearRgb::new(
                FiniteF32::new(output[0]).map_err(|_| WatermarkExecutionError::NonFiniteOutput)?,
                FiniteF32::new(output[1]).map_err(|_| WatermarkExecutionError::NonFiniteOutput)?,
                FiniteF32::new(output[2]).map_err(|_| WatermarkExecutionError::NonFiniteOutput)?,
            );
        }
        Ok(())
    }

    fn sample(&self, x: f32, y: f32, sin: f32, cos: f32) -> Option<(f32, f32, f32, f32)> {
        let width = self.raster.width() as f32;
        let height = self.raster.height() as f32;
        let center_x = self.position.0 + width / 2.0;
        let center_y = self.position.1 + height / 2.0;
        let dx = x - center_x;
        let dy = y - center_y;
        let local_x = dx * cos + dy * sin + width / 2.0;
        let local_y = -dx * sin + dy * cos + height / 2.0;
        if !(0.0..width).contains(&local_x) || !(0.0..height).contains(&local_y) {
            return None;
        }
        let x0 = local_x.floor() as i32;
        let y0 = local_y.floor() as i32;
        let tx = local_x - x0 as f32;
        let ty = local_y - y0 as f32;
        let mut sample = [0.0; 4];
        for (offset_y, weight_y) in [(0, 1.0 - ty), (1, ty)] {
            for (offset_x, weight_x) in [(0, 1.0 - tx), (1, tx)] {
                let px = x0 + offset_x;
                let py = y0 + offset_y;
                if px < 0
                    || py < 0
                    || px >= self.raster.width() as i32
                    || py >= self.raster.height() as i32
                {
                    continue;
                }
                let index = (py as usize * self.raster.width() as usize + px as usize) * 4;
                let weight = weight_x * weight_y;
                for channel in 0..4 {
                    sample[channel] +=
                        f32::from(self.raster.pixels()[index + channel]) / 255.0 * weight;
                }
            }
        }
        Some((sample[0], sample[1], sample[2], sample[3]))
    }
}

fn raster_dimensions(
    source_width: f32,
    source_height: f32,
    scale: f32,
    mode: WatermarkScaleMode,
    image: RasterDimensions,
) -> Result<(u32, u32), WatermarkExecutionError> {
    let source_ratio = source_width / source_height;
    let (width, height) = match mode {
        WatermarkScaleMode::Width => (
            image.width() as f32 * scale,
            image.width() as f32 * scale / source_ratio,
        ),
        WatermarkScaleMode::Height => (
            image.height() as f32 * scale * source_ratio,
            image.height() as f32 * scale,
        ),
        WatermarkScaleMode::Fit => {
            let edge = image.width().min(image.height()) as f32 * scale;
            (edge * source_ratio, edge)
        }
    };
    let width = width.round().clamp(0.0, 16_384.0) as u32;
    let height = height.round().clamp(0.0, 16_384.0) as u32;
    if width == 0 || height == 0 {
        return Ok((1, 1));
    }
    Ok((width, height))
}

fn anchor_position(
    anchor: WatermarkAnchor,
    image: RasterDimensions,
    raster: (u32, u32),
    x: f32,
    y: f32,
) -> (f32, f32) {
    let free_x = image.width() as f32 - raster.0 as f32;
    let free_y = image.height() as f32 - raster.1 as f32;
    let (base_x, base_y) = match anchor {
        WatermarkAnchor::TopLeft => (0.0, 0.0),
        WatermarkAnchor::Top => (free_x / 2.0, 0.0),
        WatermarkAnchor::TopRight => (free_x, 0.0),
        WatermarkAnchor::Left => (0.0, free_y / 2.0),
        WatermarkAnchor::Center => (free_x / 2.0, free_y / 2.0),
        WatermarkAnchor::Right => (free_x, free_y / 2.0),
        WatermarkAnchor::BottomLeft => (0.0, free_y),
        WatermarkAnchor::Bottom => (free_x / 2.0, free_y),
        WatermarkAnchor::BottomRight => (free_x, free_y),
    };
    (base_x + x, base_y + y)
}

fn decode_premultiplied(channel: f32, alpha: f32) -> f32 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let encoded = (channel / alpha).clamp(0.0, 1.0);
    if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}
