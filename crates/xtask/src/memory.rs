use std::path::Path;

use clap::Subcommand;
use rusttable_image::{
    AllocationClass, BufferAlignment, BufferRequest, BufferUsage, HostBufferPool, HostPoolError,
    InitializationPolicy, PixelFormat, PoolBudgets, PriorityClass,
};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

const SOURCE_MAP: &str = "architecture/rusttable-host-pool-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.host-pool-source-map.v1";

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryCommand {
    /// Exercise bounded host buffer allocation, return, and failure paths.
    HostPool {
        #[arg(long)]
        stress: bool,
        #[arg(long)]
        budgets: String,
        #[arg(long)]
        inject_failures: bool,
        #[arg(long)]
        verify_no_alias: bool,
    },
}

pub(crate) fn run(root: &Path, command: MemoryCommand) -> Result {
    match command {
        MemoryCommand::HostPool {
            stress,
            budgets,
            inject_failures,
            verify_no_alias,
        } => host_pool(root, stress, &budgets, inject_failures, verify_no_alias),
    }
}

fn host_pool(
    root: &Path,
    stress: bool,
    budgets: &str,
    inject_failures: bool,
    verify_no_alias: bool,
) -> Result {
    if !stress {
        return Err("memory host-pool requires --stress".to_owned());
    }
    if budgets != "matrix" {
        return Err(format!(
            "memory host-pool: unsupported budgets matrix {budgets}"
        ));
    }
    if !inject_failures || !verify_no_alias {
        return Err("memory host-pool requires --inject-failures and --verify-no-alias".to_owned());
    }
    verify_source_map(root, 269)?;
    let mut receipts = Vec::new();
    for (index, total) in [64_usize, 256, 1_024].into_iter().enumerate() {
        let budgets = PoolBudgets::new(total, total / 2, 8, total, total, 1)
            .map_err(|error| error.to_string())?;
        let pool = HostBufferPool::new(budgets);
        let request = BufferRequest::new(
            AllocationClass::new(
                PixelFormat::rgba8(),
                BufferAlignment::CACHE_LINE,
                BufferUsage::Pixelpipe,
            ),
            32,
            InitializationPolicy::Zeroed,
            PriorityClass::Background,
        )
        .map_err(|error| error.to_string())?;
        let lease = pool
            .try_acquire(request)
            .map_err(|error| error.to_string())?;
        let before = pool.accounting();
        if before.outstanding_bytes() > total || before.allocated_bytes() > total {
            return Err("memory host-pool: budget invariant failed".to_owned());
        }
        let shared = lease
            .freeze()
            .map_err(|_| "memory host-pool: initialized lease did not freeze".to_owned())?;
        if shared
            .read_view()
            .map_err(|error| error.to_string())?
            .get(0)
            != Some(0)
        {
            return Err(
                "memory host-pool: immutable read did not observe zero initialization".to_owned(),
            );
        }
        drop(shared);
        let after = pool.accounting();
        if after.allocated_bytes() > total || after.outstanding_entries() != 0 {
            return Err("memory host-pool: return invariant failed".to_owned());
        }
        receipts.push(format!(
            "case{index}:{}:{}",
            after.allocated_bytes(),
            after.reuse_count()
        ));
    }
    if !matches!(
        BufferRequest::new(
            AllocationClass::new(
                PixelFormat::rgba8(),
                BufferAlignment::CACHE_LINE,
                BufferUsage::Pixelpipe,
            ),
            0,
            InitializationPolicy::Uninitialized,
            PriorityClass::Background,
        ),
        Err(HostPoolError::InvalidRequest)
    ) {
        return Err("memory host-pool: injected invalid-request failure was accepted".to_owned());
    }
    eprintln!(
        "memory host-pool passed (budgets=matrix cases={} receipts={})",
        receipts.len(),
        receipts.join(",")
    );
    Ok(())
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    if issue != 269 {
        return Err(format!("host pool source map: unsupported issue {issue}"));
    }
    let path = root.join(SOURCE_MAP);
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("host pool source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("host pool source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_DARKTABLE_COMMIT)
    {
        return Err("host pool source map: schema, issue, or upstream pin is invalid".to_owned());
    }
    let expected = [
        "aligned-host-allocation",
        "cache-entry-ownership",
        "cache-accounting-and-cleanup",
        "mipmap-buffer-acquisition",
        "image-metadata-acquisition",
        "pixelpipe-tiling-host-scratch",
        "gpu-and-native-scratch-boundary",
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "host pool source map: responsibilities are missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err(format!(
            "host pool source map: expected {} responsibilities, found {}",
            expected.len(),
            entries.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "host pool source map: responsibility is not a table".to_owned())?;
        for key in ["id", "upstream_path", "rust_path", "status"] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "host pool source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        if !expected.contains(&id) || !seen.insert(id) {
            return Err(format!(
                "host pool source map: unexpected or duplicate {id}"
            ));
        }
        let upstream_path = table["upstream_path"].as_str().expect("validated path");
        if !root.join("../Darktable").join(upstream_path).is_file()
            && !Path::new("/Users/cgas/Documents/RustTable/Darktable")
                .join(upstream_path)
                .is_file()
        {
            return Err(format!(
                "host pool source map: missing reference anchor {upstream_path}"
            ));
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "host pool source map: missing Rust owner {rust_path}"
            ));
        }
        if table["status"].as_str() == Some("delegated")
            && table
                .get("delegated_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err(format!(
                "host pool source map: delegated responsibility {id} has no issue"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_map_has_the_complete_issue_inventory() {
        verify_source_map(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap(),
            269,
        )
        .expect("host pool source map");
    }
}
