use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

use super::Result;
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const COMMAND_OUTPUT_LIMIT: usize = 64 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_BLOB_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Index,
    Tree,
    Worktree,
}

impl SourceKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Tree => "tree",
            Self::Worktree => "worktree",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    Regular,
    Symlink,
    Submodule,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceEntry {
    pub(crate) mode: u32,
    pub(crate) object_id: String,
    pub(crate) stage: u8,
    pub(crate) path: String,
    pub(crate) valid_utf8: bool,
    pub(crate) kind: EntryKind,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BlobData {
    Bytes(Vec<u8>),
    TooLarge(u64),
}

pub(crate) fn resolve_source(source: &str, ci: bool) -> Result<SourceKind> {
    match source {
        "auto" if ci => Ok(SourceKind::Tree),
        "index" | "auto" => Ok(SourceKind::Index),
        "tree" => Ok(SourceKind::Tree),
        "worktree" => Ok(SourceKind::Worktree),
        other => Err(format!(
            "repo.verify-files: invalid source {other:?}; expected auto, index, tree, or worktree"
        )),
    }
}

pub(crate) fn read_entries(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    source: SourceKind,
    treeish: &str,
) -> Result<Vec<SourceEntry>> {
    let (args, label) = match source {
        SourceKind::Index | SourceKind::Worktree => {
            (vec!["ls-files", "--stage", "-z"], source.label())
        }
        SourceKind::Tree => (
            vec!["ls-tree", "-r", "-z", "--full-tree", treeish],
            source.label(),
        ),
    };
    let request = ProcessRequest::new("git", args)
        .profile(EnvironmentProfile::GitTool)
        .current_dir(root.path())
        .limits(ProcessLimits {
            max_stdout_bytes: COMMAND_OUTPUT_LIMIT,
            max_stderr_bytes: 64 * 1024,
            timeout: COMMAND_TIMEOUT,
        });
    let result = runner
        .run(request)
        .map_err(|error| format!("git {label} listing: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "git {label} listing failed ({}): {}",
            result.receipt.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    match source {
        SourceKind::Tree => parse_tree(&result.stdout),
        SourceKind::Index | SourceKind::Worktree => parse_index(&result.stdout),
    }
}

pub(crate) fn read_blob_contents(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    entries: &[SourceEntry],
) -> Result<BTreeMap<String, BlobData>> {
    let regular = entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Regular && entry.stage == 0)
        .collect::<Vec<_>>();
    let mut sizes = BTreeMap::new();
    let check = batch_git(
        root,
        runner,
        "--batch-check",
        &regular,
        COMMAND_OUTPUT_LIMIT,
    )?;
    for (entry, line) in regular.iter().zip(batch_lines(&check)) {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() == 2 && fields[1] == "missing" {
            return Err(format!("{}: missing-blob: {}", entry.path, entry.object_id));
        }
        if fields.len() != 3 || fields[0] != entry.object_id || fields[1] != "blob" {
            return Err(format!(
                "{}: object-id-mismatch: unexpected cat-file header {line:?}",
                entry.path
            ));
        }
        let size = fields[2]
            .parse::<u64>()
            .map_err(|_| format!("{}: invalid-blob-size: {line:?}", entry.path))?;
        sizes.insert(entry.object_id.clone(), size);
    }

    let eligible = regular
        .iter()
        .filter(|entry| sizes.get(&entry.object_id).copied().unwrap_or_default() <= MAX_BLOB_BYTES)
        .copied()
        .collect::<Vec<_>>();
    let batch = batch_git(root, runner, "--batch", &eligible, COMMAND_OUTPUT_LIMIT)?;
    let mut contents = BTreeMap::new();
    let mut cursor = 0;
    for entry in eligible {
        let header_end = batch[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| format!("{}: malformed cat-file header", entry.path))?
            + cursor;
        let header = std::str::from_utf8(&batch[cursor..header_end])
            .map_err(|_| format!("{}: invalid cat-file header", entry.path))?;
        let fields = header.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != entry.object_id || fields[1] != "blob" {
            return Err(format!(
                "{}: object-id-mismatch: unexpected cat-file header {header:?}",
                entry.path
            ));
        }
        let size = fields[2]
            .parse::<usize>()
            .map_err(|_| format!("{}: invalid-blob-size: {header:?}", entry.path))?;
        let content_start = header_end + 1;
        let content_end = content_start
            .checked_add(size)
            .ok_or_else(|| format!("{}: blob-size-overflow", entry.path))?;
        if content_end >= batch.len() || batch[content_end] != b'\n' {
            return Err(format!("{}: truncated cat-file blob", entry.path));
        }
        contents.insert(
            entry.path.clone(),
            BlobData::Bytes(batch[content_start..content_end].to_vec()),
        );
        cursor = content_end + 1;
    }
    for entry in regular {
        if let Some(size) = sizes.get(&entry.object_id).copied()
            && size > MAX_BLOB_BYTES
        {
            contents.insert(entry.path.clone(), BlobData::TooLarge(size));
        }
    }
    Ok(contents)
}

fn batch_git(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    mode: &str,
    entries: &[&SourceEntry],
    output_limit: usize,
) -> Result<Vec<u8>> {
    let input = entries
        .iter()
        .flat_map(|entry| entry.object_id.as_bytes().iter().copied().chain(*b"\n"))
        .collect::<Vec<_>>();
    let request = ProcessRequest::new("git", ["cat-file", mode])
        .profile(EnvironmentProfile::GitTool)
        .current_dir(root.path())
        .stdin_bytes(input)
        .limits(ProcessLimits {
            max_stdout_bytes: output_limit,
            max_stderr_bytes: 64 * 1024,
            timeout: COMMAND_TIMEOUT,
        });
    let result = runner
        .run(request)
        .map_err(|error| format!("git cat-file {mode}: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "git cat-file {mode} failed ({}): {}",
            result.receipt.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    if result.receipt.stdout_truncated {
        return Err(format!("git cat-file {mode}: bounded output exceeded"));
    }
    Ok(result.stdout)
}

fn batch_lines(output: &[u8]) -> Vec<String> {
    output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect()
}

pub(crate) fn parse_index(output: &[u8]) -> Result<Vec<SourceEntry>> {
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
        let mode = parse_mode(fields.next())?;
        let object_id = fields
            .next()
            .ok_or_else(|| "git index record has no object id".to_owned())?;
        validate_object_id(object_id)?;
        let stage = fields
            .next()
            .ok_or_else(|| "git index record has no stage".to_owned())?
            .parse::<u8>()
            .map_err(|_| "git index record has invalid stage".to_owned())?;
        let path_bytes = &record[separator + 1..];
        let valid_utf8 = std::str::from_utf8(path_bytes).is_ok();
        let path = path_text(path_bytes);
        entries.push(SourceEntry {
            mode,
            object_id: object_id.to_owned(),
            stage,
            path,
            valid_utf8,
            kind: mode_kind(mode),
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

pub(crate) fn parse_tree(output: &[u8]) -> Result<Vec<SourceEntry>> {
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "git tree record has no path separator".to_owned())?;
        let header = std::str::from_utf8(&record[..separator])
            .map_err(|_| "git tree record has invalid header UTF-8".to_owned())?;
        let mut fields = header.split_ascii_whitespace();
        let mode = parse_mode(fields.next())?;
        let object_type = fields
            .next()
            .ok_or_else(|| "git tree record has no object type".to_owned())?;
        let object_id = fields
            .next()
            .ok_or_else(|| "git tree record has no object id".to_owned())?;
        validate_object_id(object_id)?;
        let path_bytes = &record[separator + 1..];
        let valid_utf8 = std::str::from_utf8(path_bytes).is_ok();
        let path = path_text(path_bytes);
        let kind = match object_type {
            "blob" => mode_kind(mode),
            "commit" => EntryKind::Submodule,
            _ => EntryKind::Unsupported,
        };
        entries.push(SourceEntry {
            mode,
            object_id: object_id.to_owned(),
            stage: 0,
            path,
            valid_utf8,
            kind,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn parse_mode(value: Option<&str>) -> Result<u32> {
    let value = value.ok_or_else(|| "git entry has no mode".to_owned())?;
    u32::from_str_radix(value, 8).map_err(|_| format!("git entry has invalid mode {value}"))
}

fn validate_object_id(object_id: &str) -> Result<()> {
    if (object_id.len() != 40 && object_id.len() != 64)
        || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("git entry has invalid object id {object_id:?}"));
    }
    Ok(())
}

fn path_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(path) => path.to_owned(),
        Err(_) => format!("<invalid-utf8:{}>", hex(bytes)),
    }
}

fn mode_kind(mode: u32) -> EntryKind {
    match mode & 0o170_000 {
        0o100_000 => EntryKind::Regular,
        0o120_000 => EntryKind::Symlink,
        0o160_000 => EntryKind::Submodule,
        _ => EntryKind::Unsupported,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").expect("writing a String cannot fail");
        output
    })
}

#[cfg(test)]
mod tests {
    use super::{EntryKind, parse_index, parse_tree};

    #[test]
    fn index_fixture_retains_blob_id_and_stage() {
        let entries =
            parse_index(b"100644 0123456789012345678901234567890123456789 2\tfixture.md\0")
                .expect("index parses");
        assert_eq!(
            entries[0].object_id,
            "0123456789012345678901234567890123456789"
        );
        assert_eq!(entries[0].stage, 2);
        assert_eq!(entries[0].kind, EntryKind::Regular);
    }

    #[test]
    fn tree_fixture_rejects_non_blob_entries_as_non_regular() {
        let entries =
            parse_tree(b"160000 commit 0123456789012345678901234567890123456789\tmodule\0")
                .expect("tree parses");
        assert_eq!(entries[0].kind, EntryKind::Submodule);
    }
}
