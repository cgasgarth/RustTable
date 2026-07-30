use rusttable_masks::{MaskExecutionError, MaskRoi};
use rusttable_processing::{OperationMaskSet, OperationMaskSetError};

use super::{CpuPixelpipeError, CpuPixelpipeSnapshot};

pub(super) fn resolve_masks(
    request: &CpuPixelpipeSnapshot,
) -> Result<Option<OperationMaskSet>, CpuPixelpipeError> {
    let Some(graph) = request.mask_graph() else {
        return Ok(None);
    };
    let mut store = request.mask_store().cloned();
    let mut entries = Vec::new();
    for node in request.graph().nodes() {
        if let Some(mask) = graph
            .evaluate_for_operation(node.operation().operation_id().get(), store.as_mut())
            .map_err(|source| CpuPixelpipeError::MaskEvaluation { source })?
        {
            entries.push((node.operation().operation_id(), mask));
        }
    }
    if entries.is_empty() {
        return Ok(None);
    }
    OperationMaskSet::from_entries(entries)
        .map(Some)
        .map_err(|source| CpuPixelpipeError::MaskBinding { source })
}

pub(super) fn crop_masks(
    masks: &OperationMaskSet,
    tile: crate::CpuPixelpipeTile,
) -> Result<OperationMaskSet, CpuPixelpipeError> {
    let roi = MaskRoi::new(
        tile.origin_x(),
        tile.origin_y(),
        tile.dimensions().width(),
        tile.dimensions().height(),
    )
    .map_err(|_| CpuPixelpipeError::MaskEvaluation {
        source: MaskExecutionError::InvalidRoi,
    })?;
    masks
        .crop(roi)
        .map_err(|source: OperationMaskSetError| CpuPixelpipeError::MaskBinding { source })
}
