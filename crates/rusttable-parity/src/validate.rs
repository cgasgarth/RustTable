use std::collections::BTreeMap;

use crate::model::{Capability, Manifest, SummaryGroup};
use crate::scan::ScanError;

const VALID_STATUSES: &[&str] = &["required", "redesigned", "unsupported"];
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

/// Checks IDs, statuses, issue ownership numbers, evidence, and deterministic summary.
///
/// # Errors
///
/// Returns the first stable validation diagnostic.
pub fn validate_manifest(manifest: &Manifest) -> Result<(), ScanError> {
    if manifest.schema_version != 1 {
        return Err(ScanError::InvalidManifest {
            message: format!("unsupported schema version {}", manifest.schema_version),
        });
    }
    let mut ids = Vec::new();
    for capability in &manifest.capabilities {
        validate_capability_fields(
            &capability.id,
            &capability.status,
            &capability.issue_numbers,
        )?;
        if capability.reference_path.trim().is_empty() {
            return Err(ScanError::InvalidManifest {
                message: format!("missing reference path for {}", capability.id),
            });
        }
        if capability.reference_symbol.trim().is_empty() {
            return Err(ScanError::InvalidManifest {
                message: format!("missing reference symbol for {}", capability.id),
            });
        }
        if capability.category.trim().is_empty() {
            return Err(ScanError::InvalidManifest {
                message: format!("missing category for {}", capability.id),
            });
        }
        if capability.test_evidence.is_empty() {
            return Err(ScanError::InvalidManifest {
                message: format!("missing test evidence for {}", capability.id),
            });
        }
        if ids.iter().any(|known| known == &capability.id) {
            return Err(ScanError::DuplicateCapabilityId {
                id: capability.id.clone(),
            });
        }
        ids.push(capability.id.clone());
    }
    let expected_summary = summary_for(&manifest.capabilities);
    if manifest.summary != expected_summary {
        return Err(ScanError::InvalidManifest {
            message: "summary is not the deterministic summary of capabilities".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_capability_fields(
    id: &str,
    status: &str,
    issue_numbers: &[u64],
) -> Result<(), ScanError> {
    if !VALID_STATUSES.contains(&status) {
        return Err(ScanError::InvalidStatus {
            value: status.to_owned(),
            id: id.to_owned(),
        });
    }
    if issue_numbers.is_empty() {
        return Err(ScanError::InvalidOverride {
            id: id.to_owned(),
            message: "at least one GitHub issue number is required".to_owned(),
        });
    }
    for number in issue_numbers {
        if *number == 0 {
            return Err(ScanError::InvalidIssueNumber {
                number: *number,
                id: id.to_owned(),
            });
        }
    }
    Ok(())
}

pub(crate) fn summary_for(capabilities: &[Capability]) -> Vec<SummaryGroup> {
    let mut groups = BTreeMap::<(String, String), SummaryGroup>::new();
    for capability in capabilities {
        let issue_number = capability.issue_numbers.first().copied().unwrap_or(159);
        let key = (
            phase(issue_number).to_owned(),
            milestone(issue_number).to_owned(),
        );
        let group = groups.entry(key.clone()).or_insert_with(|| SummaryGroup {
            phase: key.0.clone(),
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
    let mut summary = groups.into_values().collect::<Vec<_>>();
    summary.sort_by(|left, right| {
        left.milestone
            .cmp(&right.milestone)
            .then_with(|| left.phase.cmp(&right.phase))
    });
    summary
}

fn phase(issue_number: u64) -> &'static str {
    match issue_number {
        159..=183 => "foundation",
        184..=221 => "compatibility",
        222..=256 => "image",
        257..=415 => "processing",
        420..=422 => "product-ui",
        _ => "release",
    }
}

fn milestone(issue_number: u64) -> &'static str {
    match issue_number {
        159..=183 => "1",
        184..=221 => "3",
        222..=256 => "4",
        257..=415 => "5",
        420..=422 => "2",
        _ => "6",
    }
}
