use super::{
    CpuFactory, FactoryError, ImplementationIdentity, MigrationBinding, OperationDefinition,
    PreparedCpuOperation, REGISTRY_BUILD_ID, RoiKind,
};
use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, basicadj_descriptor};
use rusttable_core::Operation;

fn prepare_basicadj(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_basicadj(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn basicadj_definition() -> OperationDefinition {
    let descriptor = basicadj_descriptor();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare_basicadj,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Identity,
            true,
            false,
        )),
        None,
        vec![MigrationBinding::new(1, 2, "basicadj.migration.v1")],
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.basicadj"),
            1,
            format!("{REGISTRY_BUILD_ID}.basicadj"),
        ),
        vec![
            "iop.basicadj.params.v1-v2".to_owned(),
            "iop.basicadj.cpu".to_owned(),
            "iop.basicadj.stage-order".to_owned(),
        ],
    )
}
