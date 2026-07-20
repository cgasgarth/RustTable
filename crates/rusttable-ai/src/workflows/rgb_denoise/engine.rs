#![expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "the processing seam mirrors Darktable's fixed tile and pixel policies"
)]

use rusttable_pixelpipe::{RgbaF32Image, RgbaF32Pixel};
use sha2::{Digest, Sha256};

use super::detail::{DetailError, apply_detail_recovery};
use super::ports::{ModelTile, RgbDenoiseControl, RgbDenoiseModel, RgbDenoiseObserver};
use super::{
    CollisionPolicy, ModelDescriptor, ModelError, ModelTask, ProviderSelection, ProviderUsed,
    RgbDenoisePlan, RgbDenoiseProgress, RgbDenoiseRequest, RgbDenoiseStage,
};

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn luma(pixel: &[f32; 4]) -> f32 {
    LUMA[0] * pixel[0] + LUMA[1] * pixel[1] + LUMA[2] * pixel[2]
}

pub struct ProcessedImage {
    pub pixels: Vec<[f32; 4]>,
    pub plan: RgbDenoisePlan,
    pub provider: ProviderUsed,
    pub shadow_boost: bool,
    pub artifact_key: [u8; 32],
}

pub fn process(
    request: &RgbDenoiseRequest,
    model: &dyn RgbDenoiseModel,
    observer: &dyn RgbDenoiseObserver,
    control: &dyn RgbDenoiseControl,
) -> Result<ProcessedImage, ProcessError> {
    validate_model(model.descriptor())?;
    let dimensions = request.input().descriptor().dimensions();
    let width = usize::try_from(dimensions.width()).map_err(|_| ProcessError::DimensionOverflow)?;
    let height =
        usize::try_from(dimensions.height()).map_err(|_| ProcessError::DimensionOverflow)?;
    let tile_size = usize::try_from(model.descriptor().tile_size())
        .map_err(|_| ProcessError::DimensionOverflow)?;
    let overlap = usize::try_from(model.descriptor().overlap())
        .map_err(|_| ProcessError::DimensionOverflow)?;
    let step = tile_size
        .checked_sub(2 * overlap)
        .ok_or(ProcessError::InvalidTilePlan)?;
    let columns = width.div_ceil(step);
    let rows = height.div_ceil(step);
    let tile_count = u64::try_from(
        columns
            .checked_mul(rows)
            .ok_or(ProcessError::TileCountOverflow)?,
    )
    .map_err(|_| ProcessError::TileCountOverflow)?;
    let plan = RgbDenoisePlan {
        dimensions: rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
            .map_err(|_| ProcessError::DimensionOverflow)?,
        tile_count,
        detail_recovery_strength: request.strength().detail_recovery_strength(),
        preserve_wide_gamut: request.preserve_wide_gamut(),
    };
    observer.progress(RgbDenoiseProgress {
        stage: RgbDenoiseStage::Validate,
        completed: 1,
        total: 1,
    });
    if control.is_cancelled(RgbDenoiseStage::RenderSnapshot) {
        return Err(ProcessError::Cancelled(RgbDenoiseStage::RenderSnapshot));
    }
    observer.progress(RgbDenoiseProgress {
        stage: RgbDenoiseStage::RenderSnapshot,
        completed: 1,
        total: 1,
    });

    let shadow_boost = model.descriptor().shadow_boost() && has_deep_shadows(request.input());
    let source = request.input().pixels();
    let mut output = vec![[0.0; 4]; source.len()];
    let gamut_mask = source
        .iter()
        .map(|pixel| in_model_gamut(request, pixel))
        .collect::<Vec<_>>();
    let mut provider = requested_provider(request.provider());
    let mut tile_in = vec![0.0; 3 * tile_size * tile_size];
    let mut completed = 0_u64;
    for tile_y in 0..rows {
        let y = tile_y * step;
        for tile_x in 0..columns {
            if control.is_cancelled(RgbDenoiseStage::Inference) {
                return Err(ProcessError::Cancelled(RgbDenoiseStage::Inference));
            }
            let x = tile_x * step;
            fill_model_tile(
                request,
                source,
                width,
                height,
                x,
                y,
                tile_size,
                overlap,
                shadow_boost,
                &mut tile_in,
            );
            let tile = ModelTile {
                width: model.descriptor().tile_size(),
                height: model.descriptor().tile_size(),
                planar_rgb: &tile_in,
            };
            let inferred = model.infer(provider, tile);
            let tile_out = match (request.provider(), provider, inferred) {
                (ProviderSelection::Auto, ProviderUsed::Gpu, Err(_)) => {
                    provider = ProviderUsed::Cpu;
                    model
                        .infer(provider, tile)
                        .map_err(|source| ProcessError::Model { source })?
                }
                (_, _, Ok(value)) => value,
                (_, _, Err(source)) => return Err(ProcessError::Model { source }),
            };
            if tile_out.len() != 3 * tile_size * tile_size {
                return Err(ProcessError::Model {
                    source: ModelError::InvalidTileOutput,
                });
            }
            let valid_width = (width - x).min(step);
            let valid_height = (height - y).min(step);
            for local_y in 0..valid_height {
                for local_x in 0..valid_width {
                    let global = (y + local_y) * width + x + local_x;
                    let tile_index = (overlap + local_y) * tile_size + overlap + local_x;
                    let rgb = restore_model_pixel(
                        request,
                        &tile_out,
                        tile_index,
                        tile_size,
                        shadow_boost,
                    );
                    output[global] = [rgb[0], rgb[1], rgb[2], source[global].alpha()];
                }
            }
            completed += 1;
            observer.progress(RgbDenoiseProgress {
                stage: RgbDenoiseStage::Inference,
                completed,
                total: tile_count,
            });
        }
    }
    if request.preserve_wide_gamut() {
        preserve_wide_gamut(&mut output, source, &gamut_mask, width, height);
    }
    if control.is_cancelled(RgbDenoiseStage::DetailRecovery) {
        return Err(ProcessError::Cancelled(RgbDenoiseStage::DetailRecovery));
    }
    apply_detail_recovery(
        &source
            .iter()
            .map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()])
            .collect::<Vec<_>>(),
        &mut output,
        dimensions.width(),
        dimensions.height(),
        request.strength(),
    )
    .map_err(|source| ProcessError::Detail { source })?;
    observer.progress(RgbDenoiseProgress {
        stage: RgbDenoiseStage::DetailRecovery,
        completed: 1,
        total: 1,
    });
    if control.is_cancelled(RgbDenoiseStage::ColorTransform) {
        return Err(ProcessError::Cancelled(RgbDenoiseStage::ColorTransform));
    }
    for pixel in &mut output {
        let transformed = request
            .output_profile()
            .working_to_output()
            .apply(pixel[0], pixel[1], pixel[2]);
        pixel[..3].copy_from_slice(&transformed);
        if !pixel.iter().all(|value| value.is_finite()) {
            return Err(ProcessError::NonFiniteOutput);
        }
    }
    observer.progress(RgbDenoiseProgress {
        stage: RgbDenoiseStage::ColorTransform,
        completed: 1,
        total: 1,
    });
    Ok(ProcessedImage {
        pixels: output,
        plan,
        provider,
        shadow_boost,
        artifact_key: artifact_key(request, model.descriptor(), provider),
    })
}

fn validate_model(descriptor: &ModelDescriptor) -> Result<(), ProcessError> {
    if descriptor.task() != ModelTask::RgbDenoise {
        return Err(ProcessError::Model {
            source: ModelError::WrongTask,
        });
    }
    if descriptor.scale() != 1 {
        return Err(ProcessError::Model {
            source: ModelError::UnsupportedScale {
                scale: descriptor.scale(),
            },
        });
    }
    if !descriptor.qualified() {
        return Err(ProcessError::Model {
            source: ModelError::Unqualified,
        });
    }
    Ok(())
}

fn requested_provider(selection: ProviderSelection) -> ProviderUsed {
    match selection {
        ProviderSelection::Cpu => ProviderUsed::Cpu,
        ProviderSelection::Auto | ProviderSelection::Gpu => ProviderUsed::Gpu,
    }
}

fn fill_model_tile(
    request: &RgbDenoiseRequest,
    source: &[RgbaF32Pixel],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    tile_size: usize,
    overlap: usize,
    shadow_boost: bool,
    destination: &mut [f32],
) {
    let plane = tile_size * tile_size;
    for local_y in 0..tile_size {
        let source_y = reflect(y as isize + local_y as isize - overlap as isize, height);
        for local_x in 0..tile_size {
            let source_x = reflect(x as isize + local_x as isize - overlap as isize, width);
            let pixel = source[source_y * width + source_x];
            let model = request.working_profile().working_to_model().apply(
                pixel.red(),
                pixel.green(),
                pixel.blue(),
            );
            let model = model.map(|value| {
                let value = if shadow_boost && value > 0.0 {
                    value.sqrt()
                } else {
                    value
                };
                linear_to_srgb(value.clamp(0.0, 1.0))
            });
            let index = local_y * tile_size + local_x;
            destination[index] = model[0];
            destination[plane + index] = model[1];
            destination[2 * plane + index] = model[2];
        }
    }
}

fn restore_model_pixel(
    request: &RgbDenoiseRequest,
    output: &[f32],
    index: usize,
    tile_size: usize,
    shadow_boost: bool,
) -> [f32; 3] {
    let plane = tile_size * tile_size;
    let model = [
        output[index],
        output[plane + index],
        output[2 * plane + index],
    ];
    let working = model.map(|value| {
        let value = srgb_to_linear(value);
        if shadow_boost { value * value } else { value }
    });
    request
        .working_profile()
        .model_to_working()
        .apply(working[0], working[1], working[2])
}

fn in_model_gamut(request: &RgbDenoiseRequest, pixel: &RgbaF32Pixel) -> bool {
    let model = request.working_profile().working_to_model().apply(
        pixel.red(),
        pixel.green(),
        pixel.blue(),
    );
    model.iter().all(|value| (-0.01..=1.01).contains(value))
}

fn preserve_wide_gamut(
    output: &mut [[f32; 4]],
    source: &[RgbaF32Pixel],
    mask: &[bool],
    width: usize,
    height: usize,
) {
    for (index, in_gamut) in mask.iter().copied().enumerate() {
        if !in_gamut {
            let source = source[index];
            output[index][..3].copy_from_slice(&[source.red(), source.green(), source.blue()]);
        }
    }
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if mask[index] {
                continue;
            }
            let source = source[index];
            let original_luma =
                luma(&[source.red(), source.green(), source.blue(), source.alpha()]);
            let mut sum = 0.0;
            let mut count = 0_u32;
            for neighbor_y in y.saturating_sub(2)..=(y + 2).min(height - 1) {
                for neighbor_x in x.saturating_sub(2)..=(x + 2).min(width - 1) {
                    let neighbor = neighbor_y * width + neighbor_x;
                    if mask[neighbor] {
                        sum += luma(&output[neighbor]);
                        count += 1;
                    }
                }
            }
            if count > 0 {
                let delta = sum / count as f32 - original_luma;
                output[index][0] += delta;
                output[index][1] += delta;
                output[index][2] += delta;
            }
        }
    }
}

fn has_deep_shadows(image: &RgbaF32Image) -> bool {
    let dimensions = image.descriptor().dimensions();
    let width = usize::try_from(dimensions.width()).unwrap_or(0);
    let height = usize::try_from(dimensions.height()).unwrap_or(0);
    let mut dark = 0_u64;
    let mut total = 0_u64;
    for y in (0..height).step_by(16) {
        for x in (0..width).step_by(16) {
            let pixel = image.pixels()[y * width + x];
            if luma(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]) < 0.005 {
                dark += 1;
            }
            total += 1;
        }
    }
    total > 0 && dark * 10 >= total
}

fn linear_to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        12.92 * value.max(0.0)
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 {
        value.max(0.0) / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn reflect(index: isize, length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let period = 2 * (length as isize - 1);
    let normalized = index.rem_euclid(period);
    if normalized < length as isize {
        normalized as usize
    } else {
        (period - normalized) as usize
    }
}

fn artifact_key(
    request: &RgbDenoiseRequest,
    model: &ModelDescriptor,
    provider: ProviderUsed,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.ai.rgb-denoise.v1");
    hasher.update(request.input().source_identity().as_bytes());
    hasher.update(request.render_identity());
    hasher.update(request.working_profile().identity().as_bytes());
    hasher.update(request.output_profile().identity().as_bytes());
    hasher.update(model.model_id().as_bytes());
    hasher.update([model.scale()]);
    hasher.update(model.tile_size().to_le_bytes());
    hasher.update(model.overlap().to_le_bytes());
    hasher.update([
        request.strength().get(),
        u8::from(request.preserve_wide_gamut()),
    ]);
    hasher.update([match provider {
        ProviderUsed::Cpu => 0,
        ProviderUsed::Gpu => 1,
    }]);
    hasher.update([match request.collision() {
        CollisionPolicy::Fail => 0,
        CollisionPolicy::UniqueSuffix => 1,
    }]);
    hasher.finalize().into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessError {
    Model { source: ModelError },
    InvalidTilePlan,
    TileCountOverflow,
    DimensionOverflow,
    NonFiniteOutput,
    Detail { source: DetailError },
    Cancelled(RgbDenoiseStage),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RGB denoise processing failed: {self:?}")
    }
}

impl std::error::Error for ProcessError {}
