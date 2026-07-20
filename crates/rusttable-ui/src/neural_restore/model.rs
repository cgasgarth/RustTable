//! Immutable preview state and ports for the GTK neural-restore surface.

#![allow(clippy::missing_errors_doc)]

use std::collections::BTreeMap;

use crate::ai_models::{AiProvider, ModelHash};
use rusttable_core::PhotoId;

pub const NEURAL_RESTORE_FOCUS_ORDER: [&str; 11] = [
    "neural-restore-task",
    "neural-restore-model",
    "neural-restore-provider",
    "neural-restore-strength",
    "neural-restore-wide-gamut",
    "neural-restore-scale",
    "neural-restore-comparison",
    "neural-restore-split",
    "neural-restore-cancel",
    "neural-restore-status",
    "neural-restore-preview",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoSourceKind {
    BayerRaw,
    XTransRaw,
    LinearRaw,
    Raster,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoSelection {
    photo: Option<PhotoId>,
    source_kind: Option<PhotoSourceKind>,
    renderable: bool,
    revision: u64,
    multiple: bool,
}

impl PhotoSelection {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            photo: None,
            source_kind: None,
            renderable: false,
            revision: 0,
            multiple: false,
        }
    }
    #[must_use]
    pub const fn single(
        photo: PhotoId,
        source_kind: PhotoSourceKind,
        renderable: bool,
        revision: u64,
    ) -> Self {
        Self {
            photo: Some(photo),
            source_kind: Some(source_kind),
            renderable,
            revision,
            multiple: false,
        }
    }
    #[must_use]
    pub const fn multiple() -> Self {
        Self {
            photo: None,
            source_kind: None,
            renderable: false,
            revision: 0,
            multiple: true,
        }
    }
    #[must_use]
    pub const fn photo(&self) -> Option<PhotoId> {
        self.photo
    }
    #[must_use]
    pub const fn source_kind(&self) -> Option<PhotoSourceKind> {
        self.source_kind
    }
    #[must_use]
    pub const fn renderable(&self) -> bool {
        self.renderable
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn is_multiple(&self) -> bool {
        self.multiple
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreTask {
    RawDenoise,
    RgbDenoise,
    Upscale,
}

impl RestoreTask {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RawDenoise => "RAW denoise",
            Self::RgbDenoise => "RGB denoise",
            Self::Upscale => "Upscale",
        }
    }
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::RawDenoise, Self::RgbDenoise, Self::Upscale]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonMode {
    Restored,
    Source,
    SideBySide,
    Split,
}

impl ComparisonMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Restored => "Restored",
            Self::Source => "Source / current edit",
            Self::SideBySide => "Side by side",
            Self::Split => "Draggable split",
        }
    }
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Restored, Self::Source, Self::SideBySide, Self::Split]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreSettings {
    raw_strength: u8,
    rgb_strength: u8,
    preserve_wide_gamut: bool,
    scale: u8,
}

impl Default for RestoreSettings {
    fn default() -> Self {
        Self {
            raw_strength: 50,
            rgb_strength: 50,
            preserve_wide_gamut: true,
            scale: 2,
        }
    }
}
impl RestoreSettings {
    #[must_use]
    pub const fn raw_strength(self) -> u8 {
        self.raw_strength
    }
    #[must_use]
    pub const fn rgb_strength(self) -> u8 {
        self.rgb_strength
    }
    #[must_use]
    pub const fn preserve_wide_gamut(self) -> bool {
        self.preserve_wide_gamut
    }
    #[must_use]
    pub const fn scale(self) -> u8 {
        self.scale
    }
    pub fn set_raw_strength(&mut self, value: u8) {
        self.raw_strength = value.min(100);
    }
    pub fn set_rgb_strength(&mut self, value: u8) {
        self.rgb_strength = value.min(100);
    }
    pub fn set_preserve_wide_gamut(&mut self, value: bool) {
        self.preserve_wide_gamut = value;
    }
    pub fn set_scale(&mut self, value: u8) {
        if matches!(value, 2 | 4) {
            self.scale = value;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportState {
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
    rotation: u16,
    focus_x: f32,
    focus_y: f32,
    split: f32,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            rotation: 0,
            focus_x: 0.5,
            focus_y: 0.5,
            split: 0.5,
        }
    }
}
impl ViewportState {
    #[must_use]
    pub const fn zoom(self) -> f32 {
        self.zoom
    }
    #[must_use]
    pub const fn pan(self) -> (f32, f32) {
        (self.pan_x, self.pan_y)
    }
    #[must_use]
    pub const fn rotation(self) -> u16 {
        self.rotation
    }
    #[must_use]
    pub const fn focus(self) -> (f32, f32) {
        (self.focus_x, self.focus_y)
    }
    #[must_use]
    pub const fn split(self) -> f32 {
        self.split
    }
    pub fn set_split(&mut self, value: f32) {
        self.split = value.clamp(0.0, 1.0);
    }
    pub fn adjust_split(&mut self, delta: f32) {
        self.set_split(self.split + delta);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PreviewCacheKey {
    photo: PhotoId,
    revision: u64,
    task: RestoreTaskKey,
    model: ModelHash,
    provider: AiProviderKey,
    settings: SettingsKey,
    roi: Roi,
    profile: String,
    implementation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum RestoreTaskKey {
    RawDenoise,
    RgbDenoise,
    Upscale,
}
impl From<RestoreTask> for RestoreTaskKey {
    fn from(value: RestoreTask) -> Self {
        match value {
            RestoreTask::RawDenoise => Self::RawDenoise,
            RestoreTask::RgbDenoise => Self::RgbDenoise,
            RestoreTask::Upscale => Self::Upscale,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AiProviderKey {
    Cpu,
    CoreMl,
    DirectMl,
    Cuda,
}
impl From<AiProvider> for AiProviderKey {
    fn from(value: AiProvider) -> Self {
        match value {
            AiProvider::Cpu => Self::Cpu,
            AiProvider::CoreMl => Self::CoreMl,
            AiProvider::DirectMl => Self::DirectMl,
            AiProvider::Cuda => Self::Cuda,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct SettingsKey(u8, u8, bool, u8);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Roi {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub scale: u8,
}

impl PreviewCacheKey {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        selection: &PhotoSelection,
        task: RestoreTask,
        model: ModelHash,
        provider: AiProvider,
        settings: RestoreSettings,
        roi: Roi,
        profile: impl Into<String>,
        implementation: impl Into<String>,
    ) -> Option<Self> {
        Some(Self {
            photo: selection.photo()?,
            revision: selection.revision(),
            task: task.into(),
            model,
            provider: provider.into(),
            settings: SettingsKey(
                settings.raw_strength(),
                settings.rgb_strength(),
                settings.preserve_wide_gamut(),
                settings.scale(),
            ),
            roi,
            profile: profile.into(),
            implementation: implementation.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl PreviewFrame {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, PreviewFrameError> {
        let expected = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|value| value.checked_mul(4))
            .ok_or(PreviewFrameError::Overflow)?;
        if width == 0 || height == 0 {
            return Err(PreviewFrameError::Empty);
        }
        if u64::try_from(pixels.len()).ok() != Some(expected) {
            return Err(PreviewFrameError::PixelCount);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFrameError {
    Empty,
    Overflow,
    PixelCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewArtifact {
    source: PreviewFrame,
    restored: PreviewFrame,
    output_width: u32,
    output_height: u32,
    provider: AiProvider,
    cache_hit: bool,
}
impl PreviewArtifact {
    #[must_use]
    pub fn new(
        source: PreviewFrame,
        restored: PreviewFrame,
        output_width: u32,
        output_height: u32,
        provider: AiProvider,
    ) -> Self {
        Self {
            source,
            restored,
            output_width,
            output_height,
            provider,
            cache_hit: false,
        }
    }
    #[must_use]
    pub const fn source(&self) -> &PreviewFrame {
        &self.source
    }
    #[must_use]
    pub const fn restored(&self) -> &PreviewFrame {
        &self.restored
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> (u32, u32) {
        (self.output_width, self.output_height)
    }
    #[must_use]
    pub const fn provider(&self) -> AiProvider {
        self.provider
    }
    #[must_use]
    pub const fn cache_hit(&self) -> bool {
        self.cache_hit
    }
    pub fn mark_cache_hit(&mut self) {
        self.cache_hit = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewEligibility {
    Eligible,
    NoSelection,
    MultipleSelection,
    UnsupportedSource,
    MissingModel,
    ProviderUnavailable,
    PreviewBudgetExceeded,
    ServiceUnavailable,
}
impl PreviewEligibility {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Eligible => "Ready for a single-photo preview",
            Self::NoSelection => "Select exactly one photo",
            Self::MultipleSelection => {
                "Neural restore preview accepts one photo; batch processing is separate"
            }
            Self::UnsupportedSource => "This source is not eligible for the selected restore task",
            Self::MissingModel => {
                "No enabled model is installed for this task; open AI Models settings"
            }
            Self::ProviderUnavailable => "No qualified provider is available for this model",
            Self::PreviewBudgetExceeded => "The visible region exceeds the preview budget",
            Self::ServiceUnavailable => "Preview service unavailable; no inference was started",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStage {
    Debouncing,
    Planning,
    Inference,
    Presentation,
}
impl PreviewStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debouncing => "Waiting for preview changes",
            Self::Planning => "Planning visible region",
            Self::Inference => "Running bounded preview inference",
            Self::Presentation => "Presenting preview",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewFailure {
    Cancelled,
    StaleGeneration,
    ServiceUnavailable,
    Ineligible(PreviewEligibility),
    Inference,
    Presentation,
    ColorConversion,
    ResourceBudget,
}
impl PreviewFailure {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Cancelled => "Preview cancelled",
            Self::StaleGeneration => "Preview result discarded because a newer request exists",
            Self::ServiceUnavailable => "Preview service unavailable; no inference was started",
            Self::Ineligible(reason) => reason.label(),
            Self::Inference => "Preview inference failed; the source and catalog are unchanged",
            Self::Presentation => "Preview could not be presented",
            Self::ColorConversion => "Preview color presentation failed",
            Self::ResourceBudget => "Preview resource budget exceeded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewRequest {
    pub(crate) generation: u64,
    pub(crate) key: PreviewCacheKey,
    pub(crate) task: RestoreTask,
    pub(crate) model: ModelHash,
    pub(crate) provider: AiProvider,
    pub(crate) settings: RestoreSettings,
    pub(crate) roi: Roi,
}
impl PreviewRequest {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn key(&self) -> &PreviewCacheKey {
        &self.key
    }
    #[must_use]
    pub const fn task(&self) -> RestoreTask {
        self.task
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
    pub const fn settings(&self) -> RestoreSettings {
        self.settings
    }
    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeuralRestoreSnapshot {
    selection: PhotoSelection,
    eligibility: Vec<(RestoreTask, PreviewEligibility)>,
    models: Vec<(RestoreTask, ModelHash, String)>,
    providers: Vec<AiProvider>,
    announcement: String,
}
impl NeuralRestoreSnapshot {
    #[must_use]
    pub fn unavailable(selection: PhotoSelection) -> Self {
        Self {
            selection,
            eligibility: RestoreTask::all()
                .into_iter()
                .map(|task| (task, PreviewEligibility::ServiceUnavailable))
                .collect(),
            models: Vec::new(),
            providers: Vec::new(),
            announcement: "Neural restore preview service is not connected (#498/#499/#501/#502)."
                .to_owned(),
        }
    }
    #[must_use]
    pub const fn selection(&self) -> &PhotoSelection {
        &self.selection
    }
    #[must_use]
    pub fn eligibility(&self, task: RestoreTask) -> PreviewEligibility {
        self.eligibility
            .iter()
            .find(|entry| entry.0 == task)
            .map_or(PreviewEligibility::ServiceUnavailable, |entry| entry.1)
    }
    pub fn models(&self, task: RestoreTask) -> impl Iterator<Item = (&ModelHash, &str)> {
        self.models
            .iter()
            .filter(move |entry| entry.0 == task)
            .map(|entry| (&entry.1, entry.2.as_str()))
    }
    #[must_use]
    pub fn providers(&self) -> &[AiProvider] {
        &self.providers
    }
    #[must_use]
    pub fn announcement(&self) -> &str {
        &self.announcement
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewServiceError {
    Unavailable,
    Ineligible(PreviewEligibility),
    Cancelled,
    Failed,
}
pub trait NeuralRestorePreviewPort {
    fn snapshot(
        &mut self,
        selection: &PhotoSelection,
    ) -> Result<NeuralRestoreSnapshot, PreviewServiceError>;
    fn request_preview(&mut self, request: &PreviewRequest) -> Result<u64, PreviewServiceError>;
    fn cancel_preview(&mut self, job: u64) -> Result<(), PreviewServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NeuralRestoreAction {
    SetSelection(PhotoSelection),
    SelectTask(RestoreTask),
    SelectModel(Option<ModelHash>),
    SelectProvider(AiProvider),
    SetRawStrength(u8),
    SetRgbStrength(u8),
    SetWideGamut(bool),
    SetScale(u8),
    SetComparison(ComparisonMode),
    AdjustSplit(i8),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewStatus {
    Idle,
    Debouncing,
    Running(PreviewStage),
    Ready,
    CacheHit,
    Failed(PreviewFailure),
    Ineligible(PreviewEligibility),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NeuralRestoreViewModel {
    pub(crate) snapshot: NeuralRestoreSnapshot,
    pub(crate) task: RestoreTask,
    pub(crate) model: Option<ModelHash>,
    pub(crate) provider: Option<AiProvider>,
    pub(crate) settings: RestoreSettings,
    pub(crate) comparison: ComparisonMode,
    pub(crate) viewport: ViewportState,
    pub(crate) generation: u64,
    pub(crate) status: PreviewStatus,
    pub(crate) artifact: Option<PreviewArtifact>,
    pub(crate) cache_size: usize,
    pub(crate) announcement: String,
}
impl Default for NeuralRestoreViewModel {
    fn default() -> Self {
        Self::unavailable()
    }
}
impl NeuralRestoreViewModel {
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            snapshot: NeuralRestoreSnapshot::unavailable(PhotoSelection::none()),
            task: RestoreTask::RgbDenoise,
            model: None,
            provider: None,
            settings: RestoreSettings::default(),
            comparison: ComparisonMode::Restored,
            viewport: ViewportState::default(),
            generation: 0,
            status: PreviewStatus::Ineligible(PreviewEligibility::ServiceUnavailable),
            artifact: None,
            cache_size: 0,
            announcement: "Neural restore preview service unavailable".to_owned(),
        }
    }
    #[must_use]
    pub const fn snapshot(&self) -> &NeuralRestoreSnapshot {
        &self.snapshot
    }
    #[must_use]
    pub const fn task(&self) -> RestoreTask {
        self.task
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
    pub const fn settings(&self) -> RestoreSettings {
        self.settings
    }
    #[must_use]
    pub const fn comparison(&self) -> ComparisonMode {
        self.comparison
    }
    #[must_use]
    pub const fn viewport(&self) -> ViewportState {
        self.viewport
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn status(&self) -> &PreviewStatus {
        &self.status
    }
    #[must_use]
    pub const fn artifact(&self) -> Option<&PreviewArtifact> {
        self.artifact.as_ref()
    }
    #[must_use]
    pub const fn cache_size(&self) -> usize {
        self.cache_size
    }
    #[must_use]
    pub fn announcement(&self) -> &str {
        &self.announcement
    }
    pub(crate) fn invalidate(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.artifact = None;
        self.status = PreviewStatus::Debouncing;
    }
}

#[derive(Debug)]
pub struct PreviewCache {
    entries: BTreeMap<PreviewCacheKey, PreviewArtifact>,
    capacity: usize,
}
impl PreviewCache {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity: capacity.max(1),
        }
    }
    #[must_use]
    pub fn get(&self, key: &PreviewCacheKey) -> Option<PreviewArtifact> {
        self.entries.get(key).cloned().map(|mut artifact| {
            artifact.mark_cache_hit();
            artifact
        })
    }
    pub fn insert(&mut self, key: PreviewCacheKey, artifact: PreviewArtifact) {
        if self.entries.len() >= self.capacity
            && let Some(key) = self.entries.keys().next().cloned()
        {
            self.entries.remove(&key);
        }
        self.entries.insert(key, artifact);
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> ModelHash {
        ModelHash::new("a".repeat(64)).expect("hash")
    }
    #[test]
    fn selection_and_eligibility_keep_multiple_photos_out_of_preview() {
        assert!(PhotoSelection::multiple().is_multiple());
        assert_eq!(
            PreviewEligibility::MultipleSelection.label(),
            "Neural restore preview accepts one photo; batch processing is separate"
        );
    }
    #[test]
    fn split_handle_is_keyboard_adjustable_and_bounded() {
        let mut viewport = ViewportState::default();
        viewport.adjust_split(-2.0);
        assert!(viewport.split().abs() < f32::EPSILON);
        viewport.adjust_split(3.0);
        assert!((viewport.split() - 1.0).abs() < f32::EPSILON);
    }
    #[test]
    fn preview_cache_is_bounded() {
        let photo = PhotoSelection::single(
            PhotoId::new(1).expect("photo"),
            PhotoSourceKind::Raster,
            true,
            1,
        );
        let key = PreviewCacheKey::new(
            &photo,
            RestoreTask::RgbDenoise,
            hash(),
            AiProvider::Cpu,
            RestoreSettings::default(),
            Roi {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                scale: 1,
            },
            "sRGB",
            "test",
        )
        .expect("key");
        let frame = PreviewFrame::new(1, 1, vec![0; 4]).expect("frame");
        let mut cache = PreviewCache::new(1);
        cache.insert(
            key,
            PreviewArtifact::new(frame.clone(), frame, 1, 1, AiProvider::Cpu),
        );
        assert_eq!(cache.len(), 1);
    }
}
