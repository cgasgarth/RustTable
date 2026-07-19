use std::collections::BTreeSet;

use super::super::super::issue_spec::{SectionRole, extract_sections, normalize_text};
use super::super::model::{Anchor, Inventory, Selector};

pub(super) fn anchors_from_body(number: u64, body: &str, inventory: &Inventory) -> Vec<Anchor> {
    let sections = extract_sections(body);
    let Some(source) = sections.get(SectionRole::SourceAnchors) else {
        return Vec::new();
    };
    let owners = section_code_spans(body, "rust ownership");
    let rust_owners = if owners.is_empty() {
        vec![format!("issue-{number}-rust-owner")]
    } else {
        owners
    };
    let paths = inventory
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut anchors = Vec::new();
    for line in normalize_text(&source.content).lines() {
        let spans = code_spans(line);
        let Some((path_index, path)) = spans
            .iter()
            .enumerate()
            .find(|(_, span)| paths.contains(span.as_str()))
        else {
            continue;
        };
        let selector_text = spans
            .iter()
            .enumerate()
            .filter(|(index, span)| *index != path_index && !paths.contains(span.as_str()))
            .map(|(_, span)| span.as_str())
            .next();
        let selector = selector_text.map_or(Selector::InventoryPath, |text| Selector::Symbol {
            name: symbol_name(text),
        });
        if !seen.insert((path.to_owned(), selector.clone())) {
            continue;
        }
        let Some(blob_id) = inventory
            .entries
            .iter()
            .find(|entry| entry.path == *path)
            .map(|entry| entry.object_id.clone())
        else {
            continue;
        };
        anchors.push(Anchor {
            path: path.to_owned(),
            blob_id,
            selector,
            role: role_for(line),
            evidence: format!("issue-{number}-source-anchor-section"),
            rust_owners: rust_owners.clone(),
            oracle_fixtures: vec![format!("issue.{number}.source")],
        });
    }
    anchors
}

pub(super) fn decision_from_body(body: &str, label: &str) -> Vec<String> {
    let prefix = format!("**{label}:**");
    normalize_text(body)
        .lines()
        .find_map(|line| {
            let content = line.trim().strip_prefix(&prefix)?.trim();
            (!content.is_empty()).then(|| vec![content.to_owned()])
        })
        .unwrap_or_default()
}

pub(super) fn decision_or_default(body: &str, label: &str, default: &str) -> Vec<String> {
    let values = decision_from_body(body, label);
    if values.is_empty() {
        vec![default.to_owned()]
    } else {
        values
    }
}

fn role_for(line: &str) -> String {
    let lower = line
        .split('|')
        .nth(1)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lower.contains("schema") || lower.contains("table") {
        "data-schema".to_owned()
    } else if lower.contains("ui") || lower.contains("view") || lower.contains("bauhaus") {
        "ui-reference".to_owned()
    } else if lower.contains("integration") || lower.contains("kernel") {
        "integration-boundary".to_owned()
    } else {
        "authoritative-behavior".to_owned()
    }
}

fn symbol_name(text: &str) -> String {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .find(|token| {
            token
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        })
        .unwrap_or(text)
        .to_owned()
}

fn section_code_spans(body: &str, heading: &str) -> Vec<String> {
    let normalized = normalize_text(body);
    let mut in_section = false;
    let mut spans = Vec::new();
    for line in normalized.lines() {
        if line
            .trim_start_matches('#')
            .trim()
            .eq_ignore_ascii_case(heading)
        {
            in_section = true;
            continue;
        }
        if in_section && line.trim_start().starts_with('#') {
            break;
        }
        if in_section {
            spans.extend(code_spans(line));
        }
    }
    spans
}

fn code_spans(line: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let span = after[..end].trim();
        if !span.is_empty() {
            spans.push(span.to_owned());
        }
        rest = &after[end + 1..];
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::symbol_name;

    #[test]
    fn extracts_an_identifier_from_a_function_like_anchor() {
        assert_eq!(symbol_name("project(darktable VERSION"), "project");
        assert_eq!(
            symbol_name("add_subdirectory(external)"),
            "add_subdirectory"
        );
    }
}
