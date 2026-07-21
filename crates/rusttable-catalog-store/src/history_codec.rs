use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_catalog::{
    HistoryBranch, HistoryBranchId, HistoryCursor, HistoryEvidence, HistoryEvidenceKind,
    HistoryOperationKind, HistoryOperationSummary, HistoryRevision, HistoryRevisionId,
    HistorySnapshot, HistorySnapshotId, HistoryStateSnapshot, HistoryVersion,
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
}

fn text(bytes: &[u8]) -> Result<String, ()> {
    String::from_utf8(bytes.to_vec()).map_err(|_| ())
}

fn optional_revision(value: Option<u64>) -> Result<Option<HistoryRevisionId>, ()> {
    value
        .map(|id| HistoryRevisionId::new(id).ok_or(()))
        .transpose()
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
    }
}

fn decode_evidence_kind(value: u8) -> Result<HistoryEvidenceKind, ()> {
    match value {
        1 => Ok(HistoryEvidenceKind::Export),
        2 => Ok(HistoryEvidenceKind::Migration),
        _ => Err(()),
    }
}
