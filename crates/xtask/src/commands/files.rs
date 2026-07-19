use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::time::Duration;

use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use super::{Result, report};
use crate::process::{ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const POLICY_PATH: &str = "quality/repository-files.toml";
const INDEX_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct Policy {
    version: u32,
    max_path_length: usize,
    #[serde(rename = "case_collision_policy")]
    case_mode: String,
    unicode_normalization: String,
    #[serde(rename = "symlink_policy")]
    symlinks: String,
    governed_roots: Vec<String>,
    allowed_extensions: Vec<String>,
    allowed_filenames: Vec<String>,
    empty_filenames: Vec<String>,
    binary_extensions: Vec<String>,
    binary_filenames: Vec<String>,
    executable_paths: Vec<String>,
    manifest_extensions: Vec<String>,
    reserved_windows_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexEntry {
    mode: u32,
    path: String,
    valid_utf8: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Violation {
    path: String,
    rule: String,
    detail: String,
}

impl Violation {
    fn new(path: impl Into<String>, rule: &'static str, detail: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            rule: rule.to_owned(),
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}: {}", self.path, self.rule, self.detail)
    }
}

pub(super) fn run(root: &RepositoryRoot, runner: &ProcessRunner) -> Result {
    let policy = Policy::load(root)?;
    policy.validate()?;
    let entries = tracked_entries(root, runner)?;
    let violations = inspect_repository(root, &policy, &entries);
    if !violations.is_empty() {
        let mut message = format!("repo.verify-files: {} violation(s)", violations.len());
        for violation in violations {
            write!(message, "\n{violation}").expect("writing a String cannot fail");
        }
        return Err(message);
    }

    let governed = entries
        .iter()
        .filter(|entry| is_governed(&entry.path, &policy.governed_roots))
        .count();
    let binary = entries
        .iter()
        .filter(|entry| is_governed(&entry.path, &policy.governed_roots))
        .filter(|entry| is_binary(&entry.path, &policy))
        .count();
    Ok(report(
        root,
        "repo.verify-files",
        serde_json::json!({
            "policy": "rusttable.repository-files.v1",
            "tracked_files": entries.len(),
            "governed_files": governed,
            "binary_files": binary,
            "case_collision_policy": policy.case_mode.clone(),
            "unicode_normalization": policy.unicode_normalization.clone(),
        }),
    ))
}

impl Policy {
    fn load(root: &RepositoryRoot) -> Result<Self> {
        let path = root.join(POLICY_PATH);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("{POLICY_PATH}: cannot read policy: {error}"))?;
        toml::from_str(&source).map_err(|error| format!("{POLICY_PATH}: invalid TOML: {error}"))
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            return Err(format!(
                "{POLICY_PATH}: unsupported version {}",
                self.version
            ));
        }
        if self.max_path_length == 0 {
            return Err(format!("{POLICY_PATH}: max_path_length must be positive"));
        }
        if self.case_mode != "reject" {
            return Err(format!(
                "{POLICY_PATH}: case_collision_policy must be reject"
            ));
        }
        if self.unicode_normalization != "nfc" {
            return Err(format!("{POLICY_PATH}: unicode_normalization must be nfc"));
        }
        if self.symlinks != "reject" {
            return Err(format!("{POLICY_PATH}: symlink_policy must be reject"));
        }
        if self.governed_roots.is_empty() || self.allowed_extensions.is_empty() {
            return Err(format!(
                "{POLICY_PATH}: governed roots and allowed extensions must be non-empty"
            ));
        }
        let executable_paths = self.executable_paths.iter().collect::<BTreeSet<_>>();
        if executable_paths.len() != self.executable_paths.len() {
            return Err(format!(
                "{POLICY_PATH}: executable_paths contains duplicates"
            ));
        }
        Ok(())
    }
}

fn tracked_entries(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<Vec<IndexEntry>> {
    let request = ProcessRequest::new("git", ["ls-files", "--stage", "-z"])
        .current_dir(root.path())
        .environment(env::vars().collect())
        .limits(ProcessLimits {
            max_stdout_bytes: INDEX_OUTPUT_LIMIT,
            max_stderr_bytes: 64 * 1024,
            timeout: Duration::from_secs(5),
        });
    let result = runner
        .run(request)
        .map_err(|error| format!("git ls-files: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "git ls-files failed ({}): {}",
            result.receipt.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    parse_index(&result.stdout)
}

fn parse_index(output: &[u8]) -> Result<Vec<IndexEntry>> {
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "git index record has no path separator".to_owned())?;
        let header = std::str::from_utf8(&record[..separator])
            .map_err(|_| "git index record has invalid header UTF-8".to_owned())?;
        let mut fields = header.split_ascii_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| "git index record has no mode".to_owned())
            .and_then(|value| {
                u32::from_str_radix(value, 8)
                    .map_err(|_| format!("git index record has invalid mode {value}"))
            })?;
        if fields.next().is_none() || fields.next().is_none() {
            return Err("git index record has incomplete object fields".to_owned());
        }
        let path_bytes = &record[separator + 1..];
        let valid_utf8 = std::str::from_utf8(path_bytes).is_ok();
        let path = match std::str::from_utf8(path_bytes) {
            Ok(path) => path.to_owned(),
            Err(_) => format!("<invalid-utf8:{}>", hex(path_bytes)),
        };
        entries.push(IndexEntry {
            mode,
            path,
            valid_utf8,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn inspect_repository(
    root: &RepositoryRoot,
    policy: &Policy,
    entries: &[IndexEntry],
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut nfc_paths = BTreeMap::<String, Vec<String>>::new();
    let mut folded_paths = BTreeMap::<String, Vec<String>>::new();
    let mut present = BTreeSet::new();

    for entry in entries {
        if !is_governed(&entry.path, &policy.governed_roots) {
            continue;
        }
        present.insert(entry.path.clone());
        violations.extend(inspect_path(&entry.path, policy));
        if !entry.valid_utf8 {
            violations.push(Violation::new(
                &entry.path,
                "invalid-path-utf8",
                "repository-relative path is not UTF-8",
            ));
            continue;
        }
        validate_mode(entry, policy, &mut violations);
        let nfc = normalized_path(&entry.path);
        let folded = nfc.chars().flat_map(char::to_lowercase).collect::<String>();
        nfc_paths.entry(nfc).or_default().push(entry.path.clone());
        folded_paths
            .entry(folded)
            .or_default()
            .push(entry.path.clone());

        let full_path = root.join(&entry.path);
        let metadata = match fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                violations.push(Violation::new(
                    &entry.path,
                    "missing-file",
                    format!("tracked file cannot be inspected: {error}"),
                ));
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            violations.push(Violation::new(
                &entry.path,
                "symlink",
                "symlinks are not allowed in governed roots",
            ));
            continue;
        }
        if !metadata.is_file() {
            violations.push(Violation::new(
                &entry.path,
                "not-regular-file",
                "tracked governed entry is not a regular file",
            ));
            continue;
        }
        let bytes = match fs::read(&full_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                violations.push(Violation::new(
                    &entry.path,
                    "read-file",
                    format!("cannot read tracked file: {error}"),
                ));
                continue;
            }
        };
        violations.extend(inspect_contents(&entry.path, &bytes, policy));
    }

    violations.extend(collision_violations(
        &nfc_paths,
        "unicode-normalization-collision",
    ));
    violations.extend(collision_violations(&folded_paths, "case-collision"));
    for executable in &policy.executable_paths {
        if is_governed(executable, &policy.governed_roots) && !present.contains(executable) {
            violations.push(Violation::new(
                executable,
                "executable-path-missing",
                "declared executable path is not tracked",
            ));
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

fn inspect_path(path: &str, policy: &Policy) -> Vec<Violation> {
    let mut violations = Vec::new();
    if path.len() > policy.max_path_length {
        violations.push(Violation::new(
            path,
            "path-length",
            format!("{} bytes exceeds {}", path.len(), policy.max_path_length),
        ));
    }
    if path.starts_with('/') || path.contains('\\') {
        violations.push(Violation::new(
            path,
            "path-separator",
            "logical repository paths must use forward slashes",
        ));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            violations.push(Violation::new(
                path,
                "path-traversal",
                "path contains an empty, dot, or parent component",
            ));
        }
        if component.ends_with(' ') || component.ends_with('.') {
            violations.push(Violation::new(
                path,
                "trailing-space-dot",
                format!("path component {component:?} is not portable on Windows"),
            ));
        }
        if is_reserved_windows_name(component, policy) {
            violations.push(Violation::new(
                path,
                "windows-reserved-name",
                format!("path component {component:?} is reserved on Windows"),
            ));
        }
    }
    if !policy
        .allowed_filenames
        .iter()
        .any(|name| file_name(path) == Some(name.as_str()))
        && !policy
            .allowed_extensions
            .iter()
            .any(|extension| file_extension(path).is_some_and(|value| value == *extension))
    {
        violations.push(Violation::new(
            path,
            "extension",
            format!("file extension {:?} is not declared", file_extension(path)),
        ));
    }
    violations
}

fn validate_mode(entry: &IndexEntry, policy: &Policy, violations: &mut Vec<Violation>) {
    let executable = policy
        .executable_paths
        .iter()
        .any(|path| path == &entry.path);
    let expected = if executable { 0o100_755 } else { 0o100_644 };
    if entry.mode != expected {
        let rule = if entry.mode == 0o100_755 || expected == 0o100_755 {
            "executable-bit"
        } else {
            "file-mode"
        };
        violations.push(Violation::new(
            &entry.path,
            rule,
            format!("index mode {:o}, expected {:o}", entry.mode, expected),
        ));
    }
}

fn inspect_text(path: &str, bytes: &[u8]) -> Vec<Violation> {
    let mut violations = Vec::new();
    if bytes.windows(2).any(|window| window == b"\r\n") {
        violations.push(Violation::new(
            path,
            "crlf",
            "CRLF line ending found; use LF",
        ));
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        violations.push(Violation::new(path, "bom", "UTF-8 BOM is not allowed"));
    }
    if bytes.contains(&0) {
        violations.push(Violation::new(
            path,
            "nul",
            "NUL byte is not allowed in text",
        ));
    }
    if std::str::from_utf8(bytes).is_err() {
        violations.push(Violation::new(
            path,
            "invalid-utf8",
            "text is not valid UTF-8",
        ));
    }
    let trailing_newlines = bytes
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\n')
        .count();
    match trailing_newlines {
        0 => violations.push(Violation::new(
            path,
            "missing-final-newline",
            "text must end with exactly one LF",
        )),
        1 => {}
        count => violations.push(Violation::new(
            path,
            "extra-final-newline",
            format!("text ends with {count} LF bytes; expected one"),
        )),
    }
    violations
}

fn inspect_contents(path: &str, bytes: &[u8], policy: &Policy) -> Vec<Violation> {
    if is_binary(path, policy) {
        return Vec::new();
    }
    if bytes.is_empty()
        && policy
            .empty_filenames
            .iter()
            .any(|name| path.ends_with(name))
    {
        return Vec::new();
    }
    let mut violations = inspect_text(path, bytes);
    violations.extend(inspect_manifest_paths(path, bytes, policy));
    violations
}

fn inspect_manifest_paths(path: &str, bytes: &[u8], policy: &Policy) -> Vec<Violation> {
    let Some(extension) = file_extension(path) else {
        return Vec::new();
    };
    if !policy
        .manifest_extensions
        .iter()
        .any(|candidate| candidate == extension)
    {
        return Vec::new();
    }
    let source = String::from_utf8_lossy(bytes);
    let mut candidates = Vec::new();
    match extension {
        "toml" => {
            if let Ok(value) = toml::from_str::<toml::Value>(&source) {
                collect_manifest_strings(&value, None, &mut candidates);
            }
        }
        "json" => {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&source) {
                collect_json_strings(&value, None, &mut candidates);
            }
        }
        _ => {
            for line in source.lines() {
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                if is_manifest_key(key.trim()) {
                    candidates.push(value.trim().trim_matches(['"', '\'']).to_owned());
                }
            }
        }
    }
    candidates
        .into_iter()
        .filter(|candidate| candidate.contains("..") || is_absolute_path(candidate))
        .filter(|candidate| manifest_escapes_repository(path, candidate))
        .map(|candidate| {
            Violation::new(
                path,
                "manifest-path-traversal",
                format!("manifest path {candidate:?} escapes the repository"),
            )
        })
        .collect()
}

fn collect_manifest_strings(value: &toml::Value, key: Option<&str>, output: &mut Vec<String>) {
    match value {
        toml::Value::String(value) if key.is_some_and(is_manifest_key) => {
            output.push(value.clone());
        }
        toml::Value::Array(values) => values
            .iter()
            .for_each(|value| collect_manifest_strings(value, key, output)),
        toml::Value::Table(values) => values.iter().for_each(|(key, value)| {
            collect_manifest_strings(value, Some(key), output);
        }),
        _ => {}
    }
}

fn collect_json_strings(value: &serde_json::Value, key: Option<&str>, output: &mut Vec<String>) {
    match value {
        serde_json::Value::String(value) if key.is_some_and(is_manifest_key) => {
            output.push(value.clone());
        }
        serde_json::Value::Array(values) => values
            .iter()
            .for_each(|value| collect_json_strings(value, key, output)),
        serde_json::Value::Object(values) => values.iter().for_each(|(key, value)| {
            collect_json_strings(value, Some(key), output);
        }),
        _ => {}
    }
}

fn is_manifest_key(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    [
        "path",
        "paths",
        "root",
        "roots",
        "dir",
        "directory",
        "file",
        "files",
        "manifest",
        "output",
    ]
    .iter()
    .any(|candidate| key == *candidate || key.ends_with(&format!("_{candidate}")))
}

fn manifest_escapes_repository(manifest: &str, candidate: &str) -> bool {
    if is_absolute_path(candidate) {
        return true;
    }
    let mut components = manifest.split('/').collect::<Vec<_>>();
    components.pop();
    let normalized = candidate.replace('\\', "/");
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return true;
                }
            }
            value => components.push(value),
        }
    }
    false
}

fn is_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn collision_violations(
    paths: &BTreeMap<String, Vec<String>>,
    rule: &'static str,
) -> Vec<Violation> {
    paths
        .values()
        .filter_map(|paths| {
            let mut unique = paths.clone();
            unique.sort();
            unique.dedup();
            (unique.len() > 1).then_some(unique)
        })
        .flat_map(|paths| {
            let detail = format!("collides with [{}]", paths.join(", "));
            paths
                .iter()
                .map(|path| Violation::new(path, rule, detail.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalized_path(path: &str) -> String {
    path.nfc().to_string()
}

fn is_governed(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|root| match root.as_str() {
        "<root>" => !path.contains('/'),
        "crates/rusttable-*" => path
            .strip_prefix("crates/")
            .is_some_and(|rest| rest.starts_with("rusttable-") && rest.contains('/')),
        root => path == root || path.starts_with(&format!("{root}/")),
    })
}

fn is_binary(path: &str, policy: &Policy) -> bool {
    policy
        .binary_filenames
        .iter()
        .any(|name| path.ends_with(name))
        || file_extension(path).is_some_and(|extension| {
            policy
                .binary_extensions
                .iter()
                .any(|candidate| candidate == extension)
        })
}

fn file_extension(path: &str) -> Option<&str> {
    let name = file_name(path)?;
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty() && !extension.is_empty()).then_some(extension)
}

fn file_name(path: &str) -> Option<&str> {
    path.rsplit('/').next()
}

fn is_reserved_windows_name(component: &str, policy: &Policy) -> bool {
    let stem = component.split('.').next().unwrap_or_default();
    policy
        .reserved_windows_names
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing a String cannot fail");
        output
    })
}

#[cfg(test)]
mod tests {
    use super::{
        IndexEntry, Policy, Violation, collision_violations, inspect_manifest_paths, inspect_path,
        inspect_text, is_binary, manifest_escapes_repository, normalized_path, parse_index,
        validate_mode,
    };

    fn policy() -> Policy {
        Policy {
            version: 1,
            max_path_length: 240,
            case_mode: "reject".to_owned(),
            unicode_normalization: "nfc".to_owned(),
            symlinks: "reject".to_owned(),
            governed_roots: vec!["<root>".to_owned(), "fixtures".to_owned()],
            allowed_extensions: vec!["md".to_owned(), "raw".to_owned(), "toml".to_owned()],
            allowed_filenames: vec!["LICENSE".to_owned()],
            empty_filenames: vec![".gitkeep".to_owned()],
            binary_extensions: vec!["raw".to_owned()],
            binary_filenames: Vec::new(),
            executable_paths: vec!["scripts/run.sh".to_owned()],
            manifest_extensions: vec!["toml".to_owned()],
            reserved_windows_names: vec!["CON".to_owned(), "NUL".to_owned()],
        }
    }

    #[test]
    fn synthetic_index_fixture_preserves_modes_and_paths() {
        let output = b"100644 deadbeef 0\tREADME.md\0\
100755 deadbeef 0\tscripts/run.sh\0";
        let entries = parse_index(output).expect("synthetic index parses");
        assert_eq!(entries[0].mode, 0o100_644);
        assert_eq!(entries[0].path, "README.md");
        assert_eq!(entries[1].mode, 0o100_755);
        assert_eq!(entries[1].path, "scripts/run.sh");
    }

    #[test]
    fn executable_mode_drift_is_reported_as_a_stable_rule() {
        let mut violations = Vec::new();
        validate_mode(
            &IndexEntry {
                mode: 0o100_644,
                path: "scripts/run.sh".to_owned(),
                valid_utf8: true,
            },
            &policy(),
            &mut violations,
        );
        assert_eq!(violations[0].path, "scripts/run.sh");
        assert_eq!(violations[0].rule, "executable-bit");
    }

    #[test]
    fn text_fixture_rules_cover_crlf_bom_invalid_utf8_nul_and_newlines() {
        let violations = inspect_text("fixture.md", b"\xef\xbb\xbfbad\r\n\xff\0\n\n");
        let rules = violations
            .iter()
            .map(|violation| violation.rule.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            rules,
            ["crlf", "bom", "nul", "invalid-utf8", "extra-final-newline"]
        );
    }

    #[test]
    fn binary_fixture_is_not_decoded_as_text() {
        let policy = policy();
        assert!(is_binary("fixtures/picture.raw", &policy));
        assert!(super::inspect_contents("picture.raw", &[0, 0xff, 0], &policy).is_empty());
    }

    #[test]
    fn path_fixtures_cover_windows_portability_and_manifest_escape() {
        let policy = policy();
        let violations = inspect_path("fixtures/CON.txt", &policy);
        assert!(violations.iter().any(|violation| {
            violation.path == "fixtures/CON.txt" && violation.rule == "windows-reserved-name"
        }));
        assert!(
            inspect_path("fixtures/name. ", &policy)
                .iter()
                .any(|violation| violation.rule == "trailing-space-dot")
        );
        assert!(
            inspect_path("fixtures/notLICENSE", &policy)
                .iter()
                .any(|violation| violation.rule == "extension")
        );
        assert!(manifest_escapes_repository(
            "crates/rusttable-core/Cargo.toml",
            "../../../outside"
        ));
        assert!(!manifest_escapes_repository(
            "crates/rusttable-core/Cargo.toml",
            "../rusttable-image"
        ));
        let violations = inspect_manifest_paths(
            "fixtures/manifest.toml",
            b"path = \"../../outside\"\n",
            &Policy {
                manifest_extensions: vec!["toml".to_owned()],
                ..policy
            },
        );
        assert_eq!(violations[0].rule, "manifest-path-traversal");
    }

    #[test]
    fn case_and_unicode_normalization_collisions_are_deterministic() {
        let composed = "fixtures/Caf\u{e9}.toml".to_owned();
        let decomposed = "fixtures/Cafe\u{301}.toml".to_owned();
        assert_eq!(normalized_path(&composed), normalized_path(&decomposed));
        let mut normalized = std::collections::BTreeMap::new();
        normalized.insert(
            "fixtures/café.toml".to_owned(),
            vec![composed.clone(), decomposed.clone()],
        );
        let violations = collision_violations(&normalized, "unicode-normalization-collision");
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].path, decomposed);
        assert_eq!(violations[0].rule, "unicode-normalization-collision");
        let mut case = std::collections::BTreeMap::new();
        case.insert(
            "fixtures/readme.md".to_owned(),
            vec![
                "fixtures/README.md".to_owned(),
                "fixtures/readme.md".to_owned(),
            ],
        );
        assert_eq!(collision_violations(&case, "case-collision").len(), 2);
        case.insert(
            "fixtures/duplicate.md".to_owned(),
            vec![
                "fixtures/duplicate.md".to_owned(),
                "fixtures/duplicate.md".to_owned(),
            ],
        );
        assert_eq!(collision_violations(&case, "case-collision").len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_fixture_is_inspected_without_following_the_target() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let directory = std::env::temp_dir().join(format!(
            "rusttable-file-policy-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("fixture directory");
        std::fs::write(directory.join("target.md"), b"target\n").expect("target");
        symlink("target.md", directory.join("link.md")).expect("symlink");
        let metadata = std::fs::symlink_metadata(directory.join("link.md")).expect("metadata");
        assert!(metadata.file_type().is_symlink());
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn violation_display_contains_exact_path_and_rule() {
        let violation = Violation::new("fixtures/a.md", "bom", "UTF-8 BOM is not allowed");
        assert_eq!(
            violation.to_string(),
            "fixtures/a.md: bom: UTF-8 BOM is not allowed"
        );
    }
}
