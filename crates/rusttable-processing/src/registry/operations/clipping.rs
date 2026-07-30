//! Registry wiring for the Darktable-compatible clipping operation.

use super::{
    FactoryError, GpuBinding, MigrationBinding, OperationDefinition, PreparedCpuOperation, RoiKind,
};
use crate::descriptor::{DescriptorId, clipping_descriptor};
use rusttable_core::Operation;

fn prepare_clipping(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_clipping(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn clipping_definition() -> OperationDefinition {
    super::geometry_definition_with_gpu(
        clipping_descriptor(),
        prepare_clipping,
        &[
            "iop.clipping.descriptor",
            "iop.clipping.migrations.v2-v5",
            "iop.clipping.cpu",
            "iop.clipping.wgpu",
        ],
        RoiKind::Distortion,
        (2..5).map(|version| {
            MigrationBinding::new(
                version,
                version + 1,
                format!("clipping.migration.v{version}"),
            )
        }),
        Some(GpuBinding::new(
            "rusttable.clipping.wgpu",
            1,
            Vec::<String>::new(),
            vec!["rgba32float".to_owned()],
        )),
    )
}
