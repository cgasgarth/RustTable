use std::collections::BTreeMap;

use crate::model::{Capability, Manifest, SummaryGroup};
use crate::scan::ScanError;

const VALID_STATUSES: &[&str] = &["required", "redesigned", "unsupported"];
const LAST_ISSUE_SEQUENCE: u16 = 257;

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

/// Checks IDs, statuses, issue sequences, evidence, and deterministic summary.
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
            &capability.issue_sequences,
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
    issue_sequences: &[String],
) -> Result<(), ScanError> {
    if !VALID_STATUSES.contains(&status) {
        return Err(ScanError::InvalidStatus {
            value: status.to_owned(),
            id: id.to_owned(),
        });
    }
    if issue_sequences.is_empty() {
        return Err(ScanError::InvalidOverride {
            id: id.to_owned(),
            message: "at least one issue sequence is required".to_owned(),
        });
    }
    for sequence in issue_sequences {
        let valid_shape =
            sequence.len() == 4 && sequence.chars().all(|character| character.is_ascii_digit());
        let valid_range = sequence
            .parse::<u16>()
            .is_ok_and(|number| (1..=LAST_ISSUE_SEQUENCE).contains(&number));
        if !valid_shape || !valid_range {
            return Err(ScanError::StaleIssueSequence {
                sequence: sequence.clone(),
                id: id.to_owned(),
            });
        }
    }
    Ok(())
}

pub(crate) fn summary_for(capabilities: &[Capability]) -> Vec<SummaryGroup> {
    let mut groups = BTreeMap::<(String, String), SummaryGroup>::new();
    for capability in capabilities {
        let sequence = capability
            .issue_sequences
            .first()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(1);
        let key = (phase(sequence).to_owned(), milestone(sequence).to_owned());
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

fn phase(sequence: u16) -> &'static str {
    match sequence {
        1..=25 => "foundation",
        26..=65 => "compatibility",
        66..=100 => "image",
        101..=265 => "processing",
        266..=300 => "product-ui",
        _ => "release",
    }
}

fn milestone(sequence: u16) -> &'static str {
    match sequence {
        1..=25 => "1",
        26..=65 => "3",
        66..=100 => "4",
        101..=265 => "5",
        266..=300 => "2",
        _ => "6",
    }
}
