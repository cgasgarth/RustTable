use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, OperationDescriptor, RoiKind};
use crate::registry::{
    CpuFactory, CpuPrepare, FactoryError, GpuBinding, ImplementationIdentity, MigrationBinding,
    OperationDefinition, PreparedCpuOperation,
};
use rusttable_core::Operation;

pub(crate) fn operation_descriptor_for(operation: &ProcessingOperation) -> DescriptorId {
    match operation.kind() {
        crate::ProcessingOperationKind::BasicAdj { .. } => {
            crate::descriptor::basicadj_descriptor().id
        }
        crate::ProcessingOperationKind::Exposure { .. } => {
            crate::descriptor::exposure_descriptor().id
        }
        crate::ProcessingOperationKind::LinearOffset { .. } => {
            crate::descriptor::linear_offset_descriptor().id
        }
        crate::ProcessingOperationKind::RgbGain { .. } => {
            crate::descriptor::rgb_gain_descriptor().id
        }
        crate::ProcessingOperationKind::Invert { .. } => crate::descriptor::invert_descriptor().id,
        crate::ProcessingOperationKind::Dither { .. } => crate::descriptor::dither_descriptor().id,
        crate::ProcessingOperationKind::Grain { .. } => crate::descriptor::grain_descriptor().id,
        crate::ProcessingOperationKind::Highlights { .. } => {
            crate::descriptor::highlights_descriptor().id
        }
        crate::ProcessingOperationKind::ColorReconstruction { .. } => {
            crate::descriptor::color_reconstruction_descriptor().id
        }
        crate::ProcessingOperationKind::ColorIn { .. } => {
            crate::descriptor::colorin_descriptor().id
        }
        crate::ProcessingOperationKind::Primaries { .. } => {
            crate::descriptor::primaries_descriptor().id
        }
        crate::ProcessingOperationKind::ColorOut { .. } => {
            crate::descriptor::colorout_descriptor().id
        }
        crate::ProcessingOperationKind::ColorCorrection { .. } => {
            crate::descriptor::colorcorrection_descriptor().id
        }
        crate::ProcessingOperationKind::Temperature { .. } => {
            crate::descriptor::temperature_descriptor().id
        }
        crate::ProcessingOperationKind::Bloom { .. } => crate::descriptor::bloom_descriptor().id,
        crate::ProcessingOperationKind::Soften { .. } => crate::descriptor::soften_descriptor().id,
        crate::ProcessingOperationKind::Relight { .. } => {
            crate::descriptor::relight_descriptor().id
        }
        crate::ProcessingOperationKind::Shadhi { .. } => crate::descriptor::shadhi_descriptor().id,
        crate::ProcessingOperationKind::Vignette { .. } => {
            crate::descriptor::vignette_descriptor().id
        }
        crate::ProcessingOperationKind::GraduatedNd { .. } => {
            crate::descriptor::graduatednd_descriptor().id
        }
        crate::ProcessingOperationKind::Crop { .. } => crate::descriptor::crop_descriptor().id,
        crate::ProcessingOperationKind::Flip { .. } => crate::descriptor::flip_descriptor().id,
        crate::ProcessingOperationKind::RotatePixels { .. } => {
            crate::descriptor::rotatepixels_descriptor().id
        }
        crate::ProcessingOperationKind::ScalePixels { .. } => {
            crate::descriptor::scalepixels_descriptor().id
        }
        crate::ProcessingOperationKind::FinalScale { .. } => {
            crate::descriptor::finalscale_descriptor().id
        }
        crate::ProcessingOperationKind::EnlargeCanvas { .. } => {
            crate::descriptor::enlargecanvas_descriptor().id
        }
        crate::ProcessingOperationKind::Perspective { .. } => {
            crate::descriptor::perspective_descriptor().id
        }
        crate::ProcessingOperationKind::LensCorrection { .. } => {
            crate::descriptor::lenscorrection_descriptor().id
        }
    }
}

pub(crate) fn prepare_highlights(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_highlights(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub(crate) fn prepare_color_reconstruction(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_color_reconstruction(operation)
            .map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub(crate) fn highlights_definition() -> OperationDefinition {
    reconstruction_definition(
        crate::descriptor::highlights_descriptor(),
        prepare_highlights,
        crate::operations::highlights::wgpu_passes(),
        &[
            "iop.highlights.params.v1-v4",
            "iop.highlights.scalar",
            "iop.highlights.masks",
        ],
        4,
    )
}

pub(crate) fn color_reconstruction_definition() -> OperationDefinition {
    reconstruction_definition(
        crate::descriptor::color_reconstruction_descriptor(),
        prepare_color_reconstruction,
        crate::operations::colorreconstruction::wgpu_passes(),
        &[
            "iop.colorreconstruction.params.v1-v3",
            "iop.colorreconstruction.scalar",
            "iop.colorreconstruction.luminance",
        ],
        3,
    )
}

fn reconstruction_definition<const N: usize>(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    passes: [&'static str; N],
    evidence: &'static [&'static str],
    target_version: u16,
) -> OperationDefinition {
    let compatibility_name = descriptor.id.compatibility_name.clone();
    let migrations = (1..target_version)
        .map(|from| {
            MigrationBinding::new(
                from,
                from + 1,
                format!("{}.migration.v{from}", descriptor.id.compatibility_name),
            )
        })
        .collect();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            RoiKind::FullImage,
            false,
            true,
        )),
        Some(GpuBinding::new(
            format!("rusttable.{compatibility_name}.wgsl"),
            1,
            passes.into_iter().map(str::to_owned),
            ["rgba32float".to_owned()],
        )),
        migrations,
        ImplementationIdentity::new(
            format!(
                "{}.{compatibility_name}",
                crate::registry::REGISTRY_BUILD_ID
            ),
            1,
            format!(
                "{}.{compatibility_name}",
                crate::registry::REGISTRY_BUILD_ID
            ),
        ),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}
