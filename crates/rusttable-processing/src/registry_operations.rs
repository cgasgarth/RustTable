//! Built-in operation factories and registry validation helpers.

#[path = "registry_censorize.rs"]
mod registry_censorize;
pub use registry_censorize::censorize_definition;
#[path = "registry_clahe.rs"]
mod registry_clahe;
pub use registry_clahe::clahe_definition;
#[path = "registry_defringe.rs"]
mod registry_defringe;
pub use registry_defringe::defringe_definition;

use super::{
    CpuFactory, CpuPrepare, ExecutionBackend, FactoryError, GpuBinding, ImplementationIdentity,
    MigrationBinding, OperationCapability, OperationDefinition, OperationDefinitionFactory,
    PreparedCpuOperation, REGISTRY_BUILD_ID, REGISTRY_SCHEMA, RegistryValidationError, RoiKind,
};
use crate::ProcessingOperation;
use crate::descriptor::{
    DescriptorId, OperationDescriptor, OperationFlags, bloom_descriptor, crop_descriptor,
    dither_descriptor, enlargecanvas_descriptor, exposure_descriptor, finalscale_descriptor,
    flip_descriptor, graduatednd_descriptor, grain_descriptor, invert_descriptor,
    linear_offset_descriptor, relight_descriptor, rgb_gain_descriptor, rotatepixels_descriptor,
    scalepixels_descriptor, shadhi_descriptor, soften_descriptor, temperature_descriptor,
    vignette_descriptor,
};
use rusttable_core::Operation;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub(super) fn unavailable(reason: &str) -> OperationCapability {
    OperationCapability {
        backend: ExecutionBackend::Cpu,
        available: false,
        reason: Some(reason.to_owned()),
    }
}

pub(super) fn validate_definition(
    definition: &OperationDefinition,
    findings: &mut Vec<RegistryValidationError>,
) {
    let id = &definition.descriptor.id;
    if let Err(error) = definition.descriptor.validate() {
        findings.push(RegistryValidationError::Descriptor(
            id.clone(),
            error.to_string(),
        ));
    }
    let cpu_present = definition.cpu.is_some();
    if !cpu_present && definition.availability().is_available() {
        findings.push(RegistryValidationError::MissingCpu(id.rust_id.clone()));
    }
    if definition.availability().is_available()
        && (definition.descriptor.capability.cpu_supported != cpu_present
            || definition.gpu.as_ref().map(GpuBinding::tier)
                != definition.descriptor.capability.gpu_tier
            || definition.cpu.is_some_and(|cpu| {
                cpu.roi != definition.descriptor.roi
                    || cpu.tileable
                        != definition
                            .descriptor
                            .flags
                            .contains(OperationFlags::TILEABLE)
                    || cpu.full_image_analysis
                        != definition
                            .descriptor
                            .flags
                            .contains(OperationFlags::ANALYSIS)
            }))
    {
        findings.push(RegistryValidationError::CapabilityMismatch(
            id.rust_id.clone(),
        ));
    }
    if definition.identity.implementation_id.is_empty()
        || definition.identity.version == 0
        || definition.identity.build_id.is_empty()
        || definition.identity.version != id.implementation_version
    {
        findings.push(RegistryValidationError::IdentityDrift(id.rust_id.clone()));
    }
    if definition.evidence_ids.is_empty() {
        findings.push(RegistryValidationError::EvidenceDrift(id.rust_id.clone()));
    }
    let mut evidence = BTreeSet::new();
    for evidence_id in &definition.evidence_ids {
        if evidence_id.is_empty() || !evidence.insert(evidence_id) {
            findings.push(RegistryValidationError::DuplicateEvidence(
                id.rust_id.clone(),
            ));
        }
    }
    if definition
        .descriptor
        .flags
        .contains(OperationFlags::MANDATORY)
        && !definition.availability.is_available()
    {
        findings.push(RegistryValidationError::MandatoryUnavailable(
            id.rust_id.clone(),
        ));
    }
    let sources = &definition.descriptor.migration.source_versions;
    let mut expected = sources.clone();
    expected.sort_unstable();
    expected.dedup();
    if expected != *sources || !sources.contains(&definition.descriptor.migration.target_version) {
        findings.push(RegistryValidationError::MigrationGap(id.rust_id.clone()));
    }
    let mut edges = BTreeSet::new();
    for migration in &definition.migrations {
        if migration.from_version == 0
            || migration.to_version != migration.from_version.saturating_add(1)
            || !edges.insert((migration.from_version, migration.to_version))
            || migration.evidence_id.is_empty()
            || !sources.contains(&migration.from_version)
            || !sources.contains(&migration.to_version)
        {
            findings.push(RegistryValidationError::MigrationGap(id.rust_id.clone()));
        }
    }
    for version in sources {
        if *version < definition.descriptor.migration.target_version
            && !edges.contains(&(*version, version.saturating_add(1)))
        {
            findings.push(RegistryValidationError::MigrationGap(id.rust_id.clone()));
        }
    }
}

pub(super) fn snapshot_hash(definitions: &[OperationDefinition]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(REGISTRY_SCHEMA.as_bytes());
    for definition in definitions {
        hasher.update(
            definition
                .descriptor
                .canonical_bytes()
                .expect("validated descriptor"),
        );
        hasher.update(definition.identity.implementation_id.as_bytes());
        hasher.update(definition.identity.version.to_be_bytes());
        hasher.update(definition.identity.build_id.as_bytes());
        for evidence in &definition.evidence_ids {
            hasher.update(evidence.as_bytes());
            hasher.update([0]);
        }
        for migration in &definition.migrations {
            hasher.update(migration.from_version.to_be_bytes());
            hasher.update(migration.to_version.to_be_bytes());
            hasher.update(migration.evidence_id.as_bytes());
        }
    }
    hasher.finalize().into()
}

pub(super) fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

pub(super) fn operation_descriptor_for(operation: &ProcessingOperation) -> DescriptorId {
    crate::registry_reconstruction::operation_descriptor_for(operation)
}

fn prepare_exposure(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_exposure(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_linear_offset(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_linear_offset(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_rgb_gain(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_rgb_gain(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_invert(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_invert(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_dither(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_dither(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_grain(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_grain(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_relight(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_relight(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_shadhi(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_shadhi(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_temperature(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_temperature(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_crop(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_crop(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_flip(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_flip(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_rotatepixels(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_rotatepixels(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_scalepixels(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_scalepixels(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_finalscale(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_finalscale(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_enlargecanvas(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_enlargecanvas(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_perspective(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_perspective(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_lenscorrection(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_lenscorrection(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_bloom(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_bloom(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_soften(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_soften(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_vignette(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_vignette(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_graduatednd(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        crate::operation::compile_graduatednd(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn definition(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
) -> OperationDefinition {
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Identity,
            true,
            false,
        )),
        None,
        Vec::new(),
        ImplementationIdentity::new(REGISTRY_BUILD_ID, 1, REGISTRY_BUILD_ID),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}

pub fn exposure_definition() -> OperationDefinition {
    geometry_definition_with_gpu(
        exposure_descriptor(),
        prepare_exposure,
        &[
            "iop.exposure.descriptor",
            "iop.exposure.cpu",
            "iop.exposure.wgpu",
        ],
        RoiKind::Identity,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.exposure.wgpu",
            1,
            vec![
                "f32-storage".to_owned(),
                "deterministic-row-major".to_owned(),
            ],
            vec!["rgba32float".to_owned()],
        )),
    )
}

pub fn linear_offset_definition() -> OperationDefinition {
    definition(
        linear_offset_descriptor(),
        prepare_linear_offset,
        &[
            "rusttable.linear-offset.descriptor",
            "rusttable.linear-offset.cpu",
        ],
    )
}

pub fn rgb_gain_definition() -> OperationDefinition {
    definition(
        rgb_gain_descriptor(),
        prepare_rgb_gain,
        &["iop.rgb-gain.descriptor", "iop.rgb-gain.cpu"],
    )
}

pub fn invert_definition() -> OperationDefinition {
    compatibility_definition(
        invert_descriptor(),
        prepare_invert,
        &[
            "iop.invert.params.v1-v2",
            "iop.invert.cpu.rgb",
            "iop.invert.deprecated-visibility",
        ],
        false,
    )
}

pub fn dither_definition() -> OperationDefinition {
    compatibility_definition(
        dither_descriptor(),
        prepare_dither,
        &[
            "iop.dither.params.v1-v2",
            "iop.dither.cpu.random-posterize",
            "iop.dither.cpu.floyd-steinberg-full-image",
        ],
        false,
    )
}

pub fn grain_definition() -> OperationDefinition {
    let descriptor = grain_descriptor();
    let compatibility = descriptor.id.compatibility_name.clone();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare_grain,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Identity,
            true,
            false,
        )),
        Some(GpuBinding::new(
            format!("rusttable.{compatibility}.wgsl"),
            1,
            crate::operations::grain::wgpu_passes()
                .into_iter()
                .map(str::to_owned),
            ["rgba32float".to_owned()],
        )),
        vec![MigrationBinding::new(1, 2, "grain.migration.v1-v2")],
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
            1,
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
        ),
        vec![
            "iop.grain.params.v1-v2".to_owned(),
            "iop.grain.cpu.deterministic-noise".to_owned(),
            "iop.grain.wgpu.point".to_owned(),
            "iop.grain.luminance-lut".to_owned(),
        ],
    )
}

pub fn vignette_definition() -> OperationDefinition {
    compatibility_definition_with_migrations(
        vignette_descriptor(),
        prepare_vignette,
        &[
            "iop.vignette.params.v1-v4",
            "iop.vignette.cpu.full-image",
            "iop.vignette.noise.counter",
            "iop.vignette.alpha-preserve",
        ],
        false,
        (1..4).map(|version| {
            MigrationBinding::new(
                version,
                version + 1,
                format!("vignette.migration.v{version}"),
            )
        }),
    )
}

pub fn graduatednd_definition() -> OperationDefinition {
    compatibility_definition_with_migrations(
        graduatednd_descriptor(),
        prepare_graduatednd,
        &[
            "iop.graduatednd.params.v1",
            "iop.graduatednd.cpu.full-image-line",
            "iop.graduatednd.presets",
            "iop.graduatednd.alpha-preserve",
        ],
        false,
        std::iter::empty(),
    )
}

pub fn relight_definition() -> OperationDefinition {
    compatibility_definition_with_migrations(
        relight_descriptor(),
        prepare_relight,
        &[
            "iop.relight.params.v1",
            "iop.relight.cpu.rgb-boundary",
            "iop.relight.presets",
            "iop.relight.deprecated-visibility",
        ],
        false,
        std::iter::empty(),
    )
}

pub fn shadhi_definition() -> OperationDefinition {
    compatibility_definition_with_migrations(
        shadhi_descriptor(),
        prepare_shadhi,
        &[
            "iop.shadhi.params.v1-v5",
            "iop.shadhi.migrations.v1-v5",
            "iop.shadhi.cpu.rgb-gaussian",
            "iop.shadhi.shared-gaussian-plan",
            "iop.shadhi.alpha-preserve",
        ],
        true,
        (1..5).map(|from| {
            MigrationBinding::new(
                from,
                from + 1,
                format!("shadhi.migration.v{from}-v{}", from + 1),
            )
        }),
    )
}

fn compatibility_definition(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
    full_image_analysis: bool,
) -> OperationDefinition {
    let compatibility = descriptor.id.compatibility_name.clone();
    compatibility_definition_with_migrations(
        descriptor,
        prepare,
        evidence,
        full_image_analysis,
        [MigrationBinding::new(
            1,
            2,
            format!("{compatibility}.migration.v1-v2"),
        )],
    )
}

fn compatibility_definition_with_migrations(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
    full_image_analysis: bool,
    migrations: impl IntoIterator<Item = MigrationBinding>,
) -> OperationDefinition {
    let compatibility = descriptor.id.compatibility_name.clone();
    let tileable = descriptor.flags.contains(OperationFlags::TILEABLE);
    let roi = descriptor.roi;
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            roi,
            tileable,
            full_image_analysis,
        )),
        None,
        migrations.into_iter().collect(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
            1,
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
        ),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}

pub fn temperature_definition() -> OperationDefinition {
    let descriptor = temperature_descriptor();
    let compatibility = descriptor.id.compatibility_name.clone();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare_temperature,
            crate::evaluate::execute_prepared_operation,
            RoiKind::Identity,
            true,
            false,
        )),
        None,
        (1..4)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("{compatibility}.migration.v{version}"),
                )
            })
            .collect(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
            1,
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
        ),
        vec![
            "iop.temperature.params.v2-v4".to_owned(),
            "iop.temperature.cpu".to_owned(),
            "iop.temperature.chromaticity-cat".to_owned(),
            "iop.temperature.receipt".to_owned(),
        ],
    )
}

fn geometry_definition(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
    roi: RoiKind,
    migrations: impl IntoIterator<Item = MigrationBinding>,
) -> OperationDefinition {
    let tileable = descriptor.flags.contains(OperationFlags::TILEABLE);
    let full_image_analysis = descriptor.flags.contains(OperationFlags::ANALYSIS);
    geometry_definition_with_traits(
        descriptor,
        prepare,
        evidence,
        roi,
        migrations,
        None,
        tileable,
        full_image_analysis,
    )
}

fn geometry_definition_with_gpu(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
    roi: RoiKind,
    migrations: impl IntoIterator<Item = MigrationBinding>,
    gpu: Option<GpuBinding>,
) -> OperationDefinition {
    let tileable = descriptor.flags.contains(OperationFlags::TILEABLE);
    let full_image_analysis = descriptor.flags.contains(OperationFlags::ANALYSIS);
    geometry_definition_with_traits(
        descriptor,
        prepare,
        evidence,
        roi,
        migrations,
        gpu,
        tileable,
        full_image_analysis,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn geometry_definition_with_traits(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
    roi: RoiKind,
    migrations: impl IntoIterator<Item = MigrationBinding>,
    gpu: Option<GpuBinding>,
    tileable: bool,
    full_image_analysis: bool,
) -> OperationDefinition {
    let compatibility = descriptor.id.compatibility_name.clone();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            roi,
            tileable,
            full_image_analysis,
        )),
        gpu,
        migrations.into_iter().collect(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
            1,
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
        ),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}

fn full_image_definition(
    descriptor: OperationDescriptor,
    prepare: CpuPrepare,
    evidence: &'static [&'static str],
) -> OperationDefinition {
    let compatibility = descriptor.id.compatibility_name.clone();
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            RoiKind::FullImage,
            false,
            true,
        )),
        None,
        Vec::new(),
        ImplementationIdentity::new(
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
            1,
            format!("{REGISTRY_BUILD_ID}.{compatibility}"),
        ),
        evidence.iter().map(|id| (*id).to_owned()).collect(),
    )
}

pub fn bloom_definition() -> OperationDefinition {
    full_image_definition(
        bloom_descriptor(),
        prepare_bloom,
        &[
            "iop.bloom.params.v1",
            "iop.bloom.cpu",
            "iop.bloom.convolution",
            "iop.bloom.alpha-preserve",
        ],
    )
}

pub fn soften_definition() -> OperationDefinition {
    full_image_definition(
        soften_descriptor(),
        prepare_soften,
        &[
            "iop.soften.params.v1",
            "iop.soften.cpu",
            "iop.soften.convolution",
            "iop.soften.alpha-preserve",
        ],
    )
}

pub fn crop_definition() -> OperationDefinition {
    geometry_definition(
        crop_descriptor(),
        prepare_crop,
        &["iop.crop.descriptor", "iop.crop.cpu", "iop.crop.geometry"],
        RoiKind::Crop,
        (1..3).map(|version| {
            MigrationBinding::new(version, version + 1, format!("crop.migration.v{version}"))
        }),
    )
}

pub fn flip_definition() -> OperationDefinition {
    geometry_definition(
        flip_descriptor(),
        prepare_flip,
        &["iop.flip.descriptor", "iop.flip.cpu", "iop.flip.geometry"],
        RoiKind::Distortion,
        (1..2).map(|version| {
            MigrationBinding::new(version, version + 1, format!("flip.migration.v{version}"))
        }),
    )
}

pub fn rotatepixels_definition() -> OperationDefinition {
    geometry_definition_with_gpu(
        rotatepixels_descriptor(),
        prepare_rotatepixels,
        &[
            "iop.rotatepixels.descriptor",
            "iop.rotatepixels.cpu",
            "iop.rotatepixels.wgpu",
        ],
        RoiKind::Distortion,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.rotatepixels.wgpu",
            1,
            Vec::<String>::new(),
            vec!["rgba32float".to_owned()],
        )),
    )
}

pub fn scalepixels_definition() -> OperationDefinition {
    geometry_definition_with_gpu(
        scalepixels_descriptor(),
        prepare_scalepixels,
        &[
            "iop.scalepixels.descriptor",
            "iop.scalepixels.cpu",
            "iop.scalepixels.wgpu",
        ],
        RoiKind::Scale,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.scalepixels.wgpu",
            1,
            vec![
                "scalepixels_image_resampler".to_owned(),
                "scalepixels_mask_resampler".to_owned(),
            ],
            vec!["rgba32float".to_owned()],
        )),
    )
}

pub fn finalscale_definition() -> OperationDefinition {
    geometry_definition_with_gpu(
        finalscale_descriptor(),
        prepare_finalscale,
        &[
            "iop.finalscale.descriptor",
            "iop.finalscale.cpu",
            "iop.finalscale.wgpu",
        ],
        RoiKind::Scale,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.finalscale.wgpu",
            1,
            vec!["finalscale_resampler".to_owned()],
            vec!["rgba32float".to_owned()],
        )),
    )
}

pub fn enlargecanvas_definition() -> OperationDefinition {
    geometry_definition_with_gpu(
        enlargecanvas_descriptor(),
        prepare_enlargecanvas,
        &[
            "iop.enlargecanvas.descriptor",
            "iop.enlargecanvas.cpu",
            "iop.enlargecanvas.wgpu",
        ],
        RoiKind::Scale,
        std::iter::empty(),
        Some(GpuBinding::new(
            "rusttable.enlargecanvas.wgpu",
            1,
            vec!["enlargecanvas_fill_copy".to_owned()],
            vec!["rgba32float".to_owned()],
        )),
    )
}

pub fn perspective_definition() -> OperationDefinition {
    geometry_definition_with_traits(
        crate::descriptor::perspective_descriptor(),
        prepare_perspective,
        &[
            "iop.perspective.descriptor",
            "iop.perspective.cpu",
            "iop.perspective.analysis",
        ],
        RoiKind::Distortion,
        (1..5).map(|version| {
            MigrationBinding::new(version, version + 1, format!("ashift.migration.v{version}"))
        }),
        None,
        false,
        true,
    )
}

pub fn lenscorrection_definition() -> OperationDefinition {
    geometry_definition(
        crate::descriptor::lenscorrection_descriptor(),
        prepare_lenscorrection,
        &[
            "iop.lenscorrection.descriptor",
            "iop.lenscorrection.cpu",
            "iop.lenscorrection.lensfun",
        ],
        RoiKind::Distortion,
        std::iter::empty(),
    )
}

/// Defines the static built-in factory slice used by startup and xtask.
#[macro_export]
macro_rules! builtin_operations {
    () => {
        &[
            $crate::registry::exposure_definition as $crate::registry::OperationDefinitionFactory,
            $crate::registry::basicadj_definition,
            $crate::registry::linear_offset_definition,
            $crate::registry::rgb_gain_definition,
            $crate::registry::invert_definition,
            $crate::registry::defringe_definition,
            $crate::registry::clahe_definition,
            $crate::registry::dither_definition,
            $crate::registry::grain_definition,
            $crate::registry::relight_definition,
            $crate::registry::shadhi_definition,
            $crate::registry::temperature_definition,
            $crate::registry::bloom_definition,
            $crate::registry::soften_definition,
            $crate::registry::censorize_definition,
            $crate::registry::vignette_definition,
            $crate::registry::graduatednd_definition,
            $crate::registry::crop_definition,
            $crate::registry::flip_definition,
            $crate::registry::rotatepixels_definition,
            $crate::registry::scalepixels_definition,
            $crate::registry::finalscale_definition,
            $crate::registry::enlargecanvas_definition,
            $crate::registry::perspective_definition,
            $crate::registry::lenscorrection_definition,
            $crate::registry::mask_manager_definition,
            $crate::registry::retouch_definition,
            $crate::registry_reconstruction::highlights_definition,
            $crate::registry_reconstruction::color_reconstruction_definition,
            $crate::registry_color::colorin_definition,
            $crate::registry_color::primaries_definition,
            $crate::registry_color::colorout_definition,
            $crate::registry_color::colorcorrection_definition,
        ]
    };
    ($($factory:path),+ $(,)?) => {
        &[$($factory as $crate::registry::OperationDefinitionFactory),+]
    };
}
pub static BUILTIN_OPERATIONS: &[OperationDefinitionFactory] = crate::builtin_operations!();
