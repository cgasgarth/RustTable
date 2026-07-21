//! Registry binding for the deprecated full-image CLAHE compatibility node.

use crate::descriptor::{DescriptorId, clahe_descriptor};
use crate::registry::{
    CpuFactory, FactoryError, ImplementationIdentity, OperationDefinition, PreparedCpuOperation,
    REGISTRY_BUILD_ID, RoiKind,
};
use rusttable_core::Operation;

fn prepare_clahe(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_clahe(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn clahe_definition() -> OperationDefinition {
    OperationDefinition::new(
        clahe_descriptor(),
        Some(CpuFactory::new(
            prepare_clahe,
            crate::evaluate::execute_prepared_operation,
            RoiKind::FullImage,
            false,
            true,
        )),
        None,
        Vec::new(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.clahe"),
            1,
            format!("{REGISTRY_BUILD_ID}.clahe"),
        ),
        vec![
            "iop.clahe.params.v1".to_owned(),
            "iop.clahe.cpu.scalar-reference".to_owned(),
            "iop.clahe.hsl-shared".to_owned(),
            "iop.clahe.full-image-plan".to_owned(),
            "iop.clahe.mask-blend".to_owned(),
            "iop.clahe.receipt".to_owned(),
        ],
    )
}
