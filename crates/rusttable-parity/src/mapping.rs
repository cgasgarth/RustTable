use crate::model::{Discovered, IssueIndex, IssueOwnership};

pub(crate) fn map_discovered(
    index: &IssueIndex,
    kind: &str,
    name: &str,
    path: &str,
) -> Option<Discovered> {
    let id = format!("{kind}.{name}");
    let records = index
        .ownership
        .iter()
        .filter(|record| record.capability_id == id)
        .collect::<Vec<_>>();
    if records.is_empty() {
        return None;
    }
    let first = records[0];
    let ownership = records
        .iter()
        .map(|record| IssueOwnership {
            issue_number: record.issue_number,
            role: record.role.clone(),
            milestone: issue(index, record.issue_number).milestone.clone(),
            priority: issue(index, record.issue_number).priority.clone(),
        })
        .collect();
    Some(Discovered {
        id,
        reference_path: path.to_owned(),
        reference_symbol: name.to_owned(),
        category: first.category.clone(),
        status: first.status.clone(),
        ownership,
        structural_evidence: first.structural_evidence.clone(),
        behavioral_evidence: first.behavioral_evidence.clone(),
        acceptance_test_id: first.acceptance_test_id.clone(),
        redesign_note: first.redesign_note.clone(),
    })
}

fn issue(index: &IssueIndex, number: u64) -> &crate::model::IssueRecord {
    index
        .issues
        .iter()
        .find(|record| record.number == number)
        .expect("validated issue index must contain every ownership issue")
}

pub(crate) fn ownership_for<'a>(
    index: &'a IssueIndex,
    id: &str,
) -> impl Iterator<Item = &'a crate::model::OwnershipRecord> {
    index
        .ownership
        .iter()
        .filter(move |record| record.capability_id == id)
}
