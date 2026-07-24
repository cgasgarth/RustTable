use super::{EvaluationError, blend};
use crate::{LinearRgb, PipelineStepIndex, ProcessingOperationKind, RasterDimensions, RgbChannel};
use rusttable_core::OperationId;
use rusttable_masks::MaskRaster;

#[derive(Debug, Clone, Copy)]
pub(super) struct OperationMaskRoute<'a> {
    native_values: Option<&'a [f32]>,
    working_rgb_blend: Option<&'a MaskRaster>,
}

impl<'a> OperationMaskRoute<'a> {
    pub(super) fn new(kind: &ProcessingOperationKind, mask: Option<&'a MaskRaster>) -> Self {
        if matches!(kind, ProcessingOperationKind::Shadhi { .. }) {
            Self {
                native_values: mask.map(MaskRaster::values),
                working_rgb_blend: None,
            }
        } else {
            Self {
                native_values: None,
                working_rgb_blend: mask,
            }
        }
    }

    pub(super) const fn native_values(self) -> Option<&'a [f32]> {
        self.native_values
    }

    pub(super) const fn working_rgb_blend(self) -> Option<&'a MaskRaster> {
        self.working_rgb_blend
    }
}

pub(super) fn validate_operation_mask(
    mask: &MaskRaster,
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
    mask: &MaskRaster,
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
