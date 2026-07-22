use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusttable_core::{ImageMetadata, MetadataEntry, PhotoId, Revision};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CATALOG_METADATA_SCHEMA_VERSION: u16 = 1;
const MAX_NAMESPACE_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_BINARY_BYTES: usize = 64 * 1024;
const MAX_VALUES: usize = 64;
const MAX_CANDIDATES_PER_FIELD: usize = 64;
const MAX_BATCH_PHOTOS: usize = 4_096;
const MAX_EDITS_PER_PHOTO: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogMetadataError {
    EmptyKeyPart,
    InvalidKeyPart,
    KeyTooLong,
    EmptyValues,
    TooManyValues,
    TooManyCandidates,
    TooManyEdits,
    DuplicateValue,
    ValueTooLarge,
    InvalidRational,
    NonFiniteFloat,
    UnsupportedSchema(u16),
    RevisionConflict {
        expected: Revision,
        actual: Revision,
    },
    RevisionOverflow,
    DuplicatePhoto(PhotoId),
    BatchTooLarge,
    PhotoNotFound(PhotoId),
    Unavailable,
    CorruptPersistedData,
    CommitFailure,
}

impl fmt::Display for CatalogMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyKeyPart => formatter.write_str("metadata key parts cannot be empty"),
            Self::InvalidKeyPart => {
                formatter.write_str("metadata key parts cannot contain controls")
            }
            Self::KeyTooLong => formatter.write_str("metadata key part exceeds its byte limit"),
            Self::EmptyValues => formatter.write_str("metadata values cannot be empty"),
            Self::TooManyValues => formatter.write_str("metadata value count exceeds its limit"),
            Self::TooManyCandidates => {
                formatter.write_str("metadata field candidate count exceeds its limit")
            }
            Self::TooManyEdits => {
                formatter.write_str("metadata photo edit count exceeds its limit")
            }
            Self::DuplicateValue => {
                formatter.write_str("metadata values cannot contain duplicates")
            }
            Self::ValueTooLarge => formatter.write_str("metadata value exceeds its byte limit"),
            Self::InvalidRational => {
                formatter.write_str("metadata rational denominator cannot be zero")
            }
            Self::NonFiniteFloat => formatter.write_str("metadata float must be finite"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported metadata schema {version}")
            }
            Self::RevisionConflict { expected, actual } => {
                write!(
                    formatter,
                    "metadata revision conflict: expected {expected}, actual {actual}"
                )
            }
            Self::RevisionOverflow => formatter.write_str("metadata revision overflow"),
            Self::DuplicatePhoto(photo_id) => {
                write!(formatter, "metadata batch duplicates photo {photo_id}")
            }
            Self::BatchTooLarge => formatter.write_str("metadata batch exceeds its photo limit"),
            Self::PhotoNotFound(photo_id) => {
                write!(formatter, "metadata photo {photo_id} was not found")
            }
            Self::Unavailable => formatter.write_str("metadata store is unavailable"),
            Self::CorruptPersistedData => formatter.write_str("catalog metadata is corrupt"),
            Self::CommitFailure => formatter.write_str("metadata transaction did not commit"),
        }
    }
}

impl std::error::Error for CatalogMetadataError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CatalogMetadataKey {
    namespace: String,
    name: String,
}

impl CatalogMetadataKey {
    /// Preserves an upstream namespaced key exactly after bounded validation.
    ///
    /// # Errors
    /// Returns an error for empty, control-containing, or oversized parts.
    pub fn new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, CatalogMetadataError> {
        let namespace = namespace.into();
        let name = name.into();
        validate_key_part(&namespace, MAX_NAMESPACE_BYTES)?;
        validate_key_part(&name, MAX_NAME_BYTES)?;
        Ok(Self { namespace, name })
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

fn validate_key_part(value: &str, maximum: usize) -> Result<(), CatalogMetadataError> {
    if value.is_empty() {
        return Err(CatalogMetadataError::EmptyKeyPart);
    }
    if value.len() > maximum {
        return Err(CatalogMetadataError::KeyTooLong);
    }
    if value.chars().any(char::is_control) {
        return Err(CatalogMetadataError::InvalidKeyPart);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CatalogMetadataValue {
    Text(String),
    Integer(i64),
    Unsigned(u64),
    Rational { numerator: u64, denominator: u64 },
    SignedRational { numerator: i64, denominator: i64 },
    FloatBits(u64),
    Boolean(bool),
    DateTime(String),
    Binary(Vec<u8>),
}

impl CatalogMetadataValue {
    /// Creates a finite floating-point metadata value with canonical zero.
    ///
    /// # Errors
    /// Returns [`CatalogMetadataError::NonFiniteFloat`] for NaN or infinity.
    pub fn float(value: f64) -> Result<Self, CatalogMetadataError> {
        if !value.is_finite() {
            return Err(CatalogMetadataError::NonFiniteFloat);
        }
        Ok(Self::FloatBits(if value == 0.0 {
            0.0_f64.to_bits()
        } else {
            value.to_bits()
        }))
    }

    fn validate(&self) -> Result<(), CatalogMetadataError> {
        match self {
            Self::Text(value) | Self::DateTime(value) if value.len() > MAX_TEXT_BYTES => {
                Err(CatalogMetadataError::ValueTooLarge)
            }
            Self::Binary(value) if value.len() > MAX_BINARY_BYTES => {
                Err(CatalogMetadataError::ValueTooLarge)
            }
            Self::Rational { denominator: 0, .. } | Self::SignedRational { denominator: 0, .. } => {
                Err(CatalogMetadataError::InvalidRational)
            }
            Self::FloatBits(bits) if !f64::from_bits(*bits).is_finite() => {
                Err(CatalogMetadataError::NonFiniteFloat)
            }
            _ => Ok(()),
        }
    }

    fn hash_into(&self, digest: &mut Sha256) {
        match self {
            Self::Text(value) => hash_bytes(digest, 0, value.as_bytes()),
            Self::Integer(value) => hash_bytes(digest, 1, &value.to_be_bytes()),
            Self::Unsigned(value) => hash_bytes(digest, 2, &value.to_be_bytes()),
            Self::Rational {
                numerator,
                denominator,
            } => {
                hash_bytes(digest, 3, &numerator.to_be_bytes());
                digest.update(denominator.to_be_bytes());
            }
            Self::SignedRational {
                numerator,
                denominator,
            } => {
                hash_bytes(digest, 4, &numerator.to_be_bytes());
                digest.update(denominator.to_be_bytes());
            }
            Self::FloatBits(value) => hash_bytes(digest, 5, &value.to_be_bytes()),
            Self::Boolean(value) => hash_bytes(digest, 6, &[u8::from(*value)]),
            Self::DateTime(value) => hash_bytes(digest, 7, value.as_bytes()),
            Self::Binary(value) => hash_bytes(digest, 8, value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CatalogMetadataValues(Vec<CatalogMetadataValue>);

impl CatalogMetadataValues {
    /// Creates an ordered multi-value while preserving source representation.
    ///
    /// # Errors
    /// Returns an error for empty, duplicate, invalid, or excessive values.
    pub fn new(values: Vec<CatalogMetadataValue>) -> Result<Self, CatalogMetadataError> {
        if values.is_empty() {
            return Err(CatalogMetadataError::EmptyValues);
        }
        if values.len() > MAX_VALUES {
            return Err(CatalogMetadataError::TooManyValues);
        }
        let mut seen = BTreeSet::new();
        for value in &values {
            value.validate()?;
            if !seen.insert(value) {
                return Err(CatalogMetadataError::DuplicateValue);
            }
        }
        Ok(Self(values))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[CatalogMetadataValue] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CatalogMetadataPrivacy {
    Public,
    Private,
    Sensitive,
}

impl CatalogMetadataPrivacy {
    #[must_use]
    pub const fn is_exportable(self) -> bool {
        matches!(self, Self::Public)
    }

    #[must_use]
    pub const fn is_indexable(self) -> bool {
        !matches!(self, Self::Sensitive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CatalogMetadataSource {
    Imported,
    RawMetadataReceipt,
    GeneratedTechnical,
    RecipeOverride,
    CatalogEdit,
}

impl CatalogMetadataSource {
    const fn precedence(self) -> u8 {
        match self {
            Self::Imported => 0,
            Self::RawMetadataReceipt => 1,
            Self::GeneratedTechnical => 2,
            Self::RecipeOverride => 3,
            Self::CatalogEdit => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CatalogMetadataProvenance {
    source: CatalogMetadataSource,
    evidence_sha256: [u8; 32],
}

impl CatalogMetadataProvenance {
    #[must_use]
    pub const fn new(source: CatalogMetadataSource, evidence_sha256: [u8; 32]) -> Self {
        Self {
            source,
            evidence_sha256,
        }
    }

    /// Links catalog values to `RawMetadataReceipt::normalized_sha256` without retaining paths or camera serials.
    #[must_use]
    pub const fn raw_metadata_receipt(normalized_sha256: [u8; 32]) -> Self {
        Self::new(CatalogMetadataSource::RawMetadataReceipt, normalized_sha256)
    }

    #[must_use]
    pub const fn source(&self) -> CatalogMetadataSource {
        self.source
    }

    #[must_use]
    pub const fn evidence_sha256(&self) -> [u8; 32] {
        self.evidence_sha256
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CatalogMetadataCandidate {
    values: CatalogMetadataValues,
    privacy: CatalogMetadataPrivacy,
    provenance: CatalogMetadataProvenance,
}

impl fmt::Debug for CatalogMetadataCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CatalogMetadataCandidate")
            .field("value_count", &self.values.0.len())
            .field("privacy", &self.privacy)
            .field("provenance", &self.provenance)
            .finish()
    }
}

impl CatalogMetadataCandidate {
    #[must_use]
    pub const fn new(
        values: CatalogMetadataValues,
        privacy: CatalogMetadataPrivacy,
        provenance: CatalogMetadataProvenance,
    ) -> Self {
        Self {
            values,
            privacy,
            provenance,
        }
    }

    #[must_use]
    pub const fn values(&self) -> &CatalogMetadataValues {
        &self.values
    }

    #[must_use]
    pub const fn privacy(&self) -> CatalogMetadataPrivacy {
        self.privacy
    }

    #[must_use]
    pub const fn provenance(&self) -> &CatalogMetadataProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMetadataConflict {
    candidates: Vec<CatalogMetadataCandidate>,
}

impl CatalogMetadataConflict {
    #[must_use]
    pub fn candidates(&self) -> &[CatalogMetadataCandidate] {
        &self.candidates
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMetadataField {
    selected: CatalogMetadataCandidate,
    conflict: Option<CatalogMetadataConflict>,
}

impl CatalogMetadataField {
    #[must_use]
    pub const fn selected(&self) -> &CatalogMetadataCandidate {
        &self.selected
    }

    #[must_use]
    pub const fn conflict(&self) -> Option<&CatalogMetadataConflict> {
        self.conflict.as_ref()
    }

    fn reconcile(mut candidates: Vec<CatalogMetadataCandidate>) -> Self {
        candidates.sort_by(|left, right| {
            candidate_rank(right)
                .cmp(&candidate_rank(left))
                .then_with(|| left.cmp(right))
        });
        candidates.dedup();
        let selected = candidates.remove(0);
        let conflict = (!candidates.is_empty()).then_some(CatalogMetadataConflict { candidates });
        Self { selected, conflict }
    }
}

fn candidate_rank(candidate: &CatalogMetadataCandidate) -> u8 {
    candidate.provenance.source.precedence()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogMetadataDocument {
    schema_version: u16,
    #[serde(with = "photo_id_serde")]
    photo_id: PhotoId,
    revision: u64,
    fields: BTreeMap<CatalogMetadataKey, CatalogMetadataField>,
}

impl CatalogMetadataDocument {
    #[must_use]
    pub fn empty(photo_id: PhotoId) -> Self {
        Self {
            schema_version: CATALOG_METADATA_SCHEMA_VERSION,
            photo_id,
            revision: 0,
            fields: BTreeMap::new(),
        }
    }

    /// Reconciles unordered source candidates using stable precedence and lexical tie-breaking.
    ///
    /// # Errors
    /// Returns an error if no candidates are supplied for a key.
    pub fn reconcile(
        photo_id: PhotoId,
        candidates: impl IntoIterator<Item = (CatalogMetadataKey, CatalogMetadataCandidate)>,
    ) -> Result<Self, CatalogMetadataError> {
        let mut grouped: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (key, candidate) in candidates {
            grouped.entry(key).or_default().push(candidate);
        }
        if grouped
            .values()
            .any(|values| values.len() > MAX_CANDIDATES_PER_FIELD)
        {
            return Err(CatalogMetadataError::TooManyCandidates);
        }
        let fields = grouped
            .into_iter()
            .map(|(key, values)| (key, CatalogMetadataField::reconcile(values)))
            .collect();
        Ok(Self {
            schema_version: CATALOG_METADATA_SCHEMA_VERSION,
            photo_id,
            revision: 0,
            fields,
        })
    }

    /// Imports the existing immutable technical metadata contract without changing values.
    #[must_use]
    pub fn from_image_metadata(
        photo_id: PhotoId,
        metadata: &ImageMetadata,
        provenance: &CatalogMetadataProvenance,
    ) -> Self {
        let fields = metadata
            .iter()
            .map(|(_, entry)| metadata_entry(entry, provenance.clone()))
            .map(|(key, candidate)| (key, CatalogMetadataField::reconcile(vec![candidate])))
            .collect();
        Self {
            schema_version: CATALOG_METADATA_SCHEMA_VERSION,
            photo_id,
            revision: 0,
            fields,
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        Revision::from_u64(self.revision)
    }

    #[must_use]
    pub fn fields(&self) -> &BTreeMap<CatalogMetadataKey, CatalogMetadataField> {
        &self.fields
    }

    /// Applies edits atomically to this value and advances its revision once.
    ///
    /// # Errors
    /// Returns a revision or validation error without changing this document.
    pub fn apply(
        &self,
        expected: Revision,
        edits: &[CatalogMetadataEdit],
    ) -> Result<Self, CatalogMetadataError> {
        if expected != self.revision() {
            return Err(CatalogMetadataError::RevisionConflict {
                expected,
                actual: self.revision(),
            });
        }
        let next = expected
            .checked_increment()
            .map_err(|_| CatalogMetadataError::RevisionOverflow)?;
        let mut updated = self.clone();
        for edit in edits {
            match edit {
                CatalogMetadataEdit::Set { key, candidate } => {
                    updated.fields.insert(
                        key.clone(),
                        CatalogMetadataField::reconcile(vec![candidate.clone()]),
                    );
                }
                CatalogMetadataEdit::Clear { key } => {
                    updated.fields.remove(key);
                }
            }
        }
        updated.revision = next.get();
        Ok(updated)
    }

    /// Validates an object decoded from persistence.
    ///
    /// # Errors
    /// Returns a typed corruption cause for unsupported or invalid data.
    pub fn validate(&self) -> Result<(), CatalogMetadataError> {
        if self.schema_version != CATALOG_METADATA_SCHEMA_VERSION {
            return Err(CatalogMetadataError::UnsupportedSchema(self.schema_version));
        }
        for (key, field) in &self.fields {
            validate_key_part(key.namespace(), MAX_NAMESPACE_BYTES)?;
            validate_key_part(key.name(), MAX_NAME_BYTES)?;
            validate_candidate(field.selected())?;
            if let Some(conflict) = field.conflict() {
                if conflict.candidates.is_empty() {
                    return Err(CatalogMetadataError::CorruptPersistedData);
                }
                if conflict.candidates.len() >= MAX_CANDIDATES_PER_FIELD {
                    return Err(CatalogMetadataError::TooManyCandidates);
                }
                for candidate in &conflict.candidates {
                    validate_candidate(candidate)?;
                }
                let mut candidates = vec![field.selected.clone()];
                candidates.extend(conflict.candidates.iter().cloned());
                if CatalogMetadataField::reconcile(candidates) != *field {
                    return Err(CatalogMetadataError::CorruptPersistedData);
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn canonical_sha256(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(self.schema_version.to_be_bytes());
        digest.update(self.photo_id.get().to_be_bytes());
        digest.update(self.revision.to_be_bytes());
        for (key, field) in &self.fields {
            hash_bytes(&mut digest, 0, key.namespace.as_bytes());
            hash_bytes(&mut digest, 1, key.name.as_bytes());
            hash_candidate(&mut digest, &field.selected);
            if let Some(conflict) = &field.conflict {
                digest.update((conflict.candidates.len() as u64).to_be_bytes());
                for candidate in &conflict.candidates {
                    hash_candidate(&mut digest, candidate);
                }
            } else {
                digest.update(0_u64.to_be_bytes());
            }
        }
        digest.finalize().into()
    }

    #[must_use]
    pub fn index_terms(&self) -> Vec<CatalogMetadataIndexTerm> {
        self.fields
            .iter()
            .filter(|(_, field)| field.selected.privacy.is_indexable())
            .flat_map(|(key, field)| {
                field
                    .selected
                    .values
                    .as_slice()
                    .iter()
                    .map(move |value| CatalogMetadataIndexTerm::new(self.photo_id, key, value))
            })
            .collect()
    }

    #[must_use]
    pub fn diagnostic(&self) -> CatalogMetadataDiagnostic {
        CatalogMetadataDiagnostic {
            photo_id: self.photo_id,
            revision: self.revision(),
            field_count: self.fields.len(),
            conflict_count: self
                .fields
                .values()
                .filter(|field| field.conflict.is_some())
                .count(),
        }
    }
}

fn validate_candidate(candidate: &CatalogMetadataCandidate) -> Result<(), CatalogMetadataError> {
    CatalogMetadataValues::new(candidate.values.0.clone()).map(|_| ())
}

fn metadata_entry(
    entry: &MetadataEntry,
    provenance: CatalogMetadataProvenance,
) -> (CatalogMetadataKey, CatalogMetadataCandidate) {
    let (name, value) = match entry {
        MetadataEntry::CameraMake(value) => (
            "Make",
            CatalogMetadataValue::Text(value.as_str().to_owned()),
        ),
        MetadataEntry::CameraModel(value) => (
            "Model",
            CatalogMetadataValue::Text(value.as_str().to_owned()),
        ),
        MetadataEntry::LensModel(value) => (
            "LensModel",
            CatalogMetadataValue::Text(value.as_str().to_owned()),
        ),
        MetadataEntry::CaptureDateTimeOriginal(value) => (
            "DateTimeOriginal",
            CatalogMetadataValue::DateTime(value.as_str().to_owned()),
        ),
        MetadataEntry::Orientation(value) => (
            "Orientation",
            CatalogMetadataValue::Unsigned(u64::from(value.code())),
        ),
        MetadataEntry::ExposureTime(value) => (
            "ExposureTime",
            CatalogMetadataValue::Rational {
                numerator: value.numerator(),
                denominator: value.denominator(),
            },
        ),
        MetadataEntry::FNumber(value) => (
            "FNumber",
            CatalogMetadataValue::Rational {
                numerator: value.numerator(),
                denominator: value.denominator(),
            },
        ),
        MetadataEntry::IsoSpeed(value) => (
            "ISOSpeedRatings",
            CatalogMetadataValue::Unsigned(u64::from(value.get())),
        ),
        MetadataEntry::FocalLength(value) => (
            "FocalLength",
            CatalogMetadataValue::Rational {
                numerator: value.numerator(),
                denominator: value.denominator(),
            },
        ),
    };
    let key = CatalogMetadataKey::new("exif", name).expect("static metadata keys are valid");
    let values = CatalogMetadataValues::new(vec![value]).expect("core metadata values are valid");
    (
        key,
        CatalogMetadataCandidate::new(values, CatalogMetadataPrivacy::Public, provenance),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogMetadataEdit {
    Set {
        key: CatalogMetadataKey,
        candidate: CatalogMetadataCandidate,
    },
    Clear {
        key: CatalogMetadataKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMetadataBatchEdit {
    pub photo_id: PhotoId,
    pub expected_revision: Revision,
    pub edits: Vec<CatalogMetadataEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMetadataBatch {
    pub expected_catalog_revision: Revision,
    pub photos: Vec<CatalogMetadataBatchEdit>,
}

impl CatalogMetadataBatch {
    /// Validates deterministic batch bounds and unique photo membership.
    ///
    /// # Errors
    /// Returns an error for an oversized batch or duplicate photo IDs.
    pub fn validate(&self) -> Result<(), CatalogMetadataError> {
        if self.photos.len() > MAX_BATCH_PHOTOS {
            return Err(CatalogMetadataError::BatchTooLarge);
        }
        let mut photos = BTreeSet::new();
        for edit in &self.photos {
            if !photos.insert(edit.photo_id) {
                return Err(CatalogMetadataError::DuplicatePhoto(edit.photo_id));
            }
            if edit.edits.len() > MAX_EDITS_PER_PHOTO {
                return Err(CatalogMetadataError::TooManyEdits);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMetadataBatchReceipt {
    pub catalog_revision: Revision,
    pub photo_revisions: BTreeMap<PhotoId, Revision>,
    pub state_sha256: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogMetadataIndexTerm {
    pub photo_id: PhotoId,
    pub key_sha256: [u8; 32],
    pub value_sha256: [u8; 32],
}

impl CatalogMetadataIndexTerm {
    fn new(photo_id: PhotoId, key: &CatalogMetadataKey, value: &CatalogMetadataValue) -> Self {
        let (key_sha256, value_sha256) = Self::query_hashes(key, value);
        Self {
            photo_id,
            key_sha256,
            value_sha256,
        }
    }

    #[must_use]
    pub fn query_hashes(
        key: &CatalogMetadataKey,
        value: &CatalogMetadataValue,
    ) -> ([u8; 32], [u8; 32]) {
        let mut key_digest = Sha256::new();
        key_digest.update((key.namespace.len() as u64).to_be_bytes());
        key_digest.update(key.namespace.as_bytes());
        key_digest.update((key.name.len() as u64).to_be_bytes());
        key_digest.update(key.name.as_bytes());
        let mut value_digest = Sha256::new();
        value.hash_into(&mut value_digest);
        (key_digest.finalize().into(), value_digest.finalize().into())
    }
}

fn hash_candidate(digest: &mut Sha256, candidate: &CatalogMetadataCandidate) {
    digest.update([candidate.privacy as u8, candidate.provenance.source as u8]);
    digest.update(candidate.provenance.evidence_sha256);
    digest.update((candidate.values.0.len() as u64).to_be_bytes());
    for value in &candidate.values.0 {
        value.hash_into(digest);
    }
}

fn hash_bytes(digest: &mut Sha256, tag: u8, value: &[u8]) {
    digest.update([tag]);
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogMetadataDiagnostic {
    pub photo_id: PhotoId,
    pub revision: Revision,
    pub field_count: usize,
    pub conflict_count: usize,
}

pub trait CatalogMetadataRepository {
    /// Returns the revision covering all metadata documents.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    fn catalog_revision(&self) -> Result<Revision, CatalogMetadataError>;
    /// Loads one durable metadata document.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    fn get(
        &self,
        photo_id: PhotoId,
    ) -> Result<Option<CatalogMetadataDocument>, CatalogMetadataError>;
    /// Applies all photo edits in one atomic catalog transaction.
    ///
    /// # Errors
    /// Returns a validation, revision, storage, or commit error.
    fn apply_batch(
        &mut self,
        batch: &CatalogMetadataBatch,
    ) -> Result<CatalogMetadataBatchReceipt, CatalogMetadataError>;
    /// Finds exact non-sensitive selected values via the derived index.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    fn find(
        &self,
        key: &CatalogMetadataKey,
        value: &CatalogMetadataValue,
    ) -> Result<Vec<PhotoId>, CatalogMetadataError>;
    /// Replaces the derived index from canonical documents.
    ///
    /// # Errors
    /// Returns a typed storage, corruption, or commit error.
    fn rebuild_indexes(&mut self) -> Result<usize, CatalogMetadataError>;
}

mod photo_id_serde {
    use rusttable_core::PhotoId;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &PhotoId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u128(value.get())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PhotoId, D::Error> {
        let value = u128::deserialize(deserializer)?;
        PhotoId::new(value).ok_or_else(|| serde::de::Error::custom("photo ID cannot be zero"))
    }
}
