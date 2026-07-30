use super::{
    CpuFactory, DefinitionAvailability, FactoryError, ImplementationIdentity, MigrationBinding,
    OperationDefinition, OperationUiAvailability, PreparedCpuOperation, REGISTRY_BUILD_ID, RoiKind,
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
        vec![MigrationBinding::new(1, 2, "basicadj.migration.v1-v2")],
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
    .with_availability(DefinitionAvailability::Unavailable {
        reason: "Basic Adjust is preserved as typed compatibility data, but profile-aware luminance, automatic levels, source blend/mask semantics, and imported materialization are incomplete".to_owned(),
    })
    .with_ui_availability(OperationUiAvailability::Unavailable {
        reason: "the source-shaped Basic Adjust UI, profile-aware picker, and automatic-level action are not ported".to_owned(),
    })
}
