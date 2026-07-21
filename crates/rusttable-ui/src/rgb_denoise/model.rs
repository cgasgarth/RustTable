//! Display-safe state and service contracts for the darkroom RGB denoise module.

#![allow(clippy::cast_precision_loss, clippy::items_after_statements)]
#![allow(clippy::missing_errors_doc)]

use std::fmt;

use crate::ai_models::{AiProvider, ModelHash};
use crate::neural_restore::PhotoSelection;

pub const RGB_DENOISE_FOCUS_ORDER: [&str; 19] = [
    "rgb-denoise-model",
    "rgb-denoise-provider",
    "rgb-denoise-working-profile",
    "rgb-denoise-model-profile",
    "rgb-denoise-scale",
    "rgb-denoise-tile",
    "rgb-denoise-strength",
    "rgb-denoise-gamut",
    "rgb-denoise-shadows",
    "rgb-denoise-detail",
    "rgb-denoise-detail-strength",
    "rgb-denoise-preview",
    "rgb-denoise-full",
    "rgb-denoise-export",
    "rgb-denoise-cancel",
    "rgb-denoise-progress",
    "rgb-denoise-plan",
    "rgb-denoise-memory",
    "rgb-denoise-status",
];

pub const RGB_DENOISE_SCALES: [u8; 1] = [1];
pub const RGB_DENOISE_TILES: [u32; 3] = [128, 256, 512];
pub const RGB_DENOISE_MAX_DETAIL_STRENGTH: u8 = 100;
pub const RGB_DENOISE_MAX_STRENGTH: u8 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseModelOption {
    hash: ModelHash,
    label: String,
    qualified: bool,
    scale: u8,
    tile_size: u32,
    providers: Vec<AiProvider>,
    shadow_boost: bool,
}

impl RgbDenoiseModelOption {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hash: ModelHash,
        label: impl Into<String>,
        qualified: bool,
        scale: u8,
        tile_size: u32,
        providers: Vec<AiProvider>,
        shadow_boost: bool,
    ) -> Self {
        Self {
            hash,
            label: label.into(),
            qualified,
            scale,
            tile_size,
            providers,
            shadow_boost,
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
    pub const fn qualified(&self) -> bool {
        self.qualified
    }
    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub fn providers(&self) -> &[AiProvider] {
        &self.providers
    }
    #[must_use]
    pub const fn shadow_boost(&self) -> bool {
        self.shadow_boost
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseProfileOption {
    id: String,
    label: String,
}

impl RgbDenoiseProfileOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseSnapshot {
    generation: u64,
    selection: PhotoSelection,
    dimensions: Option<(u32, u32)>,
    models: Vec<RgbDenoiseModelOption>,
    providers: Vec<AiProvider>,
    working_profiles: Vec<RgbDenoiseProfileOption>,
    model_profiles: Vec<RgbDenoiseProfileOption>,
}

impl RgbDenoiseSnapshot {
    #[must_use]
    pub fn unavailable(selection: PhotoSelection) -> Self {
        Self {
            generation: 0,
            selection,
            dimensions: None,
            models: Vec::new(),
            providers: Vec::new(),
            working_profiles: Vec::new(),
            model_profiles: Vec::new(),
        }
    }

    #[must_use]
    pub fn available(
        selection: PhotoSelection,
        dimensions: (u32, u32),
        models: Vec<RgbDenoiseModelOption>,
        providers: Vec<AiProvider>,
        working_profiles: Vec<RgbDenoiseProfileOption>,
        model_profiles: Vec<RgbDenoiseProfileOption>,
    ) -> Self {
        Self {
            generation: 1,
            selection,
            dimensions: Some(dimensions),
            models,
            providers,
            working_profiles,
            model_profiles,
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
    pub const fn selection(&self) -> &PhotoSelection {
        &self.selection
    }
    #[must_use]
    pub const fn dimensions(&self) -> Option<(u32, u32)> {
        self.dimensions
    }
    #[must_use]
    pub fn models(&self) -> &[RgbDenoiseModelOption] {
        &self.models
    }
    pub fn qualified_models(&self) -> impl Iterator<Item = &RgbDenoiseModelOption> {
        self.models.iter().filter(|model| model.qualified())
    }
    #[must_use]
    pub fn providers(&self) -> &[AiProvider] {
        &self.providers
    }
    #[must_use]
    pub fn working_profiles(&self) -> &[RgbDenoiseProfileOption] {
        &self.working_profiles
    }
    #[must_use]
    pub fn model_profiles(&self) -> &[RgbDenoiseProfileOption] {
        &self.model_profiles
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbDenoiseGamutPolicy {
    ConvertToWorking,
    PreserveWideGamut,
}

impl RgbDenoiseGamutPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ConvertToWorking => "Convert to working gamut",
            Self::PreserveWideGamut => "Preserve wide gamut",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbDenoiseShadowPolicy {
    Disabled,
    ProtectDeepShadows,
}

impl RgbDenoiseShadowPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::ProtectDeepShadows => "Protect deep shadows",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbDenoiseDetailPolicy {
    Disabled,
    Recover,
}

impl RgbDenoiseDetailPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::Recover => "Recover detail",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbDenoiseJobKind {
    Preview,
    Full,
    Export,
}

impl RgbDenoiseJobKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Preview => "Preview",
            Self::Full => "Full render",
            Self::Export => "Export",
        }
    }
}

/// Immutable UI representation of the validated #500 RGB plan.
///
/// The app seam is expected to populate this from the backend plan and the
/// qualified #478 model/provider registry snapshot. GTK only edits intent and
/// dispatches typed requests; it never creates inference buffers or performs
/// processing itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoisePlan {
    generation: u64,
    identity: String,
    model: ModelHash,
    working_profile: String,
    model_profile: String,
    provider: AiProvider,
    scale: u8,
    tile_size: u32,
    strength: u8,
    gamut: RgbDenoiseGamutPolicy,
    shadows: RgbDenoiseShadowPolicy,
    detail: RgbDenoiseDetailPolicy,
    detail_strength: u8,
    memory_bytes: u64,
}

impl RgbDenoisePlan {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        generation: u64,
        model: ModelHash,
        working_profile: impl Into<String>,
        model_profile: impl Into<String>,
        provider: AiProvider,
        scale: u8,
        tile_size: u32,
        strength: u8,
        gamut: RgbDenoiseGamutPolicy,
        shadows: RgbDenoiseShadowPolicy,
        detail: RgbDenoiseDetailPolicy,
        detail_strength: u8,
        dimensions: (u32, u32),
    ) -> Result<Self, RgbDenoisePlanError> {
        if !RGB_DENOISE_SCALES.contains(&scale) {
            return Err(RgbDenoisePlanError::ScaleOutOfBounds { value: scale });
        }
        if !RGB_DENOISE_TILES.contains(&tile_size) {
            return Err(RgbDenoisePlanError::TileOutOfBounds { value: tile_size });
        }
        if detail_strength > RGB_DENOISE_MAX_DETAIL_STRENGTH {
            return Err(RgbDenoisePlanError::DetailOutOfBounds {
                value: detail_strength,
            });
        }
        if strength > RGB_DENOISE_MAX_STRENGTH {
            return Err(RgbDenoisePlanError::StrengthOutOfBounds { value: strength });
        }
        let pixels = u64::from(dimensions.0)
            .checked_mul(u64::from(dimensions.1))
            .ok_or(RgbDenoisePlanError::DimensionsOverflow)?;
        if pixels == 0 {
            return Err(RgbDenoisePlanError::EmptyImage);
        }
        let memory_bytes = pixels
            .checked_mul(20)
            .and_then(|bytes| bytes.checked_add(u64::from(tile_size) * u64::from(tile_size) * 12))
            .ok_or(RgbDenoisePlanError::MemoryOverflow)?;
        const MEMORY_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
        if memory_bytes > MEMORY_LIMIT {
            return Err(RgbDenoisePlanError::MemoryLimit {
                bytes: memory_bytes,
                limit: MEMORY_LIMIT,
            });
        }
        let working_profile = working_profile.into();
        let model_profile = model_profile.into();
        if working_profile.is_empty() || model_profile.is_empty() {
            return Err(RgbDenoisePlanError::MissingProfile);
        }
        let identity = format!(
            "rgb-denoise-v1:{generation}:{}:{working_profile}:{model_profile}:{}:{scale}:{tile_size}:{strength}:{}:{}:{}:{detail_strength}",
            model.as_str(),
            provider.label(),
            gamut.label(),
            shadows.label(),
            detail.label(),
        );
        Ok(Self {
            generation,
            identity,
            model,
            working_profile,
            model_profile,
            provider,
            scale,
            tile_size,
            strength,
            gamut,
            shadows,
            detail,
            detail_strength,
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
    pub const fn model(&self) -> &ModelHash {
        &self.model
    }
    #[must_use]
    pub fn working_profile(&self) -> &str {
        &self.working_profile
    }
    #[must_use]
    pub fn model_profile(&self) -> &str {
        &self.model_profile
    }
    #[must_use]
    pub const fn provider(&self) -> AiProvider {
        self.provider
    }
    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub const fn strength(&self) -> u8 {
        self.strength
    }
    #[must_use]
    pub const fn gamut(&self) -> RgbDenoiseGamutPolicy {
        self.gamut
    }
    #[must_use]
    pub const fn shadows(&self) -> RgbDenoiseShadowPolicy {
        self.shadows
    }
    #[must_use]
    pub const fn detail(&self) -> RgbDenoiseDetailPolicy {
        self.detail
    }
    #[must_use]
    pub const fn detail_strength(&self) -> u8 {
        self.detail_strength
    }
    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbDenoisePlanError {
    EmptyImage,
    DimensionsOverflow,
    ScaleOutOfBounds { value: u8 },
    TileOutOfBounds { value: u32 },
    DetailOutOfBounds { value: u8 },
    StrengthOutOfBounds { value: u8 },
    MemoryOverflow,
    MemoryLimit { bytes: u64, limit: u64 },
    MissingProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbDenoiseProviderState {
    Unavailable,
    Available,
    Selected(AiProvider),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbDenoiseProfileState {
    Unavailable,
    Available,
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbDenoiseMemoryState {
    Unknown,
    Estimated { bytes: u64 },
    Exceeded { bytes: u64, limit: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbDenoiseCancellationState {
    Idle,
    Requested,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbDenoiseProgress {
    pub completed: u64,
    pub total: u64,
}

impl RgbDenoiseProgress {
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
pub enum RgbDenoiseFailure {
    ServiceUnavailable,
    ProviderUnavailable,
    ProfileUnavailable,
    MemoryBudgetExceeded { bytes: u64, limit: u64 },
    Cancelled,
    Failed(String),
    StaleGeneration,
}

impl RgbDenoiseFailure {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::ServiceUnavailable => {
                "RGB denoise service unavailable; no inference was started.".to_owned()
            }
            Self::ProviderUnavailable => {
                "No qualified provider is available for the selected model.".to_owned()
            }
            Self::ProfileUnavailable => {
                "Working and model profiles are required before planning.".to_owned()
            }
            Self::MemoryBudgetExceeded { bytes, limit } => {
                format!("Memory estimate {bytes} B exceeds limit {limit} B.")
            }
            Self::Cancelled => "RGB denoise job cancelled.".to_owned(),
            Self::Failed(error) => format!("RGB denoise failed: {error}"),
            Self::StaleGeneration => {
                "RGB denoise result discarded because a newer plan exists.".to_owned()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbDenoiseStatus {
    Idle,
    Planning,
    Ready,
    Running {
        kind: RgbDenoiseJobKind,
        progress: RgbDenoiseProgress,
    },
    Cancelling,
    Cancelled,
    Failed(RgbDenoiseFailure),
    Completed {
        kind: RgbDenoiseJobKind,
        artifact: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseJobRequest {
    generation: u64,
    kind: RgbDenoiseJobKind,
    plan: RgbDenoisePlan,
}

impl RgbDenoiseJobRequest {
    #[must_use]
    pub const fn new(generation: u64, kind: RgbDenoiseJobKind, plan: RgbDenoisePlan) -> Self {
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
    pub const fn kind(&self) -> RgbDenoiseJobKind {
        self.kind
    }
    #[must_use]
    pub const fn plan(&self) -> &RgbDenoisePlan {
        &self.plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbDenoiseServiceEvent {
    Progress {
        generation: u64,
        job: u64,
        progress: RgbDenoiseProgress,
    },
    Completed {
        generation: u64,
        job: u64,
        artifact: Option<String>,
    },
    Failed {
        generation: u64,
        job: u64,
        error: RgbDenoiseFailure,
    },
    Cancelled {
        generation: u64,
        job: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbDenoiseServiceError {
    Unavailable,
    ProviderUnavailable,
    ProfileUnavailable,
    MemoryBudgetExceeded { bytes: u64, limit: u64 },
    Failed(String),
}

impl fmt::Display for RgbDenoiseServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RGB denoise service error: {self:?}")
    }
}
impl std::error::Error for RgbDenoiseServiceError {}

pub trait RgbDenoiseServicePort {
    /// Supplies a generation-tagged registry/profile snapshot for the current selection.
    ///
    /// A production adapter should source qualified models and providers from #478 and
    /// map immutable plans from #500; an unavailable adapter must remain unavailable.
    fn snapshot(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<RgbDenoiseSnapshot, RgbDenoiseServiceError>;
    fn request_preview(
        &mut self,
        request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError>;
    fn request_full(
        &mut self,
        request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError>;
    fn request_export(
        &mut self,
        request: &RgbDenoiseJobRequest,
    ) -> Result<u64, RgbDenoiseServiceError>;
    fn cancel(&mut self, job: u64) -> Result<(), RgbDenoiseServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbDenoiseViewModel {
    pub(crate) snapshot: RgbDenoiseSnapshot,
    pub(crate) model: Option<ModelHash>,
    pub(crate) provider: Option<AiProvider>,
    pub(crate) working_profile: Option<String>,
    pub(crate) model_profile: Option<String>,
    pub(crate) scale: u8,
    pub(crate) tile_size: u32,
    pub(crate) strength: u8,
    pub(crate) gamut: RgbDenoiseGamutPolicy,
    pub(crate) shadows: RgbDenoiseShadowPolicy,
    pub(crate) detail: RgbDenoiseDetailPolicy,
    pub(crate) detail_strength: u8,
    pub(crate) generation: u64,
    pub(crate) snapshot_generation: u64,
    pub(crate) plan: Option<RgbDenoisePlan>,
    pub(crate) provider_state: RgbDenoiseProviderState,
    pub(crate) working_profile_state: RgbDenoiseProfileState,
    pub(crate) model_profile_state: RgbDenoiseProfileState,
    pub(crate) memory_state: RgbDenoiseMemoryState,
    pub(crate) cancellation_state: RgbDenoiseCancellationState,
    pub(crate) progress: Option<RgbDenoiseProgress>,
    pub(crate) failure: Option<RgbDenoiseFailure>,
    pub(crate) completed: Option<RgbDenoiseJobKind>,
    pub(crate) status: RgbDenoiseStatus,
    pub(crate) active_job: Option<u64>,
}

impl Default for RgbDenoiseViewModel {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl RgbDenoiseViewModel {
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            snapshot: RgbDenoiseSnapshot::unavailable(PhotoSelection::none()),
            model: None,
            provider: None,
            working_profile: None,
            model_profile: None,
            scale: 1,
            tile_size: 256,
            strength: 50,
            gamut: RgbDenoiseGamutPolicy::PreserveWideGamut,
            shadows: RgbDenoiseShadowPolicy::ProtectDeepShadows,
            detail: RgbDenoiseDetailPolicy::Recover,
            detail_strength: 50,
            generation: 0,
            snapshot_generation: 0,
            plan: None,
            provider_state: RgbDenoiseProviderState::Unavailable,
            working_profile_state: RgbDenoiseProfileState::Unavailable,
            model_profile_state: RgbDenoiseProfileState::Unavailable,
            memory_state: RgbDenoiseMemoryState::Unknown,
            cancellation_state: RgbDenoiseCancellationState::Idle,
            progress: None,
            failure: Some(RgbDenoiseFailure::ServiceUnavailable),
            completed: None,
            status: RgbDenoiseStatus::Failed(RgbDenoiseFailure::ServiceUnavailable),
            active_job: None,
        }
    }

    #[must_use]
    pub const fn snapshot(&self) -> &RgbDenoiseSnapshot {
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
    pub fn working_profile(&self) -> Option<&str> {
        self.working_profile.as_deref()
    }
    #[must_use]
    pub fn model_profile(&self) -> Option<&str> {
        self.model_profile.as_deref()
    }
    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }
    #[must_use]
    pub const fn tile_size(&self) -> u32 {
        self.tile_size
    }
    #[must_use]
    pub const fn strength(&self) -> u8 {
        self.strength
    }
    #[must_use]
    pub const fn gamut(&self) -> RgbDenoiseGamutPolicy {
        self.gamut
    }
    #[must_use]
    pub const fn shadows(&self) -> RgbDenoiseShadowPolicy {
        self.shadows
    }
    #[must_use]
    pub const fn detail(&self) -> RgbDenoiseDetailPolicy {
        self.detail
    }
    #[must_use]
    pub const fn detail_strength(&self) -> u8 {
        self.detail_strength
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn plan(&self) -> Option<&RgbDenoisePlan> {
        self.plan.as_ref()
    }
    #[must_use]
    pub const fn provider_state(&self) -> RgbDenoiseProviderState {
        self.provider_state
    }
    #[must_use]
    pub const fn working_profile_state(&self) -> RgbDenoiseProfileState {
        self.working_profile_state
    }
    #[must_use]
    pub const fn model_profile_state(&self) -> RgbDenoiseProfileState {
        self.model_profile_state
    }
    #[must_use]
    pub const fn memory_state(&self) -> RgbDenoiseMemoryState {
        self.memory_state
    }
    #[must_use]
    pub const fn cancellation_state(&self) -> RgbDenoiseCancellationState {
        self.cancellation_state
    }
    #[must_use]
    pub const fn progress(&self) -> Option<RgbDenoiseProgress> {
        self.progress
    }
    #[must_use]
    pub const fn failure(&self) -> Option<&RgbDenoiseFailure> {
        self.failure.as_ref()
    }
    #[must_use]
    pub const fn completed(&self) -> Option<RgbDenoiseJobKind> {
        self.completed
    }
    #[must_use]
    pub const fn status(&self) -> &RgbDenoiseStatus {
        &self.status
    }
    #[must_use]
    pub const fn active_job(&self) -> Option<u64> {
        self.active_job
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: RgbDenoiseSnapshot) -> bool {
        if snapshot.generation() < self.snapshot_generation {
            return false;
        }
        self.snapshot_generation = snapshot.generation();
        self.snapshot = snapshot;
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbDenoiseAction {
    Refresh,
    SetSelection(PhotoSelection),
    SelectModel(Option<ModelHash>),
    SelectProvider(Option<AiProvider>),
    SelectWorkingProfile(Option<String>),
    SelectModelProfile(Option<String>),
    SetScale(u8),
    SetTile(u32),
    SetStrength(u8),
    SetGamut(RgbDenoiseGamutPolicy),
    SetShadows(RgbDenoiseShadowPolicy),
    SetDetail(RgbDenoiseDetailPolicy),
    SetDetailStrength(u8),
    Preview,
    Full,
    Export,
    Cancel,
}
