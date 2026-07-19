use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};

use super::super::Result;
use super::io::digest_json;
use super::model::{Finding, Inventory, InventoryEntry, Selector, SourceMap, SourceTree};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;

impl SourceTree {
    pub(super) fn open_baseline(reference: &Path, runner: &ProcessRunner) -> Result<Self> {
        if !reference.is_dir() {
            return Err(format!(
                "reference tree is not a directory: {}",
                reference.display()
            ));
        }
        // Resolve the accepted object directly rather than requiring the
        // read-only reference checkout to have that revision at HEAD.
        git_bytes(
            reference,
            runner,
            [
                "cat-file",
                "-e",
                &format!("{}^{{commit}}", super::PINNED_COMMIT),
            ],
        )?;
        Ok(Self {
            reference: reference.to_owned(),
            commit: super::PINNED_COMMIT.to_owned(),
        })
    }

    pub(super) fn open_commit(
        reference: &Path,
        requested_commit: &str,
        runner: &ProcessRunner,
    ) -> Result<Self> {
        if !reference.is_dir() {
            return Err(format!(
                "reference tree is not a directory: {}",
                reference.display()
            ));
        }
        git_bytes(
            reference,
            runner,
            ["cat-file", "-e", &format!("{requested_commit}^{{commit}}")],
        )?;
        Ok(Self {
            reference: reference.to_owned(),
            commit: requested_commit.to_owned(),
        })
    }

    pub(super) fn show(&self, path: &str, runner: &ProcessRunner) -> Result<Vec<u8>> {
        git_bytes(
            &self.reference,
            runner,
            ["show", &format!("{}:{path}", self.commit)],
        )
    }
}

pub(super) fn build_inventory(tree: &SourceTree, runner: &ProcessRunner) -> Result<Inventory> {
    let output = git_bytes(
        &tree.reference,
        runner,
        ["ls-tree", "-r", "-z", &tree.commit],
    )?;
    let mut entries = parse_tree(&output)?;
    fill_blob_sizes(&mut entries, tree, runner)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let tree_sha256 = digest_json(&entries)?;
    let tree_id = String::from_utf8(git_bytes(
        &tree.reference,
        runner,
        ["rev-parse", &format!("{}^{{tree}}", tree.commit)],
    )?)
    .map_err(|_| "git tree id is not valid UTF-8".to_owned())?
    .trim()
    .to_owned();
    Ok(Inventory {
        schema_version: super::INVENTORY_SCHEMA,
        repository: super::REPOSITORY.to_owned(),
        source_commit: tree.commit.clone(),
        tree_id,
        tree_sha256,
        entries,
    })
}

fn parse_tree(bytes: &[u8]) -> Result<Vec<InventoryEntry>> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or_else(|| "git ls-tree record has no path separator".to_owned())?;
            let header = std::str::from_utf8(&record[..tab])
                .map_err(|_| "git ls-tree record has invalid UTF-8 header".to_owned())?;
            let path = std::str::from_utf8(&record[tab + 1..])
                .map_err(|_| "git ls-tree path is not UTF-8".to_owned())?
                .to_owned();
            let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
            if fields.len() != 3 || !matches!(fields[1], "blob" | "commit") {
                return Err(format!("unsupported tree record for {path:?}"));
            }
            Ok(InventoryEntry {
                path: path.clone(),
                mode: fields[0].to_owned(),
                object_type: fields[1].to_owned(),
                object_id: fields[2].to_owned(),
                size_bytes: 0,
                class: classify_path(&path),
            })
        })
        .collect()
}

fn fill_blob_sizes(
    entries: &mut [InventoryEntry],
    tree: &SourceTree,
    runner: &ProcessRunner,
) -> Result<()> {
    let input = entries
        .iter()
        .filter(|entry| entry.object_type == "blob")
        .map(|entry| entry.object_id.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let output = git_bytes_with_stdin(
        &tree.reference,
        runner,
        [
            "cat-file",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ],
        format!("{input}\n").into_bytes(),
    )?;
    let mut sizes = BTreeMap::new();
    for line in String::from_utf8(output)
        .map_err(|_| "git cat-file returned invalid UTF-8".to_owned())?
        .lines()
    {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[1], "blob" | "commit") {
            return Err(format!(
                "git cat-file returned invalid object record: {line}"
            ));
        }
        let size = fields[2]
            .parse::<u64>()
            .map_err(|_| format!("invalid blob size for {}", fields[0]))?;
        sizes.insert(fields[0].to_owned(), (fields[1].to_owned(), size));
    }
    for entry in entries {
        if entry.object_type == "commit" {
            // Gitlinks point at a submodule commit.  The commit object is
            // intentionally not required in the shallow/promisor checkout;
            // the exact gitlink OID is the size-bearing source identity.
            entry.size_bytes = 0;
            continue;
        }
        let (object_type, size) = sizes
            .get(&entry.object_id)
            .ok_or_else(|| format!("missing blob size for {}", entry.path))?;
        if object_type != &entry.object_type {
            return Err(format!("object type changed for {}", entry.path));
        }
        entry.size_bytes = *size;
    }
    Ok(())
}

pub(super) fn classify_path(path: &str) -> String {
    if path.starts_with("src/iop/") {
        "image-operation".to_owned()
    } else if path.starts_with("data/kernels/") {
        "gpu-kernel".to_owned()
    } else if path.starts_with("src/imageio/") {
        "image-io-metadata".to_owned()
    } else if path.starts_with("src/views/")
        || path.starts_with("src/libs/")
        || path.starts_with("src/gui/")
        || path.starts_with("src/dtgtk/")
        || path.starts_with("src/bauhaus/")
    {
        "desktop-ui".to_owned()
    } else if path.starts_with("src/lua/") || path.starts_with("data/lua/") {
        "scripting-extension".to_owned()
    } else if path.starts_with("tests/") {
        "test-oracle".to_owned()
    } else if path.starts_with("packaging/") || path.starts_with(".github/") {
        "platform-package".to_owned()
    } else if path.starts_with("src/")
        || path.ends_with("CMakeLists.txt")
        || Path::new(path)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cmake"))
    {
        "foundation-runtime".to_owned()
    } else {
        "supporting-data".to_owned()
    }
}

pub(super) fn validate_inventory(inventory: &Inventory) -> std::result::Result<(), String> {
    if inventory.schema_version != super::INVENTORY_SCHEMA {
        return Err(format!(
            "unsupported source inventory schema {}",
            inventory.schema_version
        ));
    }
    if inventory.repository != super::REPOSITORY {
        return Err(format!(
            "inventory repository is {}, expected {}",
            inventory.repository,
            super::REPOSITORY
        ));
    }
    if inventory.source_commit != super::PINNED_COMMIT {
        return Err(format!(
            "inventory source commit is {}, expected {}",
            inventory.source_commit,
            super::PINNED_COMMIT
        ));
    }
    if inventory.tree_id.len() != 40
        || !inventory
            .tree_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("source inventory has an invalid tree id".to_owned());
    }
    if inventory.entries.is_empty() {
        return Err("source inventory is empty".to_owned());
    }
    let mut paths = BTreeSet::new();
    let mut previous_path = None;
    for entry in &inventory.entries {
        if previous_path.is_some_and(|path: &str| path >= entry.path.as_str()) {
            return Err(format!(
                "inventory entries are not strictly sorted at {}",
                entry.path
            ));
        }
        previous_path = Some(entry.path.as_str());
        if entry.path.is_empty() || !paths.insert(&entry.path) {
            return Err(format!(
                "inventory has duplicate or empty path {:?}",
                entry.path
            ));
        }
        if entry.object_id.len() != 40
            || !entry.object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!(
                "inventory has invalid object id for {}",
                entry.path
            ));
        }
        if !matches!(entry.object_type.as_str(), "blob" | "commit") {
            return Err(format!(
                "inventory entry has unsupported type: {}",
                entry.path
            ));
        }
        if entry.mode.is_empty() || entry.class.is_empty() {
            return Err(format!(
                "inventory entry is missing mode or class: {}",
                entry.path
            ));
        }
    }
    if digest_json(&inventory.entries)? != inventory.tree_sha256 {
        return Err("source inventory checksum does not match entries".to_owned());
    }
    Ok(())
}

pub(super) fn compare_inventory(expected: &Inventory, actual: &Inventory) -> Vec<Finding> {
    let old = expected
        .entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect::<BTreeMap<_, _>>();
    let new = actual
        .entries
        .iter()
        .map(|entry| (&entry.path, entry))
        .collect::<BTreeMap<_, _>>();
    let mut findings = Vec::new();
    for path in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        match (old.get(path), new.get(path)) {
            (None, Some(_)) => findings.push(Finding::error(
                "inventory-added",
                None,
                Some((*path).to_owned()),
                "path added or appeared in candidate tree",
            )),
            (Some(_), None) => findings.push(Finding::error(
                "inventory-removed",
                None,
                Some((*path).to_owned()),
                "path removed from candidate tree",
            )),
            (Some(left), Some(right)) if left != right => findings.push(Finding::error(
                "inventory-entry-changed",
                None,
                Some((*path).to_owned()),
                format!("exact inventory entry changed: expected {left:?}, actual {right:?}"),
            )),
            _ => {}
        }
    }
    if expected.tree_sha256 != actual.tree_sha256 {
        findings.push(Finding::error(
            "inventory-checksum-changed",
            None,
            None,
            format!(
                "tree checksum changed: expected {}, actual {}",
                expected.tree_sha256, actual.tree_sha256
            ),
        ));
    }
    if expected.tree_id != actual.tree_id {
        findings.push(Finding::error(
            "inventory-tree-changed",
            None,
            None,
            format!(
                "tree id changed: expected {}, actual {}",
                expected.tree_id, actual.tree_id
            ),
        ));
    }
    findings
}

pub(super) fn validate_anchors(
    map: &SourceMap,
    inventory: &Inventory,
    tree: &SourceTree,
    runner: &ProcessRunner,
    selected: Option<u64>,
) -> Result<Vec<Finding>> {
    let entries = inventory
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let mut findings = Vec::new();
    for record in &map.issues {
        if selected.is_some_and(|number| record.number != number)
            || selected.is_none() && record.source_status != "anchored"
        {
            continue;
        }
        for anchor in &record.anchors {
            let Some(entry) = entries.get(anchor.path.as_str()) else {
                findings.push(Finding::error(
                    "missing-anchor-path",
                    Some(record.number),
                    Some(anchor.path.clone()),
                    "anchor path is absent from the pinned inventory",
                ));
                continue;
            };
            if entry.object_id != anchor.blob_id {
                findings.push(Finding::error(
                    "anchor-blob-drift",
                    Some(record.number),
                    Some(anchor.path.clone()),
                    format!(
                        "anchor blob {} does not match {}",
                        anchor.blob_id, entry.object_id
                    ),
                ));
                continue;
            }
            let content = tree.show(&anchor.path, runner)?;
            if !selector_matches(&content, &anchor.selector) {
                findings.push(Finding::error(
                    "missing-selector",
                    Some(record.number),
                    Some(anchor.path.clone()),
                    format!("selector {:?} is absent", anchor.selector),
                ));
            }
        }
    }
    Ok(findings)
}

pub(super) fn selector_matches(content: &[u8], selector: &Selector) -> bool {
    let text = String::from_utf8_lossy(content);
    match selector {
        Selector::InventoryPath => true,
        Selector::BoundedRange { start, end, .. } => {
            *start >= 1 && start <= end && *end <= text.lines().count() as u64
        }
        Selector::Symbol { name } => exact_identifier(&text, name),
        Selector::Table { name } => {
            exact_identifier(&text, name)
                && text.lines().any(|line| {
                    let lower = line.to_ascii_lowercase();
                    (lower.contains("create table")
                        || lower.contains("alter table")
                        || lower.contains("sqlite3"))
                        && exact_identifier(line, name)
                })
        }
        Selector::ConfigKey { name } => text.lines().any(|line| {
            exact_identifier(line, name)
                && (line.contains('=') || line.contains(':') || line.contains('"'))
        }),
        Selector::Kernel { name } => text.lines().any(|line| {
            exact_identifier(line, name)
                && (line.contains("kernel")
                    || line.contains("__kernel")
                    || line.contains("program"))
        }),
        Selector::Action { name } | Selector::Registration { name } => {
            exact_identifier(&text, name)
        }
    }
}

fn exact_identifier(content: &str, expected: &str) -> bool {
    !expected.is_empty()
        && content
            .split(|character: char| !(character == '_' || character.is_ascii_alphanumeric()))
            .any(|token| token == expected)
}

fn git_bytes<I, S>(directory: &Path, runner: &ProcessRunner, arguments: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let request = ProcessRequest::new("git", arguments)
        .profile(EnvironmentProfile::GitTool)
        .current_dir(directory)
        .limits(ProcessLimits {
            max_stdout_bytes: OUTPUT_LIMIT,
            max_stderr_bytes: 64 * 1024,
            timeout: Some(COMMAND_TIMEOUT),
        });
    let result = runner
        .run(request)
        .map_err(|error| format!("git source inspection: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "git source inspection failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.stdout)
}

fn git_bytes_with_stdin<I, S>(
    directory: &Path,
    runner: &ProcessRunner,
    arguments: I,
    stdin: Vec<u8>,
) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let request = ProcessRequest::new("git", arguments)
        .profile(EnvironmentProfile::GitTool)
        .current_dir(directory)
        .stdin_bytes(stdin)
        .limits(ProcessLimits {
            max_stdout_bytes: OUTPUT_LIMIT,
            max_stderr_bytes: 64 * 1024,
            timeout: Some(COMMAND_TIMEOUT),
        });
    let result = runner
        .run(request)
        .map_err(|error| format!("git source inspection: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "git source inspection failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.stdout)
}
