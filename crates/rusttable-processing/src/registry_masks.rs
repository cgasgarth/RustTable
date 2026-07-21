//! Registry bindings for cross-operation masks and multiscale retouch.

use super::{FactoryError, OperationDefinition, PreparedCpuOperation, RoiKind};
use crate::descriptor::{DescriptorId, mask_manager_descriptor, retouch_descriptor};
use rusttable_core::Operation;

fn prepare_mask_manager(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::ProcessingOperation::compile_mask_manager(operation)
            .map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_retouch(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::ProcessingOperation::compile_retouch(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn mask_manager_definition() -> OperationDefinition {
    super::registry_operations::geometry_definition_with_traits(
        mask_manager_descriptor(),
        prepare_mask_manager,
        &[
            "iop.mask_manager.params.v2",
            "iop.mask_manager.identity",
            "iop.mask_manager.graph",
        ],
        RoiKind::Identity,
        std::iter::empty(),
        None,
        false,
        false,
    )
}

pub fn retouch_definition() -> OperationDefinition {
    super::registry_operations::geometry_definition_with_traits(
        retouch_descriptor(),
        prepare_retouch,
        &[
            "iop.retouch.params.v1",
            "iop.retouch.wavelet.atrous",
            "iop.retouch.cpu.tiled",
        ],
        RoiKind::FullImage,
        std::iter::empty(),
        None,
        false,
        false,
    )
}
