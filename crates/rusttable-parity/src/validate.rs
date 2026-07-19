use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::model::{Capability, CapabilityReceipt, IssueIndex, Manifest, SummaryGroup};
use crate::scan::ScanError;

const VALID_STATUSES: &[&str] = &["required", "redesigned", "unsupported"];
const VALID_PRIORITIES: &[&str] = &["P0", "P1", "P2", "P3", "P4"];
const REPOSITORY: &str = "cgasgarth/RustTable";
const PARENT_ISSUE: u64 = 158;
type OwnershipByIssue = BTreeMap<u64, BTreeSet<String>>;
type OwnershipByCapability = BTreeMap<String, usize>;

/// Parses a normalized capability manifest.
///
/// # Errors
///
/// Returns an error when TOML syntax or the manifest shape is invalid.
pub fn parse_manifest(contents: &str) -> Result<Manifest, ScanError> {
    toml::from_str(contents).map_err(|error| ScanError::InvalidManifest {
        message: error.to_string(),
    })
}

pub(crate) fn parse_issue_index(contents: &str) -> Result<IssueIndex, ScanError> {
    toml::from_str(contents).map_err(|error| ScanError::InvalidIssueIndex {
        message: error.to_string(),
    })
}

/// Validates and renders a manifest with stable TOML ordering.
///
/// # Errors
///
/// Returns an error when a manifest invariant fails or TOML serialization
/// cannot represent the model.
pub fn render_manifest(manifest: &Manifest) -> Result<String, ScanError> {
    validate_manifest(manifest)?;
    let mut rendered =
        toml::to_string_pretty(manifest).map_err(|error| ScanError::Serialization {
            message: error.to_string(),
        })?;
    rendered.insert_str(
        0,
        "# GENERATED FILE: rusttable-parity scan-darktable; do not hand-edit.\n\n",
    );
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

/// Renders the deterministic inventory receipt paired with a manifest.
///
/// # Errors
///
/// Returns an error when the manifest cannot be validated or serialized.
pub fn render_receipt(manifest: &Manifest) -> Result<String, ScanError> {
    let manifest_bytes = render_manifest(manifest)?;
    let mut digest = Sha256::new();
    digest.update(manifest_bytes.as_bytes());
    let mut manifest_sha256 = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut manifest_sha256, "{byte:02x}").expect("writing to a String cannot fail");
    }
    let mut multiply_mapped = manifest
        .capabilities
        .iter()
        .filter(|capability| capability.ownership.len() > 1)
        .map(|capability| capability.id.clone())
        .collect::<Vec<_>>();
    multiply_mapped.sort();
    let mut unsupported = manifest
        .capabilities
        .iter()
        .filter(|capability| capability.status == "unsupported")
        .map(|capability| capability.id.clone())
        .collect::<Vec<_>>();
    unsupported.sort();
    let receipt = CapabilityReceipt {
        schema_version: 1,
        manifest_sha256,
        capability_count: manifest.capabilities.len(),
        ownership_count: manifest
            .capabilities
            .iter()
            .map(|capability| capability.ownership.len())
            .sum(),
        issue_count: manifest
            .capabilities
            .iter()
            .flat_map(|capability| {
                capability
                    .ownership
                    .iter()
                    .map(|ownership| ownership.issue_number)
            })
            .collect::<BTreeSet<_>>()
            .len(),
        unmapped_capabilities: Vec::new(),
        multiply_mapped_capabilities: multiply_mapped,
        stale_issue_numbers: Vec::new(),
        unsupported_capabilities: unsupported,
        evidence_incomplete_capabilities: Vec::new(),
    };
    let mut rendered =
        toml::to_string_pretty(&receipt).map_err(|error| ScanError::Serialization {
            message: error.to_string(),
        })?;
    rendered.insert_str(
        0,
        "# GENERATED FILE: rusttable-parity capability inventory receipt; do not hand-edit.\n\n",
    );
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

/// Checks the manifest shape without consulting GitHub metadata.
///
/// # Errors
///
/// Returns the first stable validation diagnostic.
pub fn validate_manifest(manifest: &Manifest) -> Result<(), ScanError> {
    if manifest.schema_version != 2 {
        return Err(ScanError::InvalidManifest {
            message: format!("unsupported schema version {}", manifest.schema_version),
        });
    }
    let mut ids = BTreeSet::new();
    for capability in &manifest.capabilities {
        validate_capability_shape(capability)?;
        if !ids.insert(capability.id.clone()) {
            return Err(ScanError::DuplicateCapabilityId {
                id: capability.id.clone(),
            });
        }
    }
    let expected_summary = summary_for(&manifest.capabilities);
    if manifest.summary != expected_summary {
        return Err(ScanError::InvalidManifest {
            message: "summary is not the deterministic priority/milestone summary".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_manifest_with_issue_index(
    manifest: &Manifest,
    index: &IssueIndex,
) -> Result<(), ScanError> {
    validate_manifest(manifest)?;
    let known = manifest
        .capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect::<Vec<_>>();
    validate_issue_index(index, Some(&known))?;
    for capability in &manifest.capabilities {
        let records = index
            .ownership
            .iter()
            .filter(|record| record.capability_id == capability.id)
            .collect::<Vec<_>>();
        let expected = records
            .iter()
            .map(|record| {
                let issue = issue(index, record.issue_number);
                (
                    record.issue_number,
                    record.role.as_str(),
                    issue.milestone.as_str(),
                    issue.priority.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        let actual = capability
            .ownership
            .iter()
            .map(|ownership| {
                (
                    ownership.issue_number,
                    ownership.role.as_str(),
                    ownership.milestone.as_str(),
                    ownership.priority.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        if actual != expected {
            return Err(ScanError::InvalidManifest {
                message: format!(
                    "manifest ownership differs from issue index for {}",
                    capability.id
                ),
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_issue_index(
    index: &IssueIndex,
    known_capability_ids: Option<&[String]>,
) -> Result<(), ScanError> {
    validate_index_header(index)?;
    let issues = collect_issue_metadata(index)?;
    let (ownership_by_issue, ownership_by_capability) = collect_ownership_metadata(index, &issues)?;
    validate_ownership_consistency(index)?;
    validate_capability_inventory(index, &ownership_by_capability)?;
    validate_issue_declarations(index, &ownership_by_issue)?;
    if let Some(known) = known_capability_ids {
        let indexed = index.capability_ids.iter().collect::<BTreeSet<_>>();
        for id in known {
            if !indexed.contains(&id) {
                return Err(ScanError::MissingOwnership { id: id.clone() });
            }
        }
    }
    Ok(())
}

fn validate_index_header(index: &IssueIndex) -> Result<(), ScanError> {
    if index.schema_version != 1 {
        return Err(ScanError::InvalidIssueIndex {
            message: format!("unsupported schema version {}", index.schema_version),
        });
    }
    if index.repository != REPOSITORY {
        return Err(ScanError::InvalidIssueIndex {
            message: format!("wrong repository {}", index.repository),
        });
    }
    if index.parent_issue != PARENT_ISSUE {
        return Err(ScanError::InvalidIssueIndex {
            message: format!("wrong parent issue #{}", index.parent_issue),
        });
    }
    Ok(())
}

fn collect_issue_metadata(
    index: &IssueIndex,
) -> Result<BTreeMap<u64, &crate::model::IssueRecord>, ScanError> {
    let mut issues = BTreeMap::new();
    for record in &index.issues {
        if record.number == 0 || record.title.trim().is_empty() {
            return Err(ScanError::InvalidIssueIndex {
                message: format!("issue metadata is incomplete for #{}", record.number),
            });
        }
        if record.title.trim_start().starts_with('[') {
            return Err(ScanError::StaleIssue {
                issue_number: record.number,
                reason: "sequence placeholder title".to_owned(),
            });
        }
        if record.parent_issue != PARENT_ISSUE {
            return Err(ScanError::StaleIssue {
                issue_number: record.number,
                reason: "issue is not a child of #158".to_owned(),
            });
        }
        if record.milestone.trim().is_empty() {
            return Err(ScanError::StaleIssue {
                issue_number: record.number,
                reason: "missing milestone".to_owned(),
            });
        }
        if record.state != "open" && record.state != "closed" {
            return Err(ScanError::InvalidIssueIndex {
                message: format!("invalid state {:?} for #{}", record.state, record.number),
            });
        }
        if record.state == "open" && !VALID_PRIORITIES.contains(&record.priority.as_str()) {
            return Err(ScanError::StaleIssue {
                issue_number: record.number,
                reason: "open issue must have exactly one priority label".to_owned(),
            });
        }
        if record.state_reason.as_deref() == Some("not_planned")
            || record.state_reason.as_deref() == Some("duplicate")
        {
            let replacement = record
                .replacement_issue
                .and_then(|number| index.issues.iter().find(|issue| issue.number == number));
            if replacement.is_none_or(|issue| {
                issue.state_reason.as_deref() == Some("not_planned")
                    || issue.state_reason.as_deref() == Some("duplicate")
            }) {
                return Err(ScanError::StaleIssue {
                    issue_number: record.number,
                    reason: "closed without an active replacement".to_owned(),
                });
            }
        }
        if issues.insert(record.number, record).is_some() {
            return Err(ScanError::InvalidIssueIndex {
                message: format!("duplicate issue metadata for #{}", record.number),
            });
        }
    }
    Ok(issues)
}

fn collect_ownership_metadata(
    index: &IssueIndex,
    issues: &BTreeMap<u64, &crate::model::IssueRecord>,
) -> Result<(OwnershipByIssue, OwnershipByCapability), ScanError> {
    let mut ownership_keys = BTreeSet::new();
    let mut ownership_by_issue = BTreeMap::<u64, BTreeSet<String>>::new();
    let mut ownership_by_capability = BTreeMap::<String, usize>::new();
    for record in &index.ownership {
        if !issues.contains_key(&record.issue_number) {
            return Err(ScanError::StaleIssue {
                issue_number: record.issue_number,
                reason: format!(
                    "ownership references an unindexed issue for {}",
                    record.capability_id
                ),
            });
        }
        if record.capability_id.trim().is_empty() || record.role.trim().is_empty() {
            return Err(ScanError::InvalidIssueIndex {
                message: "ownership capability and role are required".to_owned(),
            });
        }
        if !index
            .capability_ids
            .iter()
            .any(|id| id == &record.capability_id)
        {
            return Err(ScanError::UnknownIssueCapability {
                id: record.capability_id.clone(),
            });
        }
        validate_capability_fields(
            &record.capability_id,
            &record.status,
            1,
            &record.behavioral_evidence,
            &record.acceptance_test_id,
        )?;
        if !ownership_keys.insert((
            record.capability_id.clone(),
            record.issue_number,
            record.role.clone(),
        )) {
            return Err(ScanError::DuplicateOwnership {
                id: record.capability_id.clone(),
                issue_number: record.issue_number,
                role: record.role.clone(),
            });
        }
        ownership_by_issue
            .entry(record.issue_number)
            .or_default()
            .insert(record.capability_id.clone());
        *ownership_by_capability
            .entry(record.capability_id.clone())
            .or_default() += 1;
        if record
            .behavioral_evidence
            .iter()
            .any(|evidence| evidence.starts_with("reference-scan:"))
        {
            return Err(ScanError::InvalidIssueIndex {
                message: format!(
                    "structural evidence cannot be behavioral for {}",
                    record.capability_id
                ),
            });
        }
    }
    Ok((ownership_by_issue, ownership_by_capability))
}

fn validate_ownership_consistency(index: &IssueIndex) -> Result<(), ScanError> {
    let mut shapes = BTreeMap::<
        String,
        (
            String,
            String,
            Vec<String>,
            Vec<String>,
            String,
            Option<String>,
        ),
    >::new();
    for record in &index.ownership {
        let shape = (
            record.category.clone(),
            record.status.clone(),
            record.structural_evidence.clone(),
            record.behavioral_evidence.clone(),
            record.acceptance_test_id.clone(),
            record.redesign_note.clone(),
        );
        if let Some(previous) = shapes.get(&record.capability_id)
            && previous != &shape
        {
            return Err(ScanError::InvalidIssueIndex {
                message: format!(
                    "ownership metadata differs across roles for {}",
                    record.capability_id
                ),
            });
        }
        shapes.insert(record.capability_id.clone(), shape);
    }
    Ok(())
}

fn validate_capability_inventory(
    index: &IssueIndex,
    ownership_by_capability: &OwnershipByCapability,
) -> Result<(), ScanError> {
    let indexed_capabilities = index
        .capability_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let owned_capabilities = ownership_by_capability
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if indexed_capabilities != owned_capabilities {
        return Err(ScanError::InvalidIssueIndex {
            message: "capability ID inventory differs from ownership records".to_owned(),
        });
    }
    Ok(())
}

fn validate_issue_declarations(
    index: &IssueIndex,
    ownership_by_issue: &OwnershipByIssue,
) -> Result<(), ScanError> {
    for record in &index.issues {
        let declared = record
            .capability_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let owned = ownership_by_issue
            .get(&record.number)
            .cloned()
            .unwrap_or_default();
        if declared != owned {
            return Err(ScanError::InvalidIssueIndex {
                message: format!(
                    "issue #{} capability declaration differs from ownership",
                    record.number
                ),
            });
        }
    }
    Ok(())
}

pub(crate) fn validate_capability_fields(
    id: &str,
    status: &str,
    ownership_count: usize,
    behavioral_evidence: &[String],
    acceptance_test_id: &str,
) -> Result<(), ScanError> {
    if !VALID_STATUSES.contains(&status) {
        return Err(ScanError::InvalidStatus {
            value: status.to_owned(),
            id: id.to_owned(),
        });
    }
    if ownership_count == 0 {
        return Err(ScanError::MissingOwnership { id: id.to_owned() });
    }
    if behavioral_evidence.is_empty() {
        return Err(ScanError::InvalidIssueIndex {
            message: format!("missing behavioral evidence for {id}"),
        });
    }
    if acceptance_test_id.trim().is_empty() {
        return Err(ScanError::InvalidIssueIndex {
            message: format!("missing acceptance test ID for {id}"),
        });
    }
    Ok(())
}

fn validate_capability_shape(capability: &Capability) -> Result<(), ScanError> {
    validate_capability_fields(
        &capability.id,
        &capability.status,
        capability.ownership.len(),
        &capability.behavioral_evidence,
        &capability.acceptance_test_id,
    )?;
    if capability.reference_path.trim().is_empty()
        || capability.reference_symbol.trim().is_empty()
        || capability.category.trim().is_empty()
    {
        return Err(ScanError::InvalidManifest {
            message: format!(
                "reference, symbol, and category are required for {}",
                capability.id
            ),
        });
    }
    if capability.structural_evidence.is_empty() {
        return Err(ScanError::InvalidManifest {
            message: format!("missing structural evidence for {}", capability.id),
        });
    }
    let mut ownership = BTreeSet::new();
    for record in &capability.ownership {
        if !VALID_PRIORITIES.contains(&record.priority.as_str())
            || record.milestone.trim().is_empty()
        {
            return Err(ScanError::InvalidManifest {
                message: format!("invalid issue governance for {}", capability.id),
            });
        }
        if !ownership.insert((record.issue_number, record.role.clone())) {
            return Err(ScanError::DuplicateOwnership {
                id: capability.id.clone(),
                issue_number: record.issue_number,
                role: record.role.clone(),
            });
        }
    }
    Ok(())
}

pub(crate) fn summary_for(capabilities: &[Capability]) -> Vec<SummaryGroup> {
    let mut groups = BTreeMap::<(String, String), SummaryGroup>::new();
    for capability in capabilities {
        let ownership = capability.ownership.first().expect("validated ownership");
        let key = (ownership.priority.clone(), ownership.milestone.clone());
        let group = groups.entry(key.clone()).or_insert_with(|| SummaryGroup {
            priority: key.0.clone(),
            milestone: key.1.clone(),
            capability_count: 0,
            required: 0,
            redesigned: 0,
            unsupported: 0,
        });
        group.capability_count += 1;
        match capability.status.as_str() {
            "required" => group.required += 1,
            "redesigned" => group.redesigned += 1,
            "unsupported" => group.unsupported += 1,
            _ => {}
        }
    }
    groups.into_values().collect()
}

fn issue(index: &IssueIndex, number: u64) -> &crate::model::IssueRecord {
    index
        .issues
        .iter()
        .find(|record| record.number == number)
        .expect("validated issue index contains every ownership issue")
}
