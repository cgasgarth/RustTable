//! Display-safe state and service contracts for linear RAW AI denoise.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_errors_doc)]

use std::fmt;

use crate::ai_models::{AiProvider, AiTask, ModelHash};
use crate::neural_restore::{PhotoSelection, PhotoSourceKind};

pub const RAW_DENOISE_FOCUS_ORDER: [&str; 18] = [
    "raw-denoise-model",
    "raw-denoise-provider",
    "raw-denoise-layout",
    "raw-denoise-calibration",
    "raw-denoise-profile",
    "raw-denoise-strength",
    "raw-denoise-tile",
    "raw-denoise-plan-policy",
    "raw-denoise-output-policy",
    "raw-denoise-preview",
    "raw-denoise-full",
    "raw-denoise-export",
    "raw-denoise-cancel",
    "raw-denoise-progress",
    "raw-denoise-plan",
    "raw-denoise-memory",
    "raw-denoise-source",
    "raw-denoise-status",
];

pub const RAW_DENOISE_TILES: [u32; 3] = [128, 256, 512];
pub const RAW_DENOISE_MAX_STRENGTH: u8 = 100;
const MEMORY_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoiseSourceLayout {
    XTrans,
    AlreadyLinear,
    Unsupported,
    Unavailable,
}

impl RawDenoiseSourceLayout {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::XTrans => "X-Trans RAW",
            Self::AlreadyLinear => "Already-linear RAW",
            Self::Unsupported => "Unsupported source layout",
            Self::Unavailable => "Source layout unavailable",
        }
    }

    #[must_use]
    pub const fn supported(self) -> bool {
        matches!(self, Self::XTrans | Self::AlreadyLinear)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoiseCalibrationStatus {
    Present,
    Missing,
    Unavailable,
}

impl RawDenoiseCalibrationStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Present => "Calibration present",
            Self::Missing => "Calibration missing",
            Self::Unavailable => "Calibration unavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoiseProfileStatus {
    LinearRec2020,
    Missing,
    Unavailable,
}

impl RawDenoiseProfileStatus {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LinearRec2020 => "Linear Rec.2020",
            Self::Missing => "Linear Rec.2020 profile missing",
            Self::Unavailable => "Profile status unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoiseSourceInfo {
    source_identity: String,
    edit_identity: String,
    output_identity: String,
    dimensions: Option<(u32, u32)>,
    layout: RawDenoiseSourceLayout,
    calibration: RawDenoiseCalibrationStatus,
    profile: RawDenoiseProfileStatus,
}

impl RawDenoiseSourceInfo {
    #[must_use]
    pub fn from_selection(selection: &PhotoSelection) -> Self {
        let layout = match selection.source_kind() {
            Some(PhotoSourceKind::XTransRaw) => RawDenoiseSourceLayout::XTrans,
            Some(PhotoSourceKind::LinearRaw) => RawDenoiseSourceLayout::AlreadyLinear,
            Some(PhotoSourceKind::BayerRaw | PhotoSourceKind::Raster) => {
                RawDenoiseSourceLayout::Unsupported
            }
            None => RawDenoiseSourceLayout::Unavailable,
        };
        Self {
            source_identity: selection.photo().map_or_else(
                || "source:none".to_owned(),
                |photo| format!("photo:{photo}"),
            ),
            edit_identity: format!("edit-revision:{}", selection.revision()),
            output_identity: "dng-output:unresolved".to_owned(),
            dimensions: None,
            layout,
            calibration: RawDenoiseCalibrationStatus::Unavailable,
            profile: RawDenoiseProfileStatus::Unavailable,
        }
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn available(
        source_identity: impl Into<String>,
        edit_identity: impl Into<String>,
        output_identity: impl Into<String>,
        dimensions: (u32, u32),
        layout: RawDenoiseSourceLayout,
        calibration: RawDenoiseCalibrationStatus,
        profile: RawDenoiseProfileStatus,
    ) -> Self {
        Self {
            source_identity: source_identity.into(),
            edit_identity: edit_identity.into(),
            output_identity: output_identity.into(),
            dimensions: Some(dimensions),
            layout,
            calibration,
            profile,
        }
    }

    #[must_use]
    pub fn source_identity(&self) -> &str {
        &self.source_identity
    }
    #[must_use]
    pub fn edit_identity(&self) -> &str {
        &self.edit_identity
    }
    #[must_use]
    pub fn output_identity(&self) -> &str {
        &self.output_identity
    }
    #[must_use]
    pub const fn dimensions(&self) -> Option<(u32, u32)> {
        self.dimensions
    }
    #[must_use]
    pub const fn layout(&self) -> RawDenoiseSourceLayout {
        self.layout
    }
    #[must_use]
    pub const fn calibration(&self) -> RawDenoiseCalibrationStatus {
        self.calibration
    }
    #[must_use]
    pub const fn profile(&self) -> RawDenoiseProfileStatus {
        self.profile
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoiseModelOption {
    hash: ModelHash,
    label: String,
    task: AiTask,
    qualified: bool,
    supported_layouts: Vec<RawDenoiseSourceLayout>,
    tile_size: u32,
    providers: Vec<AiProvider>,
}

impl RawDenoiseModelOption {
    #[must_use]
    pub fn new(
        hash: ModelHash,
        label: impl Into<String>,
        qualified: bool,
        supported_layouts: Vec<RawDenoiseSourceLayout>,
        tile_size: u32,
        providers: Vec<AiProvider>,
    ) -> Self {
        Self {
            hash,
            label: label.into(),
            task: AiTask::RawLinearDenoise,
            qualified,
            supported_layouts,
            tile_size,
            providers,
        }
    }
    #[must_use]
    pub const fn hash(&self) -> &ModelHash {
        &self.hash
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    #[must_use]
    pub const fn task(&self) -> AiTask {
        self.task
    }
    #[must_use]
    pub const fn qualified(&self) -> bool {
        self.qualified
    }
    #[must_use]
    pub fn supports_layout(&self, layout: RawDenoiseSourceLayout) -> bool {
        self.supported_layouts.contains(&layout)
    }
    #[must_use]
    pub fn supported_layouts(&self) -> &[RawDenoiseSourceLayout] {
        &self.supported_layouts
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub fn providers(&self) -> &[AiProvider] {
        &self.providers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoiseSnapshot {
    generation: u64,
    selection: PhotoSelection,
    source: RawDenoiseSourceInfo,
    models: Vec<RawDenoiseModelOption>,
    providers: Vec<AiProvider>,
}

impl RawDenoiseSnapshot {
    #[must_use]
    pub fn unavailable(selection: PhotoSelection) -> Self {
        Self {
            source: RawDenoiseSourceInfo::from_selection(&selection),
            generation: 0,
            selection,
            models: Vec::new(),
            providers: Vec::new(),
        }
    }

    #[must_use]
    pub fn available(
        selection: PhotoSelection,
        source: RawDenoiseSourceInfo,
        models: Vec<RawDenoiseModelOption>,
        providers: Vec<AiProvider>,
    ) -> Self {
        Self {
            generation: 1,
            selection,
            source,
            models,
            providers,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
    #[must_use]
    pub fn selection(&self) -> &PhotoSelection {
        &self.selection
    }
    #[must_use]
    pub const fn source(&self) -> &RawDenoiseSourceInfo {
        &self.source
    }
    #[must_use]
    pub fn models(&self) -> &[RawDenoiseModelOption] {
        &self.models
    }
    pub fn qualified_models(&self) -> impl Iterator<Item = &RawDenoiseModelOption> {
        self.models.iter().filter(|model| model.qualified())
    }
    #[must_use]
    pub fn providers(&self) -> &[AiProvider] {
        &self.providers
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoisePlanPolicy {
    MinimalRaw,
}

impl RawDenoisePlanPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MinimalRaw => "Minimal RAW plan",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoiseOutputPolicy {
    PreviewBuffer,
    PublishDng,
    PublishAndImport,
}

impl RawDenoiseOutputPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PreviewBuffer => "Preview buffer only",
            Self::PublishDng => "Publish DNG",
            Self::PublishAndImport => "Publish and import DNG",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawDenoiseJobKind {
    Preview,
    Full,
    Export,
}

impl RawDenoiseJobKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Full => "Full render",
            Self::Export => "Export",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoisePlan {
    generation: u64,
    identity: String,
    source_identity: String,
    edit_identity: String,
    output_identity: String,
    model: ModelHash,
    provider: AiProvider,
    layout: RawDenoiseSourceLayout,
    strength: u8,
    tile_size: u32,
    plan_policy: RawDenoisePlanPolicy,
    output_policy: RawDenoiseOutputPolicy,
    memory_bytes: u64,
}

impl RawDenoisePlan {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        generation: u64,
        source: &RawDenoiseSourceInfo,
        model: &RawDenoiseModelOption,
        provider: AiProvider,
        strength: u8,
        tile_size: u32,
        plan_policy: RawDenoisePlanPolicy,
        output_policy: RawDenoiseOutputPolicy,
    ) -> Result<Self, RawDenoisePlanError> {
        if !source.layout().supported() {
            return Err(RawDenoisePlanError::UnsupportedLayout(source.layout()));
        }
        if source.calibration() != RawDenoiseCalibrationStatus::Present {
            return Err(RawDenoisePlanError::MissingCalibration);
        }
        if source.profile() != RawDenoiseProfileStatus::LinearRec2020 {
            return Err(RawDenoisePlanError::MissingProfile);
        }
        if !model.qualified() {
            return Err(RawDenoisePlanError::ModelUnavailable);
        }
        if !model.supports_layout(source.layout()) {
            return Err(RawDenoisePlanError::UnsupportedLayout(source.layout()));
        }
        if !model.providers().contains(&provider) {
            return Err(RawDenoisePlanError::ProviderUnavailable);
        }
        if !RAW_DENOISE_TILES.contains(&tile_size) {
            return Err(RawDenoisePlanError::TileOutOfBounds { value: tile_size });
        }
        if strength > RAW_DENOISE_MAX_STRENGTH {
            return Err(RawDenoisePlanError::StrengthOutOfBounds { value: strength });
        }
        let (width, height) = source.dimensions().ok_or(RawDenoisePlanError::EmptyImage)?;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(RawDenoisePlanError::DimensionsOverflow)?;
        if pixels == 0 {
            return Err(RawDenoisePlanError::EmptyImage);
        }
        let memory_bytes = pixels
            .checked_mul(24)
            .and_then(|bytes| bytes.checked_add(u64::from(tile_size) * u64::from(tile_size) * 16))
            .ok_or(RawDenoisePlanError::MemoryOverflow)?;
        if memory_bytes > MEMORY_LIMIT {
            return Err(RawDenoisePlanError::MemoryLimit {
                bytes: memory_bytes,
                limit: MEMORY_LIMIT,
            });
        }
        if source.source_identity().is_empty()
            || source.edit_identity().is_empty()
            || source.output_identity().is_empty()
        {
            return Err(RawDenoisePlanError::MissingIdentity);
        }
        let identity = format!(
            "raw-linear-denoise-v1:{generation}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            source.source_identity(),
            source.edit_identity(),
            model.hash().as_str(),
            provider.label(),
            source.layout().label(),
            strength,
            tile_size,
            plan_policy.label(),
            output_policy.label(),
            source.output_identity(),
        );
        Ok(Self {
            generation,
            identity,
            source_identity: source.source_identity().to_owned(),
            edit_identity: source.edit_identity().to_owned(),
            output_identity: source.output_identity().to_owned(),
            model: model.hash().clone(),
            provider,
            layout: source.layout(),
            strength,
            tile_size,
            plan_policy,
            output_policy,
            memory_bytes,
        })
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
    #[must_use]
    pub fn source_identity(&self) -> &str {
        &self.source_identity
    }
    #[must_use]
    pub fn edit_identity(&self) -> &str {
        &self.edit_identity
    }
    #[must_use]
    pub fn output_identity(&self) -> &str {
        &self.output_identity
    }
    #[must_use]
    pub const fn model(&self) -> &ModelHash {
        &self.model
    }
    #[must_use]
    pub const fn provider(&self) -> AiProvider {
        self.provider
    }
    #[must_use]
    pub const fn layout(&self) -> RawDenoiseSourceLayout {
        self.layout
    }
    #[must_use]
    pub const fn strength(&self) -> u8 {
        self.strength
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub const fn plan_policy(&self) -> RawDenoisePlanPolicy {
        self.plan_policy
    }
    #[must_use]
    pub const fn output_policy(&self) -> RawDenoiseOutputPolicy {
        self.output_policy
    }
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDenoisePlanError {
    UnsupportedLayout(RawDenoiseSourceLayout),
    MissingCalibration,
    MissingProfile,
    ModelUnavailable,
    ProviderUnavailable,
    EmptyImage,
    DimensionsOverflow,
    TileOutOfBounds { value: u32 },
    StrengthOutOfBounds { value: u8 },
    MemoryOverflow,
    MemoryLimit { bytes: u64, limit: u64 },
    MissingIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDenoiseProviderState {
    Unavailable,
    Available,
    Selected(AiProvider),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDenoiseMemoryState {
    Unknown,
    Estimated { bytes: u64 },
    Exceeded { bytes: u64, limit: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDenoiseCancellationState {
    Idle,
    Requested,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDenoiseProgress {
    pub completed: u64,
    pub total: u64,
}

impl RawDenoiseProgress {
    #[must_use]
    pub const fn fraction(self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.completed as f64 / self.total as f64
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseFailure {
    BackendUnavailable,
    UnsupportedLayout(RawDenoiseSourceLayout),
    MissingCalibration,
    MissingProfile,
    ModelUnavailable,
    ProviderUnavailable,
    MemoryBudgetExceeded { bytes: u64, limit: u64 },
    Cancelled,
    Failed(String),
    StaleGeneration,
}

impl RawDenoiseFailure {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::BackendUnavailable => "RAW linear denoise app service is unavailable; no inference or DNG/catalog write was performed.".to_owned(),
            Self::UnsupportedLayout(layout) => format!("{} is not supported by RawLinearDenoise.", layout.label()),
            Self::MissingCalibration => "RAW calibration is missing; black/white and camera calibration are required.".to_owned(),
            Self::MissingProfile => "Linear Rec.2020 source/profile evidence is required before planning.".to_owned(),
            Self::ModelUnavailable => "No qualified RawLinearDenoise model is available for this source.".to_owned(),
            Self::ProviderUnavailable => "No qualified provider is available for the selected RawLinearDenoise model.".to_owned(),
            Self::MemoryBudgetExceeded { bytes, limit } => format!("RAW plan memory estimate {bytes} B exceeds limit {limit} B."),
            Self::Cancelled => "RAW denoise job cancelled; no staged output was published.".to_owned(),
            Self::Failed(error) => format!("RAW denoise failed: {error}"),
            Self::StaleGeneration => "RAW denoise result discarded because a newer source/edit/model generation exists.".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseStatus {
    Idle,
    Planning,
    Ready,
    Running {
        kind: RawDenoiseJobKind,
        progress: RawDenoiseProgress,
    },
    Cancelling,
    Cancelled,
    Failed(RawDenoiseFailure),
    PendingPublication {
        kind: RawDenoiseJobKind,
        artifact: String,
    },
    Completed {
        kind: RawDenoiseJobKind,
        artifact: Option<String>,
    },
    Imported {
        kind: RawDenoiseJobKind,
        artifact: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoiseJobRequest {
    generation: u64,
    kind: RawDenoiseJobKind,
    plan: RawDenoisePlan,
}

impl RawDenoiseJobRequest {
    #[must_use]
    pub const fn new(generation: u64, kind: RawDenoiseJobKind, plan: RawDenoisePlan) -> Self {
        Self {
            generation,
            kind,
            plan,
        }
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn kind(&self) -> RawDenoiseJobKind {
        self.kind
    }
    #[must_use]
    pub const fn plan(&self) -> &RawDenoisePlan {
        &self.plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseServiceEvent {
    Progress {
        generation: u64,
        job: u64,
        progress: RawDenoiseProgress,
    },
    PendingPublication {
        generation: u64,
        job: u64,
        artifact: String,
    },
    Completed {
        generation: u64,
        job: u64,
        artifact: Option<String>,
    },
    Imported {
        generation: u64,
        job: u64,
        artifact: String,
    },
    Failed {
        generation: u64,
        job: u64,
        error: RawDenoiseFailure,
    },
    Cancelled {
        generation: u64,
        job: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseServiceError {
    BackendUnavailable,
    UnsupportedLayout,
    MissingCalibration,
    MissingProfile,
    ModelUnavailable,
    ProviderUnavailable,
    MemoryBudgetExceeded { bytes: u64, limit: u64 },
    Cancelled,
    Failed(String),
}

impl fmt::Display for RawDenoiseServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RAW denoise service error: {self:?}")
    }
}
impl std::error::Error for RawDenoiseServiceError {}

pub trait RawDenoiseServicePort {
    /// Provides the source contract and qualified `RawLinearDenoise` registry view.
    /// The app adapter owns decoder/calibration/profile lookup, inference, DNG publication,
    /// and catalog import; GTK only receives this immutable projection.
    fn snapshot(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<RawDenoiseSnapshot, RawDenoiseServiceError>;
    fn request_preview(
        &mut self,
        request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError>;
    fn request_full(
        &mut self,
        request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError>;
    fn request_export(
        &mut self,
        request: &RawDenoiseJobRequest,
    ) -> Result<u64, RawDenoiseServiceError>;
    fn cancel(&mut self, job: u64) -> Result<(), RawDenoiseServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDenoiseViewModel {
    pub(crate) snapshot: RawDenoiseSnapshot,
    pub(crate) model: Option<ModelHash>,
    pub(crate) provider: Option<AiProvider>,
    pub(crate) strength: u8,
    pub(crate) tile_size: u32,
    pub(crate) plan_policy: RawDenoisePlanPolicy,
    pub(crate) output_policy: RawDenoiseOutputPolicy,
    pub(crate) generation: u64,
    pub(crate) snapshot_generation: u64,
    pub(crate) plan: Option<RawDenoisePlan>,
    pub(crate) provider_state: RawDenoiseProviderState,
    pub(crate) memory_state: RawDenoiseMemoryState,
    pub(crate) cancellation_state: RawDenoiseCancellationState,
    pub(crate) progress: Option<RawDenoiseProgress>,
    pub(crate) failure: Option<RawDenoiseFailure>,
    pub(crate) completed: Option<RawDenoiseJobKind>,
    pub(crate) status: RawDenoiseStatus,
    pub(crate) active_job: Option<u64>,
    pub(crate) cancellation_job: Option<u64>,
}

impl Default for RawDenoiseViewModel {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl RawDenoiseViewModel {
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            snapshot: RawDenoiseSnapshot::unavailable(PhotoSelection::none()),
            model: None,
            provider: None,
            strength: 50,
            tile_size: 256,
            plan_policy: RawDenoisePlanPolicy::MinimalRaw,
            output_policy: RawDenoiseOutputPolicy::PublishAndImport,
            generation: 0,
            snapshot_generation: 0,
            plan: None,
            provider_state: RawDenoiseProviderState::Unavailable,
            memory_state: RawDenoiseMemoryState::Unknown,
            cancellation_state: RawDenoiseCancellationState::Idle,
            progress: None,
            failure: Some(RawDenoiseFailure::BackendUnavailable),
            completed: None,
            status: RawDenoiseStatus::Failed(RawDenoiseFailure::BackendUnavailable),
            active_job: None,
            cancellation_job: None,
        }
    }
    #[must_use]
    pub const fn snapshot(&self) -> &RawDenoiseSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn model(&self) -> Option<&ModelHash> {
        self.model.as_ref()
    }
    #[must_use]
    pub const fn provider(&self) -> Option<AiProvider> {
        self.provider
    }
    #[must_use]
    pub const fn strength(&self) -> u8 {
        self.strength
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub const fn plan_policy(&self) -> RawDenoisePlanPolicy {
        self.plan_policy
    }
    #[must_use]
    pub const fn output_policy(&self) -> RawDenoiseOutputPolicy {
        self.output_policy
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn plan(&self) -> Option<&RawDenoisePlan> {
        self.plan.as_ref()
    }
    #[must_use]
    pub const fn provider_state(&self) -> RawDenoiseProviderState {
        self.provider_state
    }
    #[must_use]
    pub const fn memory_state(&self) -> RawDenoiseMemoryState {
        self.memory_state
    }
    #[must_use]
    pub const fn cancellation_state(&self) -> RawDenoiseCancellationState {
        self.cancellation_state
    }
    #[must_use]
    pub const fn progress(&self) -> Option<RawDenoiseProgress> {
        self.progress
    }
    #[must_use]
    pub const fn failure(&self) -> Option<&RawDenoiseFailure> {
        self.failure.as_ref()
    }
    #[must_use]
    pub const fn completed(&self) -> Option<RawDenoiseJobKind> {
        self.completed
    }
    #[must_use]
    pub const fn status(&self) -> &RawDenoiseStatus {
        &self.status
    }
    #[must_use]
    pub const fn active_job(&self) -> Option<u64> {
        self.active_job
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: RawDenoiseSnapshot) -> bool {
        if snapshot.generation() < self.snapshot_generation {
            return false;
        }
        self.snapshot_generation = snapshot.generation();
        self.snapshot = snapshot;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDenoiseAction {
    Refresh,
    SetSelection(PhotoSelection),
    SelectModel(Option<ModelHash>),
    SelectProvider(Option<AiProvider>),
    SetStrength(u8),
    SetTile(u32),
    SetPlanPolicy(RawDenoisePlanPolicy),
    SetOutputPolicy(RawDenoiseOutputPolicy),
    Preview,
    Full,
    Export,
    Cancel,
}
