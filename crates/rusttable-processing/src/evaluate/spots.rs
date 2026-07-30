use crate::{LinearRgb, PipelineStepIndex, RasterDimensions};
use rusttable_core::OperationId;

pub(crate) fn apply_spots(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    parameters: &crate::SpotsParametersV2,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
) -> Result<(), crate::EvaluationError> {
    let config = crate::SpotsConfig::from_parameters(parameters).map_err(|error| {
        crate::EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: error.to_string(),
        }
    })?;
    let plan = crate::SpotsPlan::new(config, dimensions).map_err(|error| {
        crate::EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: error.to_string(),
        }
    })?;
    plan.execute_linear_rgb(pixels, || false)
        .map(|_| ())
        .map_err(|error| crate::EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: error.to_string(),
        })
}
