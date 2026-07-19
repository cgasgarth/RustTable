use std::collections::{BTreeMap, BTreeSet};

use super::{Policy, Result, SourceInfo};
use crate::root::RepositoryRoot;

#[derive(Debug, Clone)]
pub(super) struct LcovFile {
    pub(super) path: String,
    pub(super) functions: BTreeMap<(u64, String), u64>,
    pub(super) lines: BTreeMap<u64, u64>,
    pub(super) regions: BTreeMap<(u64, String, String), u64>,
    pub(super) region_counts: Option<(u64, u64)>,
}

pub(super) fn parse_lcov(
    source: &str,
    root: &RepositoryRoot,
    sources: &BTreeMap<String, SourceInfo>,
    policy: &Policy,
) -> Result<Vec<LcovFile>> {
    let mut files = Vec::<LcovFile>::new();
    let mut current: Option<LcovFile> = None;
    let mut saw_record = false;
    for line in source.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(raw_path) = line.strip_prefix("SF:") {
            if current.is_some() {
                return Err("malformed LCOV: nested source record".to_owned());
            }
            let path = normalize_path(raw_path, root)?;
            let Some(_) = sources.get(&path) else {
                return Err(format!(
                    "LCOV references stale or non-workspace source {path}"
                ));
            };
            current = Some(LcovFile {
                path,
                functions: BTreeMap::new(),
                lines: BTreeMap::new(),
                regions: BTreeMap::new(),
                region_counts: None,
            });
            saw_record = true;
        } else if line == "end_of_record" {
            let file = current
                .take()
                .ok_or_else(|| "malformed LCOV: record without source".to_owned())?;
            files.push(file);
        } else if let Some(file) = current.as_mut() {
            parse_record_line(file, line)?;
        } else if !line.starts_with("TN:") {
            return Err(format!("malformed LCOV record: {line}"));
        }
    }
    if current.is_some() {
        return Err("malformed LCOV: unterminated source record".to_owned());
    }
    if !saw_record || files.is_empty() {
        return Err("coverage evidence is empty".to_owned());
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut seen = BTreeSet::new();
    for file in &files {
        if !seen.insert(file.path.clone()) {
            return Err(format!(
                "malformed LCOV: duplicate source record {}",
                file.path
            ));
        }
    }
    if files.iter().all(|file| file.lines.is_empty()) {
        return Err("coverage evidence contains no line records".to_owned());
    }
    let _ = policy;
    Ok(files)
}

fn parse_record_line(file: &mut LcovFile, line: &str) -> Result<()> {
    if let Some(value) = line.strip_prefix("FN:") {
        let (line_number, name) = value
            .split_once(',')
            .ok_or_else(|| "malformed LCOV FN".to_owned())?;
        let line_number = line_number
            .parse::<u64>()
            .map_err(|_| "malformed LCOV FN line".to_owned())?;
        file.functions
            .entry((line_number, name.to_owned()))
            .or_insert(0);
    } else if let Some(value) = line.strip_prefix("FNDA:") {
        let (hits, name) = value
            .split_once(',')
            .ok_or_else(|| "malformed LCOV FNDA".to_owned())?;
        let hits = hits
            .parse::<u64>()
            .map_err(|_| "malformed LCOV FNDA hits".to_owned())?;
        let entry = file
            .functions
            .iter_mut()
            .find(|((_, candidate), _)| candidate == name)
            .ok_or_else(|| "malformed LCOV FNDA without FN".to_owned())?;
        *entry.1 = entry.1.saturating_add(hits);
    } else if let Some(value) = line.strip_prefix("DA:") {
        let (line_number, hits) = value
            .split_once(',')
            .ok_or_else(|| "malformed LCOV DA".to_owned())?;
        let line_number = line_number
            .split_once(',')
            .map_or(line_number, |(number, _)| number)
            .parse::<u64>()
            .map_err(|_| "malformed LCOV DA line".to_owned())?;
        let hits = hits
            .split_once(',')
            .map_or(hits, |(count, _)| count)
            .parse::<u64>()
            .map_err(|_| "malformed LCOV DA hits".to_owned())?;
        let entry = file.lines.entry(line_number).or_insert(0);
        *entry = entry.saturating_add(hits);
    } else if let Some(value) = line.strip_prefix("BRDA:") {
        let fields = value.split(',').collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err("malformed LCOV BRDA".to_owned());
        }
        let line_number = fields[0]
            .parse::<u64>()
            .map_err(|_| "malformed LCOV BRDA line".to_owned())?;
        let hits = if fields[3] == "-" {
            0
        } else {
            fields[3]
                .parse::<u64>()
                .map_err(|_| "malformed LCOV BRDA hits".to_owned())?
        };
        let entry = file
            .regions
            .entry((line_number, fields[1].to_owned(), fields[2].to_owned()))
            .or_insert(0);
        *entry = entry.saturating_add(hits);
    } else if ["FNF:", "FNH:", "LF:", "LH:", "BRF:", "BRH:"]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    {
        let value = line
            .split_once(':')
            .map(|(_, value)| value)
            .unwrap_or_default();
        value
            .parse::<u64>()
            .map_err(|_| format!("malformed LCOV count: {line}"))?;
    } else if !line.starts_with("TN:") {
        return Err(format!("malformed LCOV record: {line}"));
    }
    Ok(())
}

pub(super) fn normalize_path(raw: &str, root: &RepositoryRoot) -> Result<String> {
    let mut path = raw.trim().replace('\\', "/");
    if path.starts_with("file://") {
        path.drain(.."file://".len());
    }
    let root = root.path().to_string_lossy().replace('\\', "/");
    let root_prefix = format!("{root}/");
    if path.starts_with(&root_prefix) {
        path.drain(..root_prefix.len());
    } else if let Some(index) = path.find("crates/") {
        path.drain(..index);
    }
    while path.starts_with("./") {
        path.drain(..2);
    }
    if path.starts_with('/')
        || path.split('/').any(|part| part == "..")
        || !path.starts_with("crates/")
    {
        return Err(format!("LCOV source path is outside the workspace: {raw}"));
    }
    Ok(path)
}
