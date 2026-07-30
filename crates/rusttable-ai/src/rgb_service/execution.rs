use std::fmt;

use sha2::{Digest, Sha256};

use crate::{CancellationToken, Provider, ProviderPolicy};

use super::detail::{DetailError, recover_detail};
use super::image::RgbAiImage;
use super::plan::{RgbAiPlan, RgbAiPlanError, RgbAiTile, extended_srgb_decode};
use super::receipt::RgbAiReceipt;
use super::tensor::RgbAiTileInput;

/// Provider errors are intentionally bounded and contain no native diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Unavailable,
    Execution { code: &'static str },
    Cancelled,
    InvalidOutput,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RGB AI provider failed: {self:?}")
    }
}
impl std::error::Error for ProviderError {}

pub trait RgbAiProvider: Send + Sync {
    fn supports(&self, provider: Provider) -> bool;
    fn infer(
        &self,
        provider: Provider,
        input: &RgbAiTileInput,
        cancellation: &CancellationToken,
    ) -> Result<RgbAiTileOutput, ProviderError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiTileOutput {
    width: u32,
    height: u32,
    nchw_rgb: Vec<f32>,
}

impl RgbAiTileOutput {
    pub fn new(width: u32, height: u32, nchw_rgb: Vec<f32>) -> Result<Self, ProviderError> {
        let plane = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(ProviderError::InvalidOutput)?;
        if nchw_rgb.len() != plane.checked_mul(3).ok_or(ProviderError::InvalidOutput)?
            || nchw_rgb.iter().any(|value| !value.is_finite())
        {
            return Err(ProviderError::InvalidOutput);
        }
        Ok(Self {
            width,
            height,
            nchw_rgb,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub fn nchw_rgb(&self) -> &[f32] {
        &self.nchw_rgb
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiOutput {
    dimensions: crate::ImageDimensions,
    profile: rusttable_color::ColorEncoding,
    pixels: Vec<[f32; 4]>,
    detail_residual: Vec<f32>,
    receipt: RgbAiReceipt,
}

impl RgbAiOutput {
    #[must_use]
    pub const fn dimensions(&self) -> crate::ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn profile(&self) -> rusttable_color::ColorEncoding {
        self.profile
    }
    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }
    #[must_use]
    pub fn detail_residual(&self) -> &[f32] {
        &self.detail_residual
    }
    #[must_use]
    pub const fn receipt(&self) -> &RgbAiReceipt {
        &self.receipt
    }
}

pub trait RgbAiPublication {
    fn begin(&mut self, plan: &RgbAiPlan) -> Result<(), RgbAiPublicationError>;
    fn publish_rows(&mut self, y: u32, rows: &[[f32; 4]]) -> Result<(), RgbAiPublicationError>;
    fn finish(
        &mut self,
        output: &RgbAiOutput,
    ) -> Result<RgbAiPublicationReceipt, RgbAiPublicationError>;
    fn discard(&mut self);
    fn rows_committed(&self) -> u32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbAiPublicationReceipt {
    pub rows: u32,
    pub output_identity: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbAiPublicationError {
    Rejected { code: &'static str },
    Cancelled,
}

impl fmt::Display for RgbAiPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RGB AI publication failed: {self:?}")
    }
}
impl std::error::Error for RgbAiPublicationError {}

#[derive(Clone, Copy)]
pub struct RgbAiExecutor<'a> {
    provider: &'a dyn RgbAiProvider,
}

impl<'a> RgbAiExecutor<'a> {
    #[must_use]
    pub const fn new(provider: &'a dyn RgbAiProvider) -> Self {
        Self { provider }
    }

    pub fn run(
        &self,
        plan: &RgbAiPlan,
        source: &RgbAiImage,
        cancellation: &CancellationToken,
    ) -> Result<RgbAiOutput, RgbAiExecutionError> {
        self.run_internal(plan, source, cancellation)
    }

    pub fn run_published(
        &self,
        plan: &RgbAiPlan,
        source: &RgbAiImage,
        cancellation: &CancellationToken,
        publication: &mut dyn RgbAiPublication,
    ) -> Result<(RgbAiOutput, RgbAiPublicationReceipt), RgbAiExecutionError> {
        let output = match self.run_internal(plan, source, cancellation) {
            Ok(output) => output,
            Err(error) => {
                publication.discard();
                return Err(error);
            }
        };
        if cancellation.is_cancelled() {
            publication.discard();
            return Err(RgbAiExecutionError::Cancelled);
        }
        publication.begin(plan).map_err(|error| {
            publication.discard();
            RgbAiExecutionError::Publication(error)
        })?;
        let height = output.dimensions.height();
        let width = usize::try_from(output.dimensions.width())
            .map_err(|_| RgbAiExecutionError::ArithmeticOverflow)?;
        for y in 0..height {
            if cancellation.is_cancelled() {
                publication.discard();
                return Err(RgbAiExecutionError::Cancelled);
            }
            let start = usize::try_from(y)
                .map_err(|_| RgbAiExecutionError::ArithmeticOverflow)?
                .checked_mul(width)
                .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
            let end = start
                .checked_add(width)
                .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
            publication
                .publish_rows(y, &output.pixels[start..end])
                .map_err(|error| {
                    publication.discard();
                    RgbAiExecutionError::Publication(error)
                })?;
        }
        let receipt = publication.finish(&output).map_err(|error| {
            publication.discard();
            RgbAiExecutionError::Publication(error)
        })?;
        Ok((output, receipt))
    }

    fn run_internal(
        &self,
        plan: &RgbAiPlan,
        source: &RgbAiImage,
        cancellation: &CancellationToken,
    ) -> Result<RgbAiOutput, RgbAiExecutionError> {
        if source.identity() != plan.source_identity()
            || source.dimensions() != plan.source_dimensions()
        {
            return Err(RgbAiExecutionError::SourcePlanMismatch);
        }
        if cancellation.is_cancelled() {
            return Err(RgbAiExecutionError::Cancelled);
        }
        let mut provider = plan.initial_provider();
        let mut retried = false;
        let staged = loop {
            if !self.provider.supports(provider) {
                if matches!(plan.provider_policy(), ProviderPolicy::Auto)
                    && provider != Provider::Cpu
                    && !retried
                {
                    provider = Provider::Cpu;
                    retried = true;
                    continue;
                }
                return Err(RgbAiExecutionError::Provider(ProviderError::Unavailable));
            }
            match self.infer_all(plan, source, provider, cancellation) {
                Ok(staged) => break staged,
                Err(RgbAiExecutionError::Provider(error))
                    if matches!(plan.provider_policy(), ProviderPolicy::Auto)
                        && provider != Provider::Cpu
                        && !retried
                        && !matches!(error, ProviderError::Cancelled) =>
                {
                    provider = Provider::Cpu;
                    retried = true;
                }
                Err(error) => return Err(error),
            }
        };
        if cancellation.is_cancelled() {
            return Err(RgbAiExecutionError::Cancelled);
        }
        let source_model = model_pixels(plan, source, cancellation)?;
        let mut pixels = staged.pixels;
        if plan.gamut().enabled() && plan.scale() == 1 {
            preserve_gamut(plan, &source_model, &mut pixels);
        }
        let (detail_residual, detail_receipt) = if let Some(detail) = plan.detail() {
            let (residual, receipt) = recover_detail(
                &source_model,
                &mut pixels,
                plan.output_dimensions().width(),
                plan.output_dimensions().height(),
                detail,
                || cancellation.is_cancelled(),
            )
            .map_err(RgbAiExecutionError::Detail)?;
            (residual, Some(receipt))
        } else {
            (Vec::new(), None)
        };
        if cancellation.is_cancelled() {
            return Err(RgbAiExecutionError::Cancelled);
        }
        apply_output_transform(plan, &mut pixels, cancellation)?;
        let plan_hash = plan.plan_hash().map_err(RgbAiExecutionError::Plan)?;
        let output_identity = output_identity(plan_hash, &pixels);
        let color = plan.color_receipt().map_err(RgbAiExecutionError::Plan)?;
        let receipt = RgbAiReceipt {
            plan_hash,
            source_identity: plan.source_identity(),
            model_hash: plan.model_hash(),
            provider,
            output_identity,
            tiles: plan.tiles_receipt(),
            memory: plan.memory(),
            color,
            detail: detail_receipt,
        };
        Ok(RgbAiOutput {
            dimensions: plan.output_dimensions(),
            profile: plan.output_profile(),
            pixels,
            detail_residual,
            receipt,
        })
    }

    fn infer_all(
        &self,
        plan: &RgbAiPlan,
        source: &RgbAiImage,
        provider: Provider,
        cancellation: &CancellationToken,
    ) -> Result<StagedImage, RgbAiExecutionError> {
        let output_pixels = plan
            .output_dimensions()
            .pixels()
            .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
        let mut output = vec![[0.0; 4]; output_pixels];
        for tile in plan.tiles() {
            if cancellation.is_cancelled() {
                return Err(RgbAiExecutionError::Cancelled);
            }
            let input = plan
                .make_tile_input(source, *tile, cancellation)
                .map_err(RgbAiExecutionError::Plan)?;
            let tile_output = self
                .provider
                .infer(provider, &input, cancellation)
                .map_err(RgbAiExecutionError::Provider)?;
            assemble_tile(plan, source, *tile, &tile_output, &mut output)?;
        }
        Ok(StagedImage { pixels: output })
    }
}

#[derive(Debug)]
struct StagedImage {
    pixels: Vec<[f32; 4]>,
}

fn assemble_tile(
    plan: &RgbAiPlan,
    source: &RgbAiImage,
    tile: RgbAiTile,
    tile_output: &RgbAiTileOutput,
    destination: &mut [[f32; 4]],
) -> Result<(), RgbAiExecutionError> {
    let scale = plan.scale();
    let expected_width = tile
        .input_dimensions()
        .0
        .checked_mul(scale)
        .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
    let expected_height = tile
        .input_dimensions()
        .1
        .checked_mul(scale)
        .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
    if tile_output.width() != expected_width || tile_output.height() != expected_height {
        return Err(RgbAiExecutionError::Provider(ProviderError::InvalidOutput));
    }
    let tile_plane = usize::try_from(tile_output.width())
        .unwrap()
        .checked_mul(usize::try_from(tile_output.height()).unwrap())
        .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
    let crop = plan.valid_crop();
    for local_y in 0..tile.output_dimensions().1 {
        for local_x in 0..tile.output_dimensions().0 {
            let output_x = tile.output_origin().0 + local_x;
            let output_y = tile.output_origin().1 + local_y;
            let index = usize::try_from(output_y)
                .unwrap()
                .checked_mul(usize::try_from(plan.output_dimensions().width()).unwrap())
                .and_then(|value| value.checked_add(usize::try_from(output_x).unwrap()))
                .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
            let model_x = crop.left.checked_mul(scale).unwrap() + local_x;
            let model_y = crop.top.checked_mul(scale).unwrap() + local_y;
            let model_index = usize::try_from(model_y)
                .unwrap()
                .checked_mul(usize::try_from(tile_output.width()).unwrap())
                .and_then(|value| value.checked_add(usize::try_from(model_x).unwrap()))
                .ok_or(RgbAiExecutionError::ArithmeticOverflow)?;
            let mut rgb = [
                extended_srgb_decode(tile_output.nchw_rgb()[model_index])
                    .map_err(|_| RgbAiExecutionError::NonFiniteOutput)?,
                extended_srgb_decode(tile_output.nchw_rgb()[tile_plane + model_index])
                    .map_err(|_| RgbAiExecutionError::NonFiniteOutput)?,
                extended_srgb_decode(tile_output.nchw_rgb()[2 * tile_plane + model_index])
                    .map_err(|_| RgbAiExecutionError::NonFiniteOutput)?,
            ];
            if plan.shadow().enabled() {
                for channel in &mut rgb {
                    *channel *= *channel;
                }
            }
            let source_x = (output_x / scale).min(source.dimensions().width() - 1);
            let source_y = (output_y / scale).min(source.dimensions().height() - 1);
            let source_pixel = source.pixels()[usize::try_from(source_y).unwrap()
                * source.dimensions().width() as usize
                + usize::try_from(source_x).unwrap()];
            destination[index] = [
                rgb[0],
                rgb[1],
                rgb[2],
                match plan.alpha() {
                    crate::AlphaPolicy::Opaque => 1.0,
                    crate::AlphaPolicy::PreserveNearest => source_pixel[3],
                },
            ];
        }
    }
    Ok(())
}

fn model_pixels(
    plan: &RgbAiPlan,
    source: &RgbAiImage,
    cancellation: &CancellationToken,
) -> Result<Vec<[f32; 4]>, RgbAiExecutionError> {
    source
        .pixels()
        .iter()
        .map(|pixel| {
            if cancellation.is_cancelled() {
                return Err(RgbAiExecutionError::Cancelled);
            }
            let rgb = plan
                .input_transform()
                .apply_rgb([pixel[0], pixel[1], pixel[2]], || {
                    cancellation.is_cancelled()
                })
                .map_err(|_| RgbAiExecutionError::NonFiniteOutput)?;
            Ok([rgb[0], rgb[1], rgb[2], pixel[3]])
        })
        .collect()
}

fn apply_output_transform(
    plan: &RgbAiPlan,
    pixels: &mut [[f32; 4]],
    cancellation: &CancellationToken,
) -> Result<(), RgbAiExecutionError> {
    for pixel in pixels {
        if cancellation.is_cancelled() {
            return Err(RgbAiExecutionError::Cancelled);
        }
        let rgb = plan
            .output_transform()
            .apply_rgb([pixel[0], pixel[1], pixel[2]], || {
                cancellation.is_cancelled()
            })
            .map_err(|_| RgbAiExecutionError::NonFiniteOutput)?;
        pixel[..3].copy_from_slice(&rgb);
        if pixel.iter().any(|value| !value.is_finite()) {
            return Err(RgbAiExecutionError::NonFiniteOutput);
        }
    }
    Ok(())
}

fn preserve_gamut(plan: &RgbAiPlan, source: &[[f32; 4]], output: &mut [[f32; 4]]) {
    let width = usize::try_from(plan.source_dimensions().width()).unwrap_or(0);
    let height = usize::try_from(plan.source_dimensions().height()).unwrap_or(0);
    let mask = plan.gamut_mask();
    let luminance = |pixel: &[f32; 4]| 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
    for (index, in_gamut) in mask.iter().copied().enumerate() {
        if !in_gamut {
            output[index][..3].copy_from_slice(&source[index][..3]);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if mask[index] {
                continue;
            }
            let mut sum = 0.0;
            let mut count = 0_u32;
            for neighbor_y in y.saturating_sub(2)..=(y + 2).min(height - 1) {
                for neighbor_x in x.saturating_sub(2)..=(x + 2).min(width - 1) {
                    let neighbor = neighbor_y * width + neighbor_x;
                    if mask[neighbor] {
                        sum += luminance(&output[neighbor]);
                        count += 1;
                    }
                }
            }
            if count > 0 {
                let delta = sum / count as f32 - luminance(&source[index]);
                for channel in &mut output[index][..3] {
                    *channel += delta;
                }
            }
        }
    }
}

fn output_identity(plan_hash: [u8; 32], pixels: &[[f32; 4]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable-ai-rgb-output-v1");
    hasher.update(plan_hash);
    for pixel in pixels {
        for value in pixel {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, PartialEq)]
pub enum RgbAiExecutionError {
    SourcePlanMismatch,
    Provider(ProviderError),
    Detail(DetailError),
    Plan(RgbAiPlanError),
    Publication(RgbAiPublicationError),
    NonFiniteOutput,
    ArithmeticOverflow,
    Cancelled,
}

impl fmt::Display for RgbAiExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RGB AI execution failed: {self:?}")
    }
}
impl std::error::Error for RgbAiExecutionError {}
