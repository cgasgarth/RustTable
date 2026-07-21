//! Registry binding for the full-image censorize operation.

use super::{FactoryError, OperationDefinition, PreparedCpuOperation, full_image_definition};
use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, censorize_descriptor};
use rusttable_core::Operation;

fn prepare_censorize(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_censorize(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn censorize_definition() -> OperationDefinition {
    full_image_definition(
        censorize_descriptor(),
        prepare_censorize,
        &[
            "iop.censorize.params.v1",
            "iop.censorize.cpu.scalar-reference",
            "iop.censorize.rng.splitmix32-xoshiro128plus",
            "iop.censorize.pixelization.five-point",
            "iop.censorize.mask-blend-seam",
            "iop.censorize.receipt",
        ],
    )
}
