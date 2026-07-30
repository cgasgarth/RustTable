//! Registry binding for the deprecated legacy spot-removal operation.

use super::{FactoryError, OperationDefinition, PreparedCpuOperation, RoiKind};
use crate::descriptor::DescriptorId;
use rusttable_core::Operation;

fn prepare_spots(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::ProcessingOperation::compile_spots(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn spots_definition() -> OperationDefinition {
    super::operations::geometry_definition_with_traits(
        crate::descriptor::spots_descriptor(),
        prepare_spots,
        &[
            "iop.spots.params.v1-v2",
            "iop.spots.migration.v1-v2",
            "iop.spots.cpu.ordered-clone-heal",
            "iop.spots.roi.source-destination-union",
            "iop.spots.deprecated-visibility",
        ],
        RoiKind::FullImage,
        [super::MigrationBinding::new(1, 2, "spots.migration.v1-v2")],
        None,
        false,
        false,
    )
}
