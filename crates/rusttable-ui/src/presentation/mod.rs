use std::collections::{BTreeMap, BTreeSet};

use rusttable_core::PhotoId;

pub mod basic_edit;

pub use basic_edit::{
    BasicEditField, BasicEditInspectorViewModel, BasicEditSaveState, BasicEditValues,
};

const MAX_PRESENTATION_TEXT_BYTES: usize = 256;
const MAX_SELECTED_PREVIEW_RGBA8_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresentationText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationTextError {
    Empty,
    WhitespaceOnly,
    AsciiControlCharacter { byte_index: usize, value: char },
    TooLong { byte_length: usize },
}

impl PresentationText {
    /// Creates bounded, display-safe presentation text.
    ///
    /// # Errors
    ///
    /// Returns a stable error when the value is empty, whitespace-only, contains an ASCII control
    /// character, or exceeds the presentation byte limit.
    pub fn new(value: impl Into<String>) -> Result<Self, PresentationTextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PresentationTextError::Empty);
        }
        if value.trim().is_empty() {
            return Err(PresentationTextError::WhitespaceOnly);
        }
        if value.len() > MAX_PRESENTATION_TEXT_BYTES {
            return Err(PresentationTextError::TooLong {
                byte_length: value.len(),
            });
        }
        if let Some((byte_index, value)) = value
            .char_indices()
            .find(|(_, value)| value.is_ascii_control())
        {
            return Err(PresentationTextError::AsciiControlCharacter { byte_index, value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoCardViewModel {
    id: PhotoId,
    title: PresentationText,
    secondary: Option<PresentationText>,
}

impl PhotoCardViewModel {
    #[must_use]
    pub fn new(id: PhotoId, title: PresentationText, secondary: Option<PresentationText>) -> Self {
        Self {
            id,
            title,
            secondary,
        }
    }

    #[must_use]
    pub fn id(&self) -> PhotoId {
        self.id
    }

    #[must_use]
    pub fn title(&self) -> &PresentationText {
        &self.title
    }

    #[must_use]
    pub fn secondary(&self) -> Option<&PresentationText> {
        self.secondary.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoFactViewModel {
    label: PresentationText,
    value: PresentationText,
}

impl PhotoFactViewModel {
    #[must_use]
    pub fn new(label: PresentationText, value: PresentationText) -> Self {
        Self { label, value }
    }

    #[must_use]
    pub fn label(&self) -> &PresentationText {
        &self.label
    }

    #[must_use]
    pub fn value(&self) -> &PresentationText {
        &self.value
    }
}

/// Dimensions for a decoded preview image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewDimensions {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewDimensionsError {
    ZeroWidth,
    ZeroHeight,
}

impl PreviewDimensions {
    /// Creates non-zero image dimensions.
    ///
    /// # Errors
    ///
    /// Returns a stable error when either image dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, PreviewDimensionsError> {
        if width == 0 {
            return Err(PreviewDimensionsError::ZeroWidth);
        }
        if height == 0 {
            return Err(PreviewDimensionsError::ZeroHeight);
        }

        Ok(Self { width, height })
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }
}

/// A validation error for ready eight-bit RGBA preview data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rgba8PreviewMetadataError {
    ByteLengthOverflow,
    TooLarge { byte_length: usize },
    IncorrectByteLength { expected: usize, actual: usize },
}

/// Validated presentation data for a ready, eight-bit RGBA selected preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8PreviewMetadata {
    dimensions: PreviewDimensions,
    status: PresentationText,
    pixels: Vec<u8>,
}

impl Rgba8PreviewMetadata {
    /// Creates a validated, bounded RGBA8 preview.
    ///
    /// # Errors
    ///
    /// Returns a stable error when the pixel data cannot represent `width * height * 4` bytes,
    /// overflows platform address space, or exceeds the presentation boundary's byte limit.
    pub fn new(
        dimensions: PreviewDimensions,
        status: PresentationText,
        pixels: Vec<u8>,
    ) -> Result<Self, Rgba8PreviewMetadataError> {
        let expected = rgba8_byte_len(dimensions)?;
        if expected > MAX_SELECTED_PREVIEW_RGBA8_BYTES {
            return Err(Rgba8PreviewMetadataError::TooLarge {
                byte_length: expected,
            });
        }
        if pixels.len() > MAX_SELECTED_PREVIEW_RGBA8_BYTES {
            return Err(Rgba8PreviewMetadataError::TooLarge {
                byte_length: pixels.len(),
            });
        }
        if pixels.len() != expected {
            return Err(Rgba8PreviewMetadataError::IncorrectByteLength {
                expected,
                actual: pixels.len(),
            });
        }

        Ok(Self {
            dimensions,
            status,
            pixels,
        })
    }

    #[must_use]
    pub fn dimensions(&self) -> PreviewDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn status(&self) -> &PresentationText {
        &self.status
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

fn rgba8_byte_len(dimensions: PreviewDimensions) -> Result<usize, Rgba8PreviewMetadataError> {
    let width = usize::try_from(dimensions.width())
        .map_err(|_| Rgba8PreviewMetadataError::ByteLengthOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| Rgba8PreviewMetadataError::ByteLengthOverflow)?;

    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(Rgba8PreviewMetadataError::ByteLengthOverflow)
}

/// A display-safe explanation for a failed selected preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedPreviewFailure {
    detail: PresentationText,
}

impl SelectedPreviewFailure {
    #[must_use]
    pub fn new(detail: PresentationText) -> Self {
        Self { detail }
    }

    #[must_use]
    pub fn detail(&self) -> &PresentationText {
        &self.detail
    }
}

/// Immutable presentation state for the preview of the selected photo.
///
/// This model intentionally contains display metadata only. Image pixels, rendering services,
/// catalog storage, and filesystem paths belong to application and domain layers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SelectedPreviewState {
    Loading,
    Ready(Rgba8PreviewMetadata),
    #[default]
    Unavailable,
    Failed(SelectedPreviewFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoDetailViewModel {
    id: PhotoId,
    title: PresentationText,
    facts: Vec<PhotoFactViewModel>,
    selected_preview: SelectedPreviewState,
}

impl PhotoDetailViewModel {
    #[must_use]
    pub fn new(id: PhotoId, title: PresentationText, facts: Vec<PhotoFactViewModel>) -> Self {
        Self {
            id,
            title,
            facts,
            selected_preview: SelectedPreviewState::default(),
        }
    }

    #[must_use]
    pub fn with_selected_preview(mut self, selected_preview: SelectedPreviewState) -> Self {
        self.selected_preview = selected_preview;
        self
    }

    #[must_use]
    pub fn id(&self) -> PhotoId {
        self.id
    }

    #[must_use]
    pub fn title(&self) -> &PresentationText {
        &self.title
    }

    #[must_use = "iterate over the detail facts"]
    pub fn facts(&self) -> impl Iterator<Item = &PhotoFactViewModel> {
        self.facts.iter()
    }

    #[must_use]
    pub fn selected_preview(&self) -> &SelectedPreviewState {
        &self.selected_preview
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoWorkspaceViewModelError {
    DuplicateCardId { id: PhotoId },
    DuplicateDetailId { id: PhotoId },
    MissingDetail { id: PhotoId },
    OrphanDetail { id: PhotoId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhotoWorkspaceViewModel {
    cards: Vec<PhotoCardViewModel>,
    details: BTreeMap<PhotoId, PhotoDetailViewModel>,
}

impl PhotoWorkspaceViewModel {
    /// Builds a workspace while checking card/detail topology.
    ///
    /// # Errors
    ///
    /// Returns a stable error when card or detail identifiers are duplicated or when the two
    /// collections do not describe the same set of photos.
    pub fn new(
        cards: Vec<PhotoCardViewModel>,
        details: Vec<PhotoDetailViewModel>,
    ) -> Result<Self, PhotoWorkspaceViewModelError> {
        let mut card_ids = BTreeSet::new();
        for card in &cards {
            if !card_ids.insert(card.id()) {
                return Err(PhotoWorkspaceViewModelError::DuplicateCardId { id: card.id() });
            }
        }

        let mut detail_lookup = BTreeMap::new();
        for detail in details {
            let id = detail.id();
            if detail_lookup.insert(id, detail).is_some() {
                return Err(PhotoWorkspaceViewModelError::DuplicateDetailId { id });
            }
        }

        for card in &cards {
            if !detail_lookup.contains_key(&card.id()) {
                return Err(PhotoWorkspaceViewModelError::MissingDetail { id: card.id() });
            }
        }
        for id in detail_lookup.keys() {
            if !card_ids.contains(id) {
                return Err(PhotoWorkspaceViewModelError::OrphanDetail { id: *id });
            }
        }

        Ok(Self {
            cards,
            details: detail_lookup,
        })
    }

    #[must_use]
    pub fn cards(&self) -> impl ExactSizeIterator<Item = &PhotoCardViewModel> {
        self.cards.iter()
    }

    #[must_use]
    pub fn detail(&self, id: PhotoId) -> Option<&PhotoDetailViewModel> {
        self.details.get(&id)
    }

    /// Returns this workspace with one selected-photo preview state replaced.
    ///
    /// Returns `None` when no detail exists for `photo_id`; the cards and all other details are
    /// preserved unchanged.
    #[must_use]
    pub fn with_selected_preview(
        mut self,
        photo_id: PhotoId,
        selected_preview: SelectedPreviewState,
    ) -> Option<Self> {
        self.details.get_mut(&photo_id)?.selected_preview = selected_preview;
        Some(self)
    }

    #[must_use]
    pub fn details(&self) -> impl ExactSizeIterator<Item = &PhotoDetailViewModel> {
        self.details.values()
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoFactViewModel, PhotoWorkspaceViewModel,
        PhotoWorkspaceViewModelError, PresentationText, PresentationTextError, PreviewDimensions,
        PreviewDimensionsError, Rgba8PreviewMetadata, Rgba8PreviewMetadataError,
        SelectedPreviewFailure, SelectedPreviewState,
    };

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("test photo ID is non-zero")
    }

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("test text is valid")
    }

    fn card(value: u128) -> PhotoCardViewModel {
        PhotoCardViewModel::new(id(value), text("Title"), None)
    }

    fn detail(value: u128) -> PhotoDetailViewModel {
        PhotoDetailViewModel::new(
            id(value),
            text("Title"),
            vec![PhotoFactViewModel::new(text("Camera"), text("Example"))],
        )
    }

    #[test]
    fn presentation_text_rejects_empty_and_whitespace() {
        assert_eq!(PresentationText::new(""), Err(PresentationTextError::Empty));
        assert_eq!(
            PresentationText::new(" \t\n"),
            Err(PresentationTextError::WhitespaceOnly)
        );
    }

    #[test]
    fn presentation_text_rejects_controls_and_reports_length() {
        assert_eq!(
            PresentationText::new("ok\u{0007}"),
            Err(PresentationTextError::AsciiControlCharacter {
                byte_index: 2,
                value: '\u{0007}',
            })
        );
        let too_long = "a".repeat(257);
        assert_eq!(
            PresentationText::new(too_long),
            Err(PresentationTextError::TooLong { byte_length: 257 })
        );
    }

    #[test]
    fn empty_workspace_is_constructible_and_has_no_values() {
        let workspace = PhotoWorkspaceViewModel::new(Vec::new(), Vec::new()).expect("empty input");

        assert_eq!(workspace.cards().len(), 0);
        assert_eq!(workspace.details().len(), 0);
    }

    #[test]
    fn workspace_preserves_card_order_and_sorts_detail_lookup() {
        let workspace =
            PhotoWorkspaceViewModel::new(vec![card(2), card(1)], vec![detail(1), detail(2)])
                .expect("matching values");
        let same_workspace =
            PhotoWorkspaceViewModel::new(vec![card(2), card(1)], vec![detail(1), detail(2)])
                .expect("matching values");

        assert_eq!(workspace, same_workspace);

        assert_eq!(
            workspace
                .cards()
                .map(PhotoCardViewModel::id)
                .collect::<Vec<_>>(),
            vec![id(2), id(1)]
        );
        assert_eq!(
            workspace.detail(id(1)).map(PhotoDetailViewModel::id),
            Some(id(1))
        );
        assert_eq!(
            workspace
                .details()
                .map(PhotoDetailViewModel::id)
                .collect::<Vec<_>>(),
            vec![id(1), id(2)]
        );

        let card = workspace.cards().next().expect("card exists");
        assert_eq!(card.title().as_str(), "Title");
        assert_eq!(card.secondary(), None);
        let detail = workspace.detail(id(1)).expect("detail exists");
        assert_eq!(detail.title().as_str(), "Title");
        assert_eq!(
            detail.selected_preview(),
            &SelectedPreviewState::Unavailable
        );
        let fact = detail.facts().next().expect("fact exists");
        assert_eq!(fact.label().as_str(), "Camera");
        assert_eq!(fact.value().as_str(), "Example");
    }

    #[test]
    fn workspace_rejects_duplicate_cards_before_other_errors() {
        assert_eq!(
            PhotoWorkspaceViewModel::new(vec![card(1), card(1)], Vec::new()),
            Err(PhotoWorkspaceViewModelError::DuplicateCardId { id: id(1) })
        );
    }

    #[test]
    fn workspace_rejects_duplicate_details() {
        assert_eq!(
            PhotoWorkspaceViewModel::new(vec![card(1)], vec![detail(1), detail(1)]),
            Err(PhotoWorkspaceViewModelError::DuplicateDetailId { id: id(1) })
        );
    }

    #[test]
    fn workspace_rejects_missing_detail() {
        assert_eq!(
            PhotoWorkspaceViewModel::new(vec![card(1)], Vec::new()),
            Err(PhotoWorkspaceViewModelError::MissingDetail { id: id(1) })
        );
    }

    #[test]
    fn workspace_rejects_orphan_detail() {
        assert_eq!(
            PhotoWorkspaceViewModel::new(Vec::new(), vec![detail(1)]),
            Err(PhotoWorkspaceViewModelError::OrphanDetail { id: id(1) })
        );
    }

    #[test]
    fn preview_dimensions_reject_zero_axes() {
        assert_eq!(
            PreviewDimensions::new(0, 100),
            Err(PreviewDimensionsError::ZeroWidth)
        );
        assert_eq!(
            PreviewDimensions::new(100, 0),
            Err(PreviewDimensionsError::ZeroHeight)
        );
    }

    #[test]
    fn selected_preview_metadata_stays_with_detail_without_external_dependencies() {
        let dimensions = PreviewDimensions::new(2, 1).expect("non-zero dimensions");
        let pixels = vec![12, 34, 56, 255, 78, 90, 123, 255];
        let preview = SelectedPreviewState::Ready(
            Rgba8PreviewMetadata::new(dimensions, text("Edited preview ready"), pixels.clone())
                .expect("valid RGBA8 pixels"),
        );
        let detail = PhotoDetailViewModel::new(id(1), text("Title"), Vec::new())
            .with_selected_preview(preview.clone());

        assert_eq!(detail.selected_preview(), &preview);
        assert_eq!(
            detail.selected_preview(),
            &SelectedPreviewState::Ready(
                Rgba8PreviewMetadata::new(dimensions, text("Edited preview ready"), pixels)
                    .expect("valid RGBA8 pixels")
            )
        );
    }

    #[test]
    fn selected_preview_failure_exposes_display_safe_detail() {
        let failure = SelectedPreviewFailure::new(text("The preview could not be decoded."));

        assert_eq!(
            failure.detail().as_str(),
            "The preview could not be decoded."
        );
    }

    #[test]
    fn rgba8_preview_rejects_wrong_lengths_and_oversized_dimensions() {
        let dimensions = PreviewDimensions::new(2, 1).expect("non-zero dimensions");
        assert_eq!(
            Rgba8PreviewMetadata::new(dimensions, text("Ready"), vec![0; 7]),
            Err(Rgba8PreviewMetadataError::IncorrectByteLength {
                expected: 8,
                actual: 7,
            })
        );

        let oversized = PreviewDimensions::new(8_193, 2_048).expect("non-zero dimensions");
        assert_eq!(
            Rgba8PreviewMetadata::new(oversized, text("Ready"), Vec::new()),
            Err(Rgba8PreviewMetadataError::TooLarge {
                byte_length: 67_117_056,
            })
        );
    }

    #[test]
    fn workspace_replaces_only_the_selected_preview_immutably() {
        let workspace =
            PhotoWorkspaceViewModel::new(vec![card(2), card(1)], vec![detail(1), detail(2)])
                .expect("matching values");
        let preview = SelectedPreviewState::Ready(
            Rgba8PreviewMetadata::new(
                PreviewDimensions::new(2, 1).expect("non-zero dimensions"),
                text("Edited preview ready"),
                vec![12, 34, 56, 255, 78, 90, 123, 255],
            )
            .expect("valid RGBA8 pixels"),
        );

        let updated = workspace
            .clone()
            .with_selected_preview(id(2), preview.clone())
            .expect("selected photo exists");

        assert_eq!(
            workspace
                .detail(id(2))
                .map(PhotoDetailViewModel::selected_preview),
            Some(&SelectedPreviewState::Unavailable)
        );
        assert_eq!(
            updated
                .detail(id(2))
                .map(PhotoDetailViewModel::selected_preview),
            Some(&preview)
        );
        assert_eq!(
            updated
                .detail(id(1))
                .map(PhotoDetailViewModel::selected_preview),
            Some(&SelectedPreviewState::Unavailable)
        );
        assert_eq!(
            updated
                .cards()
                .map(PhotoCardViewModel::id)
                .collect::<Vec<_>>(),
            vec![id(2), id(1)]
        );
        assert_eq!(workspace.with_selected_preview(id(99), preview), None);
    }
}
