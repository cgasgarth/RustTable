// Durable history snapshot documents derived from `src/common/history_snapshot.c`.
//
// The native code stores rows below a history end and restores transient module
// state later.  This Rust responsibility is deliberately bounded to the
// durable ordered prefix, operation metadata, and opaque canonical payloads.
// It does not claim native equivalence for transient mask restoration or module
// order; those responsibilities remain explicitly deferred to their owners.

use std::collections::BTreeMap;
use std::fmt;

use rusttable_core::{OperationId, OperationKey, PhotoId};
use sha2::{Digest, Sha256};

use super::canonical::{
    CanonicalEncodingError, canonical_edit_bytes, canonical_mask_blend_bytes,
    canonical_pipeline_bytes,
};
use super::state::HistoryState;
use super::types::{HistoryBranch, HistoryRevision, HistoryRevisionId, HistorySnapshotId};

pub const HISTORY_SNAPSHOT_SCHEMA_VERSION: u16 = 1;

/// Maximum encoded size accepted by the snapshot boundary.
///
/// The limit is deliberately larger than one canonical payload because a
/// snapshot may contain several revisions, while still bounding allocations
/// made while decoding untrusted catalog bytes.
pub const MAX_HISTORY_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

const MAX_HISTORY_SNAPSHOT_REVISIONS: usize = 1_000_000;
const MAX_HISTORY_SNAPSHOT_OPERATIONS: usize = 1_000_000;
const MAX_OPERATION_KEY_BYTES: usize = 128;
const MAX_CANONICAL_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;
const MAX_REVISION_PAYLOAD_BYTES: usize = MAX_CANONICAL_PAYLOAD_BYTES * 3 + 32;
const SNAPSHOT_MAGIC: &[u8; 8] = b"RTHSNAP\0";
const REVISION_PAYLOAD_MAGIC: &[u8; 8] = b"RTHREV\0\x01";
const EDIT_MAGIC: &[u8; 8] = b"RTEDIT\0\x01";
const MASK_BLEND_MAGIC: &[u8; 13] = b"RTMASKBLEND\0\x01";
const PIPELINE_MAGIC: &[u8; 8] = b"RTPIPE\0\x01";

/// A single native-style history operation in snapshot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshotOperation {
    operation_id: OperationId,
    key: String,
    enabled: bool,
}

impl HistorySnapshotOperation {
    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// One immutable revision copied into a history snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshotRevision {
    id: HistoryRevisionId,
    order: u64,
    operations: Vec<HistorySnapshotOperation>,
    canonical_payload: Vec<u8>,
}

impl HistorySnapshotRevision {
    #[must_use]
    pub const fn id(&self) -> HistoryRevisionId {
        self.id
    }

    /// Returns the zero-based native history number for this revision.
    #[must_use]
    pub const fn order(&self) -> u64 {
        self.order
    }

    #[must_use]
    pub fn operations(&self) -> &[HistorySnapshotOperation] {
        &self.operations
    }

    /// Returns the opaque canonical payload retained for repository restore.
    #[must_use]
    pub fn canonical_payload(&self) -> &[u8] {
        &self.canonical_payload
    }

    #[must_use]
    pub fn payload_hash(&self) -> [u8; 32] {
        Sha256::digest(&self.canonical_payload).into()
    }
}

/// A database-independent, ordered snapshot of the current history prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshotDocument {
    photo_id: PhotoId,
    snapshot_id: HistorySnapshotId,
    history_end: u64,
    revisions: Vec<HistorySnapshotRevision>,
}

impl HistorySnapshotDocument {
    /// Creates the native empty-history snapshot.
    #[must_use]
    pub const fn empty(photo_id: PhotoId, snapshot_id: HistorySnapshotId) -> Self {
        Self {
            photo_id,
            snapshot_id,
            history_end: 0,
            revisions: Vec::new(),
        }
    }

    /// Captures the ordered prefix ending at the branch cursor.
    ///
    /// The native implementation copies rows with `num < history_end`.  A
    /// Rust branch has an explicit lineage and cursor, so the equivalent
    /// prefix is selected before any payload is copied.  Revisions after an
    /// undo cursor are redo state and are intentionally not part of a current
    /// snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the branch cursor or lineage does not resolve to
    /// the supplied immutable revisions, when a revision belongs to another
    /// photo, or when canonical payload encoding fails.
    ///
    /// # Panics
    ///
    /// Panics only if the reserved nonzero main-branch ID is changed to zero.
    pub fn capture(
        photo_id: PhotoId,
        snapshot_id: HistorySnapshotId,
        branch: &HistoryBranch,
        revisions: &[HistoryRevision],
    ) -> Result<Self, HistorySnapshotBuildError> {
        let mut by_id = BTreeMap::new();
        for revision in revisions {
            if by_id.insert(revision.id(), revision).is_some() {
                return Err(HistorySnapshotBuildError::DuplicateRevision(revision.id()));
            }
        }

        let main = super::types::HistoryBranchId::new(1).expect("literal branch ID is nonzero");
        if (branch.id() == main) != branch.origin().is_none()
            || branch
                .origin()
                .is_some_and(|origin| !branch.lineage().contains(&origin))
        {
            return Err(HistorySnapshotBuildError::InvalidBranchOrigin);
        }
        if branch.lineage().windows(2).any(|pair| {
            by_id
                .get(&pair[1])
                .is_none_or(|revision| revision.parent() != Some(pair[0]))
        }) {
            return Err(HistorySnapshotBuildError::InvalidBranchLineage);
        }
        if branch.lineage().first().is_some_and(|id| {
            by_id
                .get(id)
                .is_none_or(|revision| revision.parent().is_some())
        }) {
            return Err(HistorySnapshotBuildError::InvalidBranchLineage);
        }
        if has_duplicate_ids(branch.lineage()) {
            return Err(HistorySnapshotBuildError::InvalidBranchLineage);
        }

        let current_end = match branch.cursor() {
            None if branch.lineage().is_empty() => 0,
            None => return Err(HistorySnapshotBuildError::InvalidBranchCursor),
            Some(cursor) => branch
                .lineage()
                .iter()
                .position(|revision| *revision == cursor)
                .map(|index| index + 1)
                .ok_or(HistorySnapshotBuildError::CursorNotInBranch(cursor))?,
        };
        let history_end =
            u64::try_from(current_end).map_err(|_| HistorySnapshotBuildError::TooLarge {
                actual: current_end,
            })?;

        if current_end > MAX_HISTORY_SNAPSHOT_REVISIONS {
            return Err(HistorySnapshotBuildError::TooLarge {
                actual: current_end,
            });
        }

        let mut captured = Vec::with_capacity(current_end);
        for (order, revision_id) in branch.lineage()[..current_end].iter().copied().enumerate() {
            let revision = by_id
                .get(&revision_id)
                .copied()
                .ok_or(HistorySnapshotBuildError::MissingRevision(revision_id))?;
            if revision.payload().edit().photo_id() != photo_id {
                return Err(HistorySnapshotBuildError::PhotoMismatch {
                    revision: revision_id,
                });
            }
            captured.push(snapshot_revision(
                u64::try_from(order)
                    .map_err(|_| HistorySnapshotBuildError::TooLarge { actual: order })?,
                revision,
            )?);
        }

        let document = Self {
            photo_id,
            snapshot_id,
            history_end,
            revisions: captured,
        };
        if document.serialized_size() > MAX_HISTORY_SNAPSHOT_BYTES {
            return Err(HistorySnapshotBuildError::TooLarge {
                actual: document.serialized_size(),
            });
        }
        Ok(document)
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn snapshot_id(&self) -> HistorySnapshotId {
        self.snapshot_id
    }

    /// The number of ordered revisions copied into this snapshot.
    #[must_use]
    pub const fn history_end(&self) -> u64 {
        self.history_end
    }

    #[must_use]
    pub fn revisions(&self) -> &[HistorySnapshotRevision] {
        &self.revisions
    }

    /// Selects the current revision, matching the last row below
    /// `history_end`; disabled rows do not move the selection.
    #[must_use]
    pub fn current_revision(&self) -> Option<HistoryRevisionId> {
        self.revisions.last().map(HistorySnapshotRevision::id)
    }

    /// Returns all current-history operations in their persisted order.
    pub fn current_history(&self) -> impl Iterator<Item = &HistorySnapshotOperation> {
        self.revisions
            .iter()
            .flat_map(|revision| revision.operations.iter())
    }

    /// Returns current-history operations while preserving the native enabled
    /// filter: disabled operations are omitted, enabled operations retain
    /// their original order.
    pub fn enabled_history(&self) -> impl Iterator<Item = &HistorySnapshotOperation> {
        self.current_history()
            .filter(|operation| operation.is_enabled())
    }

    #[must_use]
    pub fn revision(&self, id: HistoryRevisionId) -> Option<&HistorySnapshotRevision> {
        self.revisions.iter().find(|revision| revision.id() == id)
    }

    /// Verifies that every copied revision still matches the current photo graph.
    ///
    /// Snapshot storage is intentionally separate from the mutable history state,
    /// so a restart must not accept a document whose IDs or copied payloads have
    /// outlived the graph that produced it.
    #[must_use]
    pub fn matches_current_graph(&self, state: &HistoryState) -> bool {
        self.photo_id == state.photo_id()
            && self.history_end == self.revisions.len() as u64
            && (self.revisions.is_empty() == (state.revisions().next().is_none()))
            && self.revisions.windows(2).all(|pair| {
                state
                    .revision(pair[1].id())
                    .is_some_and(|revision| revision.parent() == Some(pair[0].id()))
            })
            && self.revisions.iter().enumerate().all(|(order, snapshot)| {
                let Some(revision) = state.revision(snapshot.id()) else {
                    return false;
                };
                u64::try_from(order).is_ok_and(|order| {
                    snapshot.order() == order
                        && snapshot_revision(order, revision)
                            .is_ok_and(|expected| expected == *snapshot)
                })
            })
    }

    /// Serializes a bounded, versioned representation suitable for a
    /// repository boundary.  No database or application state is touched.
    ///
    /// # Errors
    ///
    /// Returns an error if the document exceeds the encoded size limit or an
    /// internal invariant needed by the wire format is not satisfied.
    pub fn serialize(&self) -> Result<Vec<u8>, HistorySnapshotSerializationError> {
        if self.history_end != self.revisions.len() as u64
            || self.revisions.len() > MAX_HISTORY_SNAPSHOT_REVISIONS
        {
            return Err(HistorySnapshotSerializationError::InvalidState);
        }

        let mut bytes = Vec::with_capacity(self.serialized_size().min(MAX_HISTORY_SNAPSHOT_BYTES));
        bytes.extend_from_slice(SNAPSHOT_MAGIC);
        bytes.extend_from_slice(&HISTORY_SNAPSHOT_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.photo_id.get().to_le_bytes());
        bytes.extend_from_slice(&self.snapshot_id.get().to_le_bytes());
        bytes.extend_from_slice(&self.history_end.to_le_bytes());
        put_count(&mut bytes, self.revisions.len())?;
        for revision in &self.revisions {
            bytes.extend_from_slice(&revision.id.get().to_le_bytes());
            bytes.extend_from_slice(&revision.order.to_le_bytes());
            put_bytes(&mut bytes, &revision.canonical_payload)?;
            if revision.operations.len() > MAX_HISTORY_SNAPSHOT_OPERATIONS {
                return Err(HistorySnapshotSerializationError::TooLarge {
                    actual: revision.operations.len(),
                });
            }
            put_count(&mut bytes, revision.operations.len())?;
            for operation in &revision.operations {
                bytes.extend_from_slice(&operation.operation_id.get().to_le_bytes());
                put_text(&mut bytes, &operation.key)?;
                bytes.push(u8::from(operation.enabled));
            }
        }
        checked_bytes(bytes)
    }

    /// Computes the content identity of the complete ordered snapshot.
    ///
    /// The hash covers the versioned wire representation, including ordering,
    /// operation enabled state, and canonical payload bytes.
    ///
    /// # Errors
    ///
    /// Returns the same bounded-size or invariant error as [`Self::serialize`].
    pub fn stable_hash(&self) -> Result<[u8; 32], HistorySnapshotSerializationError> {
        Ok(Sha256::digest(self.serialize()?).into())
    }

    /// Decodes a snapshot without accepting truncated, reordered, oversized,
    /// or otherwise malformed payloads.
    ///
    /// # Errors
    ///
    /// Returns a typed error and never returns a partially decoded document.
    pub fn deserialize(bytes: &[u8]) -> Result<Self, HistorySnapshotDecodeError> {
        if bytes.len() > MAX_HISTORY_SNAPSHOT_BYTES {
            return Err(HistorySnapshotDecodeError::TooLarge {
                actual: bytes.len(),
            });
        }
        let mut reader = Reader::new(bytes);
        if reader.read_exact(SNAPSHOT_MAGIC.len())? != SNAPSHOT_MAGIC {
            return Err(HistorySnapshotDecodeError::InvalidMagic);
        }
        let version = reader.read_u16()?;
        if version != HISTORY_SNAPSHOT_SCHEMA_VERSION {
            return Err(HistorySnapshotDecodeError::UnsupportedVersion(version));
        }
        let photo_id =
            PhotoId::new(reader.read_u128()?).ok_or(HistorySnapshotDecodeError::InvalidId)?;
        let snapshot_id = HistorySnapshotId::new(reader.read_u64()?)
            .ok_or(HistorySnapshotDecodeError::InvalidId)?;
        let history_end = reader.read_u64()?;
        let revision_count = reader.read_count(MAX_HISTORY_SNAPSHOT_REVISIONS)?;
        if history_end != revision_count as u64 {
            return Err(HistorySnapshotDecodeError::InvalidHistoryEnd);
        }
        if revision_count > reader.remaining() / 32 {
            return Err(HistorySnapshotDecodeError::InvalidCount);
        }

        let mut revisions = Vec::with_capacity(revision_count);
        for expected_order in 0..revision_count {
            let id = HistoryRevisionId::new(reader.read_u64()?)
                .ok_or(HistorySnapshotDecodeError::InvalidId)?;
            let order = reader.read_u64()?;
            if order != expected_order as u64
                || revisions
                    .iter()
                    .any(|revision: &HistorySnapshotRevision| revision.id == id)
            {
                return Err(HistorySnapshotDecodeError::InvalidOrder);
            }
            let canonical_payload = reader.read_bytes(MAX_REVISION_PAYLOAD_BYTES)?;
            if !valid_revision_payload(&canonical_payload) {
                return Err(HistorySnapshotDecodeError::InvalidPayload);
            }
            let operation_count = reader.read_count(MAX_HISTORY_SNAPSHOT_OPERATIONS)?;
            if operation_count > reader.remaining() / 18 {
                return Err(HistorySnapshotDecodeError::InvalidCount);
            }
            let mut operations = Vec::with_capacity(operation_count);
            for _ in 0..operation_count {
                let operation_id = OperationId::new(reader.read_u128()?)
                    .ok_or(HistorySnapshotDecodeError::InvalidId)?;
                let key_bytes = reader.read_bytes(MAX_OPERATION_KEY_BYTES)?;
                let key = String::from_utf8(key_bytes)
                    .map_err(|_| HistorySnapshotDecodeError::InvalidUtf8)?;
                OperationKey::new(key.clone())
                    .map_err(|_| HistorySnapshotDecodeError::InvalidOperationKey)?;
                let enabled = match reader.read_u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(HistorySnapshotDecodeError::InvalidBoolean),
                };
                operations.push(HistorySnapshotOperation {
                    operation_id,
                    key,
                    enabled,
                });
            }
            revisions.push(HistorySnapshotRevision {
                id,
                order,
                operations,
                canonical_payload,
            });
        }
        if !reader.is_empty() {
            return Err(HistorySnapshotDecodeError::TrailingBytes);
        }
        Ok(Self {
            photo_id,
            snapshot_id,
            history_end,
            revisions,
        })
    }

    fn serialized_size(&self) -> usize {
        let revision_size = self
            .revisions
            .iter()
            .map(|revision| {
                8 + 8
                    + 4
                    + revision.canonical_payload.len()
                    + 4
                    + revision
                        .operations
                        .iter()
                        .map(|operation| 16 + 4 + operation.key.len() + 1)
                        .sum::<usize>()
            })
            .sum::<usize>();
        8 + 2 + 16 + 8 + 8 + 4 + revision_size
    }
}

fn has_duplicate_ids(ids: &[HistoryRevisionId]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    ids.iter().any(|id| !seen.insert(*id))
}

fn snapshot_revision(
    order: u64,
    revision: &HistoryRevision,
) -> Result<HistorySnapshotRevision, HistorySnapshotBuildError> {
    let payload = revision.payload();
    let edit = canonical_edit_bytes(payload.edit())
        .map_err(HistorySnapshotBuildError::CanonicalEncoding)?;
    let mask_blend = canonical_mask_blend_bytes(payload.mask_bytes(), payload.pipeline_bytes())
        .map_err(HistorySnapshotBuildError::CanonicalEncoding)?;
    let pipeline = canonical_pipeline_bytes(payload.pipeline_bytes())
        .map_err(HistorySnapshotBuildError::CanonicalEncoding)?;
    let mut canonical_payload = Vec::with_capacity(
        REVISION_PAYLOAD_MAGIC.len() + 4 + edit.len() + 4 + mask_blend.len() + 4 + pipeline.len(),
    );
    canonical_payload.extend_from_slice(REVISION_PAYLOAD_MAGIC);
    put_bytes_unchecked(&mut canonical_payload, &edit);
    put_bytes_unchecked(&mut canonical_payload, &mask_blend);
    put_bytes_unchecked(&mut canonical_payload, &pipeline);
    if canonical_payload.len() > MAX_REVISION_PAYLOAD_BYTES {
        return Err(HistorySnapshotBuildError::TooLarge {
            actual: canonical_payload.len(),
        });
    }
    let operations = payload
        .edit()
        .operations()
        .map(|operation| HistorySnapshotOperation {
            operation_id: operation.id(),
            key: operation.key().as_str().to_owned(),
            enabled: operation.is_enabled(),
        })
        .collect();
    Ok(HistorySnapshotRevision {
        id: revision.id(),
        order,
        operations,
        canonical_payload,
    })
}

fn valid_revision_payload(payload: &[u8]) -> bool {
    let mut reader = Reader::new(payload);
    let Ok(magic) = reader.read_exact(REVISION_PAYLOAD_MAGIC.len()) else {
        return false;
    };
    if magic != REVISION_PAYLOAD_MAGIC {
        return false;
    }
    for expected_magic in [EDIT_MAGIC.as_slice(), MASK_BLEND_MAGIC, PIPELINE_MAGIC] {
        let Ok(segment) = reader.read_bytes(MAX_CANONICAL_PAYLOAD_BYTES) else {
            return false;
        };
        if !segment.starts_with(expected_magic) {
            return false;
        }
    }
    reader.is_empty()
}

fn put_count(bytes: &mut Vec<u8>, count: usize) -> Result<(), HistorySnapshotSerializationError> {
    let count =
        u32::try_from(count).map_err(|_| HistorySnapshotSerializationError::LengthOverflow)?;
    bytes.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn put_text(bytes: &mut Vec<u8>, text: &str) -> Result<(), HistorySnapshotSerializationError> {
    put_bytes(bytes, text.as_bytes())
}

fn put_bytes(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), HistorySnapshotSerializationError> {
    let length = u32::try_from(value.len())
        .map_err(|_| HistorySnapshotSerializationError::LengthOverflow)?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

fn put_bytes_unchecked(bytes: &mut Vec<u8>, value: &[u8]) {
    let length = u32::try_from(value.len()).expect("canonical payload length is bounded");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value);
}

fn checked_bytes(bytes: Vec<u8>) -> Result<Vec<u8>, HistorySnapshotSerializationError> {
    if bytes.len() > MAX_HISTORY_SNAPSHOT_BYTES {
        Err(HistorySnapshotSerializationError::TooLarge {
            actual: bytes.len(),
        })
    } else {
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySnapshotBuildError {
    DuplicateRevision(HistoryRevisionId),
    MissingRevision(HistoryRevisionId),
    CursorNotInBranch(HistoryRevisionId),
    InvalidBranchCursor,
    InvalidBranchOrigin,
    InvalidBranchLineage,
    PhotoMismatch { revision: HistoryRevisionId },
    CanonicalEncoding(CanonicalEncodingError),
    TooLarge { actual: usize },
}

impl fmt::Display for HistorySnapshotBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateRevision(id) => write!(formatter, "history revision {id} is duplicated"),
            Self::MissingRevision(id) => write!(formatter, "history revision {id} is missing"),
            Self::CursorNotInBranch(id) => {
                write!(formatter, "history cursor {id} is not in branch lineage")
            }
            Self::InvalidBranchCursor => formatter.write_str("history branch cursor is invalid"),
            Self::InvalidBranchOrigin => formatter.write_str("history branch origin is invalid"),
            Self::InvalidBranchLineage => formatter.write_str("history branch lineage is invalid"),
            Self::PhotoMismatch { revision } => write!(
                formatter,
                "history revision {revision} belongs to another photo"
            ),
            Self::CanonicalEncoding(source) => write!(
                formatter,
                "history snapshot payload encoding failed: {source}"
            ),
            Self::TooLarge { actual } => write!(
                formatter,
                "history snapshot contains {actual} oversized items"
            ),
        }
    }
}

impl std::error::Error for HistorySnapshotBuildError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySnapshotSerializationError {
    LengthOverflow,
    TooLarge { actual: usize },
    InvalidState,
}

impl fmt::Display for HistorySnapshotSerializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthOverflow => {
                formatter.write_str("history snapshot length overflows wire width")
            }
            Self::TooLarge { actual } => write!(formatter, "history snapshot is {actual} bytes"),
            Self::InvalidState => formatter.write_str("history snapshot state is not serializable"),
        }
    }
}

impl std::error::Error for HistorySnapshotSerializationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySnapshotDecodeError {
    TooShort,
    InvalidMagic,
    UnsupportedVersion(u16),
    Truncated,
    InvalidId,
    InvalidCount,
    InvalidHistoryEnd,
    InvalidOrder,
    InvalidPayload,
    InvalidUtf8,
    InvalidOperationKey,
    InvalidBoolean,
    TrailingBytes,
    TooLarge { actual: usize },
}

impl fmt::Display for HistorySnapshotDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => formatter.write_str("history snapshot payload is too short"),
            Self::InvalidMagic => formatter.write_str("history snapshot payload has invalid magic"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "history snapshot schema {version} is unsupported"
            ),
            Self::Truncated => formatter.write_str("history snapshot payload is truncated"),
            Self::InvalidId => formatter.write_str("history snapshot contains a zero ID"),
            Self::InvalidCount => formatter.write_str("history snapshot contains an invalid count"),
            Self::InvalidHistoryEnd => {
                formatter.write_str("history snapshot history end does not match its revisions")
            }
            Self::InvalidOrder => {
                formatter.write_str("history snapshot revision ordering is invalid")
            }
            Self::InvalidPayload => {
                formatter.write_str("history snapshot canonical payload is invalid")
            }
            Self::InvalidUtf8 => formatter.write_str("history snapshot contains invalid UTF-8"),
            Self::InvalidOperationKey => {
                formatter.write_str("history snapshot contains an invalid operation key")
            }
            Self::InvalidBoolean => {
                formatter.write_str("history snapshot contains an invalid enabled flag")
            }
            Self::TrailingBytes => {
                formatter.write_str("history snapshot payload has trailing bytes")
            }
            Self::TooLarge { actual } => {
                write!(formatter, "history snapshot payload is {actual} bytes")
            }
        }
    }
}

impl std::error::Error for HistorySnapshotDecodeError {}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8], HistorySnapshotDecodeError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(HistorySnapshotDecodeError::TooShort)?;
        if end > self.bytes.len() {
            return Err(HistorySnapshotDecodeError::Truncated);
        }
        let value = &self.bytes[self.position..end];
        self.position = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8, HistorySnapshotDecodeError> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, HistorySnapshotDecodeError> {
        Ok(u16::from_le_bytes(
            self.read_exact(2)?.try_into().expect("length checked"),
        ))
    }

    fn read_u64(&mut self) -> Result<u64, HistorySnapshotDecodeError> {
        Ok(u64::from_le_bytes(
            self.read_exact(8)?.try_into().expect("length checked"),
        ))
    }

    fn read_u128(&mut self) -> Result<u128, HistorySnapshotDecodeError> {
        Ok(u128::from_le_bytes(
            self.read_exact(16)?.try_into().expect("length checked"),
        ))
    }

    fn read_count(&mut self, maximum: usize) -> Result<usize, HistorySnapshotDecodeError> {
        let count = usize::try_from(self.read_u32()?)
            .map_err(|_| HistorySnapshotDecodeError::InvalidCount)?;
        if count > maximum {
            Err(HistorySnapshotDecodeError::InvalidCount)
        } else {
            Ok(count)
        }
    }

    fn read_u32(&mut self) -> Result<u32, HistorySnapshotDecodeError> {
        Ok(u32::from_le_bytes(
            self.read_exact(4)?.try_into().expect("length checked"),
        ))
    }

    fn read_bytes(&mut self, maximum: usize) -> Result<Vec<u8>, HistorySnapshotDecodeError> {
        let length = usize::try_from(self.read_u32()?)
            .map_err(|_| HistorySnapshotDecodeError::InvalidCount)?;
        if length > maximum {
            return Err(HistorySnapshotDecodeError::InvalidCount);
        }
        Ok(self.read_exact(length)?.to_vec())
    }

    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    const fn is_empty(&self) -> bool {
        self.position == self.bytes.len()
    }
}

impl From<HistorySnapshotDecodeError> for HistorySnapshotSerializationError {
    fn from(_: HistorySnapshotDecodeError) -> Self {
        Self::InvalidState
    }
}
