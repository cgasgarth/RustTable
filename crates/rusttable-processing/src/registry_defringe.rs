//! Registry binding for the typed, deprecated Lab defringe compatibility node.

use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, defringe_descriptor};
use crate::registry::{
    CpuFactory, FactoryError, ImplementationIdentity, OperationDefinition, PreparedCpuOperation,
    REGISTRY_BUILD_ID, RoiKind,
};
use rusttable_core::Operation;

fn prepare_defringe(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_defringe(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn defringe_definition() -> OperationDefinition {
    OperationDefinition::new(
        defringe_descriptor(),
        Some(CpuFactory::new(
            prepare_defringe,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Neighborhood,
            true,
            true,
        )),
        None,
        Vec::new(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.defringe"),
            1,
            format!("{REGISTRY_BUILD_ID}.defringe"),
        ),
        vec![
            "iop.defringe.params.v1".to_owned(),
            "iop.defringe.cpu.scalar-reference".to_owned(),
            "iop.defringe.gaussian.bounded-lab".to_owned(),
            "iop.defringe.fibonacci-lattice".to_owned(),
            "iop.defringe.mask-blend".to_owned(),
            "iop.defringe.receipt".to_owned(),
        ],
    )
}
