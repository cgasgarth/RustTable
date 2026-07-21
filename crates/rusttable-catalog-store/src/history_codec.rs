use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_catalog::{
    HistoryBranch, HistoryBranchId, HistoryCursor, HistoryEvidence, HistoryEvidenceKind,
    HistoryJournalEntry, HistoryOperationKind, HistoryOperationSummary, HistoryProvenance,
    HistoryRevision, HistoryRevisionId, HistorySnapshot, HistorySnapshotId, HistoryStateSnapshot,
    HistoryVersion,
};
use rusttable_core::{Edit, OperationId, OperationKey, PhotoId};

const FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredMeta {
    version: u8,
    photo_id: [u8; 16],
    history_version: u64,
    next_revision_id: u64,
    next_branch_id: u64,
    next_snapshot_id: u64,
    active_branch: u64,
    branches: Vec<StoredBranch>,
    snapshots: Vec<StoredSnapshot>,
    evidence: Vec<StoredEvidence>,
    #[serde(default)]
    commit_sequence: u64,
    #[serde(default)]
    journal: Vec<StoredJournal>,
    #[serde(default)]
    provenance: Vec<StoredProvenanceRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredBranch {
    id: u64,
    name: Vec<u8>,
    origin: Option<u64>,
    lineage: Vec<u64>,
    cursor: Option<u64>,
    redo: Vec<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSnapshot {
    id: u64,
    name: Vec<u8>,
    branch: u64,
    revision: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEvidence {
    revision: u64,
    kind: u8,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredJournal {
    sequence: u64,
    kind: u8,
    revision: Option<u64>,
    before_branch: u64,
    before_revision: Option<u64>,
    after_branch: u64,
    after_revision: Option<u64>,
    restore_from: Option<u64>,
    provenance: StoredProvenance,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredProvenance {
    kind: u8,
    schema: u32,
    source_id: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRevision {
    version: u8,
    id: u64,
    parent: Option<u64>,
    edit: Vec<u8>,
    mask: Vec<u8>,
    pipeline: Vec<u8>,
    summary: StoredSummary,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredSummary {
    kind: u8,
    operation_id: Option<[u8; 16]>,
    operation_key: Option<Vec<u8>>,
    label: Vec<u8>,
}

pub(crate) fn encode_meta(snapshot: &HistoryStateSnapshot) -> Result<Vec<u8>, ()> {
    let branches = snapshot
        .branches()
        .iter()
        .map(|branch| StoredBranch {
            id: branch.id().get(),
            name: branch.name().as_bytes().to_vec(),
            origin: branch.origin().map(HistoryRevisionId::get),
            lineage: branch
                .lineage()
                .iter()
                .map(|revision| revision.get())
                .collect(),
            cursor: branch.cursor().map(HistoryRevisionId::get),
            redo: branch
                .redo()
                .iter()
                .map(|revision| revision.get())
                .collect(),
        })
        .collect();
    let snapshots = snapshot
        .snapshots()
        .iter()
        .map(|value| StoredSnapshot {
            id: value.id().get(),
            name: value.name().as_bytes().to_vec(),
            branch: value.cursor().branch().get(),
            revision: value.cursor().revision().map(HistoryRevisionId::get),
        })
        .collect();
    let evidence = snapshot
        .evidence()
        .iter()
        .map(|value| StoredEvidence {
            revision: value.revision().get(),
            kind: encode_evidence_kind(value.kind()),
        })
        .collect();
    let journal = snapshot
        .journal()
        .iter()
        .map(|entry| StoredJournal {
            sequence: entry.sequence(),
            kind: encode_operation_kind(entry.kind()),
            revision: entry.revision().map(HistoryRevisionId::get),
            before_branch: entry.before().branch().get(),
            before_revision: entry.before().revision().map(HistoryRevisionId::get),
            after_branch: entry.after().branch().get(),
            after_revision: entry.after().revision().map(HistoryRevisionId::get),
            restore_from: entry.restore_from().map(HistoryRevisionId::get),
            provenance: encode_provenance(entry.provenance()),
        })
        .collect();
    let provenance = snapshot
        .provenance()
        .iter()
        .map(|(revision, value)| StoredProvenanceRecord {
            revision: revision.get(),
            provenance: encode_provenance(value),
        })
        .collect();
    to_allocvec(&StoredMeta {
        version: FORMAT_VERSION,
        photo_id: snapshot.photo_id().get().to_be_bytes(),
        history_version: snapshot.version().get(),
        next_revision_id: snapshot.next_revision_id(),
        next_branch_id: snapshot.next_branch_id(),
        next_snapshot_id: snapshot.next_snapshot_id(),
        active_branch: snapshot.active_branch().get(),
        branches,
        snapshots,
        evidence,
        commit_sequence: snapshot.commit_sequence(),
        journal,
        provenance,
    })
    .map_err(|_| ())
}

pub(crate) fn decode_meta(bytes: &[u8]) -> Result<DecodedMeta, ()> {
    let stored: StoredMeta = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != FORMAT_VERSION {
        return Err(());
    }
    let photo_id = PhotoId::new(u128::from_be_bytes(stored.photo_id)).ok_or(())?;
    let active_branch = HistoryBranchId::new(stored.active_branch).ok_or(())?;
    let branches = stored
        .branches
        .into_iter()
        .map(|branch| {
            Ok(HistoryBranch::from_parts(
                HistoryBranchId::new(branch.id).ok_or(())?,
                text(&branch.name)?,
                optional_revision(branch.origin)?,
                branch
                    .lineage
                    .into_iter()
                    .map(|id| HistoryRevisionId::new(id).ok_or(()))
                    .collect::<Result<Vec<_>, _>>()?,
                optional_revision(branch.cursor)?,
                branch
                    .redo
                    .into_iter()
                    .map(|id| HistoryRevisionId::new(id).ok_or(()))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots = stored
        .snapshots
        .into_iter()
        .map(|snapshot| {
            Ok(HistorySnapshot::from_parts(
                HistorySnapshotId::new(snapshot.id).ok_or(())?,
                text(&snapshot.name)?,
                HistoryCursor::new(
                    HistoryBranchId::new(snapshot.branch).ok_or(())?,
                    optional_revision(snapshot.revision)?,
                ),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let evidence = stored
        .evidence
        .into_iter()
        .map(|value| {
            Ok(HistoryEvidence::new(
                HistoryRevisionId::new(value.revision).ok_or(())?,
                decode_evidence_kind(value.kind)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let journal = stored
        .journal
        .into_iter()
        .map(|entry| {
            Ok(HistoryJournalEntry::new(
                entry.sequence,
                decode_operation_kind(entry.kind)?,
                optional_revision(entry.revision)?,
                cursor(entry.before_branch, entry.before_revision)?,
                cursor(entry.after_branch, entry.after_revision)?,
                optional_revision(entry.restore_from)?,
                decode_provenance(&entry.provenance)?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let provenance = stored
        .provenance
        .into_iter()
        .map(|record| {
            Ok((
                HistoryRevisionId::new(record.revision).ok_or(())?,
                decode_provenance(&record.provenance)?,
            ))
        })
        .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
    Ok(DecodedMeta {
        photo_id,
        version: HistoryVersion::from_u64(stored.history_version),
        next_revision_id: stored.next_revision_id,
        next_branch_id: stored.next_branch_id,
        next_snapshot_id: stored.next_snapshot_id,
        active_branch,
        branches,
        snapshots,
        evidence,
        commit_sequence: stored.commit_sequence,
        journal,
        provenance,
    })
}

pub(crate) fn encode_revision(revision: &HistoryRevision) -> Result<Vec<u8>, ()> {
    let payload = revision.payload();
    let summary = payload.summary();
    to_allocvec(&StoredRevision {
        version: FORMAT_VERSION,
        id: revision.id().get(),
        parent: revision.parent().map(HistoryRevisionId::get),
        edit: super::edit_codec::encode(payload.edit())?,
        mask: payload.mask_bytes().to_vec(),
        pipeline: payload.pipeline_bytes().to_vec(),
        summary: StoredSummary {
            kind: encode_operation_kind(summary.kind()),
            operation_id: summary
                .operation_id()
                .map(|value| value.get().to_be_bytes()),
            operation_key: summary
                .operation_key()
                .map(|value| value.as_str().as_bytes().to_vec()),
            label: summary.label().as_bytes().to_vec(),
        },
    })
    .map_err(|_| ())
}

pub(crate) fn decode_revision(bytes: &[u8]) -> Result<HistoryRevision, ()> {
    let stored: StoredRevision = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != FORMAT_VERSION {
        return Err(());
    }
    let summary = HistoryOperationSummary::new(
        decode_operation_kind(stored.summary.kind)?,
        stored
            .summary
            .operation_id
            .map(|value| OperationId::new(u128::from_be_bytes(value)).ok_or(()))
            .transpose()?,
        stored
            .summary
            .operation_key
            .map(|value| OperationKey::new(text(&value)?).map_err(|_| ()))
            .transpose()?,
        text(&stored.summary.label)?,
    )
    .map_err(|_| ())?;
    let edit: Edit = super::edit_codec::decode(&stored.edit)?;
    Ok(HistoryRevision::new(
        HistoryRevisionId::new(stored.id).ok_or(())?,
        optional_revision(stored.parent)?,
        rusttable_catalog::HistoryPayload::new(edit, stored.mask, stored.pipeline, summary),
    ))
}

pub(crate) struct DecodedMeta {
    pub(crate) photo_id: PhotoId,
    pub(crate) version: HistoryVersion,
    pub(crate) next_revision_id: u64,
    pub(crate) next_branch_id: u64,
    pub(crate) next_snapshot_id: u64,
    pub(crate) active_branch: HistoryBranchId,
    pub(crate) branches: Vec<HistoryBranch>,
    pub(crate) snapshots: Vec<HistorySnapshot>,
    pub(crate) evidence: Vec<HistoryEvidence>,
    pub(crate) commit_sequence: u64,
    pub(crate) journal: Vec<HistoryJournalEntry>,
    pub(crate) provenance: std::collections::BTreeMap<HistoryRevisionId, HistoryProvenance>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredProvenanceRecord {
    revision: u64,
    provenance: StoredProvenance,
}

fn text(bytes: &[u8]) -> Result<String, ()> {
    String::from_utf8(bytes.to_vec()).map_err(|_| ())
}

fn optional_revision(value: Option<u64>) -> Result<Option<HistoryRevisionId>, ()> {
    value
        .map(|id| HistoryRevisionId::new(id).ok_or(()))
        .transpose()
}

fn cursor(branch: u64, revision: Option<u64>) -> Result<HistoryCursor, ()> {
    Ok(HistoryCursor::new(
        HistoryBranchId::new(branch).ok_or(())?,
        optional_revision(revision)?,
    ))
}

fn encode_provenance(value: &HistoryProvenance) -> StoredProvenance {
    match value {
        HistoryProvenance::Native => StoredProvenance {
            kind: 1,
            schema: 0,
            source_id: Vec::new(),
        },
        HistoryProvenance::Darktable { schema, source_id } => StoredProvenance {
            kind: 2,
            schema: *schema,
            source_id: source_id.as_bytes().to_vec(),
        },
        HistoryProvenance::RustTable { schema, source_id } => StoredProvenance {
            kind: 3,
            schema: *schema,
            source_id: source_id.as_bytes().to_vec(),
        },
    }
}

fn decode_provenance(value: &StoredProvenance) -> Result<HistoryProvenance, ()> {
    let source_id = text(&value.source_id)?;
    match value.kind {
        1 if value.schema == 0 && source_id.is_empty() => Ok(HistoryProvenance::native()),
        2 => Ok(HistoryProvenance::darktable(value.schema, source_id)),
        3 => Ok(HistoryProvenance::rusttable(value.schema, source_id)),
        _ => Err(()),
    }
}

fn encode_operation_kind(kind: HistoryOperationKind) -> u8 {
    match kind {
        HistoryOperationKind::Parameter => 1,
        HistoryOperationKind::Order => 2,
        HistoryOperationKind::Enable => 3,
        HistoryOperationKind::Mask => 4,
        HistoryOperationKind::Blend => 5,
        HistoryOperationKind::Style => 6,
        HistoryOperationKind::Copy => 7,
        HistoryOperationKind::Paste => 8,
        HistoryOperationKind::Reset => 9,
        HistoryOperationKind::Merge => 10,
    }
}

fn decode_operation_kind(value: u8) -> Result<HistoryOperationKind, ()> {
    match value {
        1 => Ok(HistoryOperationKind::Parameter),
        2 => Ok(HistoryOperationKind::Order),
        3 => Ok(HistoryOperationKind::Enable),
        4 => Ok(HistoryOperationKind::Mask),
        5 => Ok(HistoryOperationKind::Blend),
        6 => Ok(HistoryOperationKind::Style),
        7 => Ok(HistoryOperationKind::Copy),
        8 => Ok(HistoryOperationKind::Paste),
        9 => Ok(HistoryOperationKind::Reset),
        10 => Ok(HistoryOperationKind::Merge),
        _ => Err(()),
    }
}

fn encode_evidence_kind(kind: HistoryEvidenceKind) -> u8 {
    match kind {
        HistoryEvidenceKind::Export => 1,
        HistoryEvidenceKind::Migration => 2,
        HistoryEvidenceKind::Import => 3,
        HistoryEvidenceKind::Redo => 4,
        HistoryEvidenceKind::Restore => 5,
    }
}

fn decode_evidence_kind(value: u8) -> Result<HistoryEvidenceKind, ()> {
    match value {
        1 => Ok(HistoryEvidenceKind::Export),
        2 => Ok(HistoryEvidenceKind::Migration),
        3 => Ok(HistoryEvidenceKind::Import),
        4 => Ok(HistoryEvidenceKind::Redo),
        5 => Ok(HistoryEvidenceKind::Restore),
        _ => Err(()),
    }
}
