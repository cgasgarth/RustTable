use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use clap::Args;
use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};
use rusttable_pixelpipe::{
    AnalysisValue, Cache, CacheConfig, CacheKey, CachePrecision, CacheQuality, CancellationReason,
    CancellationScope, CancellationStage, CancellationToken, ColorIdentity, ImplementationIdentity,
    NodeBoundary, OutputIdentity, PipelineGeneration, PipelinePurpose, PipelineSnapshotIdentity,
    PublicationGeneration, PublicationIdentity, PublicationTarget, RequestId, SourceIdentity,
    TargetIdentity,
};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

const SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-cancellation-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-cancellation-source-map.v1";

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct CancellationMatrixArgs {
    #[arg(long)]
    pub rapid_generations: bool,
    #[arg(long)]
    pub shared_builds: bool,
    #[arg(long)]
    pub late_completions: bool,
    #[arg(long)]
    pub verify_no_stale_publication: bool,
}

pub(crate) fn run(root: &Path, args: &CancellationMatrixArgs) -> Result {
    if !args.rapid_generations
        || !args.shared_builds
        || !args.late_completions
        || !args.verify_no_stale_publication
    {
        return Err("cancellation matrix requires all rapid/shared/late/stale flags".to_owned());
    }
    verify_source_map(root, 272)?;
    rapid_generations()?;
    shared_builds()?;
    late_completion()?;
    eprintln!(
        "pixelpipe cancellation matrix passed (rapid-generations shared-builds late-completions stale-publication=verified)"
    );
    Ok(())
}

fn rapid_generations() -> Result {
    let initial = PipelineGeneration::new(1).map_err(|error| error.to_string())?;
    let scope = CancellationScope::root(initial);
    let newer = PipelineGeneration::new(2).map_err(|error| error.to_string())?;
    scope.cancel(CancellationReason::SupersededGeneration(newer));
    scope
        .token()
        .check(CancellationStage::Publication)
        .map_err(|error| error.to_string())
        .map_or_else(
            |_| Ok(()),
            |()| Err("superseded generation was not cancelled".to_owned()),
        )
}

fn shared_builds() -> Result {
    let cache = Arc::new(Cache::new(CacheConfig::new(1024)));
    let key = fixture_key(3)?;
    let builds = Arc::new(AtomicUsize::new(0));
    let worker_cache = cache.clone();
    let worker_builds = builds.clone();
    let worker = thread::spawn(move || {
        let token = CancellationToken::new();
        worker_cache.get_or_build(key, &token, |_| {
            worker_builds.fetch_add(1, Ordering::AcqRel);
            thread::sleep(Duration::from_millis(2));
            Ok(AnalysisValue::new([1, 2, 3]))
        })
    });
    let result = worker
        .join()
        .map_err(|_| "shared build panicked".to_owned())?;
    result.map_err(|error| error.to_string())?;
    if builds.load(Ordering::Acquire) != 1 {
        return Err("shared build executed more than once".to_owned());
    }
    Ok(())
}

fn late_completion() -> Result {
    let generation = PipelineGeneration::new(4).map_err(|error| error.to_string())?;
    let token = CancellationToken::for_generation(generation);
    let identity = PublicationIdentity::new(
        RequestId::new(1).map_err(|error| error.to_string())?,
        generation,
        PublicationTarget::Pixelpipe(TargetIdentity::consumer("acceptance")),
        SourceIdentity::new([4; 32]),
        PipelineSnapshotIdentity::new([5; 32]),
        PublicationGeneration::new(4),
    );
    let gate = rusttable_pixelpipe::PublicationGate::new(identity, token.clone());
    token.cancel();
    if gate.authorize(identity).is_ok() {
        return Err("cancelled late completion acquired a publication permit".to_owned());
    }
    Ok(())
}

fn fixture_key(seed: u8) -> Result<CacheKey> {
    let dimensions = ImageDimensions::new(4, 4).map_err(|error| error.to_string())?;
    let implementation =
        ImplementationIdentity::new("rusttable.pixelpipe.cancellation", 1, "source-tree")
            .map_err(|error| error.to_string())?;
    CacheKey::builder()
        .source(SourceIdentity::new([seed; 32]))
        .source_descriptor([seed])
        .snapshot(PipelineSnapshotIdentity::new([seed; 32]))
        .generation(PipelineGeneration::new(u64::from(seed)).map_err(|error| error.to_string())?)
        .purpose(PipelinePurpose::Preview)
        .quality(CacheQuality::Normal)
        .precision(CachePrecision::F32)
        .node(NodeBoundary::whole(implementation))
        .output(OutputIdentity::new(
            dimensions,
            Roi::full(dimensions),
            PixelFormat::rgba8(),
            ColorIdentity::new(ColorEncoding::SrgbD65, 1).map_err(|error| error.to_string())?,
            [seed; 32],
        ))
        .parameters(1, [seed])
        .build()
        .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    if issue != 272 {
        return Err(format!(
            "pixelpipe cancellation source map: unsupported issue {issue}"
        ));
    }
    let text = std::fs::read_to_string(root.join(SOURCE_MAP))
        .map_err(|error| format!("pixelpipe cancellation source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe cancellation source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_DARKTABLE_COMMIT)
    {
        return Err(
            "pixelpipe cancellation source map: schema, issue, or pin is invalid".to_owned(),
        );
    }
    let expected = [
        (
            "generation-authority",
            "src/develop/pixelpipe.h",
            "dt_dev_pixelpipe_t generation/status",
            "crates/rusttable-pixelpipe/src/pipeline_contracts.rs",
            "replaced",
            "PipelineGeneration::next",
        ),
        (
            "scope-tree",
            "src/control/jobs.c",
            "dt_control_job_cancel",
            "crates/rusttable-pixelpipe/src/cancellation.rs",
            "replaced",
            "CancellationScope",
        ),
        (
            "source-decode",
            "src/develop/pixelpipe_hb.c",
            "dt_dev_pixelpipe_process source/decode boundary",
            "crates/rusttable-pixelpipe/src/cancellation.rs",
            "replaced",
            "CancellationStage::SourceDecode",
        ),
        (
            "preparation-node",
            "src/develop/pixelpipe_hb.c",
            "dt_dev_pixelpipe_process node preparation",
            "crates/rusttable-pixelpipe/src/cancellation.rs",
            "replaced",
            "CancellationStage::Preparation/Node",
        ),
        (
            "tile-boundary",
            "src/develop/pixelpipe_hb.c",
            "dt_dev_pixelpipe_process tile rows",
            "crates/rusttable-pixelpipe/src/cpu.rs",
            "replaced",
            "CpuPixelpipeExecutor::execute_tiled_with_cancellation",
        ),
        (
            "cache-build",
            "src/common/cache.c",
            "dt_cache_get_with_caller build/wait",
            "crates/rusttable-pixelpipe/src/cache.rs",
            "replaced",
            "Cache::get_or_build",
        ),
        (
            "shared-consumer",
            "src/common/mipmap_cache.h",
            "DT_MIPMAP_BLOCKING shared request interest",
            "crates/rusttable-pixelpipe/src/cache.rs",
            "replaced",
            "ConsumerRegistration",
        ),
        (
            "host-lease",
            "src/develop/pixelpipe_hb.c",
            "pixelpipe buffer acquisition/release",
            "crates/rusttable-pixelpipe/src/host_pool.rs",
            "delegated",
            "rusttable-image HostBufferPool RAII lease",
        ),
        (
            "gpu-retirement",
            "src/common/opencl.c",
            "dt_opencl_events_reset",
            "crates/rusttable-pixelpipe/src/publication.rs",
            "replaced",
            "GpuRetirement",
        ),
        (
            "publication-gate",
            "src/develop/pixelpipe_hb.c",
            "result publication stale-generation checks",
            "crates/rusttable-pixelpipe/src/publication.rs",
            "replaced",
            "PublicationGate",
        ),
        (
            "cache-publication",
            "src/develop/pixelpipe_hb.c",
            "cache lookup and result publication",
            "crates/rusttable-pixelpipe/src/cache.rs",
            "replaced",
            "CachePublicationPermit",
        ),
        (
            "ui-file-publication",
            "src/views/darkroom.c",
            "expose and export result handoff",
            "crates/rusttable-pixelpipe/src/publication.rs",
            "replaced",
            "ProductPublicationPermit",
        ),
        (
            "shutdown-retirement",
            "src/control/jobs.c",
            "job cancellation and bounded shutdown",
            "crates/rusttable-pixelpipe/src/cancellation.rs",
            "replaced",
            "CancellationReason::Shutdown",
        ),
        (
            "status-invalidation",
            "src/develop/develop.c",
            "DT_DEV_PIPE_SYNCH source/edit invalidation",
            "crates/rusttable-pixelpipe/src/cache.rs",
            "replaced",
            "CacheScope and CancellationReason",
        ),
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe cancellation source map: responsibilities missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err(format!(
            "pixelpipe cancellation source map: expected {} responsibilities, found {}",
            expected.len(),
            entries.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "cancellation responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
            "owner",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "pixelpipe cancellation source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        let Some(expected_entry) = expected.iter().find(|entry| entry.0 == id) else {
            return Err(format!(
                "pixelpipe cancellation source map: unexpected or duplicate {id}"
            ));
        };
        if !seen.insert(id) {
            return Err(format!("pixelpipe cancellation source map: duplicate {id}"));
        }
        for (key, expected_value) in [
            ("upstream_path", expected_entry.1),
            ("upstream_symbol", expected_entry.2),
            ("rust_path", expected_entry.3),
            ("status", expected_entry.4),
            ("owner", expected_entry.5),
        ] {
            if table.get(key).and_then(toml::Value::as_str) != Some(expected_value) {
                return Err(format!(
                    "pixelpipe cancellation source map: {id} has an unexpected {key}"
                ));
            }
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "pixelpipe cancellation source map: missing Rust owner {rust_path}"
            ));
        }
    }
    Ok(())
}
