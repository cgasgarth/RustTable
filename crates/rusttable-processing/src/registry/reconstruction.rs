use crate::ProcessingOperation;
use crate::descriptor::{DescriptorId, OperationDescriptor, RoiKind};
use crate::registry::{
    CpuFactory, CpuPrepare, FactoryError, GpuBinding, ImplementationIdentity, MigrationBinding,
    OperationDefinition, OperationUiAvailability, PreparedCpuOperation,
};
use rusttable_core::Operation;

#[expect(
    clippy::too_many_lines,
    reason = "The exhaustive operation-to-descriptor mapping stays together so new operations cannot be omitted silently."
)]
pub fn operation_descriptor_for(operation: &ProcessingOperation) -> DescriptorId {
    match operation.kind() {
        crate::ProcessingOperationKind::Basecurve { .. } => {
            crate::descriptor::basecurve_descriptor().id
        }
        crate::ProcessingOperationKind::Highpass { .. } => {
            crate::descriptor::highpass_descriptor().id
        }
        crate::ProcessingOperationKind::ToneCurve { .. } => {
            crate::descriptor::tonecurve_descriptor().id
        }
        crate::ProcessingOperationKind::Colisa { .. } => crate::descriptor::colisa_descriptor().id,
        crate::ProcessingOperationKind::Agx { .. } => crate::descriptor::agx_descriptor().id,
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
        crate::ProcessingOperationKind::Censorize { .. } => {
            crate::descriptor::censorize_descriptor().id
        }
        crate::ProcessingOperationKind::Defringe { .. } => {
            crate::descriptor::defringe_descriptor().id
        }
        crate::ProcessingOperationKind::Clahe { .. } => crate::descriptor::clahe_descriptor().id,
        crate::ProcessingOperationKind::MaskManager { .. } => {
            crate::descriptor::mask_manager_descriptor().id
        }
        crate::ProcessingOperationKind::Retouch { .. } => {
            crate::descriptor::retouch_descriptor().id
        }
        crate::ProcessingOperationKind::Spots { .. } => crate::descriptor::spots_descriptor().id,
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
        crate::ProcessingOperationKind::ColorContrast { .. } => {
            crate::descriptor::colorcontrast_descriptor().id
        }
        crate::ProcessingOperationKind::ChannelMixer { .. } => {
            crate::descriptor::channelmixer_descriptor().id
        }
        crate::ProcessingOperationKind::ColorZones { .. } => {
            crate::descriptor::colorzones_descriptor().id
        }
        crate::ProcessingOperationKind::Temperature { .. } => {
            crate::descriptor::temperature_descriptor().id
        }
        crate::ProcessingOperationKind::Bloom { .. } => crate::descriptor::bloom_descriptor().id,
        crate::ProcessingOperationKind::Soften { .. } => crate::descriptor::soften_descriptor().id,
        crate::ProcessingOperationKind::Relight { .. } => {
            crate::descriptor::relight_descriptor().id
        }
        crate::ProcessingOperationKind::Velvia { .. } => crate::descriptor::velvia_descriptor().id,
        crate::ProcessingOperationKind::Vibrance { .. } => {
            crate::descriptor::vibrance_descriptor().id
        }
        crate::ProcessingOperationKind::Shadhi { .. } => crate::descriptor::shadhi_descriptor().id,
        crate::ProcessingOperationKind::Sharpen { .. } => {
            crate::descriptor::sharpen_descriptor().id
        }
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
        crate::ProcessingOperationKind::Clipping { .. } => {
            crate::descriptor::clipping_descriptor().id
        }
        crate::ProcessingOperationKind::RasterFile { .. } => {
            crate::descriptor::rasterfile_descriptor().id
        }
        crate::ProcessingOperationKind::LensCorrection { .. } => {
            crate::descriptor::lenscorrection_descriptor().id
        }
        crate::ProcessingOperationKind::Levels { .. } => crate::descriptor::levels_descriptor().id,
        crate::ProcessingOperationKind::RgbLevels { .. } => {
            crate::descriptor::rgblevels_descriptor().id
        }
        crate::ProcessingOperationKind::ColorTransfer { .. } => {
            crate::descriptor::colortransfer_descriptor().id
        }
        crate::ProcessingOperationKind::ColorMapping { .. } => {
            crate::descriptor::colormapping_descriptor().id
        }
        crate::ProcessingOperationKind::Liquify { .. } => {
            crate::descriptor::liquify_descriptor().id
        }
    }
}

pub fn prepare_highlights(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_highlights(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

pub fn prepare_color_reconstruction(
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

pub fn highlights_definition() -> OperationDefinition {
    reconstruction_definition(
        crate::descriptor::highlights_descriptor(),
        prepare_highlights,
        crate::operations::highlights::wgpu_passes(),
        &[
            "iop.highlights.params.v1-v4",
            "iop.highlights.scalar",
            "iop.highlights.masks",
        ],
        &[(1, 2), (2, 3), (3, 4)],
    )
}

pub fn color_reconstruction_definition() -> OperationDefinition {
    reconstruction_definition(
        crate::descriptor::color_reconstruction_descriptor(),
        prepare_color_reconstruction,
        crate::operations::colorreconstruction::wgpu_passes(),
        &[
            "iop.colorreconstruct.params.v1-v3",
            "iop.colorreconstruct.scalar",
            "iop.colorreconstruct.luminance",
        ],
        &[(1, 3), (2, 3)],
    )
    .with_ui_availability(OperationUiAvailability::PartiallyAvailable {
        reason: "the Color Reconstruction parameter editor is usable, but native UI adjuncts remain deferred"
            .to_owned(),
        deferred_responsibilities: [
            "iop.colorreconstruct.ui.shared-blending-and-drawn-masks",
            "iop.colorreconstruct.ui.monochrome-applicability",
            "iop.colorreconstruct.ui.preview-grid-lifecycle",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    })
}

fn reconstruction_definition<const N: usize>(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    passes: [&'static str; N],
    evidence: &'static [&'static str],
    migration_edges: &'static [(u16, u16)],
) -> OperationDefinition {
    let compatibility_name = descriptor.id.compatibility_name.clone();
    let migrations = migration_edges
        .iter()
        .map(|(from, to)| {
            MigrationBinding::new(
                *from,
                *to,
                format!(
                    "{}.migration.v{from}-v{to}",
                    descriptor.id.compatibility_name
                ),
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
