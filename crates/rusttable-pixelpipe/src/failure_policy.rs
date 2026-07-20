#![allow(clippy::missing_errors_doc)]

use std::fmt;

use crate::{
    ApproximationId, BackendPolicy, CacheError, CancellationError, CancellationStage,
    DegradationPolicy, PipelinePurpose, PipelineSnapshotIdentity, PublicationError, RequestId,
};

#[path = "failure_receipts.rs"]
mod failure_receipts;

pub use failure_receipts::{
    AttemptReceipt, OutputCandidate, OutputExpectation, OutputValidationError,
    OutputValidationReceipt, OutputValidator,
};

pub const MAX_ATTEMPTS: u8 = 6;
pub const MAX_SECONDARY_FAILURES: usize = 4;
const MAX_DETAIL_LENGTH: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailurePrecedence {
    SourceSnapshotInvalid,
    CancellationDeadline,
    InvariantDataValidation,
    Resource,
    ExecutionBackend,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureCategory {
    InvalidRequestSnapshot,
    SourceDecode,
    Preparation,
    NodeExecution,
    NonFinite,
    ResourceAllocation,
    Cache,
    CpuPanic,
    GpuValidation,
    GpuDeviceLoss,
    GpuOutOfMemory,
    GpuTimeout,
    CancellationDeadline,
    Publication,
    Invariant,
}

impl FailureCategory {
    #[must_use]
    pub const fn precedence(self) -> FailurePrecedence {
        match self {
            Self::InvalidRequestSnapshot | Self::SourceDecode => {
                FailurePrecedence::SourceSnapshotInvalid
            }
            Self::CancellationDeadline => FailurePrecedence::CancellationDeadline,
            Self::NonFinite | Self::Invariant => FailurePrecedence::InvariantDataValidation,
            Self::ResourceAllocation | Self::GpuOutOfMemory | Self::Cache => {
                FailurePrecedence::Resource
            }
            Self::Preparation
            | Self::NodeExecution
            | Self::CpuPanic
            | Self::GpuValidation
            | Self::GpuDeviceLoss
            | Self::GpuTimeout => FailurePrecedence::ExecutionBackend,
            Self::Publication => FailurePrecedence::Publication,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::InvalidRequestSnapshot => "invalid-request-snapshot",
            Self::SourceDecode => "source-decode",
            Self::Preparation => "preparation",
            Self::NodeExecution => "node-execution",
            Self::NonFinite => "nonfinite",
            Self::ResourceAllocation => "resource-allocation",
            Self::Cache => "cache",
            Self::CpuPanic => "cpu-panic",
            Self::GpuValidation => "gpu-validation",
            Self::GpuDeviceLoss => "gpu-device-loss",
            Self::GpuOutOfMemory => "gpu-out-of-memory",
            Self::GpuTimeout => "gpu-timeout",
            Self::CancellationDeadline => "cancellation-deadline",
            Self::Publication => "publication",
            Self::Invariant => "invariant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureStage {
    Request,
    SourceDecode,
    Preparation,
    Node,
    Tile,
    OutputValidation,
    Cache,
    Cleanup,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureScope {
    Request,
    Pipeline,
    Node,
    Tile,
    Cache,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureBackend {
    Cpu,
    Gpu,
}

impl FailureBackend {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureRetryability {
    Never,
    Transient,
    Resource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupAction {
    ReturnLeases,
    PoisonLeases,
    RetireGpu,
    RemovePartialOutput,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheAction {
    PreserveUpstream,
    InvalidateFailedProducer,
    SuppressFailedBuild,
    NoChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicationAction {
    Reject,
    RepublishImmutable,
    NoPublication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuarantineHint {
    None,
    Backend,
    Device,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureError {
    EmptyDetail,
    DetailTooLong,
    InvalidLocation,
}

impl fmt::Display for FailureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyDetail => "failure detail is empty",
            Self::DetailTooLong => "failure detail exceeds the bounded receipt limit",
            Self::InvalidLocation => "failure node/tile location is incompatible with its scope",
        })
    }
}

impl std::error::Error for FailureError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    category: FailureCategory,
    stage: FailureStage,
    scope: FailureScope,
    node: Option<u32>,
    tile: Option<u32>,
    backend: FailureBackend,
    retryability: FailureRetryability,
    cleanup: CleanupAction,
    cache: CacheAction,
    publication: PublicationAction,
    quarantine: QuarantineHint,
    detail: String,
}

impl Failure {
    pub fn new(
        category: FailureCategory,
        stage: FailureStage,
        scope: FailureScope,
        backend: FailureBackend,
        detail: impl Into<String>,
    ) -> Result<Self, FailureError> {
        let detail = detail.into();
        if detail.is_empty() {
            return Err(FailureError::EmptyDetail);
        }
        if detail.len() > MAX_DETAIL_LENGTH {
            return Err(FailureError::DetailTooLong);
        }
        let failure = Self {
            category,
            stage,
            scope,
            node: None,
            tile: None,
            backend,
            retryability: default_retryability(category),
            cleanup: default_cleanup(category),
            cache: default_cache_action(category),
            publication: default_publication_action(category),
            quarantine: default_quarantine(category),
            detail,
        };
        if matches!(scope, FailureScope::Node) || matches!(stage, FailureStage::Node) {
            return Err(FailureError::InvalidLocation);
        }
        Ok(failure)
    }

    #[must_use]
    pub fn with_node(mut self, node: u32) -> Self {
        self.node = Some(node);
        self.scope = FailureScope::Node;
        self.stage = FailureStage::Node;
        self
    }

    #[must_use]
    pub fn with_tile(mut self, tile: u32) -> Self {
        self.tile = Some(tile);
        self.scope = FailureScope::Tile;
        self.stage = FailureStage::Tile;
        self
    }

    #[must_use]
    pub const fn with_retryability(mut self, value: FailureRetryability) -> Self {
        self.retryability = value;
        self
    }

    #[must_use]
    pub const fn with_cleanup(mut self, value: CleanupAction) -> Self {
        self.cleanup = value;
        self
    }

    #[must_use]
    pub const fn with_cache_action(mut self, value: CacheAction) -> Self {
        self.cache = value;
        self
    }

    #[must_use]
    pub const fn with_publication_action(mut self, value: PublicationAction) -> Self {
        self.publication = value;
        self
    }

    #[must_use]
    pub const fn with_quarantine(mut self, value: QuarantineHint) -> Self {
        self.quarantine = value;
        self
    }

    #[must_use]
    pub const fn category(&self) -> FailureCategory {
        self.category
    }
    #[must_use]
    pub const fn stage(&self) -> FailureStage {
        self.stage
    }
    #[must_use]
    pub const fn scope(&self) -> FailureScope {
        self.scope
    }
    #[must_use]
    pub const fn node(&self) -> Option<u32> {
        self.node
    }
    #[must_use]
    pub const fn tile(&self) -> Option<u32> {
        self.tile
    }
    #[must_use]
    pub const fn backend(&self) -> FailureBackend {
        self.backend
    }
    #[must_use]
    pub const fn retryability(&self) -> FailureRetryability {
        self.retryability
    }
    #[must_use]
    pub const fn cleanup(&self) -> CleanupAction {
        self.cleanup
    }
    #[must_use]
    pub const fn cache_action(&self) -> CacheAction {
        self.cache
    }
    #[must_use]
    pub const fn publication_action(&self) -> PublicationAction {
        self.publication
    }
    #[must_use]
    pub const fn quarantine(&self) -> QuarantineHint {
        self.quarantine
    }
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }

    #[must_use]
    pub fn from_cancellation(error: CancellationError, backend: FailureBackend) -> Self {
        let stage = match error.stage().unwrap_or(CancellationStage::Node) {
            CancellationStage::SourceDecode => FailureStage::SourceDecode,
            CancellationStage::Preparation => FailureStage::Preparation,
            CancellationStage::Node => FailureStage::Node,
            CancellationStage::Tile => FailureStage::Tile,
            CancellationStage::CacheBuild | CancellationStage::CachePromotion => {
                FailureStage::Cache
            }
            CancellationStage::Publication => FailureStage::Publication,
            _ => FailureStage::Cleanup,
        };
        Self::bounded(
            FailureCategory::CancellationDeadline,
            stage,
            FailureScope::Pipeline,
            backend,
            error.to_string(),
        )
    }

    #[must_use]
    pub fn from_cache(error: &CacheError) -> Self {
        let category = match error {
            CacheError::Cancelled | CacheError::Cancellation(_) => {
                FailureCategory::CancellationDeadline
            }
            CacheError::InvalidValue(_) => FailureCategory::NonFinite,
            CacheError::BuilderPanicked => FailureCategory::CpuPanic,
            CacheError::CostOverflow | CacheError::OverBudgetPinned => {
                FailureCategory::ResourceAllocation
            }
            CacheError::StalePublication | CacheError::BuildNotPublished => {
                FailureCategory::Invariant
            }
            _ => FailureCategory::Cache,
        };
        Self::bounded(
            category,
            FailureStage::Cache,
            FailureScope::Cache,
            FailureBackend::Cpu,
            error.to_string(),
        )
    }

    #[must_use]
    pub fn from_publication(error: &PublicationError) -> Self {
        let category = match error {
            PublicationError::Cancelled(error) => {
                return Self::from_cancellation(*error, FailureBackend::Cpu);
            }
            PublicationError::IdentityMismatch { .. } => FailureCategory::Invariant,
            _ => FailureCategory::Publication,
        };
        Self::bounded(
            category,
            FailureStage::Publication,
            FailureScope::Publication,
            FailureBackend::Cpu,
            error.to_string(),
        )
    }

    fn bounded(
        category: FailureCategory,
        stage: FailureStage,
        scope: FailureScope,
        backend: FailureBackend,
        detail: String,
    ) -> Self {
        let mut detail = detail;
        detail.truncate(MAX_DETAIL_LENGTH);
        Self::new(category, stage, scope, backend, detail).expect("bounded failure construction")
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {:?}: {}",
            self.category.tag(),
            self.stage,
            self.detail
        )
    }
}

impl std::error::Error for Failure {}

fn default_retryability(category: FailureCategory) -> FailureRetryability {
    match category {
        FailureCategory::ResourceAllocation
        | FailureCategory::GpuOutOfMemory
        | FailureCategory::Cache => FailureRetryability::Resource,
        FailureCategory::GpuValidation
        | FailureCategory::GpuDeviceLoss
        | FailureCategory::GpuTimeout => FailureRetryability::Transient,
        _ => FailureRetryability::Never,
    }
}

const fn default_cleanup(category: FailureCategory) -> CleanupAction {
    match category {
        FailureCategory::GpuDeviceLoss | FailureCategory::GpuValidation => CleanupAction::RetireGpu,
        FailureCategory::NonFinite | FailureCategory::Invariant => CleanupAction::PoisonLeases,
        FailureCategory::Publication => CleanupAction::RemovePartialOutput,
        _ => CleanupAction::ReturnLeases,
    }
}

const fn default_cache_action(category: FailureCategory) -> CacheAction {
    match category {
        FailureCategory::Cache => CacheAction::InvalidateFailedProducer,
        FailureCategory::NonFinite | FailureCategory::Invariant => {
            CacheAction::InvalidateFailedProducer
        }
        _ => CacheAction::PreserveUpstream,
    }
}

const fn default_publication_action(category: FailureCategory) -> PublicationAction {
    match category {
        FailureCategory::Publication => PublicationAction::Reject,
        _ => PublicationAction::NoPublication,
    }
}

const fn default_quarantine(category: FailureCategory) -> QuarantineHint {
    match category {
        FailureCategory::GpuDeviceLoss
        | FailureCategory::GpuValidation
        | FailureCategory::GpuTimeout => QuarantineHint::Device,
        FailureCategory::GpuOutOfMemory => QuarantineHint::Backend,
        _ => QuarantineHint::None,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureLedger {
    primary: Option<Failure>,
    secondary: Vec<Failure>,
}

impl FailureLedger {
    /// Records a failure, promoting higher-precedence failures to the primary slot.
    pub fn record(&mut self, failure: Failure) {
        match &self.primary {
            None => self.primary = Some(failure),
            Some(primary) if failure.category.precedence() < primary.category.precedence() => {
                let old = self.primary.take();
                self.primary = Some(failure);
                if let Some(old) = old {
                    self.push_secondary(old);
                }
            }
            Some(primary)
                if failure.category.precedence() == primary.category.precedence()
                    && *primary != failure =>
            {
                self.push_secondary(failure);
            }
            Some(_) => self.push_secondary(failure),
        }
    }

    fn push_secondary(&mut self, failure: Failure) {
        if self.secondary.len() < MAX_SECONDARY_FAILURES && !self.secondary.contains(&failure) {
            self.secondary.push(failure);
        }
    }

    #[must_use]
    pub fn primary(&self) -> Option<&Failure> {
        self.primary.as_ref()
    }
    #[must_use]
    pub fn secondary(&self) -> &[Failure] {
        &self.secondary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRequest {
    purpose: PipelinePurpose,
    backend: FailureBackend,
    cpu_fallback_allowed: bool,
    approximation: Option<ApproximationId>,
    smaller_tile_available: bool,
    clean_rebuild_available: bool,
}

impl PolicyRequest {
    #[must_use]
    pub const fn new(
        purpose: PipelinePurpose,
        backend: FailureBackend,
        policy: BackendPolicy,
        _degradation: DegradationPolicy,
    ) -> Self {
        Self {
            purpose,
            backend,
            cpu_fallback_allowed: matches!(policy, BackendPolicy::CpuFallbackAllowed),
            approximation: None,
            smaller_tile_available: false,
            clean_rebuild_available: false,
        }
    }

    #[must_use]
    pub const fn purpose(&self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn backend(&self) -> FailureBackend {
        self.backend
    }
    #[must_use]
    pub const fn with_smaller_tile(mut self, available: bool) -> Self {
        self.smaller_tile_available = available;
        self
    }
    #[must_use]
    pub const fn with_clean_rebuild(mut self, available: bool) -> Self {
        self.clean_rebuild_available = available;
        self
    }
    #[must_use]
    pub const fn with_cpu_fallback(mut self, allowed: bool) -> Self {
        self.cpu_fallback_allowed = allowed;
        self
    }
    #[must_use]
    pub fn with_approximation(mut self, approximation: ApproximationId) -> Self {
        if matches!(
            self.purpose,
            PipelinePurpose::Preview | PipelinePurpose::Thumbnail
        ) {
            self.approximation = Some(approximation);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyAction {
    Stop,
    RetrySameImplementation {
        attempt: u8,
    },
    RetrySmallerTile {
        attempt: u8,
        reduction: u8,
    },
    RebuildCache {
        attempt: u8,
    },
    FallbackToCpu {
        attempt: u8,
    },
    ApproximatePreview {
        attempt: u8,
        approximation: ApproximationId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    AttemptLimit,
    RetryCycle,
    IllegalDegradation,
    NoFallback,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AttemptLimit => "failure policy attempt limit reached",
            Self::RetryCycle => "failure policy detected a repeated implementation attempt",
            Self::IllegalDegradation => "failure policy rejected degradation for exact work",
            Self::NoFallback => "failure policy has no legal fallback",
        })
    }
}

impl std::error::Error for PolicyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailurePolicy;

impl FailurePolicy {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn decide(
        &self,
        request: &PolicyRequest,
        failure: &Failure,
        attempts: &[AttemptReceipt],
    ) -> Result<PolicyAction, PolicyError> {
        let next = u8::try_from(attempts.len())
            .map_err(|_| PolicyError::AttemptLimit)?
            .saturating_add(1);
        if next > MAX_ATTEMPTS {
            return Err(PolicyError::AttemptLimit);
        }
        if failure.category.precedence() <= FailurePrecedence::InvariantDataValidation
            || matches!(
                failure.category,
                FailureCategory::CpuPanic | FailureCategory::CancellationDeadline
            )
        {
            return Ok(PolicyAction::Stop);
        }
        let same_backend = attempts
            .iter()
            .filter(|attempt| attempt.backend == request.backend)
            .count();
        if matches!(failure.category, FailureCategory::Cache)
            && failure.retryability == FailureRetryability::Resource
            && request.clean_rebuild_available
            && !attempts
                .iter()
                .any(|attempt| matches!(attempt.action, PolicyAction::RebuildCache { .. }))
        {
            return Ok(PolicyAction::RebuildCache { attempt: next });
        }
        if matches!(
            failure.category,
            FailureCategory::ResourceAllocation | FailureCategory::GpuOutOfMemory
        ) && failure.scope == FailureScope::Tile
            && request.smaller_tile_available
            && same_backend < 2
        {
            return Ok(PolicyAction::RetrySmallerTile {
                attempt: next,
                reduction: u8::try_from(same_backend + 1).unwrap_or(2),
            });
        }
        if request.backend == FailureBackend::Gpu
            && matches!(
                failure.category,
                FailureCategory::GpuOutOfMemory
                    | FailureCategory::ResourceAllocation
                    | FailureCategory::GpuValidation
                    | FailureCategory::GpuDeviceLoss
                    | FailureCategory::GpuTimeout
            )
        {
            if request.cpu_fallback_allowed
                && !attempts
                    .iter()
                    .any(|attempt| matches!(attempt.action, PolicyAction::FallbackToCpu { .. }))
            {
                return Ok(PolicyAction::FallbackToCpu { attempt: next });
            }
            if let Some(approximation) = &request.approximation
                && !attempts.iter().any(|attempt| {
                    matches!(attempt.action, PolicyAction::ApproximatePreview { .. })
                })
            {
                if matches!(
                    request.purpose,
                    PipelinePurpose::Full | PipelinePurpose::Export
                ) {
                    return Err(PolicyError::IllegalDegradation);
                }
                return Ok(PolicyAction::ApproximatePreview {
                    attempt: next,
                    approximation: approximation.clone(),
                });
            }
        }
        if failure.retryability == FailureRetryability::Transient && same_backend == 0 {
            return Ok(PolicyAction::RetrySameImplementation { attempt: next });
        }
        if let Some(approximation) = &request.approximation
            && !attempts
                .iter()
                .any(|attempt| matches!(attempt.action, PolicyAction::ApproximatePreview { .. }))
            && matches!(
                request.purpose,
                PipelinePurpose::Preview | PipelinePurpose::Thumbnail
            )
        {
            return Ok(PolicyAction::ApproximatePreview {
                attempt: next,
                approximation: approximation.clone(),
            });
        }
        Ok(PolicyAction::Stop)
    }
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalImplementation {
    CpuExact,
    GpuExact,
    CpuApproximation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalReceipt {
    request: RequestId,
    snapshot: PipelineSnapshotIdentity,
    implementation: FinalImplementation,
    attempts: Vec<AttemptReceipt>,
    primary: Option<Failure>,
    secondary: Vec<Failure>,
    output: Option<OutputValidationReceipt>,
    published: bool,
    cached: bool,
}

impl FinalReceipt {
    #[must_use]
    pub const fn request(&self) -> RequestId {
        self.request
    }
    #[must_use]
    pub const fn snapshot(&self) -> PipelineSnapshotIdentity {
        self.snapshot
    }
    #[must_use]
    pub const fn implementation(&self) -> FinalImplementation {
        self.implementation
    }
    #[must_use]
    pub fn attempts(&self) -> &[AttemptReceipt] {
        &self.attempts
    }
    #[must_use]
    pub fn primary_failure(&self) -> Option<&Failure> {
        self.primary.as_ref()
    }
    #[must_use]
    pub fn secondary_failures(&self) -> &[Failure] {
        &self.secondary
    }
    #[must_use]
    pub const fn output(&self) -> Option<OutputValidationReceipt> {
        self.output
    }
    #[must_use]
    pub const fn published(&self) -> bool {
        self.published
    }
    #[must_use]
    pub const fn cached(&self) -> bool {
        self.cached
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptBuilder {
    request: RequestId,
    snapshot: PipelineSnapshotIdentity,
    attempts: Vec<AttemptReceipt>,
    ledger: FailureLedger,
}

impl ReceiptBuilder {
    #[must_use]
    pub fn new(request: RequestId, snapshot: PipelineSnapshotIdentity) -> Self {
        Self {
            request,
            snapshot,
            attempts: Vec::new(),
            ledger: FailureLedger::default(),
        }
    }

    pub fn push_failure(&mut self, receipt: AttemptReceipt) -> Result<(), PolicyError> {
        if self.attempts.len() >= usize::from(MAX_ATTEMPTS) {
            return Err(PolicyError::AttemptLimit);
        }
        self.ledger.record(receipt.failure.clone());
        self.attempts.push(receipt);
        Ok(())
    }

    #[must_use]
    pub fn finish_failure(self) -> FinalReceipt {
        FinalReceipt {
            request: self.request,
            snapshot: self.snapshot,
            implementation: FinalImplementation::CpuExact,
            attempts: self.attempts,
            primary: self.ledger.primary,
            secondary: self.ledger.secondary,
            output: None,
            published: false,
            cached: false,
        }
    }

    #[must_use]
    pub fn finish_success(
        self,
        implementation: FinalImplementation,
        output: OutputValidationReceipt,
        published: bool,
        cached: bool,
    ) -> FinalReceipt {
        FinalReceipt {
            request: self.request,
            snapshot: self.snapshot,
            implementation,
            attempts: self.attempts,
            primary: self.ledger.primary,
            secondary: self.ledger.secondary,
            output: Some(output),
            published,
            cached,
        }
    }
}

pub trait FailureCleanupHook {
    fn cleanup(&mut self, action: CleanupAction) -> Result<(), HookError>;
}

pub trait FailureCacheHook {
    fn apply_cache_action(&mut self, action: CacheAction) -> Result<(), HookError>;
}

pub trait FailurePublicationHook {
    fn apply_publication_action(&mut self, action: PublicationAction) -> Result<(), HookError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookError(String);

impl HookError {
    pub fn new(detail: impl Into<String>) -> Result<Self, FailureError> {
        let detail = detail.into();
        if detail.is_empty() {
            return Err(FailureError::EmptyDetail);
        }
        if detail.len() > MAX_DETAIL_LENGTH {
            return Err(FailureError::DetailTooLong);
        }
        Ok(Self(detail))
    }
}

impl fmt::Display for HookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HookError {}

pub fn integrate_failure<C, K, P>(
    failure: &Failure,
    cleanup: &mut C,
    cache: &mut K,
    publication: &mut P,
) -> Result<(), HookError>
where
    C: FailureCleanupHook,
    K: FailureCacheHook,
    P: FailurePublicationHook,
{
    cleanup.cleanup(failure.cleanup())?;
    cache.apply_cache_action(failure.cache_action())?;
    publication.apply_publication_action(failure.publication_action())
}
