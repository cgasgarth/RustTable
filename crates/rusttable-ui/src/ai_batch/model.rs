//! Side-effect-free batch state and typed service port.

#![allow(clippy::derivable_impls, clippy::missing_errors_doc)]

use std::fmt;

use rusttable_core::PhotoId;

use crate::ai_models::{AiProvider, ModelHash};

pub const AI_BATCH_FOCUS_ORDER: [&str; 14] = [
    "ai-batch-task",
    "ai-batch-model",
    "ai-batch-provider",
    "ai-batch-strength",
    "ai-batch-review",
    "ai-batch-confirm",
    "ai-batch-pause",
    "ai-batch-resume",
    "ai-batch-cancel",
    "ai-batch-retry",
    "ai-batch-remove-history",
    "ai-batch-table",
    "ai-batch-progress",
    "ai-batch-status",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBatchTask {
    RawDenoise,
    RgbDenoise,
    Upscale2,
    Upscale4,
}
impl AiBatchTask {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::RawDenoise,
            Self::RgbDenoise,
            Self::Upscale2,
            Self::Upscale4,
        ]
    }
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RawDenoise => "RAW denoise",
            Self::RgbDenoise => "RGB denoise",
            Self::Upscale2 => "Upscale 2×",
            Self::Upscale4 => "Upscale 4×",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBatchEligibility {
    Eligible,
    Ineligible,
    Stale,
    MissingSource,
    AlreadyCommitted,
}
impl AiBatchEligibility {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Eligible => "Eligible",
            Self::Ineligible => "Ineligible",
            Self::Stale => "Stale revision",
            Self::MissingSource => "Missing source",
            Self::AlreadyCommitted => "Already committed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBatchEnqueuePolicy {
    ProcessEligible,
    RequireAllEligible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBatchStage {
    Queued,
    Loading,
    Inference,
    Encoding,
    Importing,
    Grouping,
    Complete,
    Failed,
    Reconcile,
}
impl AiBatchStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Loading => "Loading",
            Self::Inference => "Inference",
            Self::Encoding => "Output",
            Self::Importing => "Import",
            Self::Grouping => "Grouping",
            Self::Complete => "Complete",
            Self::Failed => "Failed",
            Self::Reconcile => "Recovery",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiBatchCollision {
    Stop,
    Skip,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBatchRecipe {
    task: AiBatchTask,
    model: Option<ModelHash>,
    provider: Option<AiProvider>,
    strength: u8,
    bit_depth: u8,
    output_profile: String,
    filename_template: String,
    destination_alias: String,
    collision: AiBatchCollision,
    import_output: bool,
    group_with_source: bool,
}
impl Default for AiBatchRecipe {
    fn default() -> Self {
        Self {
            task: AiBatchTask::RgbDenoise,
            model: None,
            provider: None,
            strength: 50,
            bit_depth: 16,
            output_profile: "sRGB".to_owned(),
            filename_template: "{filename}-ai".to_owned(),
            destination_alias: "AI output".to_owned(),
            collision: AiBatchCollision::Stop,
            import_output: true,
            group_with_source: true,
        }
    }
}
impl AiBatchRecipe {
    #[must_use]
    pub const fn task(&self) -> AiBatchTask {
        self.task
    }
    #[must_use]
    pub fn model(&self) -> Option<&ModelHash> {
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
    pub const fn bit_depth(&self) -> u8 {
        self.bit_depth
    }
    #[must_use]
    pub fn output_profile(&self) -> &str {
        &self.output_profile
    }
    #[must_use]
    pub fn filename_template(&self) -> &str {
        &self.filename_template
    }
    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }
    #[must_use]
    pub const fn import_output(&self) -> bool {
        self.import_output
    }
    #[must_use]
    pub const fn group_with_source(&self) -> bool {
        self.group_with_source
    }
    pub fn set_task(&mut self, value: AiBatchTask) {
        self.task = value;
    }
    pub fn set_model(&mut self, value: Option<ModelHash>) {
        self.model = value;
    }
    pub fn set_provider(&mut self, value: Option<AiProvider>) {
        self.provider = value;
    }
    pub fn set_strength(&mut self, value: u8) {
        self.strength = value.min(100);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBatchSelection {
    pub photo_id: PhotoId,
    pub source_revision: u64,
    pub edit_revision: u64,
    pub catalog_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBatchItem {
    selection: AiBatchSelection,
    eligibility: AiBatchEligibility,
    reason: Option<String>,
    stage: AiBatchStage,
    progress: u8,
    provider: Option<AiProvider>,
}
impl AiBatchItem {
    #[must_use]
    pub fn new(
        selection: AiBatchSelection,
        eligibility: AiBatchEligibility,
        reason: Option<String>,
    ) -> Self {
        Self {
            selection,
            eligibility,
            reason,
            stage: AiBatchStage::Queued,
            progress: 0,
            provider: None,
        }
    }
    #[must_use]
    pub const fn selection(&self) -> &AiBatchSelection {
        &self.selection
    }
    #[must_use]
    pub const fn eligibility(&self) -> AiBatchEligibility {
        self.eligibility
    }
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    #[must_use]
    pub const fn stage(&self) -> AiBatchStage {
        self.stage
    }
    #[must_use]
    pub const fn progress(&self) -> u8 {
        self.progress
    }
    #[must_use]
    pub const fn provider(&self) -> Option<AiProvider> {
        self.provider
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBatchReview {
    recipe: AiBatchRecipe,
    items: Vec<AiBatchItem>,
    policy: AiBatchEnqueuePolicy,
}
impl AiBatchReview {
    #[must_use]
    pub fn new(recipe: AiBatchRecipe, items: Vec<AiBatchItem>) -> Self {
        Self {
            recipe,
            items,
            policy: AiBatchEnqueuePolicy::ProcessEligible,
        }
    }
    #[must_use]
    pub const fn recipe(&self) -> &AiBatchRecipe {
        &self.recipe
    }
    #[must_use]
    pub fn items(&self) -> &[AiBatchItem] {
        &self.items
    }
    #[must_use]
    pub const fn policy(&self) -> AiBatchEnqueuePolicy {
        self.policy
    }
    pub fn set_policy(&mut self, policy: AiBatchEnqueuePolicy) {
        self.policy = policy;
    }
    #[must_use]
    pub fn eligible_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.eligibility() == AiBatchEligibility::Eligible)
            .count()
    }
    #[must_use]
    pub fn skipped_count(&self) -> usize {
        self.items.len().saturating_sub(self.eligible_count())
    }
    #[must_use]
    pub fn can_enqueue(&self) -> bool {
        self.eligible_count() > 0
            && (self.policy != AiBatchEnqueuePolicy::RequireAllEligible
                || self.skipped_count() == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiBatchPreflight {
    eligible: usize,
    skipped: usize,
    estimate_bytes: u64,
    estimate_memory_bytes: u64,
    warning: Option<String>,
}
impl AiBatchPreflight {
    #[must_use]
    pub const fn new(
        eligible: usize,
        skipped: usize,
        estimate_bytes: u64,
        estimate_memory_bytes: u64,
        warning: Option<String>,
    ) -> Self {
        Self {
            eligible,
            skipped,
            estimate_bytes,
            estimate_memory_bytes,
            warning,
        }
    }
    #[must_use]
    pub const fn eligible(&self) -> usize {
        self.eligible
    }
    #[must_use]
    pub const fn skipped(&self) -> usize {
        self.skipped
    }
    #[must_use]
    pub const fn estimate_bytes(&self) -> u64 {
        self.estimate_bytes
    }
    #[must_use]
    pub const fn estimate_memory_bytes(&self) -> u64 {
        self.estimate_memory_bytes
    }
    #[must_use]
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiBatchState {
    Empty,
    Reviewing(AiBatchReview),
    Preflight {
        review: AiBatchReview,
        summary: AiBatchPreflight,
    },
    Queued {
        batch_id: u64,
        summary: AiBatchPreflight,
    },
    Running {
        batch_id: u64,
        completed: usize,
        total: usize,
        stage: AiBatchStage,
    },
    Paused {
        batch_id: u64,
    },
    Recovering {
        batch_id: u64,
        failed: usize,
    },
    Complete {
        batch_id: u64,
    },
    Unavailable {
        detail: String,
    },
    Failed {
        detail: String,
    },
}
impl Default for AiBatchState {
    fn default() -> Self {
        Self::Empty
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiBatchAction {
    SelectTask(AiBatchTask),
    SelectModel(Option<ModelHash>),
    SelectProvider(Option<AiProvider>),
    SetStrength(u8),
    SetPolicy(AiBatchEnqueuePolicy),
    Review,
    Confirm,
    Pause,
    Resume,
    Cancel,
    RetryFailed,
    Reconcile,
    RemoveHistory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiBatchServiceError {
    Unavailable,
    InvalidRecipe(String),
    EmptySelection,
    PreflightRejected(String),
    Failed(String),
}
impl fmt::Display for AiBatchServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("AI batch service is unavailable"),
            Self::InvalidRecipe(detail)
            | Self::PreflightRejected(detail)
            | Self::Failed(detail) => formatter.write_str(detail),
            Self::EmptySelection => formatter.write_str("no photos are selected"),
        }
    }
}

pub trait AiBatchServicePort {
    fn review(
        &mut self,
        selection: &[AiBatchSelection],
        recipe: &AiBatchRecipe,
    ) -> Result<AiBatchReview, AiBatchServiceError>;
    fn preflight(
        &mut self,
        review: &AiBatchReview,
    ) -> Result<AiBatchPreflight, AiBatchServiceError>;
    fn enqueue(
        &mut self,
        review: &AiBatchReview,
        summary: &AiBatchPreflight,
    ) -> Result<u64, AiBatchServiceError>;
    fn pause(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
    fn resume(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
    fn cancel(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
    fn retry_failed(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
    fn reconcile(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
    fn remove_history(&mut self, batch_id: u64) -> Result<(), AiBatchServiceError>;
}
