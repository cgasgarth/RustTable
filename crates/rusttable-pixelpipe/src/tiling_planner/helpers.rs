use crate::tiling_models::{PlannedTile, PlannedTileGrid, TileInputRoi};
use crate::{
    BackendRequirement, GeometryError, NodeRequirements, TileAlignment, TileBackend, TilePlanError,
    TileRect,
};

pub(super) fn make_grid(
    requirements: &NodeRequirements,
    output: TileRect,
    width: u32,
    height: u32,
    backend: TileBackend,
) -> Result<PlannedTileGrid, TilePlanError> {
    let columns = ceil_div(output.width(), width)?;
    let rows = ceil_div(output.height(), height)?;
    let count = u64::from(columns)
        .checked_mul(u64::from(rows))
        .ok_or(TilePlanError::ArithmeticOverflow)?;
    let capacity = usize::try_from(count).map_err(|_| TilePlanError::ArithmeticOverflow)?;
    let mut tiles = Vec::with_capacity(capacity);
    for row in 0..rows {
        for column in 0..columns {
            let x = output
                .x()
                .checked_add(
                    column
                        .checked_mul(width)
                        .ok_or(TilePlanError::ArithmeticOverflow)?,
                )
                .ok_or(TilePlanError::ArithmeticOverflow)?;
            let y = output
                .y()
                .checked_add(
                    row.checked_mul(height)
                        .ok_or(TilePlanError::ArithmeticOverflow)?,
                )
                .ok_or(TilePlanError::ArithmeticOverflow)?;
            let tile = TileRect::new(
                x,
                y,
                width.min(output.width() - column * width),
                height.min(output.height() - row * height),
            );
            let input_mappings = requirements
                .roi_chain()
                .required_inputs(tile)
                .map_err(TilePlanError::Geometry)?;
            let input_rois = input_mappings
                .into_iter()
                .enumerate()
                .map(|(offset, (operation_id, rect))| {
                    let node = &requirements.nodes()[requirements.enabled_range().start + offset];
                    let record = node
                        .backend(backend)
                        .ok_or(TilePlanError::BackendUnavailable { backend })?;
                    let bounds = requirements.roi_chain().stages()[offset].input_bounds();
                    let expanded = expand_and_clip(rect, record.overlap(), bounds)?;
                    Ok(TileInputRoi {
                        operation_id,
                        rect: align_and_clip(expanded, bounds, record.alignment())?,
                    })
                })
                .collect::<Result<Vec<_>, TilePlanError>>()?;
            tiles.push(PlannedTile {
                output: tile,
                input_rois,
            });
        }
    }
    Ok(PlannedTileGrid {
        output,
        columns,
        rows,
        tiles,
    })
}

pub(super) fn align_and_clip(
    rect: TileRect,
    bounds: TileRect,
    alignment: TileAlignment,
) -> Result<TileRect, TilePlanError> {
    let clipped = rect
        .intersect(bounds)
        .ok_or(TilePlanError::OutputOutsideRoi)?;
    let x = clipped.x() / alignment.origin_x() * alignment.origin_x();
    let y = clipped.y() / alignment.origin_y() * alignment.origin_y();
    let end_x = round_up(
        clipped.end_x().ok_or(TilePlanError::ArithmeticOverflow)?,
        alignment.extent_x(),
    )?;
    let end_y = round_up(
        clipped.end_y().ok_or(TilePlanError::ArithmeticOverflow)?,
        alignment.extent_y(),
    )?;
    TileRect::new(x, y, end_x.saturating_sub(x), end_y.saturating_sub(y))
        .intersect(bounds)
        .ok_or(TilePlanError::OutputOutsideRoi)
}

pub(super) fn expand_and_clip(
    rect: TileRect,
    overlap: crate::EdgeOverlap,
    bounds: TileRect,
) -> Result<TileRect, TilePlanError> {
    let x = rect.x().saturating_sub(overlap.left());
    let y = rect.y().saturating_sub(overlap.top());
    let end_x = rect
        .end_x()
        .ok_or(TilePlanError::ArithmeticOverflow)?
        .checked_add(overlap.right())
        .ok_or(TilePlanError::ArithmeticOverflow)?;
    let end_y = rect
        .end_y()
        .ok_or(TilePlanError::ArithmeticOverflow)?
        .checked_add(overlap.bottom())
        .ok_or(TilePlanError::ArithmeticOverflow)?;
    TileRect::new(
        x,
        y,
        end_x
            .checked_sub(x)
            .ok_or(TilePlanError::ArithmeticOverflow)?,
        end_y
            .checked_sub(y)
            .ok_or(TilePlanError::ArithmeticOverflow)?,
    )
    .intersect(bounds)
    .ok_or(TilePlanError::OutputOutsideRoi)
}

pub(super) fn combine_alignment(
    left: TileAlignment,
    right: TileAlignment,
) -> Result<TileAlignment, TilePlanError> {
    TileAlignment::new(
        lcm(left.origin_x(), right.origin_x())?,
        lcm(left.origin_y(), right.origin_y())?,
        lcm(left.extent_x(), right.extent_x())?,
        lcm(left.extent_y(), right.extent_y())?,
    )
    .map_err(TilePlanError::Geometry)
}

fn lcm(left: u32, right: u32) -> Result<u32, TilePlanError> {
    left.checked_div(gcd(left, right))
        .and_then(|value| value.checked_mul(right))
        .ok_or(TilePlanError::ArithmeticOverflow)
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

pub(super) fn aligned_initial(value: u32, minimum: u32, alignment: u32) -> u32 {
    if value < minimum {
        return minimum;
    }
    (value / alignment * alignment).max(minimum)
}

pub(super) fn next_dimension(current: u32, minimum: u32, alignment: u32) -> u32 {
    let midpoint = minimum.saturating_add(current.saturating_sub(minimum) / 2);
    let aligned = midpoint / alignment * alignment;
    aligned
        .max(minimum)
        .min(current.saturating_sub(alignment).max(minimum))
}

pub(super) fn round_up(value: u32, alignment: u32) -> Result<u32, TilePlanError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .ok_or(TilePlanError::ArithmeticOverflow)
}

fn ceil_div(value: u32, divisor: u32) -> Result<u32, TilePlanError> {
    value
        .checked_add(divisor - 1)
        .map(|value| value / divisor)
        .ok_or(TilePlanError::ArithmeticOverflow)
}

pub(super) fn can_reduce(error: &TilePlanError) -> bool {
    matches!(
        error,
        TilePlanError::OverBudget { .. } | TilePlanError::MaxAllocation { .. }
    )
}

#[allow(dead_code)]
fn _keep_imports(_: BackendRequirement, _: GeometryError) {}
