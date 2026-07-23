//! Privacy-safe duplicate evidence and deterministic classification.

mod visual;

use std::collections::BTreeMap;

use rusttable_core::{ContentHash, MetadataEntry, MetadataField, PhotoId};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::{ImportRecord, ReferencePathIdentity};

pub use visual::VisualFingerprint;

pub const DUPLICATE_EVIDENCE_VERSION: u8 = 1;
pub const MAX_DUPLICATE_MATCHES: usize = 64;
pub const MAX_DUPLICATE_CANDIDATES: usize = 1_024;
pub const PROBABLE_VISUAL_HAMMING_THRESHOLD: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactContentIdentity {
    sha256: [u8; 32],
    byte_length: u64,
}

impl ExactContentIdentity {
    #[must_use]
    pub const fn new(sha256: [u8; 32], byte_length: u64) -> Self {
        Self {
            sha256,
            byte_length,
        }
    }

    #[must_use]
    pub const fn sha256(self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn byte_length(self) -> u64 {
        self.byte_length
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EmbeddedPhotoIdentity([u8; 32]);

impl EmbeddedPhotoIdentity {
    #[must_use]
    pub const fn new(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateEvidence {
    version: u8,
    photo_id: PhotoId,
    source: ReferencePathIdentity,
    exact: ExactContentIdentity,
    embedded: Option<EmbeddedPhotoIdentity>,
    visual: Option<VisualFingerprint>,
}

impl DuplicateEvidence {
    #[must_use]
    pub const fn new(
        photo_id: PhotoId,
        source: ReferencePathIdentity,
        exact: ExactContentIdentity,
        embedded: Option<EmbeddedPhotoIdentity>,
        visual: Option<VisualFingerprint>,
    ) -> Self {
        Self {
            version: DUPLICATE_EVIDENCE_VERSION,
            photo_id,
            source,
            exact,
            embedded,
            visual,
        }
    }

    #[must_use]
    pub fn from_record(
        record: &ImportRecord,
        source: ReferencePathIdentity,
        visual: Option<VisualFingerprint>,
    ) -> Self {
        let asset = record.photo().primary_asset();
        let ContentHash::Sha256(sha256) = asset.content_hash();
        Self::new(
            record.photo().id(),
            source,
            ExactContentIdentity::new(sha256, asset.byte_length().get()),
            embedded_identity(record),
            visual,
        )
    }

    #[must_use]
    pub const fn version(self) -> u8 {
        self.version
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn source(self) -> ReferencePathIdentity {
        self.source
    }

    #[must_use]
    pub const fn exact(self) -> ExactContentIdentity {
        self.exact
    }

    #[must_use]
    pub const fn embedded(self) -> Option<EmbeddedPhotoIdentity> {
        self.embedded
    }

    #[must_use]
    pub const fn visual(self) -> Option<VisualFingerprint> {
        self.visual
    }

    #[must_use]
    pub fn describes(self, record: &ImportRecord) -> bool {
        let asset = record.photo().primary_asset();
        let ContentHash::Sha256(sha256) = asset.content_hash();
        self.version == DUPLICATE_EVIDENCE_VERSION
            && self.photo_id == record.photo().id()
            && self.exact == ExactContentIdentity::new(sha256, asset.byte_length().get())
            && self.embedded == embedded_identity(record)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DuplicateClassification {
    Source,
    ExactContent,
    EmbeddedIdentity,
    ProbableVisual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DuplicateMatch {
    photo_id: PhotoId,
    classification: DuplicateClassification,
    confidence_millis: u16,
    visual_distance: Option<u32>,
}

impl DuplicateMatch {
    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn classification(self) -> DuplicateClassification {
        self.classification
    }

    #[must_use]
    pub const fn confidence_millis(self) -> u16 {
        self.confidence_millis
    }

    #[must_use]
    pub const fn visual_distance(self) -> Option<u32> {
        self.visual_distance
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DuplicateSearchResult {
    matches: Vec<DuplicateMatch>,
    truncated: bool,
}

impl DuplicateSearchResult {
    #[must_use]
    pub fn from_candidates<I>(matches: I, candidates_truncated: bool) -> Self
    where
        I: IntoIterator<Item = DuplicateMatch>,
    {
        let mut strongest = BTreeMap::<PhotoId, DuplicateMatch>::new();
        for duplicate in matches {
            strongest
                .entry(duplicate.photo_id)
                .and_modify(|current| {
                    if duplicate_sort_key(&duplicate) < duplicate_sort_key(current) {
                        *current = duplicate;
                    }
                })
                .or_insert(duplicate);
        }
        let mut matches = strongest.into_values().collect::<Vec<_>>();
        matches.sort_by_key(duplicate_sort_key);
        let truncated = candidates_truncated || matches.len() > MAX_DUPLICATE_MATCHES;
        matches.truncate(MAX_DUPLICATE_MATCHES);
        Self { matches, truncated }
    }

    #[must_use]
    pub fn matches(&self) -> impl ExactSizeIterator<Item = &DuplicateMatch> {
        self.matches.iter()
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

#[must_use]
pub fn classify_duplicate(
    candidate: DuplicateEvidence,
    existing: DuplicateEvidence,
) -> Option<DuplicateMatch> {
    let (classification, confidence_millis, visual_distance) = if candidate.source
        == existing.source
    {
        (DuplicateClassification::Source, 1_000, None)
    } else if candidate.exact == existing.exact {
        (DuplicateClassification::ExactContent, 975, None)
    } else if candidate.embedded.is_some() && candidate.embedded == existing.embedded {
        (DuplicateClassification::EmbeddedIdentity, 900, None)
    } else {
        let (Some(candidate), Some(existing)) = (candidate.visual, existing.visual) else {
            return None;
        };
        let distance = candidate.distance(existing);
        if !candidate.has_similar_aspect(existing) || distance > PROBABLE_VISUAL_HAMMING_THRESHOLD {
            return None;
        }
        let confidence = 850_u16.saturating_sub(u16::try_from(distance).unwrap_or(u16::MAX) * 25);
        (
            DuplicateClassification::ProbableVisual,
            confidence,
            Some(distance),
        )
    };
    Some(DuplicateMatch {
        photo_id: existing.photo_id,
        classification,
        confidence_millis,
        visual_distance,
    })
}

fn duplicate_sort_key(duplicate: &DuplicateMatch) -> (DuplicateClassification, u32, PhotoId) {
    (
        duplicate.classification,
        duplicate.visual_distance.unwrap_or(0),
        duplicate.photo_id,
    )
}

fn embedded_identity(record: &ImportRecord) -> Option<EmbeddedPhotoIdentity> {
    let camera_model = metadata_text(record, MetadataField::CameraModel)?;
    let captured_at = metadata_text(record, MetadataField::CaptureDateTimeOriginal)?;
    let camera_make = metadata_text(record, MetadataField::CameraMake).unwrap_or_default();
    let dimensions = record.probe().dimensions();
    let orientation = match record.metadata().get(MetadataField::Orientation) {
        Some(MetadataEntry::Orientation(value)) => value.code(),
        _ => 1,
    };
    let (width, height) = if orientation >= 5 {
        (dimensions.height(), dimensions.width())
    } else {
        (dimensions.width(), dimensions.height())
    };
    let mut digest = Sha256::new();
    digest.update(b"rusttable-embedded-photo-identity-v1\0");
    hash_normalized_text(&mut digest, camera_make);
    hash_normalized_text(&mut digest, camera_model);
    hash_normalized_text(&mut digest, captured_at);
    digest.update(width.to_be_bytes());
    digest.update(height.to_be_bytes());
    Some(EmbeddedPhotoIdentity::new(digest.finalize().into()))
}

fn metadata_text(record: &ImportRecord, field: MetadataField) -> Option<&str> {
    match record.metadata().get(field)? {
        MetadataEntry::CameraMake(value)
        | MetadataEntry::CameraModel(value)
        | MetadataEntry::CaptureDateTimeOriginal(value) => Some(value.as_str()),
        MetadataEntry::LensModel(_)
        | MetadataEntry::Orientation(_)
        | MetadataEntry::ExposureTime(_)
        | MetadataEntry::FNumber(_)
        | MetadataEntry::IsoSpeed(_)
        | MetadataEntry::FocalLength(_) => None,
    }
}

fn hash_normalized_text(digest: &mut Sha256, value: &str) {
    let normalized = value
        .trim()
        .nfkc()
        .map(|(character, _alignment)| character)
        .flat_map(char::to_lowercase)
        .collect::<String>();
    digest.update(
        u64::try_from(normalized.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    digest.update(normalized.as_bytes());
}
