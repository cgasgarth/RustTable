//! Static operation definitions, capability discovery, and safe CPU factories.

use crate::descriptor::{DescriptorId, OperationDescriptor, RoiKind};
use crate::{
    EvaluationError, LinearRgb, OperationCompileError, PipelineStepIndex, ProcessingOperation,
};
use rusttable_color::ColorEncoding;
use rusttable_core::{Operation, OperationId, OperationKey};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fmt::Write as _;
use std::sync::OnceLock;

pub const REGISTRY_SCHEMA: &str = "rusttable.operation-registry.v1";
pub const REGISTRY_BUILD_ID: &str = "rusttable-processing.operation-registry.v1";

/// Prepares owned CPU state for a definition.
pub type CpuPrepare = fn(
    operation: &Operation,
    descriptor: &DescriptorId,
) -> Result<PreparedCpuOperation, FactoryError>;

/// Executes one row or tile for a prepared definition.
pub type CpuExecute = fn(
    operation: &PreparedCpuOperation,
    step_index: PipelineStepIndex,
    pixels: &mut [LinearRgb],
    dimensions: crate::RasterDimensions,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError>;

/// Safe, owned CPU factory binding.
#[derive(Clone, Copy)]
pub struct CpuFactory {
    prepare: CpuPrepare,
    execute: CpuExecute,
    roi: RoiKind,
    tileable: bool,
    full_image_analysis: bool,
}

impl fmt::Debug for CpuFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CpuFactory")
            .field("roi", &self.roi)
            .field("tileable", &self.tileable)
            .field("full_image_analysis", &self.full_image_analysis)
            .finish_non_exhaustive()
    }
}

impl CpuFactory {
    #[must_use]
    pub const fn new(
        prepare: CpuPrepare,
        execute: CpuExecute,
        roi: RoiKind,
        tileable: bool,
        full_image_analysis: bool,
    ) -> Self {
        Self {
            prepare,
            execute,
            roi,
            tileable,
            full_image_analysis,
        }
    }

    #[must_use]
    pub const fn roi(self) -> RoiKind {
        self.roi
    }

    #[must_use]
    pub const fn tileable(self) -> bool {
        self.tileable
    }

    #[must_use]
    pub const fn full_image_analysis(self) -> bool {
        self.full_image_analysis
    }
}

/// Optional backend-neutral GPU binding metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuBinding {
    binding_id: String,
    tier: u8,
    required_features: Vec<String>,
    required_formats: Vec<String>,
}

impl GpuBinding {
    pub fn new(
        binding_id: impl Into<String>,
        tier: u8,
        required_features: impl IntoIterator<Item = String>,
        required_formats: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            binding_id: binding_id.into(),
            tier,
            required_features: required_features.into_iter().collect(),
            required_formats: required_formats.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    #[must_use]
    pub const fn tier(&self) -> u8 {
        self.tier
    }

    #[must_use]
    pub fn required_features(&self) -> &[String] {
        &self.required_features
    }

    #[must_use]
    pub fn required_formats(&self) -> &[String] {
        &self.required_formats
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationIdentity {
    implementation_id: String,
    version: u16,
    build_id: String,
}

impl ImplementationIdentity {
    pub fn new(
        implementation_id: impl Into<String>,
        version: u16,
        build_id: impl Into<String>,
    ) -> Self {
        Self {
            implementation_id: implementation_id.into(),
            version,
            build_id: build_id.into(),
        }
    }

    #[must_use]
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }

    #[must_use]
    pub fn build_id(&self) -> &str {
        &self.build_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationBinding {
    from_version: u16,
    to_version: u16,
    evidence_id: String,
}

impl MigrationBinding {
    pub fn new(from_version: u16, to_version: u16, evidence_id: impl Into<String>) -> Self {
        Self {
            from_version,
            to_version,
            evidence_id: evidence_id.into(),
        }
    }

    #[must_use]
    pub const fn from_version(&self) -> u16 {
        self.from_version
    }

    #[must_use]
    pub const fn to_version(&self) -> u16 {
        self.to_version
    }

    #[must_use]
    pub fn evidence_id(&self) -> &str {
        &self.evidence_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionAvailability {
    Available,
    Unavailable { reason: String },
}

impl DefinitionAvailability {
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Clone)]
pub struct OperationDefinition {
    descriptor: OperationDescriptor,
    cpu: Option<CpuFactory>,
    gpu: Option<GpuBinding>,
    migrations: Vec<MigrationBinding>,
    identity: ImplementationIdentity,
    evidence_ids: Vec<String>,
    availability: DefinitionAvailability,
}

impl fmt::Debug for OperationDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationDefinition")
            .field("descriptor", &self.descriptor.id)
            .field("cpu", &self.cpu)
            .field("gpu", &self.gpu)
            .field("migrations", &self.migrations)
            .field("identity", &self.identity)
            .field("evidence_ids", &self.evidence_ids)
            .field("availability", &self.availability)
            .finish()
    }
}

impl OperationDefinition {
    #[must_use]
    pub fn new(
        descriptor: OperationDescriptor,
        cpu: Option<CpuFactory>,
        gpu: Option<GpuBinding>,
        migrations: Vec<MigrationBinding>,
        identity: ImplementationIdentity,
        evidence_ids: Vec<String>,
    ) -> Self {
        Self {
            descriptor,
            cpu,
            gpu,
            migrations,
            identity,
            evidence_ids,
            availability: DefinitionAvailability::Available,
        }
    }

    #[must_use]
    pub fn with_availability(mut self, availability: DefinitionAvailability) -> Self {
        self.availability = availability;
        self
    }

    #[must_use]
    pub const fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn cpu(&self) -> Option<CpuFactory> {
        self.cpu
    }

    #[must_use]
    pub fn gpu(&self) -> Option<&GpuBinding> {
        self.gpu.as_ref()
    }

    #[must_use]
    pub fn migrations(&self) -> &[MigrationBinding] {
        &self.migrations
    }

    #[must_use]
    pub const fn identity(&self) -> &ImplementationIdentity {
        &self.identity
    }

    #[must_use]
    pub fn evidence_ids(&self) -> &[String] {
        &self.evidence_ids
    }

    #[must_use]
    pub const fn availability(&self) -> &DefinitionAvailability {
        &self.availability
    }
}

pub type OperationDefinitionFactory = fn() -> OperationDefinition;

/// A backend-neutral device capability snapshot supplied by #177/#290.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCapabilitySnapshot {
    gpu_available: bool,
    tier: Option<u8>,
    features: BTreeSet<String>,
    formats: BTreeSet<String>,
}

impl DeviceCapabilitySnapshot {
    #[must_use]
    pub fn cpu_only() -> Self {
        Self {
            gpu_available: false,
            tier: None,
            features: BTreeSet::new(),
            formats: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn gpu(
        tier: u8,
        features: impl IntoIterator<Item = String>,
        formats: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            gpu_available: true,
            tier: Some(tier),
            features: features.into_iter().collect(),
            formats: formats.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn gpu_available(&self) -> bool {
        self.gpu_available
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionBackend {
    Cpu,
    Gpu,
    CpuFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationCapability {
    pub backend: ExecutionBackend,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactoryError {
    DescriptorMismatch {
        expected: DescriptorId,
        actual: DescriptorId,
    },
    Operation(OperationCompileError),
}

impl fmt::Display for FactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DescriptorMismatch { expected, actual } => write!(
                formatter,
                "factory descriptor mismatch: expected {}, got {}",
                expected.rust_id, actual.rust_id
            ),
            Self::Operation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FactoryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryValidationError {
    Descriptor(DescriptorId, String),
    DuplicateIdentity(String),
    MissingCpu(String),
    CapabilityMismatch(String),
    MigrationGap(String),
    MandatoryUnavailable(String),
    IdentityDrift(String),
    EvidenceDrift(String),
    DuplicateEvidence(String),
}

impl fmt::Display for RegistryValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Descriptor(id, error) => {
                write!(formatter, "{}: invalid descriptor: {error}", id.rust_id)
            }
            Self::DuplicateIdentity(id) => write!(formatter, "duplicate operation identity {id}"),
            Self::MissingCpu(id) => write!(formatter, "{id}: CPU factory is mandatory"),
            Self::CapabilityMismatch(id) => write!(formatter, "{id}: capability binding mismatch"),
            Self::MigrationGap(id) => write!(formatter, "{id}: migration chain has a gap"),
            Self::MandatoryUnavailable(id) => {
                write!(formatter, "{id}: mandatory operation is unavailable")
            }
            Self::IdentityDrift(id) => {
                write!(formatter, "{id}: implementation/build identity drift")
            }
            Self::EvidenceDrift(id) => write!(formatter, "{id}: evidence is missing"),
            Self::DuplicateEvidence(id) => write!(formatter, "{id}: evidence ID is duplicated"),
        }
    }
}

impl std::error::Error for RegistryValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryBuildError {
    findings: Vec<RegistryValidationError>,
}

impl RegistryBuildError {
    #[must_use]
    pub fn findings(&self) -> &[RegistryValidationError] {
        &self.findings
    }
}

impl fmt::Display for RegistryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "operation registry rejected {} definition(s)",
            self.findings.len()
        )
    }
}

impl std::error::Error for RegistryBuildError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryLookupError {
    UnknownOperation(OperationKey),
    Factory {
        operation_id: OperationId,
        source: Box<FactoryError>,
    },
}

impl fmt::Display for RegistryLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOperation(key) => write!(formatter, "unknown operation {key}"),
            Self::Factory {
                operation_id,
                source,
            } => write!(
                formatter,
                "operation {operation_id} factory failed: {source}"
            ),
        }
    }
}

impl std::error::Error for RegistryLookupError {}

/// Owned preparation returned by a CPU factory.  The descriptor ID and
/// executor travel together, preventing a prepared value from being sent to a
/// different operation's executor.
#[derive(Debug, Clone)]
pub struct PreparedCpuOperation {
    descriptor: DescriptorId,
    operation: ProcessingOperation,
    execute: CpuExecute,
}

impl PartialEq for PreparedCpuOperation {
    fn eq(&self, other: &Self) -> bool {
        self.descriptor == other.descriptor && self.operation == other.operation
    }
}

impl Eq for PreparedCpuOperation {}

impl PreparedCpuOperation {
    #[must_use]
    pub const fn operation(&self) -> &ProcessingOperation {
        &self.operation
    }

    #[must_use]
    pub const fn descriptor(&self) -> &DescriptorId {
        &self.descriptor
    }

    /// Executes the owned preparation with its registry-bound executor.
    ///
    /// # Errors
    ///
    /// Returns the deterministic arithmetic error reported by the operation executor.
    pub fn execute(
        &self,
        step_index: PipelineStepIndex,
        pixels: &mut [LinearRgb],
        dimensions: crate::RasterDimensions,
        pixel_index_offset: usize,
    ) -> Result<(), EvaluationError> {
        (self.execute)(self, step_index, pixels, dimensions, pixel_index_offset)
    }

    pub(crate) fn prepare(
        operation: ProcessingOperation,
        descriptor: &DescriptorId,
        execute: CpuExecute,
    ) -> Result<Self, FactoryError> {
        let actual = operation_descriptor_for(&operation);
        if actual != *descriptor {
            return Err(FactoryError::DescriptorMismatch {
                expected: descriptor.clone(),
                actual,
            });
        }
        Ok(Self {
            descriptor: descriptor.clone(),
            operation,
            execute,
        })
    }

    pub(crate) const fn with_executor(mut self, execute: CpuExecute) -> Self {
        self.execute = execute;
        self
    }
}

/// Immutable canonical registry snapshot.
#[derive(Debug, Clone)]
pub struct RegistrySnapshot {
    definitions: Vec<OperationDefinition>,
    by_rust_id: BTreeMap<String, usize>,
    identity_hash: [u8; 32],
}

impl RegistrySnapshot {
    /// Builds and validates a snapshot; declaration order is not observable.
    ///
    /// # Errors
    ///
    /// Returns all deterministic definition validation findings when publication is invalid.
    pub fn try_new(factories: &[OperationDefinitionFactory]) -> Result<Self, RegistryBuildError> {
        let mut definitions: Vec<_> = factories.iter().map(|factory| factory()).collect();
        definitions.sort_by(|left, right| left.descriptor.id.cmp(&right.descriptor.id));
        let mut findings = Vec::new();
        let mut identities = BTreeSet::new();
        for definition in &definitions {
            let id = &definition.descriptor.id;
            if !identities.insert(id.rust_id.clone()) {
                findings.push(RegistryValidationError::DuplicateIdentity(
                    id.rust_id.clone(),
                ));
            }
            validate_definition(definition, &mut findings);
        }
        if !findings.is_empty() {
            return Err(RegistryBuildError { findings });
        }
        let by_rust_id = definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (definition.descriptor.id.rust_id.clone(), index))
            .collect();
        let identity_hash = snapshot_hash(&definitions);
        Ok(Self {
            definitions,
            by_rust_id,
            identity_hash,
        })
    }

    #[must_use]
    pub fn definitions(&self) -> &[OperationDefinition] {
        &self.definitions
    }

    #[must_use]
    pub fn identity_hash(&self) -> [u8; 32] {
        self.identity_hash
    }

    #[must_use]
    pub fn identity_hash_hex(&self) -> String {
        hex(&self.identity_hash)
    }

    #[must_use]
    pub fn definition(&self, rust_id: &str) -> Option<&OperationDefinition> {
        self.by_rust_id
            .get(rust_id)
            .map(|index| &self.definitions[*index])
    }

    /// Prepares an operation using the definition selected by its permanent identity.
    ///
    /// # Errors
    ///
    /// Returns an unknown-operation error for opaque imported identities or a typed factory error.
    pub fn prepare_cpu(
        &self,
        operation: &Operation,
    ) -> Result<PreparedCpuOperation, RegistryLookupError> {
        let Some(definition) = self.definition(operation.key().as_str()) else {
            return Err(RegistryLookupError::UnknownOperation(
                operation.key().clone(),
            ));
        };
        let Some(cpu) = definition.cpu else {
            return Err(RegistryLookupError::Factory {
                operation_id: operation.id(),
                source: Box::new(FactoryError::DescriptorMismatch {
                    expected: definition.descriptor.id.clone(),
                    actual: definition.descriptor.id.clone(),
                }),
            });
        };
        let prepared = (cpu.prepare)(operation, &definition.descriptor.id).map_err(|source| {
            RegistryLookupError::Factory {
                operation_id: operation.id(),
                source: Box::new(source),
            }
        })?;
        Ok(prepared.with_executor(cpu.execute))
    }

    #[must_use]
    pub fn capability(
        &self,
        rust_id: &str,
        device: &DeviceCapabilitySnapshot,
        encoding: ColorEncoding,
        mode: Option<&str>,
    ) -> Option<OperationCapability> {
        let definition = self.definition(rust_id)?;
        if !definition.availability.is_available() {
            return Some(OperationCapability {
                backend: ExecutionBackend::Cpu,
                available: false,
                reason: definition.availability.reason().map(str::to_owned),
            });
        }
        if !definition.descriptor.io.input.encodings.contains(&encoding)
            || !definition
                .descriptor
                .io
                .output
                .encodings
                .contains(&encoding)
        {
            return Some(unavailable("color encoding is not supported"));
        }
        if mode.is_some_and(|mode| {
            !definition.descriptor.capability.modes.is_empty()
                && !definition
                    .descriptor
                    .capability
                    .modes
                    .iter()
                    .any(|candidate| candidate == mode)
        }) {
            return Some(unavailable("operation mode is not supported"));
        }
        if let Some(gpu) = &definition.gpu
            && device.gpu_available
            && device.tier.is_some_and(|tier| tier >= gpu.tier)
            && gpu
                .required_features
                .iter()
                .all(|feature| device.features.contains(feature))
            && gpu
                .required_formats
                .iter()
                .all(|format| device.formats.contains(format))
        {
            return Some(OperationCapability {
                backend: ExecutionBackend::Gpu,
                available: true,
                reason: None,
            });
        }
        if definition.cpu.is_some() {
            return Some(OperationCapability {
                backend: if device.gpu_available && definition.gpu.is_some() {
                    ExecutionBackend::CpuFallback
                } else {
                    ExecutionBackend::Cpu
                },
                available: true,
                reason: if device.gpu_available && definition.gpu.is_some() {
                    Some("GPU binding unavailable; using mandatory CPU fallback".to_owned())
                } else {
                    None
                },
            });
        }
        Some(unavailable("no CPU factory is available"))
    }

    /// Returns a bounded, deterministic receipt suitable for source-accounting checks.
    #[must_use]
    pub fn receipt(&self) -> String {
        let mut receipt = format!(
            "schema = \"{REGISTRY_SCHEMA}\"\nbuild_id = \"{REGISTRY_BUILD_ID}\"\nsnapshot_hash = \"{}\"\n\n",
            self.identity_hash_hex()
        );
        for definition in &self.definitions {
            let id = &definition.descriptor.id;
            let _ = write!(
                receipt,
                "[[operation]]\nrust_id = \"{}\"\ncompatibility_name = \"{}\"\ndescriptor_version = {}\nparameter_version = {}\nimplementation_version = {}\nstatus = \"{}\"\n\n",
                id.rust_id,
                id.compatibility_name,
                id.schema_version,
                id.parameter_version,
                id.implementation_version,
                if definition.availability.is_available() {
                    "available"
                } else {
                    "unavailable"
                }
            );
        }
        receipt
    }
}

#[path = "registry_operations.rs"]
mod registry_operations;
pub use registry_operations::{
    BUILTIN_OPERATIONS, bloom_definition, crop_definition, dither_definition,
    enlargecanvas_definition, exposure_definition, finalscale_definition, flip_definition,
    invert_definition, lenscorrection_definition, linear_offset_definition, perspective_definition,
    relight_definition, rgb_gain_definition, rotatepixels_definition, scalepixels_definition,
    shadhi_definition, soften_definition, temperature_definition,
};
use registry_operations::{
    hex, operation_descriptor_for, snapshot_hash, unavailable, validate_definition,
};

/// Returns the process-wide immutable first-party registry snapshot.
///
/// # Panics
///
/// Panics only if the checked-in built-in definitions fail their own validation.
#[must_use]
pub fn builtin_registry() -> &'static RegistrySnapshot {
    static SNAPSHOT: OnceLock<RegistrySnapshot> = OnceLock::new();
    SNAPSHOT
        .get_or_init(|| RegistrySnapshot::try_new(BUILTIN_OPERATIONS).expect("built-in registry"))
}
