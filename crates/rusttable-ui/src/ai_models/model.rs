//! Display-safe model registry DTOs. No package parsing or persistence belongs here.

#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::path::PathBuf;

pub const AI_MODELS_FOCUS_ORDER: [&str; 11] = [
    "ai-models-refresh",
    "ai-models-package-picker",
    "ai-models-confirm-install",
    "ai-models-provider-policy",
    "ai-models-task",
    "ai-models-model",
    "ai-models-qualify",
    "ai-models-enabled",
    "ai-models-remove",
    "ai-models-cancel",
    "ai-models-status",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiModelsDisplayState {
    Loading,
    Empty,
    Ready,
    UnavailableProvider,
    ActiveRun,
    Failure,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AiTask {
    RawBayerDenoise,
    RawLinearDenoise,
    RgbDenoise,
    Upscale2x,
    Upscale4x,
}

impl AiTask {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RawBayerDenoise => "RAW denoise (Bayer)",
            Self::RawLinearDenoise => "RAW denoise (linear)",
            Self::RgbDenoise => "RGB denoise",
            Self::Upscale2x => "Upscale 2×",
            Self::Upscale4x => "Upscale 4×",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::RawBayerDenoise,
            Self::RawLinearDenoise,
            Self::RgbDenoise,
            Self::Upscale2x,
            Self::Upscale4x,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AiProvider {
    Cpu,
    CoreMl,
    DirectMl,
    Cuda,
}

impl AiProvider {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::CoreMl => "Core ML",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Cpu, Self::CoreMl, Self::DirectMl, Self::Cuda]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProviderPolicy {
    Auto,
    Cpu,
    CoreMl,
    DirectMl,
    Cuda,
}

impl AiProviderPolicy {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (qualified, then CPU)",
            Self::Cpu => "CPU",
            Self::CoreMl => "Core ML",
            Self::DirectMl => "DirectML",
            Self::Cuda => "CUDA",
        }
    }

    #[must_use]
    pub const fn provider(self) -> Option<AiProvider> {
        match self {
            Self::Auto => None,
            Self::Cpu => Some(AiProvider::Cpu),
            Self::CoreMl => Some(AiProvider::CoreMl),
            Self::DirectMl => Some(AiProvider::DirectMl),
            Self::Cuda => Some(AiProvider::Cuda),
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::Auto,
            Self::Cpu,
            Self::CoreMl,
            Self::DirectMl,
            Self::Cuda,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualificationState {
    Unavailable,
    Unqualified,
    Qualifying,
    Qualified,
    Drifted,
    Failed,
}

impl QualificationState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Unqualified => "Unqualified",
            Self::Qualifying => "Qualifying",
            Self::Qualified => "Qualified",
            Self::Drifted => "Drifted",
            Self::Failed => "Failed",
        }
    }

    #[must_use]
    pub const fn usable(self) -> bool {
        matches!(self, Self::Qualified)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelServiceState {
    Available,
    Unavailable,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelHash(String);

impl ModelHash {
    pub fn new(value: impl Into<String>) -> Result<Self, ModelHashError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ModelHashError::Invalid);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelHashError {
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    provider: AiProvider,
    state: QualificationState,
    runtime: Option<String>,
    last_result: Option<String>,
}

impl ProviderCapability {
    #[must_use]
    pub fn new(provider: AiProvider, state: QualificationState) -> Self {
        Self {
            provider,
            state,
            runtime: None,
            last_result: None,
        }
    }

    #[must_use]
    pub const fn provider(&self) -> AiProvider {
        self.provider
    }
    #[must_use]
    pub const fn state(&self) -> QualificationState {
        self.state
    }
    #[must_use]
    pub fn runtime(&self) -> Option<&str> {
        self.runtime.as_deref()
    }
    #[must_use]
    pub fn last_result(&self) -> Option<&str> {
        self.last_result.as_deref()
    }
    #[must_use]
    pub fn with_runtime(mut self, value: impl Into<String>) -> Self {
        self.runtime = Some(value.into());
        self
    }
    #[must_use]
    pub fn with_last_result(mut self, value: impl Into<String>) -> Self {
        self.last_result = Some(value.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    model_id: String,
    version: String,
    hash: ModelHash,
    task: AiTask,
    package_bytes: u64,
    tensor_summary: String,
    tile_summary: String,
    color_summary: String,
    enabled: bool,
    runtime_compatibility: String,
    providers: Vec<ProviderCapability>,
    last_validation: Option<String>,
}

impl InstalledModel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model_id: impl Into<String>,
        version: impl Into<String>,
        hash: ModelHash,
        task: AiTask,
        package_bytes: u64,
        tensor_summary: impl Into<String>,
        tile_summary: impl Into<String>,
        color_summary: impl Into<String>,
        providers: Vec<ProviderCapability>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            version: version.into(),
            hash,
            task,
            package_bytes,
            tensor_summary: tensor_summary.into(),
            tile_summary: tile_summary.into(),
            color_summary: color_summary.into(),
            enabled: false,
            runtime_compatibility: "Unknown until the registry service qualifies it".to_owned(),
            providers,
            last_validation: None,
        }
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    #[must_use]
    pub const fn hash(&self) -> &ModelHash {
        &self.hash
    }
    #[must_use]
    pub const fn task(&self) -> AiTask {
        self.task
    }
    #[must_use]
    pub const fn package_bytes(&self) -> u64 {
        self.package_bytes
    }
    #[must_use]
    pub fn tensor_summary(&self) -> &str {
        &self.tensor_summary
    }
    #[must_use]
    pub fn tile_summary(&self) -> &str {
        &self.tile_summary
    }
    #[must_use]
    pub fn color_summary(&self) -> &str {
        &self.color_summary
    }
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub fn runtime_compatibility(&self) -> &str {
        &self.runtime_compatibility
    }
    #[must_use]
    pub fn providers(&self) -> &[ProviderCapability] {
        &self.providers
    }
    #[must_use]
    pub fn last_validation(&self) -> Option<&str> {
        self.last_validation.as_deref()
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn set_last_validation(&mut self, value: Option<String>) {
        self.last_validation = value;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiModelsSnapshot {
    generation: u64,
    service_state: ModelServiceState,
    models: Vec<InstalledModel>,
    provider_policy: AiProviderPolicy,
    task_defaults: Vec<(AiTask, Option<ModelHash>)>,
    available_providers: Vec<ProviderCapability>,
    announcement: String,
}

impl AiModelsSnapshot {
    #[must_use]
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            generation: 0,
            service_state: ModelServiceState::Unavailable,
            models: Vec::new(),
            provider_policy: AiProviderPolicy::Auto,
            task_defaults: AiTask::all().into_iter().map(|task| (task, None)).collect(),
            available_providers: vec![ProviderCapability::new(
                AiProvider::Cpu,
                QualificationState::Unavailable,
            )],
            announcement: reason.into(),
        }
    }

    #[must_use]
    pub fn available(models: Vec<InstalledModel>) -> Self {
        Self {
            generation: 0,
            service_state: ModelServiceState::Available,
            models,
            provider_policy: AiProviderPolicy::Auto,
            task_defaults: AiTask::all().into_iter().map(|task| (task, None)).collect(),
            available_providers: vec![ProviderCapability::new(
                AiProvider::Cpu,
                QualificationState::Qualified,
            )],
            announcement: "Model registry ready".to_owned(),
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    #[must_use]
    pub const fn with_service_state(mut self, service_state: ModelServiceState) -> Self {
        self.service_state = service_state;
        self
    }

    #[must_use]
    pub fn with_available_providers(mut self, providers: Vec<ProviderCapability>) -> Self {
        self.available_providers = providers;
        self
    }

    #[must_use]
    pub const fn service_state(&self) -> ModelServiceState {
        self.service_state
    }
    #[must_use]
    pub fn models(&self) -> &[InstalledModel] {
        &self.models
    }
    #[must_use]
    pub const fn provider_policy(&self) -> AiProviderPolicy {
        self.provider_policy
    }
    #[must_use]
    pub fn task_defaults(&self) -> &[(AiTask, Option<ModelHash>)] {
        &self.task_defaults
    }
    #[must_use]
    pub fn available_providers(&self) -> &[ProviderCapability] {
        &self.available_providers
    }

    #[must_use]
    pub fn provider(&self, provider: AiProvider) -> Option<&ProviderCapability> {
        self.available_providers
            .iter()
            .find(|capability| capability.provider() == provider)
    }

    #[must_use]
    pub fn provider_unavailable(&self) -> bool {
        self.provider_policy.provider().is_some_and(|provider| {
            self.provider(provider)
                .is_none_or(|capability| capability.state() == QualificationState::Unavailable)
        })
    }
    #[must_use]
    pub fn announcement(&self) -> &str {
        &self.announcement
    }
    pub fn set_provider_policy(&mut self, policy: AiProviderPolicy) {
        self.provider_policy = policy;
    }
    pub fn set_task_default(&mut self, task: AiTask, hash: Option<ModelHash>) {
        if let Some(entry) = self.task_defaults.iter_mut().find(|entry| entry.0 == task) {
            entry.1 = hash;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSummary {
    file_name: String,
    model_id: String,
    version: String,
    hash: ModelHash,
    package_bytes: u64,
    task: AiTask,
    validation: String,
}

impl InstallSummary {
    #[must_use]
    pub fn new(
        file_name: impl Into<String>,
        model_id: impl Into<String>,
        version: impl Into<String>,
        hash: ModelHash,
        package_bytes: u64,
        task: AiTask,
        validation: impl Into<String>,
    ) -> Self {
        Self {
            file_name: file_name.into(),
            model_id: model_id.into(),
            version: version.into(),
            hash,
            package_bytes,
            task,
            validation: validation.into(),
        }
    }
    #[must_use]
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
    #[must_use]
    pub const fn hash(&self) -> &ModelHash {
        &self.hash
    }
    #[must_use]
    pub const fn package_bytes(&self) -> u64 {
        self.package_bytes
    }
    #[must_use]
    pub const fn task(&self) -> AiTask {
        self.task
    }
    #[must_use]
    pub fn validation(&self) -> &str {
        &self.validation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualificationJob {
    id: u64,
    model: ModelHash,
    provider: AiProvider,
    completed: u32,
    total: u32,
    detail: String,
}

impl QualificationJob {
    #[must_use]
    pub fn new(id: u64, model: ModelHash, provider: AiProvider, total: u32) -> Self {
        Self {
            id,
            model,
            provider,
            completed: 0,
            total: total.max(1),
            detail: "Qualification queued".to_owned(),
        }
    }
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
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
    pub const fn completed(&self) -> u32 {
        self.completed
    }
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.total
    }
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
    #[must_use]
    pub fn fraction(&self) -> f64 {
        f64::from(self.completed) / f64::from(self.total)
    }
    pub fn set_progress(&mut self, completed: u32, detail: impl Into<String>) {
        self.completed = completed.min(self.total);
        self.detail = detail.into();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiModelsFailure {
    ServiceUnavailable,
    InvalidPackage,
    HashConflict,
    InUse,
    ProviderUnavailable,
    QualificationFailed,
    Cancelled,
    Configuration,
    Transaction,
}

impl AiModelsFailure {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ServiceUnavailable => {
                "AI model service is unavailable; no package operation was performed."
            }
            Self::InvalidPackage => "The selected package did not pass bounded validation.",
            Self::HashConflict => "A different package already uses this model ID and version.",
            Self::InUse => "The model is in use by an active task and cannot be removed.",
            Self::ProviderUnavailable => {
                "That provider is unavailable or not qualified for this model."
            }
            Self::QualificationFailed => {
                "Provider qualification failed; the provider remains unusable."
            }
            Self::Cancelled => "The model operation was cancelled.",
            Self::Configuration => "The provider or task default could not be saved.",
            Self::Transaction => "The model registry transaction was not committed.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiModelsViewModel {
    snapshot: AiModelsSnapshot,
    staging: Option<InstallSummary>,
    qualification: Option<QualificationJob>,
    failure: Option<AiModelsFailure>,
    status: String,
    loading: bool,
    selected_task: AiTask,
    selected_model: Option<ModelHash>,
    selected_provider: AiProvider,
}

impl Default for AiModelsViewModel {
    fn default() -> Self {
        Self::unavailable()
    }
}

impl AiModelsViewModel {
    #[must_use]
    pub fn unavailable() -> Self {
        let snapshot =
            AiModelsSnapshot::unavailable("AI model registry service is not connected (#478).");
        Self {
            status: snapshot.announcement().to_owned(),
            snapshot,
            staging: None,
            qualification: None,
            failure: None,
            loading: false,
            selected_task: AiTask::RawBayerDenoise,
            selected_model: None,
            selected_provider: AiProvider::Cpu,
        }
    }

    #[must_use]
    pub fn loading() -> Self {
        let mut state = Self::unavailable();
        "Loading AI model registry…".clone_into(&mut state.status);
        state.loading = true;
        state
    }
    #[must_use]
    pub const fn snapshot(&self) -> &AiModelsSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn staging(&self) -> Option<&InstallSummary> {
        self.staging.as_ref()
    }
    #[must_use]
    pub const fn qualification(&self) -> Option<&QualificationJob> {
        self.qualification.as_ref()
    }
    #[must_use]
    pub fn failure(&self) -> Option<AiModelsFailure> {
        self.failure
    }
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub fn display_state(&self) -> AiModelsDisplayState {
        if self.loading {
            return AiModelsDisplayState::Loading;
        }
        if self.failure.is_some() {
            return AiModelsDisplayState::Failure;
        }
        if self.qualification.is_some() {
            return AiModelsDisplayState::ActiveRun;
        }
        if self.snapshot.service_state() == ModelServiceState::Unavailable {
            return AiModelsDisplayState::Unavailable;
        }
        if self.snapshot.service_state() == ModelServiceState::Degraded
            || self.snapshot.provider_unavailable()
        {
            return AiModelsDisplayState::UnavailableProvider;
        }
        if self.snapshot.models().is_empty() {
            return AiModelsDisplayState::Empty;
        }
        AiModelsDisplayState::Ready
    }

    #[must_use]
    pub const fn selected_task(&self) -> AiTask {
        self.selected_task
    }

    #[must_use]
    pub const fn selected_model(&self) -> Option<&ModelHash> {
        self.selected_model.as_ref()
    }

    #[must_use]
    pub const fn selected_provider(&self) -> AiProvider {
        self.selected_provider
    }

    #[must_use]
    pub const fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn begin_refresh(&mut self) {
        self.loading = true;
        self.failure = None;
        "Loading installed AI model packages…".clone_into(&mut self.status);
    }

    pub fn select_provider(&mut self, provider: AiProvider) {
        self.selected_provider = provider;
    }

    pub fn select_task(&mut self, task: AiTask) {
        self.selected_task = task;
        self.reconcile_selection();
    }

    pub fn select_model(&mut self, model: Option<ModelHash>) {
        self.selected_model = model;
        self.reconcile_selection();
    }

    pub fn replace_snapshot(&mut self, snapshot: AiModelsSnapshot) {
        let _ = self.replace_snapshot_if_newer(snapshot);
    }

    pub fn replace_snapshot_if_newer(&mut self, snapshot: AiModelsSnapshot) -> bool {
        if snapshot.generation() < self.snapshot.generation() {
            self.loading = false;
            "A stale model registry refresh was ignored; current state kept."
                .clone_into(&mut self.status);
            return false;
        }
        snapshot.announcement().clone_into(&mut self.status);
        self.snapshot = snapshot;
        self.failure = None;
        self.loading = false;
        self.reconcile_selection();
        true
    }
    pub fn set_staging(&mut self, staging: Option<InstallSummary>) {
        self.staging = staging;
    }
    pub fn set_qualification(&mut self, job: Option<QualificationJob>) {
        self.qualification = job;
    }
    pub fn announce(&mut self, value: impl Into<String>) {
        self.status = value.into();
        self.failure = None;
        self.loading = false;
    }
    pub fn fail(&mut self, failure: AiModelsFailure) {
        failure.message().clone_into(&mut self.status);
        self.failure = Some(failure);
        self.loading = false;
    }

    fn reconcile_selection(&mut self) {
        let task_models = self
            .snapshot
            .models()
            .iter()
            .filter(|model| model.task() == self.selected_task)
            .collect::<Vec<_>>();
        let selected_is_valid = self
            .selected_model
            .as_ref()
            .is_some_and(|hash| task_models.iter().any(|model| model.hash() == hash));
        if selected_is_valid {
            return;
        }
        let configured = self
            .snapshot
            .task_defaults()
            .iter()
            .find(|(task, _)| *task == self.selected_task)
            .and_then(|(_, hash)| hash.as_ref())
            .and_then(|hash| task_models.iter().find(|model| model.hash() == hash))
            .map(|model| model.hash().clone());
        self.selected_model = configured.or_else(|| {
            task_models
                .iter()
                .find(|model| model.enabled())
                .or_else(|| task_models.first())
                .map(|model| model.hash().clone())
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiModelsAction {
    Refresh,
    SelectLocalPackage(PathBuf),
    ConfirmInstall,
    CancelInstall,
    SetProviderPolicy(AiProviderPolicy),
    SelectQualificationProvider(AiProvider),
    SetTaskDefault {
        task: AiTask,
        model: Option<ModelHash>,
    },
    SetEnabled {
        model: ModelHash,
        enabled: bool,
    },
    Qualify {
        model: ModelHash,
        provider: AiProvider,
    },
    CancelQualification(u64),
    Remove(ModelHash),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiModelsServiceError {
    Unavailable,
    InvalidPackage,
    HashConflict,
    InUse,
    ProviderUnavailable,
    QualificationFailed,
    Cancelled,
    Configuration,
    Transaction,
}

impl AiModelsServiceError {
    #[must_use]
    pub(crate) const fn failure(&self) -> AiModelsFailure {
        match self {
            Self::Unavailable => AiModelsFailure::ServiceUnavailable,
            Self::InvalidPackage => AiModelsFailure::InvalidPackage,
            Self::HashConflict => AiModelsFailure::HashConflict,
            Self::InUse => AiModelsFailure::InUse,
            Self::ProviderUnavailable => AiModelsFailure::ProviderUnavailable,
            Self::QualificationFailed => AiModelsFailure::QualificationFailed,
            Self::Cancelled => AiModelsFailure::Cancelled,
            Self::Configuration => AiModelsFailure::Configuration,
            Self::Transaction => AiModelsFailure::Transaction,
        }
    }
}

pub trait AiModelsServicePort {
    fn snapshot(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError>;
    fn stage_local_package(
        &mut self,
        source: PathBuf,
    ) -> Result<InstallSummary, AiModelsServiceError>;
    fn install_staged(&mut self) -> Result<AiModelsSnapshot, AiModelsServiceError>;
    fn set_provider_policy(
        &mut self,
        policy: AiProviderPolicy,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError>;
    fn set_task_default(
        &mut self,
        task: AiTask,
        model: Option<ModelHash>,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError>;
    fn set_enabled(
        &mut self,
        model: &ModelHash,
        enabled: bool,
    ) -> Result<AiModelsSnapshot, AiModelsServiceError>;
    fn start_qualification(
        &mut self,
        model: &ModelHash,
        provider: AiProvider,
    ) -> Result<QualificationJob, AiModelsServiceError>;
    fn cancel_qualification(&mut self, job: u64) -> Result<(), AiModelsServiceError>;
    fn remove(&mut self, model: &ModelHash) -> Result<AiModelsSnapshot, AiModelsServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> ModelHash {
        ModelHash::new("A".repeat(64)).expect("hash")
    }

    #[test]
    fn model_identity_is_exactly_hash_addressed() {
        assert_eq!(hash().as_str(), "a".repeat(64));
        assert!(ModelHash::new("short").is_err());
    }

    #[test]
    fn unavailable_state_does_not_claim_a_model_or_provider() {
        let state = AiModelsViewModel::default();
        assert!(state.snapshot().models().is_empty());
        assert_eq!(
            state.snapshot().service_state(),
            ModelServiceState::Unavailable
        );
        assert_eq!(
            state.snapshot().available_providers()[0].state(),
            QualificationState::Unavailable
        );
    }

    #[test]
    fn stale_snapshot_is_ignored_and_selection_survives_new_generation() {
        let model = InstalledModel::new(
            "rgb-denoise",
            "1",
            hash(),
            AiTask::RgbDenoise,
            42,
            "NCHW f32 1×3×256×256",
            "256² overlap 16",
            "linear RGB",
            vec![ProviderCapability::new(
                AiProvider::Cpu,
                QualificationState::Qualified,
            )],
        );
        let model_hash = model.hash().clone();
        let mut state = AiModelsViewModel::default();
        state.select_task(AiTask::RgbDenoise);
        assert!(state.replace_snapshot_if_newer(
            AiModelsSnapshot::available(vec![model]).with_generation(4)
        ));
        assert_eq!(state.selected_model(), Some(&model_hash));
        assert!(
            !state.replace_snapshot_if_newer(
                AiModelsSnapshot::available(Vec::new()).with_generation(3)
            )
        );
        assert_eq!(state.selected_model(), Some(&model_hash));
        assert_eq!(state.snapshot().generation(), 4);
    }

    #[test]
    fn display_state_covers_loading_empty_provider_run_and_failure() {
        let mut state = AiModelsViewModel::loading();
        assert_eq!(state.display_state(), AiModelsDisplayState::Loading);
        state.replace_snapshot(AiModelsSnapshot::available(Vec::new()));
        assert_eq!(state.display_state(), AiModelsDisplayState::Empty);
        let mut unavailable_provider =
            AiModelsSnapshot::available(Vec::new()).with_available_providers(vec![
                ProviderCapability::new(AiProvider::CoreMl, QualificationState::Unavailable),
            ]);
        unavailable_provider.set_provider_policy(AiProviderPolicy::CoreMl);
        state.replace_snapshot(unavailable_provider);
        assert_eq!(
            state.display_state(),
            AiModelsDisplayState::UnavailableProvider
        );
        let model = ModelHash::new("b".repeat(64)).expect("hash");
        state.set_qualification(Some(QualificationJob::new(1, model, AiProvider::Cpu, 2)));
        assert_eq!(state.display_state(), AiModelsDisplayState::ActiveRun);
        state.set_qualification(None);
        state.fail(AiModelsFailure::InvalidPackage);
        assert_eq!(state.display_state(), AiModelsDisplayState::Failure);
        assert_eq!(
            state.status(),
            "The selected package did not pass bounded validation."
        );
    }
}
