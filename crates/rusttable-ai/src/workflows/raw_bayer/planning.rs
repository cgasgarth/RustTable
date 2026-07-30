use rusttable_image::ImageDimensions;

use super::ports::{RawBayerModelDescriptor, selected_provider};
use super::types::{RawBayerDenoiseRequest, RawBayerPlan, RawBayerTile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBayerPlanError {
    ModelTask,
    InvalidTensorContract,
    InvalidTile,
    ImageTooSmall,
    ArithmeticOverflow,
    MemoryLimit,
    ProviderUnqualified,
    UnsupportedCfa,
}

pub(crate) fn compile(
    request: &RawBayerDenoiseRequest,
    descriptor: &RawBayerModelDescriptor,
) -> Result<RawBayerPlan, RawBayerPlanError> {
    validate_model(descriptor)?;
    selected_provider(request.provider(), descriptor)?;
    let area = request.source().processing_area();
    let packed = ImageDimensions::new(area.width().div_ceil(2), area.height().div_ceil(2))
        .map_err(|_| RawBayerPlanError::ArithmeticOverflow)?;
    if packed.width() < descriptor.minimum_width || packed.height() < descriptor.minimum_height {
        return Err(RawBayerPlanError::ImageTooSmall);
    }
    let core_width = descriptor
        .tile_width
        .checked_sub(
            descriptor
                .valid_crop
                .left
                .checked_add(descriptor.valid_crop.right)
                .ok_or(RawBayerPlanError::ArithmeticOverflow)?,
        )
        .ok_or(RawBayerPlanError::InvalidTile)?;
    let core_height = descriptor
        .tile_height
        .checked_sub(
            descriptor
                .valid_crop
                .top
                .checked_add(descriptor.valid_crop.bottom)
                .ok_or(RawBayerPlanError::ArithmeticOverflow)?,
        )
        .ok_or(RawBayerPlanError::InvalidTile)?;
    let mut tiles = Vec::new();
    let mut y = 0;
    while y < packed.height() {
        let mut x = 0;
        while x < packed.width() {
            let input_x = x
                .saturating_sub(descriptor.valid_crop.left)
                .min(packed.width().saturating_sub(descriptor.tile_width));
            let input_y = y
                .saturating_sub(descriptor.valid_crop.top)
                .min(packed.height().saturating_sub(descriptor.tile_height));
            tiles.push(RawBayerTile::new(
                input_x,
                input_y,
                descriptor.tile_width,
                descriptor.tile_height,
                x,
                y,
                core_width.min(packed.width() - x),
                core_height.min(packed.height() - y),
            ));
            x = x
                .checked_add(core_width)
                .ok_or(RawBayerPlanError::ArithmeticOverflow)?;
        }
        y = y
            .checked_add(core_height)
            .ok_or(RawBayerPlanError::ArithmeticOverflow)?;
    }
    let tile_elements = u64::from(descriptor.tile_width)
        .checked_mul(u64::from(descriptor.tile_height))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_mul(4))
        .ok_or(RawBayerPlanError::ArithmeticOverflow)?;
    let output_elements = u64::from(packed.width())
        .checked_mul(u64::from(packed.height()))
        .and_then(|n| n.checked_mul(4))
        .and_then(|n| n.checked_mul(4))
        .ok_or(RawBayerPlanError::ArithmeticOverflow)?;
    let estimated = tile_elements
        .checked_mul(2)
        .and_then(|n| n.checked_add(output_elements))
        .and_then(|n| n.checked_add(descriptor.estimated_session_bytes))
        .ok_or(RawBayerPlanError::ArithmeticOverflow)?;
    if estimated > 512 * 1024 * 1024 {
        return Err(RawBayerPlanError::MemoryLimit);
    }
    Ok(RawBayerPlan::new(request, packed, tiles))
}

fn validate_model(descriptor: &RawBayerModelDescriptor) -> Result<(), RawBayerPlanError> {
    if descriptor.identity == [0; 32] || descriptor.task != crate::ModelTask::RawBayerDenoise {
        return Err(RawBayerPlanError::ModelTask);
    }
    if descriptor.scale == 0.0
        || !descriptor.scale.is_finite()
        || !descriptor.offset.is_finite()
        || !descriptor.domain_min.is_finite()
        || !descriptor.domain_max.is_finite()
        || descriptor.domain_max <= descriptor.domain_min
        || descriptor.estimated_session_bytes == 0
        || descriptor.tile_width == 0
        || descriptor.tile_height == 0
        || descriptor.minimum_width == 0
        || descriptor.minimum_height == 0
    {
        return Err(RawBayerPlanError::InvalidTensorContract);
    }
    if descriptor
        .valid_crop
        .left
        .checked_add(descriptor.valid_crop.right)
        .is_none()
        || descriptor
            .valid_crop
            .top
            .checked_add(descriptor.valid_crop.bottom)
            .is_none()
        || descriptor.valid_crop.left + descriptor.valid_crop.right >= descriptor.tile_width
        || descriptor.valid_crop.top + descriptor.valid_crop.bottom >= descriptor.tile_height
        || descriptor.overlap >= descriptor.tile_width.min(descriptor.tile_height)
    {
        return Err(RawBayerPlanError::InvalidTile);
    }
    Ok(())
}
