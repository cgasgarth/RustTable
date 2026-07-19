use unicode_normalization::UnicodeNormalization;

use super::extract::{extract_sections, normalize_text, parse_dependencies};
use super::types::{CapabilityMetadata, CapabilityRole, SectionRole, SectionSource};

pub(super) fn parse_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_end();
    let marker_end = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&marker_end) && trimmed.as_bytes().get(marker_end) == Some(&b' ') {
        let heading = trimmed[marker_end..].trim();
        let heading = heading.trim_end_matches('#').trim();
        (!heading.is_empty()).then_some(heading)
    } else {
        None
    }
}

pub(super) fn role_for_heading(heading: &str) -> Option<(SectionRole, SectionSource)> {
    let normalized = normalized_heading(heading);
    let v2 = [
        ("outcome", SectionRole::Outcome),
        ("fixed decisions", SectionRole::FixedDecisions),
        ("implementation scope", SectionRole::ImplementationScope),
        ("implementation", SectionRole::ImplementationScope),
        (
            "failure and edge behavior",
            SectionRole::FailureAndEdgeBehavior,
        ),
        ("test matrix", SectionRole::TestMatrix),
        ("acceptance evidence", SectionRole::AcceptanceEvidence),
        ("dependencies", SectionRole::Dependencies),
        ("one pr boundary", SectionRole::OnePrBoundary),
        ("capabilities", SectionRole::Capabilities),
        ("qualification decision", SectionRole::QualificationDecision),
        ("observed defects", SectionRole::ObservedDefects),
        ("post completion audit", SectionRole::PostCompletionAudit),
    ];
    if let Some((_, role)) = v2.iter().find(|(name, _)| *name == normalized) {
        return Some((*role, SectionSource::V2));
    }
    let legacy = [
        ("goal", SectionRole::Outcome),
        ("required implementation", SectionRole::ImplementationScope),
        ("acceptance criteria", SectionRole::AcceptanceEvidence),
    ];
    legacy
        .iter()
        .find(|(name, _)| *name == normalized)
        .map(|(_, role)| (*role, SectionSource::LegacyEquivalent))
}

pub(super) fn normalized_heading(heading: &str) -> String {
    heading
        .nfc()
        .map(|(character, _)| character)
        .collect::<String>()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn parse_capability_role(value: &str) -> CapabilityRole {
    match value.trim().to_lowercase().as_str() {
        "owner" => CapabilityRole::Owner,
        "adapter" => CapabilityRole::Adapter,
        "consumer" => CapabilityRole::Consumer,
        "compatibility" | "parity" => CapabilityRole::Compatibility,
        _ => CapabilityRole::Unknown,
    }
}

pub(super) fn parse_capabilities(content: &str) -> Vec<CapabilityMetadata> {
    let mut records = Vec::new();
    let mut current: Option<CapabilityMetadata> = None;
    for (index, raw_line) in normalize_text(content).lines().enumerate() {
        let line = raw_line
            .trim()
            .trim_start_matches(['-', '*'])
            .trim()
            .trim_matches('`');
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once(':').map_or(("name", line), |(key, value)| {
            (key.trim(), value.trim().trim_matches('`'))
        });
        match key.to_lowercase().as_str() {
            "name" | "id" | "capability" | "capability id" => {
                if let Some(record) = current.take() {
                    records.push(record);
                }
                current = Some(CapabilityMetadata {
                    name: value.to_owned(),
                    role: CapabilityRole::Unknown,
                    owner_issue: None,
                    line: index + 1,
                });
            }
            "role" => {
                if let Some(record) = current.as_mut() {
                    record.role = parse_capability_role(value);
                }
            }
            "owner" | "owner issue" | "owner_issue" => {
                if let Some(record) = current.as_mut() {
                    record.owner_issue = first_issue_reference(value);
                }
            }
            _ if line.to_lowercase().starts_with("owns ") => {
                if let Some(record) = current.as_mut() {
                    record.role = CapabilityRole::Owner;
                }
            }
            _ => {}
        }
    }
    if let Some(record) = current {
        records.push(record);
    }
    records
        .into_iter()
        .filter(|record| !record.name.is_empty())
        .collect()
}

pub(super) fn first_issue_reference(value: &str) -> Option<u64> {
    let parsed = parse_dependencies(value);
    parsed.references.first().map(|reference| reference.issue)
}

pub(super) fn has_named_value(text: &str, term: &str) -> bool {
    text.split(term)
        .skip(1)
        .any(|rest| rest.trim_start().starts_with(':') && rest[1..].split_whitespace().count() >= 1)
        || contains_any(text, &["use ", "using ", "select ", "selected ", "is "])
}

pub(super) fn contains_any(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

pub(super) fn is_boilerplate(value: &str) -> bool {
    let lower = value.to_lowercase();
    let words = lower.split_whitespace().count();
    words < 4
        || [
            "implement this issue",
            "add the requested feature",
            "write code and tests",
            "do the work",
            "tbd",
            "todo",
        ]
        .iter()
        .any(|phrase| lower.trim() == *phrase)
}

pub(super) fn is_generic_placeholder(body: &str) -> bool {
    let sections = extract_sections(body);
    sections.sections.len() < 5
        && sections.sections.values().all(|section| {
            is_boilerplate(&section.content) || section.content.split_whitespace().count() < 18
        })
}
