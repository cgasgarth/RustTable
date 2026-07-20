use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, OperationDescriptor, RoiKind};
use crate::registry::{
    CpuFactory, FactoryError, GpuBinding, ImplementationIdentity, MigrationBinding,
    OperationDefinition, PreparedCpuOperation,
};
use rusttable_core::Operation;

pub(crate) fn colorin_definition() -> OperationDefinition {
    definition(
        crate::descriptor::colorin_descriptor(),
        prepare_colorin,
        &crate::operations::colorin::wgpu_passes(),
        (1..7)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("colorin.migration.v{version}"),
                )
            })
            .collect(),
        &[
            "iop.colorin.profile-evidence",
            "iop.colorin.cpu",
            "iop.colorin.wgsl",
        ],
        false,
    )
}

pub(crate) fn primaries_definition() -> OperationDefinition {
    definition(
        crate::descriptor::primaries_descriptor(),
        prepare_primaries,
        &crate::operations::primaries::wgpu_passes(),
        Vec::new(),
        &[
            "iop.primaries.matrix",
            "iop.primaries.cpu",
            "iop.primaries.wgsl",
        ],
        false,
    )
}

pub(crate) fn colorout_definition() -> OperationDefinition {
    definition(
        crate::descriptor::colorout_descriptor(),
        prepare_colorout,
        &crate::operations::colorout::wgpu_passes(),
        (1..7)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("colorout.migration.v{version}"),
                )
            })
            .collect(),
        &[
            "iop.colorout.profile-resolution",
            "iop.colorout.cpu",
            "iop.colorout.wgsl",
            "iop.colorout.receipt",
        ],
        true,
    )
}

pub(crate) fn colorcorrection_definition() -> OperationDefinition {
    definition(
        crate::descriptor::colorcorrection_descriptor(),
        prepare_colorcorrection,
        &crate::operations::colorcorrection::wgpu_passes(),
        (1..5)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("colorcorrection.migration.v{version}"),
                )
            })
            .collect(),
        &[
            "iop.colorcorrection.legacy-parameters",
            "iop.colorcorrection.cpu",
            "iop.colorcorrection.wgsl",
            "iop.colorcorrection.receipt",
        ],
        false,
    )
}

fn prepare_colorin(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_colorin(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_primaries(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_primaries(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_colorout(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_colorout(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_colorcorrection(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_colorcorrection(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn definition(
    descriptor: OperationDescriptor,
    prepare: crate::registry::CpuPrepare,
    passes: &[&str],
    migrations: Vec<MigrationBinding>,
    evidence: &[&str],
    full_image_analysis: bool,
) -> OperationDefinition {
    let compatibility = descriptor.id.compatibility_name.clone();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Identity,
            true,
            full_image_analysis,
        )),
        Some(GpuBinding::new(
            format!("rusttable.{compatibility}.wgsl"),
            1,
            passes.iter().map(|pass| (*pass).to_owned()),
            ["rgba32float".to_owned()],
        )),
        migrations,
        ImplementationIdentity::new(
            format!("{}.{}", crate::registry::REGISTRY_BUILD_ID, compatibility),
            1,
            format!("{}.{}", crate::registry::REGISTRY_BUILD_ID, compatibility),
        ),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}
