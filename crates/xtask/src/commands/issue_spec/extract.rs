use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::helpers::{parse_heading, role_for_heading};
use super::types::{
    CanonicalSection, DependencyParse, DependencySpec, DuplicateHeading, SectionExtraction,
    SectionRole, SectionSource,
};

pub fn extract_sections(body: &str) -> SectionExtraction {
    let normalized = normalize_text(body);
    let lines: Vec<&str> = normalized.lines().collect();
    let mut headings = Vec::new();
    let mut in_fence = false;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence && let Some(heading) = parse_heading(trimmed) {
            headings.push((index, heading.to_owned()));
        }
    }

    let mut sections: BTreeMap<SectionRole, CanonicalSection> = BTreeMap::new();
    let mut duplicates = Vec::new();
    let mut legacy_equivalent = false;
    for (heading_index, (line_index, heading)) in headings.iter().enumerate() {
        let Some((role, source)) = role_for_heading(heading) else {
            continue;
        };
        legacy_equivalent |= source == SectionSource::LegacyEquivalent;
        let end = headings
            .get(heading_index + 1)
            .map_or(lines.len(), |(next_line, _)| *next_line);
        let content = normalize_text(&lines[*line_index + 1..end].join("\n"));
        let section = CanonicalSection {
            role,
            heading: heading.trim().to_owned(),
            content: content.clone(),
            line: line_index + 1,
            source,
            content_hash: sha256_hex(content.as_bytes()),
        };
        if let Some(previous) = sections.get(&role) {
            duplicates.push(DuplicateHeading {
                role,
                first_line: previous.line,
                duplicate_line: section.line,
            });
        } else {
            sections.insert(role, section);
        }
    }
    SectionExtraction {
        sections,
        duplicates,
        legacy_equivalent,
        literal_escaped_newline: normalized.contains(r"\n"),
    }
}

pub fn parse_dependencies(content: &str) -> DependencyParse {
    let normalized = normalize_text(content);
    let explicit_none = normalized.split_whitespace().any(|word| {
        word.trim_matches(|character: char| !character.is_ascii_alphanumeric())
            .eq_ignore_ascii_case("none")
    });
    let mut references = Vec::new();
    let mut invalid_numeric_tokens = Vec::new();
    for (line_index, line) in normalized.lines().enumerate() {
        let effective_line = if line
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("depends on ")
        {
            line.split('.').next().unwrap_or(line)
        } else {
            line
        };
        let bytes = effective_line.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'#' {
                let start = index + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end > start {
                    if let Ok(issue) = effective_line[start..end].parse::<u64>() {
                        references.push(DependencySpec {
                            issue,
                            line: line_index + 1,
                        });
                    }
                    index = end;
                    continue;
                }
            }
            index += 1;
        }
        for token in effective_line.split_whitespace() {
            let trimmed = token
                .trim_matches(|character: char| !character.is_ascii_digit() && character != '#');
            if !trimmed.is_empty()
                && trimmed.chars().all(|character| character.is_ascii_digit())
                && !effective_line.contains(&format!("#{trimmed}"))
            {
                invalid_numeric_tokens.push(trimmed.to_owned());
            }
        }
    }
    references.sort_by_key(|reference| (reference.issue, reference.line));
    references.dedup_by_key(|reference| reference.issue);
    invalid_numeric_tokens.sort();
    invalid_numeric_tokens.dedup();
    DependencyParse {
        references,
        explicit_none,
        invalid_numeric_tokens,
    }
}

pub fn normalize_text(value: &str) -> String {
    let value = value.strip_prefix('\u{feff}').unwrap_or(value);
    let value = value.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = value
        .nfc()
        .map(|(character, _)| character)
        .collect::<String>()
        .lines()
        .map(|line| line.trim_end_matches([' ', '\t']).to_owned())
        .collect();
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

pub fn canonical_body_hash(body: &str) -> String {
    sha256_hex(normalize_text(body).as_bytes())
}

pub fn canonical_spec_hash(sections: &SectionExtraction) -> String {
    let mut material = String::new();
    for (role, section) in &sections.sections {
        material.push_str(role.as_str());
        material.push('\n');
        material.push_str(&normalize_text(&section.content));
        material.push('\n');
    }
    sha256_hex(material.as_bytes())
}

pub fn normalized_body_fingerprint(body: &str) -> String {
    let sections = extract_sections(body);
    canonical_spec_hash(&sections)
}

pub(super) fn dependency_cycles(graph: &BTreeMap<u64, BTreeSet<u64>>) -> Vec<Vec<u64>> {
    fn visit(
        node: u64,
        graph: &BTreeMap<u64, BTreeSet<u64>>,
        visiting: &mut Vec<u64>,
        done: &mut BTreeSet<u64>,
        cycles: &mut BTreeSet<Vec<u64>>,
    ) {
        if let Some(position) = visiting.iter().position(|current| *current == node) {
            let mut cycle = visiting[position..].to_vec();
            cycle.push(node);
            cycles.insert(cycle);
            return;
        }
        if done.contains(&node) {
            return;
        }
        visiting.push(node);
        if let Some(dependencies) = graph.get(&node) {
            for dependency in dependencies {
                visit(*dependency, graph, visiting, done, cycles);
            }
        }
        visiting.pop();
        done.insert(node);
    }
    let mut done = BTreeSet::new();
    let mut cycles = BTreeSet::new();
    for node in graph.keys() {
        visit(*node, graph, &mut Vec::new(), &mut done, &mut cycles);
    }
    cycles.into_iter().collect()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
