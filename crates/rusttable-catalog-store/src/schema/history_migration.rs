use std::collections::{BTreeMap, BTreeSet};

use redb::{ReadableTable, WriteTransaction};
use rusttable_catalog::{CanonicalPayload, ContentBlobId, ContentBlobKind, RepositoryError};
use rusttable_core::PhotoId;

use super::{
    HISTORY_BLOB_REFS_TABLE, HISTORY_BLOBS_TABLE, HISTORY_REVISIONS_TABLE, HISTORY_STATE_TABLE,
};

type BlobKey = [u8; 43];
type BlobCounts = BTreeMap<BlobKey, u64>;
type BlobRecords = BTreeMap<BlobKey, Vec<u8>>;
type PhotoBlobCounts = BTreeMap<PhotoId, BlobCounts>;

const LEGACY_MIGRATION_NAME: &str = "history_blob_refs";

pub(super) fn open_history_tables(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    transaction
        .open_table(HISTORY_STATE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(HISTORY_BLOBS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}

struct HistoryGraph {
    expected_refs: BlobCounts,
    expected_blobs: BlobRecords,
    per_photo_refs: PhotoBlobCounts,
    photo_count: usize,
    revision_count: usize,
}

struct MigrationReport {
    changed_refs: usize,
    photo_count: usize,
    revision_count: usize,
    blob_count: usize,
}

pub(super) fn repair_legacy_blob_refs(database: &redb::Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    let graph = collect_history_graph(&transaction)?;
    let actual_refs = read_blob_refs(&transaction)?;

    if actual_refs == graph.expected_refs {
        drop(transaction);
        return Ok(());
    }

    validate_history_blobs(&transaction, &graph.expected_blobs)?;

    let Some(report) = legacy_migration_report(&graph, &actual_refs) else {
        return Err(RepositoryError::CorruptPersistedData);
    };

    rewrite_blob_refs(&transaction, &graph.expected_refs)?;
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)?;
    tracing::info!(
        target: "rusttable.catalog.migration",
        migration = LEGACY_MIGRATION_NAME,
        stage = "complete",
        from = "per_photo",
        to = "global",
        photos = report.photo_count,
        revisions = report.revision_count,
        blobs = report.blob_count,
        changed_refs = report.changed_refs,
        "migrated legacy history blob references"
    );
    Ok(())
}

fn collect_history_graph(transaction: &WriteTransaction) -> Result<HistoryGraph, RepositoryError> {
    let revisions = transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut expected_refs = BlobCounts::new();
    let mut expected_blobs = BlobRecords::new();
    let mut per_photo_refs = PhotoBlobCounts::new();
    let mut photos = BTreeSet::new();
    let mut revision_count = 0_usize;

    for entry in revisions
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
    {
        let (key, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
        let (photo_id, revision_id) = decode_revision_key(key.value())?;
        let revision = crate::history_codec::decode_revision(value.value())
            .map_err(|()| RepositoryError::CorruptPersistedData)?;
        if revision.id() != revision_id || revision.payload().edit().photo_id() != photo_id {
            return Err(RepositoryError::CorruptPersistedData);
        }
        photos.insert(photo_id);
        revision_count = revision_count
            .checked_add(1)
            .ok_or(RepositoryError::CorruptPersistedData)?;
        let photo_refs = per_photo_refs.entry(photo_id).or_default();
        for (blob_key, bytes) in canonical_blob_records(&revision)? {
            insert_blob_record(&mut expected_blobs, blob_key, &bytes)?;
            increment_count(&mut expected_refs, blob_key)?;
            increment_count(photo_refs, blob_key)?;
        }
    }
    drop(revisions);

    Ok(HistoryGraph {
        expected_refs,
        expected_blobs,
        per_photo_refs,
        photo_count: photos.len(),
        revision_count,
    })
}

fn validate_history_blobs(
    transaction: &WriteTransaction,
    expected_blobs: &BlobRecords,
) -> Result<(), RepositoryError> {
    let blobs = transaction
        .open_table(HISTORY_BLOBS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut seen = BTreeSet::new();
    for entry in blobs
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
    {
        let (key, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
        let key = decode_blob_key(key.value())?;
        let expected = expected_blobs
            .get(&key)
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if value.value() != expected.as_slice() {
            return Err(RepositoryError::CorruptPersistedData);
        }
        seen.insert(key);
    }
    if seen != expected_blobs.keys().copied().collect() {
        return Err(RepositoryError::CorruptPersistedData);
    }
    drop(blobs);
    Ok(())
}

fn read_blob_refs(transaction: &WriteTransaction) -> Result<BlobCounts, RepositoryError> {
    let refs = transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut output = BlobCounts::new();
    for entry in refs
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
    {
        let (key, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
        let key = decode_blob_key(key.value())?;
        let count = u64::from_be_bytes(
            value
                .value()
                .try_into()
                .map_err(|_| RepositoryError::CorruptPersistedData)?,
        );
        if count == 0 || output.insert(key, count).is_some() {
            return Err(RepositoryError::CorruptPersistedData);
        }
    }
    drop(refs);
    Ok(output)
}

fn legacy_migration_report(
    graph: &HistoryGraph,
    actual_refs: &BlobCounts,
) -> Option<MigrationReport> {
    if actual_refs.len() != graph.expected_refs.len() {
        return None;
    }
    let mut changed_refs = 0_usize;
    for (key, expected) in &graph.expected_refs {
        let actual = actual_refs.get(key)?;
        if actual == expected {
            continue;
        }
        if *actual == 0 || *actual > *expected {
            return None;
        }
        let contributor_counts = graph
            .per_photo_refs
            .values()
            .filter_map(|photo_refs| photo_refs.get(key))
            .collect::<Vec<_>>();
        if contributor_counts.len() < 2
            || !contributor_counts.iter().any(|count| **count == *actual)
        {
            return None;
        }
        changed_refs = changed_refs.checked_add(1)?;
    }
    (changed_refs > 0).then_some(MigrationReport {
        changed_refs,
        photo_count: graph.photo_count,
        revision_count: graph.revision_count,
        blob_count: graph.expected_blobs.len(),
    })
}

fn rewrite_blob_refs(
    transaction: &WriteTransaction,
    expected_refs: &BlobCounts,
) -> Result<(), RepositoryError> {
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    let old_keys = refs
        .iter()
        .map_err(|_| RepositoryError::Unavailable)?
        .map(|entry| {
            entry
                .map(|(key, _)| key.value().to_vec())
                .map_err(|_| RepositoryError::CorruptPersistedData)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in old_keys {
        refs.remove(key.as_slice())
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    for (key, count) in expected_refs {
        refs.insert(key.as_slice(), count.to_be_bytes().as_slice())
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    Ok(())
}

fn canonical_blob_records(
    revision: &rusttable_catalog::HistoryRevision,
) -> Result<[(BlobKey, Vec<u8>); 3], RepositoryError> {
    let payload = CanonicalPayload::from_history(revision.payload())
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    Ok([
        (
            blob_key(payload.edit().id()),
            payload.edit().bytes().to_vec(),
        ),
        (
            blob_key(payload.mask_blend().id()),
            payload.mask_blend().bytes().to_vec(),
        ),
        (
            blob_key(payload.pipeline().id()),
            payload.pipeline().bytes().to_vec(),
        ),
    ])
}

fn insert_blob_record(
    records: &mut BlobRecords,
    key: BlobKey,
    bytes: &[u8],
) -> Result<(), RepositoryError> {
    if records
        .insert(key, bytes.to_vec())
        .is_some_and(|existing| existing != bytes)
    {
        return Err(RepositoryError::CorruptPersistedData);
    }
    Ok(())
}

fn increment_count(counts: &mut BlobCounts, key: BlobKey) -> Result<(), RepositoryError> {
    let count = counts.entry(key).or_default();
    *count = count
        .checked_add(1)
        .ok_or(RepositoryError::CorruptPersistedData)?;
    Ok(())
}

fn decode_revision_key(
    bytes: &[u8],
) -> Result<(PhotoId, rusttable_catalog::HistoryRevisionId), RepositoryError> {
    if bytes.len() != 24 {
        return Err(RepositoryError::CorruptPersistedData);
    }
    let photo_id = PhotoId::new(u128::from_be_bytes(
        bytes[..16]
            .try_into()
            .map_err(|_| RepositoryError::CorruptPersistedData)?,
    ))
    .ok_or(RepositoryError::CorruptPersistedData)?;
    let revision_id = rusttable_catalog::HistoryRevisionId::new(u64::from_be_bytes(
        bytes[16..]
            .try_into()
            .map_err(|_| RepositoryError::CorruptPersistedData)?,
    ))
    .ok_or(RepositoryError::CorruptPersistedData)?;
    Ok((photo_id, revision_id))
}

pub(super) fn blob_key(id: ContentBlobId) -> BlobKey {
    let mut key = [0; 43];
    key[0] = id.kind().tag();
    key[1..3].copy_from_slice(&id.schema().to_be_bytes());
    key[3..11].copy_from_slice(&id.length().to_be_bytes());
    key[11..].copy_from_slice(&id.digest());
    key
}

fn decode_blob_key(bytes: &[u8]) -> Result<BlobKey, RepositoryError> {
    if bytes.len() != 43 {
        return Err(RepositoryError::CorruptPersistedData);
    }
    let kind = match bytes[0] {
        1 => ContentBlobKind::Edit,
        2 => ContentBlobKind::MaskBlend,
        3 => ContentBlobKind::Pipeline,
        _ => return Err(RepositoryError::CorruptPersistedData),
    };
    let schema = u16::from_be_bytes(
        bytes[1..3]
            .try_into()
            .map_err(|_| RepositoryError::CorruptPersistedData)?,
    );
    let length = u64::from_be_bytes(
        bytes[3..11]
            .try_into()
            .map_err(|_| RepositoryError::CorruptPersistedData)?,
    );
    let digest = bytes[11..]
        .try_into()
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let _ = ContentBlobId::from_parts(kind, schema, length, digest);
    bytes
        .try_into()
        .map_err(|_| RepositoryError::CorruptPersistedData)
}
