use super::{FactoryError, OperationDefinition, PreparedCpuOperation, RoiKind};
use crate::descriptor::DescriptorId;
use rusttable_core::Operation;

fn prepare_liquify(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::ProcessingOperation::compile_liquify(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn liquify_definition() -> OperationDefinition {
    super::geometry_definition_with_traits(
        crate::descriptor::liquify_descriptor(),
        prepare_liquify,
        &[
            "iop.liquify.params.v1",
            "iop.liquify.inverse-field.cpu",
            "iop.liquify.mask.cpu",
            "iop.liquify.wgpu-fallback",
        ],
        RoiKind::FullImage,
        std::iter::empty(),
        None,
        false,
        false,
    )
}
