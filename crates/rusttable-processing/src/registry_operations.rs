//! Built-in operation factories and registry validation helpers.

use super::{
    CpuFactory, CpuPrepare, ExecutionBackend, FactoryError, GpuBinding, ImplementationIdentity,
    MigrationBinding, OperationCapability, OperationDefinition, OperationDefinitionFactory,
    PreparedCpuOperation, REGISTRY_BUILD_ID, REGISTRY_SCHEMA, RegistryValidationError, RoiKind,
};
use crate::ProcessingOperation;
use crate::descriptor::{
    DescriptorId, OperationDescriptor, OperationFlags, crop_descriptor, exposure_descriptor,
    flip_descriptor, linear_offset_descriptor, rgb_gain_descriptor, temperature_descriptor,
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
    if !cpu_present {
        findings.push(RegistryValidationError::MissingCpu(id.rust_id.clone()));
    }
    if definition.descriptor.capability.cpu_supported != cpu_present
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
        })
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
        ProcessingOperation::compile_crop(operation).map_err(FactoryError::Operation)?,
        descriptor,
        crate::evaluate::execute_prepared_operation,
    )
}

fn prepare_flip(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError> {
    PreparedCpuOperation::prepare(
        ProcessingOperation::compile_flip(operation).map_err(FactoryError::Operation)?,
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
    definition(
        exposure_descriptor(),
        prepare_exposure,
        &["iop.exposure.descriptor", "iop.exposure.cpu"],
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
    let compatibility = descriptor.id.compatibility_name.clone();
    let tileable = descriptor.flags.contains(OperationFlags::TILEABLE);
    OperationDefinition::new(
        descriptor,
        Some(CpuFactory::new(
            prepare,
            crate::evaluate::execute_prepared_operation,
            roi,
            tileable,
            false,
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

/// Defines the static built-in factory slice used by startup and xtask.
#[macro_export]
macro_rules! builtin_operations {
    () => {
        &[
            $crate::registry::exposure_definition as $crate::registry::OperationDefinitionFactory,
            $crate::registry::linear_offset_definition,
            $crate::registry::rgb_gain_definition,
            $crate::registry::temperature_definition,
            $crate::registry::crop_definition,
            $crate::registry::flip_definition,
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
