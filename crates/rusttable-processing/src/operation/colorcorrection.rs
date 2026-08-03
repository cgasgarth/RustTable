use rusttable_core::Operation;

use crate::operations::colorcorrection::{
    COLORCORRECTION_DEFAULT_HIA, COLORCORRECTION_DEFAULT_HIB, COLORCORRECTION_DEFAULT_LOA,
    COLORCORRECTION_DEFAULT_LOB, COLORCORRECTION_DEFAULT_SATURATION, ColorCorrectionConfig,
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const COLORCORRECTION_PARAMETERS: [&str; 5] = ["hia", "hib", "loa", "lob", "saturation"];

pub fn compile_colorcorrection(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &COLORCORRECTION_PARAMETERS)?;
    let config = ColorCorrectionConfig::new(
        super::parameter_f32(operation, "hia", f64::from(COLORCORRECTION_DEFAULT_HIA))?,
        super::parameter_f32(operation, "hib", f64::from(COLORCORRECTION_DEFAULT_HIB))?,
        super::parameter_f32(operation, "loa", f64::from(COLORCORRECTION_DEFAULT_LOA))?,
        super::parameter_f32(operation, "lob", f64::from(COLORCORRECTION_DEFAULT_LOB))?,
        super::parameter_f32(
            operation,
            "saturation",
            f64::from(COLORCORRECTION_DEFAULT_SATURATION),
        )?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::ColorCorrection { config },
    })
}
