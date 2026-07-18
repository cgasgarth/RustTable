use std::collections::{BTreeMap, BTreeSet};

use rusttable_core::PhotoId;

const MAX_PRESENTATION_TEXT_BYTES: usize = 256;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoDetailViewModel {
    id: PhotoId,
    title: PresentationText,
    facts: Vec<PhotoFactViewModel>,
}

impl PhotoDetailViewModel {
    #[must_use]
    pub fn new(id: PhotoId, title: PresentationText, facts: Vec<PhotoFactViewModel>) -> Self {
        Self { id, title, facts }
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
        PhotoWorkspaceViewModelError, PresentationText, PresentationTextError,
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
}
