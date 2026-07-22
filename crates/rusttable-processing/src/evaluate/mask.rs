use super::{EvaluationError, blend};
use crate::{LinearRgb, PipelineStepIndex, RasterDimensions, RgbChannel};
use rusttable_core::OperationId;

pub(super) fn validate_operation_mask(
    mask: &rusttable_masks::MaskRaster,
    pixel_count: usize,
    dimensions: RasterDimensions,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
) -> Result<(), EvaluationError> {
    let expected = usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX);
    if mask.width() != dimensions.width()
        || mask.height() != dimensions.height()
        || mask.values().len() != pixel_count
        || mask.values().len() != expected
    {
        return Err(EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: format!(
                "mask dimensions {}x{} with {} samples do not match image {dimensions:?} with {pixel_count} pixels",
                mask.width(),
                mask.height(),
                mask.values().len(),
            ),
        });
    }
    Ok(())
}

pub(super) fn apply_mask_blend(
    pixels: &mut [LinearRgb],
    before: &[LinearRgb],
    mask: &rusttable_masks::MaskRaster,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError> {
    for (local_index, (current, original)) in pixels.iter_mut().zip(before).enumerate() {
        let coverage = mask.values()[local_index];
        if coverage.to_bits() == 1.0f32.to_bits() {
            continue;
        }
        if coverage.to_bits() == 0.0f32.to_bits() {
            *current = *original;
            continue;
        }
        let pixel_index = pixel_index_offset + local_index;
        *current = LinearRgb::new(
            blend(
                original.red().get(),
                current.red().get(),
                coverage,
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Red,
            )?,
            blend(
                original.green().get(),
                current.green().get(),
                coverage,
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Green,
            )?,
            blend(
                original.blue().get(),
                current.blue().get(),
                coverage,
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Blue,
            )?,
        );
    }
    Ok(())
}
