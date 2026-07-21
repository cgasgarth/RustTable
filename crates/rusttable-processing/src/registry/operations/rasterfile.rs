//! Registry binding for the deprecated external raster-mask operation.

use super::{FactoryError, GpuBinding, OperationDefinition, PreparedCpuOperation, RoiKind};
use crate::descriptor::{DescriptorId, rasterfile_descriptor};
use rusttable_core::Operation;

fn prepare_rasterfile(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_rasterfile(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn rasterfile_definition() -> OperationDefinition {
    super::geometry_definition_with_gpu(
        rasterfile_descriptor(),
        prepare_rasterfile,
        &[
            "iop.rasterfile.descriptor",
            "iop.rasterfile.managed-asset",
            "iop.rasterfile.cpu-mask-publication",
            "iop.rasterfile.wgpu-mask-publication",
            "iop.rasterfile.vectorization",
        ],
        RoiKind::FullImage,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.rasterfile.wgpu",
            1,
            vec!["raster-mask-upload".to_owned()],
            vec!["r32float".to_owned()],
        )),
    )
}
