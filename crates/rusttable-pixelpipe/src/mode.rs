#![allow(clippy::missing_errors_doc)]

use std::fmt;

use rusttable_color::Precision;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};

use crate::{ColorIdentity, PipelinePurpose, PipelineSnapshot, PipelineSnapshotIdentity};

pub const MODE_SCHEMA_VERSION: u16 = 1;

/// A quality preset is explicit in every mode request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModeQuality {
    Interactive,
    Balanced,
    High,
    Exact,
}

impl ModeQuality {
    #[must_use]
    pub const fn is_exact(self) -> bool {
        matches!(self, Self::Exact)
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Balanced => "balanced",
            Self::High => "high",
            Self::Exact => "exact",
        }
    }
}

/// Stable identity for a preview surface, consumer, or export destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetIdentity([u8; 32]);

impl TargetIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn preview_surface(name: &str) -> Self {
        Self(Self::named_bytes(b"preview-surface:", name))
    }
    #[must_use]
    pub fn consumer(name: &str) -> Self {
        Self(Self::named_bytes(b"consumer:", name))
    }
    #[must_use]
    pub fn export_destination(name: &str) -> Self {
        Self(Self::named_bytes(b"export-destination:", name))
    }

    fn named_bytes(prefix: &[u8], name: &str) -> [u8; 32] {
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(name.as_bytes());
        Sha256::digest(bytes).into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EmbeddedPreviewProvenance([u8; 32]);

impl EmbeddedPreviewProvenance {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// The requested sampling rule. Scaling remains a pipeline responsibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

/// Whether the request needs an exact mask result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskRequest {
    None,
    Required,
}

/// Whether the request needs an analysis result at a pipeline boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisRequest {
    None,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LatencyClass {
    Interactive,
    Settled,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Synchronization {
    Asynchronous,
    Synchronous,
}

/// Algorithmic degradation is deliberately separate from backend fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DegradationPolicy {
    None,
    ApprovedPreviewOnly,
    CpuFallback,
}

impl DegradationPolicy {
    #[must_use]
    pub const fn allows_approximation(self, purpose: PipelinePurpose) -> bool {
        matches!(self, Self::ApprovedPreviewOnly)
            && matches!(
                purpose,
                PipelinePurpose::Preview | PipelinePurpose::Thumbnail
            )
    }

    #[must_use]
    pub const fn allows_cpu_fallback(self) -> bool {
        matches!(self, Self::CpuFallback)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendPolicy {
    ExactRequestedBackend,
    CpuFallbackAllowed,
}

/// A permanent, operation-owned approximation identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApproximationId(String);

impl ApproximationId {
    /// Creates a bounded ASCII identifier that can be retained in receipts.
    pub fn new(value: impl Into<String>) -> Result<Self, ModeRequestError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value.is_ascii()
            || value.bytes().any(|byte| {
                !(byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_'))
            })
        {
            return Err(ModeRequestError::InvalidApproximationId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationInclusion {
    Processing,
    Diagnostic,
    Presentation,
}

/// Immutable capability metadata supplied by the #265 registry.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeOperationCapability {
    operation_id: u128,
    inclusion: OperationInclusion,
    mandatory: bool,
    purposes: Vec<PipelinePurpose>,
    approximations: Vec<ApproximationId>,
    supports_exact: bool,
    supports_masks: bool,
    supports_analysis: bool,
    hidden: bool,
}

impl ModeOperationCapability {
    #[must_use]
    pub fn new(operation_id: u128, inclusion: OperationInclusion, mandatory: bool) -> Self {
        Self {
            operation_id,
            inclusion,
            mandatory,
            purposes: vec![
                PipelinePurpose::Preview,
                PipelinePurpose::Full,
                PipelinePurpose::Thumbnail,
                PipelinePurpose::Export,
            ],
            approximations: Vec::new(),
            supports_exact: true,
            supports_masks: false,
            supports_analysis: false,
            hidden: false,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub const fn inclusion(&self) -> OperationInclusion {
        self.inclusion
    }
    #[must_use]
    pub const fn mandatory(&self) -> bool {
        self.mandatory
    }
    #[must_use]
    pub fn purposes(&self) -> &[PipelinePurpose] {
        &self.purposes
    }
    #[must_use]
    pub fn approximations(&self) -> &[ApproximationId] {
        &self.approximations
    }
    #[must_use]
    pub fn supports_purpose(&self, purpose: PipelinePurpose) -> bool {
        self.purposes.contains(&purpose)
    }
    #[must_use]
    pub const fn supports_exact(&self) -> bool {
        self.supports_exact
    }
    #[must_use]
    pub const fn supports_masks(&self) -> bool {
        self.supports_masks
    }
    #[must_use]
    pub const fn supports_analysis(&self) -> bool {
        self.supports_analysis
    }
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn for_purposes(mut self, purposes: impl IntoIterator<Item = PipelinePurpose>) -> Self {
        self.purposes = purposes.into_iter().collect();
        self
    }

    #[must_use]
    pub fn with_approximation(mut self, approximation: ApproximationId) -> Self {
        self.approximations.push(approximation);
        self
    }

    #[must_use]
    pub const fn exact(mut self, supports: bool) -> Self {
        self.supports_exact = supports;
        self
    }

    #[must_use]
    pub const fn masks(mut self, supports: bool) -> Self {
        self.supports_masks = supports;
        self
    }

    #[must_use]
    pub const fn analysis(mut self, supports: bool) -> Self {
        self.supports_analysis = supports;
        self
    }
    #[must_use]
    pub const fn hidden_operation(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

/// A real, deterministic basic raster stack capability fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicStackFixture {
    operations: Vec<ModeOperationCapability>,
}

impl BasicStackFixture {
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn raster() -> Self {
        let preview_approximation =
            ApproximationId::new("exposure.preview-fast").expect("constant");
        Self {
            operations: vec![
                ModeOperationCapability::new(1, OperationInclusion::Processing, false)
                    .with_approximation(preview_approximation),
                ModeOperationCapability::new(2, OperationInclusion::Processing, true),
                ModeOperationCapability::new(3, OperationInclusion::Diagnostic, false)
                    .for_purposes([PipelinePurpose::Preview, PipelinePurpose::Full]),
            ],
        }
    }

    #[must_use]
    pub fn operations(&self) -> &[ModeOperationCapability] {
        &self.operations
    }
}

/// One explicit request. It contains no caller, window, thread, or size inference.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeRequest {
    purpose: PipelinePurpose,
    quality: ModeQuality,
    target: TargetIdentity,
    output: crate::OutputSpec,
    roi: Roi,
    interpolation: Interpolation,
    precision: Precision,
    latency: LatencyClass,
    synchronization: Synchronization,
    masks: MaskRequest,
    analysis: AnalysisRequest,
    degradation: DegradationPolicy,
    backend: BackendPolicy,
    source_preview: bool,
    source_preview_provenance: Option<EmbeddedPreviewProvenance>,
}

impl ModeRequest {
    #[must_use]
    pub fn new(
        purpose: PipelinePurpose,
        quality: ModeQuality,
        target: TargetIdentity,
        output: crate::OutputSpec,
    ) -> Self {
        let roi = output.roi();
        let exact = matches!(purpose, PipelinePurpose::Full | PipelinePurpose::Export);
        Self {
            purpose,
            quality,
            target,
            output,
            roi,
            interpolation: if exact {
                Interpolation::Lanczos
            } else {
                Interpolation::Bilinear
            },
            precision: if exact {
                Precision::F64
            } else {
                Precision::F32
            },
            latency: if exact {
                LatencyClass::Background
            } else if matches!(purpose, PipelinePurpose::Thumbnail) {
                LatencyClass::Settled
            } else {
                LatencyClass::Interactive
            },
            synchronization: if exact {
                Synchronization::Synchronous
            } else {
                Synchronization::Asynchronous
            },
            masks: MaskRequest::None,
            analysis: AnalysisRequest::None,
            degradation: if exact {
                DegradationPolicy::None
            } else {
                DegradationPolicy::ApprovedPreviewOnly
            },
            backend: BackendPolicy::CpuFallbackAllowed,
            source_preview: false,
            source_preview_provenance: None,
        }
    }

    #[must_use]
    pub fn preview(output: crate::OutputSpec, target: TargetIdentity) -> Self {
        Self::new(
            PipelinePurpose::Preview,
            ModeQuality::Balanced,
            target,
            output,
        )
    }

    #[must_use]
    pub fn full(output: crate::OutputSpec, target: TargetIdentity) -> Self {
        Self::new(PipelinePurpose::Full, ModeQuality::Exact, target, output)
    }

    #[must_use]
    pub fn thumbnail(output: crate::OutputSpec, target: TargetIdentity) -> Self {
        Self::new(
            PipelinePurpose::Thumbnail,
            ModeQuality::Balanced,
            target,
            output,
        )
    }

    #[must_use]
    pub fn export(output: crate::OutputSpec, target: TargetIdentity) -> Self {
        Self::new(PipelinePurpose::Export, ModeQuality::Exact, target, output)
    }

    #[must_use]
    pub const fn purpose(&self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn quality(&self) -> ModeQuality {
        self.quality
    }
    #[must_use]
    pub const fn target(&self) -> TargetIdentity {
        self.target
    }
    #[must_use]
    pub const fn output(&self) -> &crate::OutputSpec {
        &self.output
    }
    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }
    #[must_use]
    pub const fn interpolation(&self) -> Interpolation {
        self.interpolation
    }
    #[must_use]
    pub const fn precision(&self) -> Precision {
        self.precision
    }
    #[must_use]
    pub const fn latency(&self) -> LatencyClass {
        self.latency
    }
    #[must_use]
    pub const fn synchronization(&self) -> Synchronization {
        self.synchronization
    }
    #[must_use]
    pub const fn masks(&self) -> MaskRequest {
        self.masks
    }
    #[must_use]
    pub const fn analysis(&self) -> AnalysisRequest {
        self.analysis
    }
    #[must_use]
    pub const fn degradation(&self) -> DegradationPolicy {
        self.degradation
    }
    #[must_use]
    pub const fn backend(&self) -> BackendPolicy {
        self.backend
    }
    #[must_use]
    pub const fn source_preview(&self) -> bool {
        self.source_preview
    }
    #[must_use]
    pub const fn source_preview_provenance(&self) -> Option<EmbeddedPreviewProvenance> {
        self.source_preview_provenance
    }

    #[must_use]
    pub const fn with_quality(mut self, value: ModeQuality) -> Self {
        self.quality = value;
        self
    }
    #[must_use]
    pub const fn with_roi(mut self, value: Roi) -> Self {
        self.roi = value;
        self
    }
    #[must_use]
    pub const fn with_interpolation(mut self, value: Interpolation) -> Self {
        self.interpolation = value;
        self
    }
    #[must_use]
    pub const fn with_precision(mut self, value: Precision) -> Self {
        self.precision = value;
        self
    }
    #[must_use]
    pub const fn with_latency(mut self, value: LatencyClass) -> Self {
        self.latency = value;
        self
    }
    #[must_use]
    pub const fn with_synchronization(mut self, value: Synchronization) -> Self {
        self.synchronization = value;
        self
    }
    #[must_use]
    pub const fn with_masks(mut self, value: MaskRequest) -> Self {
        self.masks = value;
        self
    }
    #[must_use]
    pub const fn with_analysis(mut self, value: AnalysisRequest) -> Self {
        self.analysis = value;
        self
    }
    #[must_use]
    pub const fn with_degradation(mut self, value: DegradationPolicy) -> Self {
        self.degradation = value;
        self
    }
    #[must_use]
    pub const fn with_backend(mut self, value: BackendPolicy) -> Self {
        self.backend = value;
        self
    }
    #[must_use]
    pub const fn with_source_preview(mut self, value: bool) -> Self {
        self.source_preview = value;
        self
    }
    #[must_use]
    pub const fn with_embedded_preview(mut self, provenance: EmbeddedPreviewProvenance) -> Self {
        self.source_preview = true;
        self.source_preview_provenance = Some(provenance);
        self
    }

    pub(crate) fn validate(&self) -> Result<(), ModeRequestError> {
        self.roi
            .within(self.output.dimensions())
            .map_err(|_| ModeRequestError::InvalidRoi)?;
        if self.output.color().encoding() == rusttable_color::ColorEncoding::Unspecified {
            return Err(ModeRequestError::MissingOutputIdentity);
        }
        if matches!(
            self.purpose,
            PipelinePurpose::Full | PipelinePurpose::Export
        ) && !matches!(
            self.degradation,
            DegradationPolicy::None | DegradationPolicy::CpuFallback
        ) {
            return Err(ModeRequestError::DegradationForbidden);
        }
        if matches!(self.purpose, PipelinePurpose::Thumbnail)
            && (self.roi.width() > self.output.dimensions().width()
                || self.roi.height() > self.output.dimensions().height())
        {
            return Err(ModeRequestError::ThumbnailUpscale);
        }
        if matches!(
            self.purpose,
            PipelinePurpose::Full | PipelinePurpose::Export
        ) && !self.quality.is_exact()
        {
            return Err(ModeRequestError::ExactQualityRequired);
        }
        if matches!(self.backend, BackendPolicy::ExactRequestedBackend)
            && matches!(self.degradation, DegradationPolicy::CpuFallback)
        {
            return Err(ModeRequestError::ContradictoryBackendFallback);
        }
        Ok(())
    }

    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"rusttable.pixelpipe.mode-request.v1");
        bytes.push(self.purpose.tag().as_bytes()[0]);
        bytes.push(self.quality.tag().as_bytes()[0]);
        bytes.extend_from_slice(&self.target.as_bytes());
        bytes.extend_from_slice(&self.output.dimensions().width().to_le_bytes());
        bytes.extend_from_slice(&self.output.dimensions().height().to_le_bytes());
        for value in [
            self.roi.x(),
            self.roi.y(),
            self.roi.width(),
            self.roi.height(),
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(self.interpolation as u8);
        bytes.push(self.precision as u8);
        bytes.push(self.latency as u8);
        bytes.push(self.synchronization as u8);
        bytes.push(self.masks as u8);
        bytes.push(self.analysis as u8);
        bytes.push(self.degradation as u8);
        bytes.push(self.backend as u8);
        bytes.push(self.output.format().sample_type() as u8);
        bytes.push(self.output.format().channels() as u8);
        bytes.push(self.output.format().alpha() as u8);
        bytes.push(self.output.format().byte_order() as u8);
        bytes.push(self.output.format().storage() as u8);
        for value in self.output.background().rgba() {
            bytes.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        bytes.push(u8::from(self.source_preview));
        if let Some(provenance) = self.source_preview_provenance {
            bytes.extend_from_slice(&provenance.as_bytes());
        }
        write_color(self.output.color(), &mut bytes);
        bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeRequestError {
    InvalidApproximationId,
    InvalidRoi,
    MissingOutputIdentity,
    DegradationForbidden,
    ThumbnailUpscale,
    ExactQualityRequired,
    ContradictoryBackendFallback,
}

impl fmt::Display for ModeRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidApproximationId => "mode approximation identity is invalid",
            Self::InvalidRoi => "mode ROI is invalid or outside output bounds",
            Self::MissingOutputIdentity => "mode output color identity is missing",
            Self::DegradationForbidden => "mode degradation is forbidden for full/export",
            Self::ThumbnailUpscale => "thumbnail requests cannot upscale",
            Self::ExactQualityRequired => "full/export requests require exact quality",
            Self::ContradictoryBackendFallback => {
                "exact backend requests cannot allow CPU fallback degradation"
            }
        })
    }
}

impl std::error::Error for ModeRequestError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeFinding {
    UnsupportedPurpose {
        operation_id: u128,
        purpose: PipelinePurpose,
    },
    UnsupportedApproximation {
        operation_id: u128,
        approximation: ApproximationId,
    },
    ExactUnavailable {
        operation_id: u128,
    },
    MaskUnsupported,
    AnalysisUnsupported,
    UnsupportedOperationApproximation {
        operation_id: u128,
    },
    EmbeddedPreviewRequiresExplicitProvenance,
    ExcludedOperation {
        operation_id: u128,
        inclusion: OperationInclusion,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModePlanningError {
    Request(ModeRequestError),
    Finding(ModeFinding),
    EmptyOperations,
}

impl fmt::Display for ModePlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pixelpipe mode planning error: {self:?}")
    }
}

impl std::error::Error for ModePlanningError {}

/// Immutable evidence emitted by the mode planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeReceipt {
    purpose: PipelinePurpose,
    quality: ModeQuality,
    included_operations: usize,
    approximations: usize,
    degraded: bool,
    cpu_fallback: bool,
}

impl ModeReceipt {
    #[must_use]
    pub const fn purpose(&self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn quality(&self) -> ModeQuality {
        self.quality
    }
    #[must_use]
    pub const fn included_operations(&self) -> usize {
        self.included_operations
    }
    #[must_use]
    pub const fn approximations(&self) -> usize {
        self.approximations
    }
    #[must_use]
    pub const fn degraded(&self) -> bool {
        self.degraded
    }
    #[must_use]
    pub const fn cpu_fallback(&self) -> bool {
        self.cpu_fallback
    }
}

/// One immutable request + snapshot + operation selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ModePlan {
    snapshot_identity: PipelineSnapshotIdentity,
    source_identity: crate::SourceIdentity,
    generation: crate::PipelineGeneration,
    request: ModeRequest,
    included_operations: Vec<u128>,
    approximations: Vec<(u128, ApproximationId)>,
    findings: Vec<ModeFinding>,
    identity: PipelineSnapshotIdentity,
    receipt: ModeReceipt,
}

impl ModePlan {
    #[must_use]
    pub const fn snapshot_identity(&self) -> PipelineSnapshotIdentity {
        self.snapshot_identity
    }
    #[must_use]
    pub const fn source_identity(&self) -> crate::SourceIdentity {
        self.source_identity
    }
    #[must_use]
    pub const fn generation(&self) -> crate::PipelineGeneration {
        self.generation
    }
    #[must_use]
    pub const fn request(&self) -> &ModeRequest {
        &self.request
    }
    #[must_use]
    pub fn included_operations(&self) -> &[u128] {
        &self.included_operations
    }
    #[must_use]
    pub fn approximations(&self) -> &[(u128, ApproximationId)] {
        &self.approximations
    }
    #[must_use]
    pub fn findings(&self) -> &[ModeFinding] {
        &self.findings
    }
    #[must_use]
    pub const fn identity(&self) -> PipelineSnapshotIdentity {
        self.identity
    }
    #[must_use]
    pub const fn receipt(&self) -> &ModeReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = self.request.canonical_bytes();
        bytes.extend_from_slice(&self.snapshot_identity.as_bytes());
        for operation in &self.included_operations {
            bytes.extend_from_slice(&operation.to_le_bytes());
        }
        for (operation, approximation) in &self.approximations {
            bytes.extend_from_slice(&operation.to_le_bytes());
            bytes.extend_from_slice(approximation.as_str().as_bytes());
            bytes.push(0);
        }
        bytes
    }

    #[must_use]
    pub fn diagnostic_sha256(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }
}

/// Plans the mode contract without reading UI, caller, thread, or source files.
#[derive(Debug, Default, Clone, Copy)]
pub struct ModePlanner;

impl ModePlanner {
    pub fn plan_request(
        &self,
        request: ModeRequest,
        snapshot: &PipelineSnapshot,
        operations: &[ModeOperationCapability],
    ) -> Result<ModePlan, ModePlanningError> {
        self.plan(snapshot, request, operations)
    }

    #[allow(clippy::too_many_lines)]
    pub fn plan(
        &self,
        snapshot: &PipelineSnapshot,
        request: ModeRequest,
        operations: &[ModeOperationCapability],
    ) -> Result<ModePlan, ModePlanningError> {
        request.validate().map_err(ModePlanningError::Request)?;
        if operations.is_empty() {
            return Err(ModePlanningError::EmptyOperations);
        }
        if request.purpose == PipelinePurpose::Thumbnail
            && (request.output.dimensions().width() > snapshot.source().dimensions().width()
                || request.output.dimensions().height() > snapshot.source().dimensions().height())
        {
            return Err(ModePlanningError::Request(
                ModeRequestError::ThumbnailUpscale,
            ));
        }
        let mut included_operations = Vec::new();
        let mut approximations = Vec::new();
        let mut supports_masks = false;
        let mut supports_analysis = false;
        for operation in operations {
            if !operation.purposes().contains(&request.purpose) {
                if operation.mandatory() {
                    return Err(ModePlanningError::Finding(
                        ModeFinding::UnsupportedPurpose {
                            operation_id: operation.operation_id(),
                            purpose: request.purpose,
                        },
                    ));
                }
                continue;
            }
            let excluded = matches!(
                operation.inclusion(),
                OperationInclusion::Diagnostic | OperationInclusion::Presentation
            ) && matches!(
                request.purpose,
                PipelinePurpose::Thumbnail | PipelinePurpose::Export
            );
            if excluded && !operation.mandatory() {
                continue;
            }
            if matches!(
                operation.inclusion(),
                OperationInclusion::Diagnostic | OperationInclusion::Presentation
            ) && operation.mandatory()
                && excluded
            {
                return Err(ModePlanningError::Finding(ModeFinding::ExcludedOperation {
                    operation_id: operation.operation_id(),
                    inclusion: operation.inclusion(),
                }));
            }
            let approximation_allowed = request.degradation.allows_approximation(request.purpose)
                && matches!(request.quality, ModeQuality::Interactive);
            if !operation.supports_exact() && !approximation_allowed {
                return Err(ModePlanningError::Finding(ModeFinding::ExactUnavailable {
                    operation_id: operation.operation_id(),
                }));
            }
            if !operation.supports_exact() && operation.approximations().is_empty() {
                return Err(ModePlanningError::Finding(
                    ModeFinding::UnsupportedOperationApproximation {
                        operation_id: operation.operation_id(),
                    },
                ));
            }
            included_operations.push(operation.operation_id());
            supports_masks |= operation.supports_masks();
            supports_analysis |= operation.supports_analysis();
            if approximation_allowed && let Some(approximation) = operation.approximations().first()
            {
                approximations.push((operation.operation_id(), approximation.clone()));
            }
        }
        included_operations.sort_unstable();
        approximations.sort_by_key(|entry| entry.0);
        if matches!(request.masks, MaskRequest::Required) && !supports_masks {
            return Err(ModePlanningError::Finding(ModeFinding::MaskUnsupported));
        }
        if matches!(request.analysis, AnalysisRequest::Required) && !supports_analysis {
            return Err(ModePlanningError::Finding(ModeFinding::AnalysisUnsupported));
        }
        if request.source_preview
            && request.purpose == PipelinePurpose::Thumbnail
            && !matches!(request.degradation, DegradationPolicy::ApprovedPreviewOnly)
        {
            return Err(ModePlanningError::Finding(
                ModeFinding::EmbeddedPreviewRequiresExplicitProvenance,
            ));
        }
        if request.source_preview && request.source_preview_provenance.is_none() {
            return Err(ModePlanningError::Finding(
                ModeFinding::EmbeddedPreviewRequiresExplicitProvenance,
            ));
        }
        let mut plan = ModePlan {
            snapshot_identity: snapshot.identity(),
            source_identity: snapshot.source().identity(),
            generation: snapshot.generation(),
            request,
            included_operations,
            approximations,
            findings: Vec::new(),
            identity: PipelineSnapshotIdentity::new([0; 32]),
            receipt: ModeReceipt {
                purpose: snapshot.purpose(),
                quality: ModeQuality::Balanced,
                included_operations: 0,
                approximations: 0,
                degraded: false,
                cpu_fallback: false,
            },
        };
        plan.identity = PipelineSnapshotIdentity::new(plan.diagnostic_sha256());
        plan.receipt = ModeReceipt {
            purpose: plan.request.purpose,
            quality: plan.request.quality,
            included_operations: plan.included_operations.len(),
            approximations: plan.approximations.len(),
            degraded: !plan.approximations.is_empty(),
            cpu_fallback: plan.request.backend == BackendPolicy::CpuFallbackAllowed
                || plan.request.degradation.allows_cpu_fallback(),
        };
        Ok(plan)
    }
}

pub type PipelineModeRequest = ModeRequest;
pub type PipelineModePlan = ModePlan;
pub type PipelineModePlanner = ModePlanner;
pub type QualityPreset = ModeQuality;

fn write_color(color: ColorIdentity, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(
        format!("{:?}:{}", color.encoding(), color.planner_version()).as_bytes(),
    );
}

impl From<&ModePlan> for PipelineSnapshotIdentity {
    fn from(plan: &ModePlan) -> Self {
        plan.identity()
    }
}
