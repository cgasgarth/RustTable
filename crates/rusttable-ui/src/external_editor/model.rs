//! Typed state and service contracts for the external-editor workflow.
//!
//! This module intentionally contains no process, filesystem, encoding, or catalog code.  The
//! GTK surface edits these bounded values and hands commands to an application-owned port.

#![allow(clippy::missing_errors_doc)]

use std::fmt;

use rusttable_core::{PhotoId, Revision};

use crate::presentation::PresentationText;

pub const MAX_ARGUMENT_ROWS: usize = 64;
pub const MAX_PRESETS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresetId(u128);

impl PresetId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JobId(u128);

impl JobId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    OwnedChildUntilExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterchangeMode {
    InPlaceTiff,
    SeparateOutputTiff,
}

impl InterchangeMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::InPlaceTiff => "In-place TIFF",
            Self::SeparateOutputTiff => "Separate output TIFF",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiffBitDepth {
    Sixteen,
    Float32,
}

impl TiffBitDepth {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sixteen => "16-bit",
            Self::Float32 => "32-bit float",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    Preserve,
    Minimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentRow {
    Literal(PresentationText),
    Placeholder(Placeholder),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    Input,
    Output,
    Xmp,
}

impl Placeholder {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Input => "{input}",
            Self::Output => "{output}",
            Self::Xmp => "{xmp}",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgumentRowError {
    Empty,
    ShellSyntax,
}

impl ArgumentRow {
    /// Creates a literal argv item, rejecting shell syntax and control text.
    ///
    /// The value is still one argument.  It is never parsed as a command line.
    pub fn literal(value: impl Into<String>) -> Result<Self, ArgumentRowError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ArgumentRowError::Empty);
        }
        if value.chars().any(is_forbidden_shell_character) {
            return Err(ArgumentRowError::ShellSyntax);
        }
        PresentationText::new(value)
            .map(Self::Literal)
            .map_err(|_| ArgumentRowError::ShellSyntax)
    }

    #[must_use]
    pub const fn placeholder(value: Placeholder) -> Self {
        Self::Placeholder(value)
    }

    #[must_use]
    pub fn display_token(&self) -> &str {
        match self {
            Self::Literal(value) => value.as_str(),
            Self::Placeholder(value) => value.token(),
        }
    }
}

fn is_forbidden_shell_character(value: char) -> bool {
    matches!(
        value,
        '\0' | '\n' | '\r' | '\'' | '"' | '`' | '\\' | '$' | '|' | '>' | '<' | '&' | ';'
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableApproval {
    Current,
    NeedsReapproval,
    Missing,
}

impl ExecutableApproval {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Current => "approved executable",
            Self::NeedsReapproval => "executable changed; re-approve",
            Self::Missing => "executable unavailable",
        }
    }

    #[must_use]
    pub const fn launchable(self) -> bool {
        matches!(self, Self::Current)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableIdentity {
    stored_alias: PresentationText,
    current_alias: Option<PresentationText>,
    approval: ExecutableApproval,
}

impl ExecutableIdentity {
    /// Stores only privacy-safe aliases; the service owns the native executable identity.
    #[must_use]
    pub fn new(
        stored_alias: PresentationText,
        current_alias: Option<PresentationText>,
        approval: ExecutableApproval,
    ) -> Self {
        Self {
            stored_alias,
            current_alias,
            approval,
        }
    }

    #[must_use]
    pub const fn stored_alias(&self) -> &PresentationText {
        &self.stored_alias
    }

    #[must_use]
    pub const fn current_alias(&self) -> Option<&PresentationText> {
        self.current_alias.as_ref()
    }

    #[must_use]
    pub const fn approval(&self) -> ExecutableApproval {
        self.approval
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorPreset {
    id: PresetId,
    revision: Revision,
    name: PresentationText,
    executable: ExecutableIdentity,
    wait_mode: WaitMode,
    interchange: InterchangeMode,
    arguments: Vec<ArgumentRow>,
    include_xmp: bool,
    bit_depth: TiffBitDepth,
    profile: PresentationText,
    metadata: MetadataPolicy,
    add_to_catalog: bool,
    group_with_source: bool,
    destination: PresentationText,
    enabled: bool,
    qualification: QualificationState,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorDraft {
    pub id: Option<PresetId>,
    pub revision: Revision,
    pub name: PresentationText,
    pub executable: Option<ExecutableIdentity>,
    pub interchange: InterchangeMode,
    pub arguments: Vec<ArgumentRow>,
    pub include_xmp: bool,
    pub bit_depth: TiffBitDepth,
    pub profile: PresentationText,
    pub metadata: MetadataPolicy,
    pub add_to_catalog: bool,
    pub group_with_source: bool,
    pub destination: PresentationText,
    pub enabled: bool,
}

impl ExternalEditorPreset {
    /// Creates a validated preset snapshot suitable for a list or invocation review.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty/oversized list or a preset with no executable arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PresetId,
        revision: Revision,
        name: PresentationText,
        executable: ExecutableIdentity,
        interchange: InterchangeMode,
        arguments: Vec<ArgumentRow>,
        profile: PresentationText,
        destination: PresentationText,
    ) -> Result<Self, PresetValidationError> {
        if arguments.is_empty() {
            return Err(PresetValidationError::NoArguments);
        }
        if arguments.len() > MAX_ARGUMENT_ROWS {
            return Err(PresetValidationError::TooManyArguments);
        }
        Ok(Self {
            id,
            revision,
            name,
            executable,
            wait_mode: WaitMode::OwnedChildUntilExit,
            interchange,
            arguments,
            include_xmp: false,
            bit_depth: TiffBitDepth::Sixteen,
            profile,
            metadata: MetadataPolicy::Preserve,
            add_to_catalog: true,
            group_with_source: true,
            destination,
            enabled: true,
            qualification: QualificationState::Unqualified,
        })
    }

    #[must_use]
    pub fn draft(&self) -> ExternalEditorDraft {
        ExternalEditorDraft {
            id: Some(self.id),
            revision: self.revision,
            name: self.name.clone(),
            executable: Some(self.executable.clone()),
            interchange: self.interchange,
            arguments: self.arguments.clone(),
            include_xmp: self.include_xmp,
            bit_depth: self.bit_depth,
            profile: self.profile.clone(),
            metadata: self.metadata,
            add_to_catalog: self.add_to_catalog,
            group_with_source: self.group_with_source,
            destination: self.destination.clone(),
            enabled: self.enabled,
        }
    }

    #[must_use]
    pub const fn id(&self) -> PresetId {
        self.id
    }
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }
    #[must_use]
    pub const fn name(&self) -> &PresentationText {
        &self.name
    }
    #[must_use]
    pub const fn executable(&self) -> &ExecutableIdentity {
        &self.executable
    }
    #[must_use]
    pub const fn wait_mode(&self) -> WaitMode {
        self.wait_mode
    }
    #[must_use]
    pub const fn interchange(&self) -> InterchangeMode {
        self.interchange
    }
    #[must_use]
    pub fn arguments(&self) -> &[ArgumentRow] {
        &self.arguments
    }
    #[must_use]
    pub const fn include_xmp(&self) -> bool {
        self.include_xmp
    }
    #[must_use]
    pub const fn bit_depth(&self) -> TiffBitDepth {
        self.bit_depth
    }
    #[must_use]
    pub const fn profile(&self) -> &PresentationText {
        &self.profile
    }
    #[must_use]
    pub const fn metadata(&self) -> MetadataPolicy {
        self.metadata
    }
    #[must_use]
    pub const fn add_to_catalog(&self) -> bool {
        self.add_to_catalog
    }
    #[must_use]
    pub const fn group_with_source(&self) -> bool {
        self.group_with_source
    }
    #[must_use]
    pub const fn destination(&self) -> &PresentationText {
        &self.destination
    }
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub const fn qualification(&self) -> QualificationState {
        self.qualification
    }

    #[must_use]
    pub fn launchability(&self, selection_count: usize) -> Launchability {
        if selection_count == 0 {
            return Launchability::NoSelection;
        }
        if !self.enabled {
            return Launchability::Disabled;
        }
        if !self.executable.approval.launchable() {
            return Launchability::ExecutableNeedsReapproval;
        }
        if !matches!(self.qualification, QualificationState::Qualified { .. }) {
            return Launchability::PresetNotQualified;
        }
        Launchability::Ready
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetValidationError {
    NoArguments,
    TooManyArguments,
}

impl fmt::Display for PresetValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoArguments => "a preset needs at least one argument row",
            Self::TooManyArguments => "the preset has too many argument rows",
        })
    }
}

impl std::error::Error for PresetValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualificationState {
    Unqualified,
    Qualified { revision: Revision },
    Failed,
    Drifted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launchability {
    Ready,
    NoSelection,
    Disabled,
    ExecutableNeedsReapproval,
    PresetNotQualified,
}

impl Launchability {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Ready => "Ready to send",
            Self::NoSelection => "Select one or more photos",
            Self::Disabled => "Preset is disabled",
            Self::ExecutableNeedsReapproval => "Re-approve the executable before sending",
            Self::PresetNotQualified => "Test this preset before sending user photos",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStage {
    Rendering,
    Staged,
    ExternalAppRunning,
    Validating,
    Publishing,
    Importing,
    Grouping,
    Reconciling,
    Completed,
    Cancelled,
    Failed,
}

impl JobStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rendering => "Rendering",
            Self::Staged => "Staged",
            Self::ExternalAppRunning => "External app running — save and close it to continue",
            Self::Validating => "Validating",
            Self::Publishing => "Publishing",
            Self::Importing => "Importing",
            Self::Grouping => "Grouping",
            Self::Reconciling => "Reconciling",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorJob {
    id: JobId,
    photo_id: PhotoId,
    stage: JobStage,
    progress_percent: u8,
    detail: PresentationText,
}

impl ExternalEditorJob {
    #[must_use]
    pub fn new(id: JobId, photo_id: PhotoId, stage: JobStage, detail: PresentationText) -> Self {
        Self {
            id,
            photo_id,
            stage,
            progress_percent: 0,
            detail,
        }
    }

    #[must_use]
    pub const fn id(&self) -> JobId {
        self.id
    }
    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn stage(&self) -> JobStage {
        self.stage
    }
    #[must_use]
    pub const fn progress_percent(&self) -> u8 {
        self.progress_percent
    }
    #[must_use]
    pub const fn detail(&self) -> &PresentationText {
        &self.detail
    }

    #[must_use]
    pub fn with_progress(mut self, progress_percent: u8) -> Self {
        self.progress_percent = progress_percent.min(100);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationReview {
    preset: PresetId,
    photos: Vec<PhotoId>,
    source_revision: Revision,
    interchange: InterchangeMode,
    bit_depth: TiffBitDepth,
    profile: PresentationText,
    destination: PresentationText,
    metadata: MetadataPolicy,
    add_to_catalog: bool,
    group_with_source: bool,
}

impl InvocationReview {
    #[must_use]
    pub fn new(
        preset: &ExternalEditorPreset,
        photos: Vec<PhotoId>,
        source_revision: Revision,
    ) -> Self {
        Self {
            preset: preset.id,
            photos,
            source_revision,
            interchange: preset.interchange,
            bit_depth: preset.bit_depth,
            profile: preset.profile.clone(),
            destination: preset.destination.clone(),
            metadata: preset.metadata,
            add_to_catalog: preset.add_to_catalog,
            group_with_source: preset.group_with_source,
        }
    }

    #[must_use]
    pub const fn preset(&self) -> PresetId {
        self.preset
    }
    #[must_use]
    pub fn photos(&self) -> &[PhotoId] {
        &self.photos
    }
    #[must_use]
    pub const fn source_revision(&self) -> Revision {
        self.source_revision
    }
    #[must_use]
    pub const fn interchange(&self) -> InterchangeMode {
        self.interchange
    }
    #[must_use]
    pub const fn bit_depth(&self) -> TiffBitDepth {
        self.bit_depth
    }
    #[must_use]
    pub const fn profile(&self) -> &PresentationText {
        &self.profile
    }
    #[must_use]
    pub const fn destination(&self) -> &PresentationText {
        &self.destination
    }
    #[must_use]
    pub const fn metadata(&self) -> MetadataPolicy {
        self.metadata
    }
    #[must_use]
    pub const fn add_to_catalog(&self) -> bool {
        self.add_to_catalog
    }
    #[must_use]
    pub const fn group_with_source(&self) -> bool {
        self.group_with_source
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualificationReceipt {
    preset: PresetId,
    revision: Revision,
    outcome: QualificationOutcome,
    detail: PresentationText,
}

impl QualificationReceipt {
    #[must_use]
    pub const fn new(
        preset: PresetId,
        revision: Revision,
        outcome: QualificationOutcome,
        detail: PresentationText,
    ) -> Self {
        Self {
            preset,
            revision,
            outcome,
            detail,
        }
    }

    #[must_use]
    pub const fn preset(&self) -> PresetId {
        self.preset
    }
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }
    #[must_use]
    pub const fn outcome(&self) -> QualificationOutcome {
        self.outcome
    }
    #[must_use]
    pub const fn detail(&self) -> &PresentationText {
        &self.detail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualificationOutcome {
    Qualified,
    Failed,
    Drifted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionAction {
    OpenDerivedPhoto,
    RevealOutput,
    ViewReceipt,
    RetryReconciliation,
    CleanStaging,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEditorViewModel {
    presets: Vec<ExternalEditorPreset>,
    selected_preset: Option<PresetId>,
    selected_photos: Vec<PhotoId>,
    source_revision: Revision,
    review: Option<InvocationReview>,
    jobs: Vec<ExternalEditorJob>,
    announcement: PresentationText,
}

impl Default for ExternalEditorViewModel {
    fn default() -> Self {
        Self {
            presets: Vec::new(),
            selected_preset: None,
            selected_photos: Vec::new(),
            source_revision: Revision::ZERO,
            review: None,
            jobs: Vec::new(),
            announcement: PresentationText::new("No external-editor preset selected")
                .expect("static UI text is valid"),
        }
    }
}

impl ExternalEditorViewModel {
    #[must_use]
    pub fn new(presets: Vec<ExternalEditorPreset>) -> Self {
        let selected_preset = presets.first().map(ExternalEditorPreset::id);
        Self {
            presets,
            selected_preset,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn presets(&self) -> &[ExternalEditorPreset] {
        &self.presets
    }
    #[must_use]
    pub const fn selected_preset(&self) -> Option<PresetId> {
        self.selected_preset
    }
    #[must_use]
    pub fn selected_photos(&self) -> &[PhotoId] {
        &self.selected_photos
    }
    #[must_use]
    pub const fn source_revision(&self) -> Revision {
        self.source_revision
    }
    #[must_use]
    pub const fn review(&self) -> Option<&InvocationReview> {
        self.review.as_ref()
    }
    #[must_use]
    pub fn jobs(&self) -> &[ExternalEditorJob] {
        &self.jobs
    }
    #[must_use]
    pub const fn announcement(&self) -> &PresentationText {
        &self.announcement
    }

    pub fn set_selection(&mut self, photos: Vec<PhotoId>, source_revision: Revision) {
        self.selected_photos = photos;
        self.source_revision = source_revision;
        self.review = None;
    }

    pub fn select_preset(&mut self, preset: Option<PresetId>) {
        self.selected_preset = preset;
        self.review = None;
    }

    pub fn set_review(&mut self, review: Option<InvocationReview>) {
        self.review = review;
    }

    pub fn replace_presets(&mut self, presets: Vec<ExternalEditorPreset>) {
        self.presets = presets.into_iter().take(MAX_PRESETS).collect();
        if self
            .selected_preset
            .is_none_or(|id| !self.presets.iter().any(|preset| preset.id == id))
        {
            self.selected_preset = self.presets.first().map(ExternalEditorPreset::id);
        }
    }

    pub fn upsert_preset(&mut self, preset: ExternalEditorPreset) {
        if let Some(existing) = self
            .presets
            .iter_mut()
            .find(|existing| existing.id == preset.id)
        {
            *existing = preset;
        } else if self.presets.len() < MAX_PRESETS {
            self.selected_preset.get_or_insert(preset.id);
            self.presets.push(preset);
        }
    }

    pub fn apply_receipt(&mut self, receipt: &QualificationReceipt) {
        if let Some(preset) = self
            .presets
            .iter_mut()
            .find(|preset| preset.id == receipt.preset)
        {
            preset.qualification = match receipt.outcome {
                QualificationOutcome::Qualified => QualificationState::Qualified {
                    revision: receipt.revision,
                },
                QualificationOutcome::Failed => QualificationState::Failed,
                QualificationOutcome::Drifted => QualificationState::Drifted,
            };
        }
        self.announcement = receipt.detail.clone();
    }

    pub fn apply_job(&mut self, job: ExternalEditorJob) {
        if let Some(existing) = self.jobs.iter_mut().find(|existing| existing.id == job.id) {
            *existing = job;
        } else if self.jobs.len() < MAX_PRESETS {
            self.jobs.push(job);
        }
    }

    pub fn announce(&mut self, text: PresentationText) {
        self.announcement = text;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendToEditorRequest {
    pub preset: PresetId,
    pub photos: Vec<PhotoId>,
    pub source_revision: Revision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEditorAction {
    NewPreset,
    SaveDraft(ExternalEditorDraft),
    SelectPreset(Option<PresetId>),
    ChooseExecutable,
    AddLiteralArgument,
    AddPlaceholderArgument(Placeholder),
    TestPreset(PresetId),
    ReviewSend,
    ConfirmSend(SendToEditorRequest),
    CancelJob(JobId),
    ReconcileJob(JobId),
    Complete(JobId, CompletionAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEditorServiceError {
    Unavailable,
    InvalidRequest,
    StaleRevision,
    NotFound,
    Failed,
}

impl fmt::Display for ExternalEditorServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "external-editor service unavailable",
            Self::InvalidRequest => "external-editor request is invalid",
            Self::StaleRevision => "the selected edit is stale",
            Self::NotFound => "external-editor job or preset was not found",
            Self::Failed => "external-editor service failed",
        })
    }
}

impl std::error::Error for ExternalEditorServiceError {}

/// Application-owned service port.  Implementations own all native/process/storage work.
pub trait ExternalEditorServicePort {
    fn list_presets(&mut self) -> Result<Vec<ExternalEditorPreset>, ExternalEditorServiceError>;
    fn save_preset(
        &mut self,
        draft: ExternalEditorDraft,
    ) -> Result<ExternalEditorPreset, ExternalEditorServiceError>;
    fn test_preset(
        &mut self,
        preset: PresetId,
    ) -> Result<QualificationReceipt, ExternalEditorServiceError>;
    fn send_to_editor(
        &mut self,
        request: SendToEditorRequest,
    ) -> Result<Vec<ExternalEditorJob>, ExternalEditorServiceError>;
    fn cancel_job(&mut self, job: JobId) -> Result<(), ExternalEditorServiceError>;
    fn reconcile_job(
        &mut self,
        job: JobId,
    ) -> Result<ExternalEditorJob, ExternalEditorServiceError>;
    fn complete(
        &mut self,
        job: JobId,
        action: CompletionAction,
    ) -> Result<(), ExternalEditorServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("valid test text")
    }
    fn preset() -> ExternalEditorPreset {
        ExternalEditorPreset::new(
            PresetId::new(1).expect("nonzero"),
            Revision::from_u64(3),
            text("Editor"),
            ExecutableIdentity::new(
                text("editor"),
                Some(text("editor")),
                ExecutableApproval::Current,
            ),
            InterchangeMode::SeparateOutputTiff,
            vec![ArgumentRow::placeholder(Placeholder::Input)],
            text("sRGB"),
            text("edited-{input}.tiff"),
        )
        .expect("valid preset")
    }

    #[test]
    fn argument_rows_reject_shell_syntax_and_keep_placeholders_typed() {
        assert_eq!(
            ArgumentRow::literal("--output"),
            Ok(ArgumentRow::Literal(text("--output")))
        );
        assert_eq!(
            ArgumentRow::literal("editor | rm -rf"),
            Err(ArgumentRowError::ShellSyntax)
        );
        assert_eq!(
            ArgumentRow::placeholder(Placeholder::Output).display_token(),
            "{output}"
        );
    }

    #[test]
    fn unqualified_or_drifted_presets_cannot_launch() {
        let preset = preset();
        assert_eq!(preset.launchability(1), Launchability::PresetNotQualified);
        assert_eq!(preset.launchability(0), Launchability::NoSelection);
    }

    #[test]
    fn receipts_update_only_the_matching_preset_revision() {
        let mut model = ExternalEditorViewModel::new(vec![preset()]);
        let receipt = QualificationReceipt {
            preset: PresetId::new(1).expect("nonzero"),
            revision: Revision::from_u64(3),
            outcome: QualificationOutcome::Qualified,
            detail: text("Preset qualified"),
        };
        model.apply_receipt(&receipt);
        assert!(matches!(
            model.presets()[0].qualification(),
            QualificationState::Qualified { .. }
        ));
        assert_eq!(model.announcement().as_str(), "Preset qualified");
    }

    #[test]
    fn progress_is_bounded_and_job_reconciliation_replaces_by_id() {
        let job_id = JobId::new(7).expect("nonzero");
        let photo_id = PhotoId::new(9).expect("nonzero");
        let mut model = ExternalEditorViewModel::default();
        model.apply_job(ExternalEditorJob::new(
            job_id,
            photo_id,
            JobStage::Staged,
            text("staged"),
        ));
        model.apply_job(
            ExternalEditorJob::new(job_id, photo_id, JobStage::Completed, text("done"))
                .with_progress(255),
        );
        assert_eq!(model.jobs().len(), 1);
        assert_eq!(model.jobs()[0].progress_percent(), 100);
        assert_eq!(model.jobs()[0].stage(), JobStage::Completed);
    }

    #[test]
    fn external_editor_ui_state_fixture_has_safe_defaults() {
        let model = ExternalEditorViewModel::default();
        assert!(model.presets().is_empty());
        assert!(model.selected_photos().is_empty());
        assert!(model.announcement().as_str().contains("preset"));
    }
}
