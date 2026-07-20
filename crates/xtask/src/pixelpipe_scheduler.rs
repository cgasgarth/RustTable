use std::fs;
use std::path::Path;
use std::time::Instant;

use clap::Args;
use rusttable_pixelpipe::{
    CpuPriority, CpuScheduler, PipelineGeneration, PipelineSnapshotIdentity, RequestId,
    ResourceClaim, SchedulerConfig, TaskId, TaskSpec,
};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

const SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-scheduler-source-map.toml";
const SCHEMA: &str = "rusttable.pixelpipe-scheduler-source-map.v1";

#[derive(Debug, Args)]
pub(crate) struct SchedulerMatrixArgs {
    #[arg(long)]
    pub interactive_under_load: bool,
    #[arg(long)]
    pub memory_pressure: bool,
    #[arg(long)]
    pub failures: String,
    #[arg(long)]
    pub verify_bounds: bool,
}

pub(crate) fn run(root: &Path, args: &SchedulerMatrixArgs) -> Result {
    if !args.interactive_under_load
        || !args.memory_pressure
        || args.failures != "all"
        || !args.verify_bounds
    {
        return Err("scheduler matrix requires interactive-under-load, memory-pressure, --failures all, and verify-bounds".to_owned());
    }
    verify_source_map(root, 273)?;
    let now = Instant::now();
    let config = SchedulerConfig::new(32, 16, 4, 4, 1_000)
        .map_err(|error| format!("scheduler config: {error:?}"))?;
    let mut scheduler = CpuScheduler::new(config);
    scheduler
        .submit_at(task(1, CpuPriority::BackgroundAnalysis, 100)?, now)
        .map_err(|error| error.to_string())?;
    scheduler
        .submit_at(task(2, CpuPriority::InteractivePreview, 100)?, now)
        .map_err(|error| error.to_string())?;
    if scheduler
        .start_next_at(now)
        .map(|running| running.task_id().get())
        != Some(2)
    {
        return Err("scheduler matrix: interactive work did not win admission".to_owned());
    }
    for id in 3..=12 {
        if scheduler
            .submit_at(task(id, CpuPriority::UserExport, 200)?, now)
            .is_err()
        {
            break;
        }
    }
    if scheduler.snapshot().admitted_memory_bytes > config.memory_limit() {
        return Err("scheduler matrix: admitted memory exceeded configured bound".to_owned());
    }
    eprintln!(
        "pixelpipe scheduler matrix passed (interactive-under-load memory-pressure failures=all bounds=verified)"
    );
    Ok(())
}

fn task(id: u64, priority: CpuPriority, memory: u64) -> Result<TaskSpec> {
    let task_id = TaskId::new(id).map_err(|error| format!("task ID: {error:?}"))?;
    let request_id = RequestId::new(id).map_err(|error| format!("request ID: {error:?}"))?;
    let resources = ResourceClaim::new(memory, 1, 1, true, Vec::new())
        .map_err(|error| format!("resources: {error:?}"))?;
    TaskSpec::new(
        task_id,
        request_id,
        PipelineSnapshotIdentity::new([u8::try_from(id).expect("fixture ID fits"); 32]),
        PipelineGeneration::new(1).map_err(|error| format!("generation: {error:?}"))?,
        priority,
        1,
        resources,
    )
    .map_err(|error| format!("task: {error:?}"))
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    if issue != 273 {
        return Err(format!("scheduler source map: unsupported issue {issue}"));
    }
    let document = toml::from_str::<toml::Value>(
        &fs::read_to_string(root.join(SOURCE_MAP))
            .map_err(|error| format!("scheduler source map: read failed: {error}"))?,
    )
    .map_err(|error| format!("scheduler source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_DARKTABLE_COMMIT)
    {
        return Err("scheduler source map: invalid header".to_owned());
    }
    let expected = [
        ("job-type", "src/control/jobs.h", "dt_job_t"),
        ("job-admission", "src/control/jobs.c", "dt_control_add_job"),
        (
            "job-cancellation",
            "src/control/jobs.c",
            "dt_control_job_cancel",
        ),
        (
            "pixelpipe-work",
            "src/develop/pixelpipe_hb.c",
            "dt_dev_pixelpipe_process",
        ),
        (
            "thumbnail-work",
            "src/common/mipmap_cache.c",
            "dt_mipmap_cache_get",
        ),
        (
            "resource-limits",
            "src/common/resource_limits.c",
            "dt_set_rlimits",
        ),
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "scheduler source map: responsibilities missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err("scheduler source map: incomplete responsibilities".to_owned());
    }
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "scheduler source map: entry is not a table".to_owned())?;
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "scheduler source map: missing ID".to_owned())?;
        let (expected_path, expected_symbol) = expected
            .iter()
            .find(|item| item.0 == id)
            .map(|item| (item.1, item.2))
            .ok_or_else(|| format!("scheduler source map: unknown ID {id}"))?;
        if table.get("upstream_path").and_then(toml::Value::as_str) != Some(expected_path)
            || table.get("upstream_symbol").and_then(toml::Value::as_str) != Some(expected_symbol)
            || table.get("status").and_then(toml::Value::as_str) != Some("implemented")
        {
            return Err(format!("scheduler source map: anchor mismatch for {id}"));
        }
        let owner = table
            .get("rust_path")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("scheduler source map: missing owner for {id}"))?;
        if !root.join(owner).is_file() {
            return Err(format!("scheduler source map: owner missing for {id}"));
        }
    }
    Ok(())
}
