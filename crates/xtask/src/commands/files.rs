use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;

use icu_casemap::CaseMapper;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use super::file_source::{self, BlobData, EntryKind, SourceEntry, SourceKind};
use super::{Result, report};
use crate::cli::{FilePolicyArgs, FileSource};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

const POLICY_PATH: &str = "quality/repository-files.toml";
const ATTRIBUTES_PATH: &str = ".gitattributes";

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
    max_path_bytes: usize,
    max_path_utf16: usize,
    max_component_bytes: usize,
    max_component_utf16: usize,
    attributes_rules: Vec<String>,
    artifact_classes: Vec<ArtifactClass>,
    magic_signatures: Vec<MagicSignature>,
}

#[derive(Debug, Deserialize)]
struct ArtifactClass {
    id: String,
    kind: String,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    filenames: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
    max_size_bytes: usize,
    #[serde(default)]
    empty_allowed: bool,
}

#[derive(Debug, Deserialize)]
struct MagicSignature {
    extension: String,
    offset: usize,
    bytes: String,
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

pub(super) fn run(
    root: &RepositoryRoot,
    arguments: &FilePolicyArgs,
    runner: &ProcessRunner,
) -> Result {
    let policy = Policy::load(root)?;
    policy.validate()?;
    let requested_source = match arguments.source {
        FileSource::Auto => "auto",
        FileSource::Index => "index",
        FileSource::Tree => "tree",
        FileSource::Worktree => "worktree",
    };
    let source = file_source::resolve_source(
        requested_source,
        std::env::var_os("CI").is_some() || std::env::var_os("GITHUB_ACTIONS").is_some(),
    )?;
    let entries = file_source::read_entries(root, runner, source, &arguments.treeish)?;
    let contents = match source {
        SourceKind::Worktree => BTreeMap::new(),
        SourceKind::Index | SourceKind::Tree => {
            file_source::read_blob_contents(root, runner, &entries)?
        }
    };
    let violations = inspect_repository(root, &policy, source, &entries, &contents);
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
            "unicode_case_folding": "icu4x-2.2.0",
            "unicode_version": "16.0",
            "source": source.label(),
            "treeish": (source == SourceKind::Tree).then_some(arguments.treeish.clone()),
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
        if self.max_path_bytes == 0
            || self.max_path_utf16 == 0
            || self.max_component_bytes == 0
            || self.max_component_utf16 == 0
        {
            return Err(format!(
                "{POLICY_PATH}: Windows path limits must be positive"
            ));
        }
        if self.attributes_rules.is_empty() || self.artifact_classes.is_empty() {
            return Err(format!(
                "{POLICY_PATH}: attributes rules and artifact classes must be non-empty"
            ));
        }
        for class in &self.artifact_classes {
            if !["text", "binary"].contains(&class.kind.as_str())
                || class.max_size_bytes == 0
                || (class.extensions.is_empty()
                    && class.filenames.is_empty()
                    && class.paths.is_empty())
            {
                return Err(format!(
                    "{POLICY_PATH}: invalid artifact class {}",
                    class.id
                ));
            }
        }
        Ok(())
    }
}

fn inspect_repository(
    root: &RepositoryRoot,
    policy: &Policy,
    source: SourceKind,
    entries: &[SourceEntry],
    contents: &BTreeMap<String, BlobData>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut nfc_paths = BTreeMap::<String, Vec<String>>::new();
    let mut folded_paths = BTreeMap::<String, Vec<String>>::new();
    let mut present = BTreeSet::new();

    for entry in entries {
        if !entry.valid_utf8 {
            violations.push(Violation::new(
                &entry.path,
                "invalid-path-utf8",
                "repository-relative path is not UTF-8",
            ));
            continue;
        }
        if !is_governed(&entry.path, &policy.governed_roots) {
            continue;
        }
        present.insert(entry.path.clone());
        violations.extend(inspect_path(&entry.path, policy));
        validate_entry(entry, policy, &mut violations);
        let nfc = normalized_path(&entry.path);
        let folded = CaseMapper::new().fold_string(&nfc).into_owned();
        nfc_paths.entry(nfc).or_default().push(entry.path.clone());
        folded_paths
            .entry(folded)
            .or_default()
            .push(entry.path.clone());
        violations.extend(inspect_entry_contents(
            root, source, contents, entry, policy,
        ));
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
    violations.extend(validate_attributes(root, policy, source, contents));
    violations.sort();
    violations.dedup();
    violations
}

fn inspect_entry_contents(
    root: &RepositoryRoot,
    source: SourceKind,
    contents: &BTreeMap<String, BlobData>,
    entry: &SourceEntry,
    policy: &Policy,
) -> Vec<Violation> {
    if entry.stage != 0 {
        return Vec::new();
    }
    let mut violations = Vec::new();
    let bytes = match source {
        SourceKind::Worktree => match fs::symlink_metadata(root.join(&entry.path)) {
            Ok(metadata) if metadata.is_file() => match fs::read(root.join(&entry.path)) {
                Ok(bytes) => bytes,
                Err(error) => {
                    violations.push(Violation::new(
                        &entry.path,
                        "read-file",
                        format!("cannot read worktree file: {error}"),
                    ));
                    return violations;
                }
            },
            Ok(metadata) if metadata.file_type().is_symlink() => {
                violations.push(Violation::new(
                    &entry.path,
                    "symlink",
                    "symlinks are not allowed in governed roots",
                ));
                return violations;
            }
            Ok(_) => {
                violations.push(Violation::new(
                    &entry.path,
                    "not-regular-file",
                    "tracked governed entry is not a regular worktree file",
                ));
                return violations;
            }
            Err(error) => {
                violations.push(Violation::new(
                    &entry.path,
                    "missing-file",
                    format!("tracked worktree file cannot be inspected: {error}"),
                ));
                return violations;
            }
        },
        SourceKind::Index | SourceKind::Tree => match contents.get(&entry.path) {
            Some(BlobData::Bytes(bytes)) => bytes.clone(),
            Some(BlobData::TooLarge(size)) => {
                violations.push(Violation::new(
                    &entry.path,
                    "file-size",
                    format!(
                        "blob is {size} bytes; bounded reader limit is {}",
                        file_source::MAX_BLOB_BYTES
                    ),
                ));
                return violations;
            }
            None => {
                violations.push(Violation::new(
                    &entry.path,
                    "missing-blob",
                    "regular governed entry has no readable Git blob",
                ));
                return violations;
            }
        },
    };
    violations.extend(inspect_contents(&entry.path, &bytes, policy));
    violations
}

fn inspect_path(path: &str, policy: &Policy) -> Vec<Violation> {
    let mut violations = Vec::new();
    if path.len() > policy.max_path_bytes || path.len() > policy.max_path_length {
        violations.push(Violation::new(
            path,
            "path-byte-length",
            format!("{} bytes exceeds {}", path.len(), policy.max_path_bytes),
        ));
    }
    let utf16_length = path.encode_utf16().count();
    if utf16_length > policy.max_path_utf16 {
        violations.push(Violation::new(
            path,
            "path-utf16-length",
            format!(
                "{utf16_length} UTF-16 code units exceeds {}",
                policy.max_path_utf16
            ),
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
        if component.len() > policy.max_component_bytes {
            violations.push(Violation::new(
                path,
                "component-byte-length",
                format!(
                    "component {component:?} exceeds {} bytes",
                    policy.max_component_bytes
                ),
            ));
        }
        let component_utf16 = component.encode_utf16().count();
        if component_utf16 > policy.max_component_utf16 {
            violations.push(Violation::new(
                path,
                "component-utf16-length",
                format!(
                    "component {component:?} exceeds {} UTF-16 code units",
                    policy.max_component_utf16
                ),
            ));
        }
        if component
            .chars()
            .any(|character| character.is_control() || "<>:\"/\\|?*".contains(character))
        {
            violations.push(Violation::new(
                path,
                "windows-invalid-character",
                format!("component {component:?} contains a Windows-invalid character"),
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
            .any(|extension| file_extension(path).is_some_and(|value| value == extension))
        && artifact_class(path, policy).is_none()
    {
        violations.push(Violation::new(
            path,
            "extension",
            format!("file extension {:?} is not declared", file_extension(path)),
        ));
    }
    violations
}

fn validate_entry(entry: &SourceEntry, policy: &Policy, violations: &mut Vec<Violation>) {
    match entry.kind {
        EntryKind::Symlink => violations.push(Violation::new(
            &entry.path,
            "symlink",
            "Git entry is a symlink; governed paths require regular blobs",
        )),
        EntryKind::Submodule => violations.push(Violation::new(
            &entry.path,
            "submodule",
            "Git entry is a submodule; governed paths require regular blobs",
        )),
        EntryKind::Unsupported => violations.push(Violation::new(
            &entry.path,
            "unsupported-mode",
            format!("Git entry mode {:o} is unsupported", entry.mode),
        )),
        EntryKind::Regular => {}
    }
    if entry.stage != 0 {
        violations.push(Violation::new(
            &entry.path,
            "unmerged-index-entry",
            format!("index stage {} is not stage zero", entry.stage),
        ));
    }
    validate_mode(entry, policy, violations);
}

fn validate_mode(entry: &SourceEntry, policy: &Policy, violations: &mut Vec<Violation>) {
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
    if bytes
        .iter()
        .enumerate()
        .any(|(index, byte)| *byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
    {
        violations.push(Violation::new(
            path,
            "lone-cr",
            "lone CR line ending found; use LF",
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
    let class = artifact_class(path, policy);
    let mut violations = Vec::new();
    if let Some(class) = class {
        if bytes.len() > class.max_size_bytes {
            violations.push(Violation::new(
                path,
                "file-size",
                format!(
                    "{} bytes exceeds {} for class {}",
                    bytes.len(),
                    class.max_size_bytes,
                    class.id
                ),
            ));
        }
        if class.kind == "binary" {
            violations.extend(validate_magic(path, bytes, class, policy));
            return violations;
        }
        if bytes.is_empty() && class.empty_allowed {
            return violations;
        }
    } else if is_binary(path, policy) {
        return violations;
    }
    if bytes.is_empty()
        && policy
            .empty_filenames
            .iter()
            .any(|name| file_name(path) == Some(name.as_str()))
    {
        return violations;
    }
    violations.extend(inspect_text(path, bytes));
    violations.extend(inspect_manifest_paths(path, bytes, policy));
    violations
}

fn artifact_class<'a>(path: &str, policy: &'a Policy) -> Option<&'a ArtifactClass> {
    policy.artifact_classes.iter().find(|class| {
        class.paths.iter().any(|candidate| candidate == path)
            || class
                .filenames
                .iter()
                .any(|candidate| file_name(path) == Some(candidate.as_str()))
            || class
                .extensions
                .iter()
                .any(|candidate| file_extension(path) == Some(candidate.as_str()))
    })
}

fn validate_magic(
    path: &str,
    bytes: &[u8],
    class: &ArtifactClass,
    policy: &Policy,
) -> Vec<Violation> {
    if !class.paths.is_empty() {
        return Vec::new();
    }
    let Some(extension) = file_extension(path) else {
        return Vec::new();
    };
    let signatures = policy
        .magic_signatures
        .iter()
        .filter(|signature| signature.extension == extension)
        .collect::<Vec<_>>();
    if signatures.is_empty() {
        return Vec::new();
    }
    let matches = signatures.iter().any(|signature| {
        let Ok(expected) = decode_hex(&signature.bytes) else {
            return false;
        };
        bytes
            .get(signature.offset..signature.offset.saturating_add(expected.len()))
            .is_some_and(|actual| actual == expected.as_slice())
    });
    if matches {
        Vec::new()
    } else {
        vec![Violation::new(
            path,
            "binary-signature",
            format!("binary class {extension} does not match a registered magic signature"),
        )]
    }
}

fn decode_hex(value: &str) -> std::result::Result<Vec<u8>, ()> {
    if !value.len().is_multiple_of(2) {
        return Err(());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| ()))
        .collect()
}

fn validate_attributes(
    root: &RepositoryRoot,
    policy: &Policy,
    source: SourceKind,
    contents: &BTreeMap<String, BlobData>,
) -> Vec<Violation> {
    let bytes = match source {
        SourceKind::Worktree => fs::read(root.join(ATTRIBUTES_PATH)).ok(),
        SourceKind::Index | SourceKind::Tree => match contents.get(ATTRIBUTES_PATH) {
            Some(BlobData::Bytes(bytes)) => Some(bytes.clone()),
            _ => None,
        },
    };
    let Some(bytes) = bytes else {
        return vec![Violation::new(
            ATTRIBUTES_PATH,
            "attributes-coverage",
            format!("{ATTRIBUTES_PATH} is absent from the selected source"),
        )];
    };
    let source = String::from_utf8_lossy(&bytes);
    policy
        .attributes_rules
        .iter()
        .filter(|required| !source.lines().any(|line| line.trim() == required.as_str()))
        .map(|required| {
            Violation::new(
                ATTRIBUTES_PATH,
                "attributes-coverage",
                format!("missing policy rule {required:?}"),
            )
        })
        .collect()
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
    if artifact_class(path, policy).is_some_and(|class| class.kind == "binary") {
        return true;
    }
    policy
        .binary_filenames
        .iter()
        .any(|name| file_name(path) == Some(name.as_str()))
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
    let trimmed = component.trim_end_matches([' ', '.']);
    let stem = trimmed.split('.').next().unwrap_or_default();
    policy
        .reserved_windows_names
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
}

#[cfg(test)]
#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
