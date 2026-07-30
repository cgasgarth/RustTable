use rusttable_catalog::HistorySnapshotDocument;

pub(crate) fn encode(snapshot: &HistorySnapshotDocument) -> Result<Vec<u8>, ()> {
    snapshot.serialize().map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<HistorySnapshotDocument, ()> {
    HistorySnapshotDocument::deserialize(bytes).map_err(|_| ())
}
