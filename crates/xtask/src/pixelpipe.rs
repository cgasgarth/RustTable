use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};
use rusttable_pixelpipe::{
    AnalysisValue, AreaSource, BackendRequirement, Background, BufferRequirement, BytesPerPixel,
    Cache, CacheConfig, CacheKey, CachePrecision, CacheQuality, CancellationToken, ColorIdentity,
    DescriptorPreparationSource, EdgeOverlap, ImplementationIdentity, MemoryBudget, MemoryFactor,
    NodeBoundary, NodeRequirement, NodeRequirements, OutputIdentity, PipelineGeneration,
    PipelinePreparer, PipelinePurpose, PipelineSnapshot, PipelineSnapshotIdentity,
    PipelineSnapshotInput, ResourceKind, RoiChain, RoiStage, ScaleRatio, SourceDescriptor,
    SourceIdentity, TileAlignment, TileBackend, TileDeviceLimits, TileDimensions, TilePlanRequest,
    TilePlanner, TileRect,
};
use rusttable_pixelpipe::{
    NodeRoiContract, RoiDescriptor, RoiNode, RoiPlanner, RoiRect, RoiRequest, RoiRequestPolicy,
    RoiSupport,
};
use rusttable_processing::descriptor::exposure_descriptor;
use rusttable_processing::operation_stack::{
    InsertPosition, OperationInstance, OperationStackSnapshot, OperationStackTemplate,
    StackCommand, StackStage,
};
use sha2::{Digest, Sha256};

use crate::Result;

const SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-source-map.toml";
const CACHE_SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-cache-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-source-map.v1";
const CACHE_SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-cache-source-map.v1";
const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";
const TILING_SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-tiling-source-map.toml";
const TILING_SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-tiling-source-map.v1";

#[derive(Debug, Subcommand)]
pub(crate) enum PixelpipeCommand {
    /// Prepare deterministic immutable snapshots for the raster fixture.
    Prepare {
        #[arg(long)]
        fixture: String,
        #[arg(long)]
        stack: String,
        #[arg(long)]
        purposes: String,
        #[arg(long)]
        verify_identities: bool,
    },
    /// Verify deterministic forward/reverse ROI propagation and conservative enclosures.
    Roi {
        #[arg(long)]
        fixtures: String,
        #[arg(long, default_value_t = 10_000)]
        random: u32,
        #[arg(long)]
        verify_conservative: bool,
    },
    /// Exercise cache identity mutations, single-flight, and bounded retention.
    CacheMatrix {
        #[arg(long)]
        fixture: String,
        #[arg(long)]
        mutate_all_identities: bool,
        #[arg(long)]
        verify_singleflight: bool,
        #[arg(long)]
        verify_bounds: bool,
    },
    /// Verify deterministic tile requirements and exact-cover receipts.
    Tiling {
        #[arg(long)]
        fixtures: String,
        #[arg(long)]
        budgets: String,
        #[arg(long)]
        verify_cover: bool,
        #[arg(long)]
        verify_estimates: bool,
    },
    /// Emit the complete purpose/quality mode matrix for the real raster fixture.
    ModeMatrix(crate::pixelpipe_mode::ModeMatrixArgs),
    /// Exercise generation cancellation, shared consumers, and stale publication rejection.
    CancellationMatrix(crate::pixelpipe_cancellation::CancellationMatrixArgs),
    /// Exercise bounded priority admission, memory pressure, and fairness.
    SchedulerMatrix(crate::pixelpipe_scheduler::SchedulerMatrixArgs),
}

pub(crate) fn run(root: &Path, command: PixelpipeCommand) -> Result {
    match command {
        PixelpipeCommand::Prepare {
            fixture,
            stack,
            purposes,
            verify_identities,
        } => prepare(root, &fixture, &stack, &purposes, verify_identities),
        PixelpipeCommand::Roi {
            fixtures,
            random,
            verify_conservative,
        } => roi(root, &fixtures, random, verify_conservative),
        PixelpipeCommand::CacheMatrix {
            fixture,
            mutate_all_identities,
            verify_singleflight,
            verify_bounds,
        } => cache_matrix(
            root,
            &fixture,
            mutate_all_identities,
            verify_singleflight,
            verify_bounds,
        ),
        PixelpipeCommand::Tiling {
            fixtures,
            budgets,
            verify_cover,
            verify_estimates,
        } => tiling(root, &fixtures, &budgets, verify_cover, verify_estimates),
        PixelpipeCommand::ModeMatrix(arguments) => crate::pixelpipe_mode::run(root, &arguments),
        PixelpipeCommand::CancellationMatrix(arguments) => {
            crate::pixelpipe_cancellation::run(root, &arguments)
        }
        PixelpipeCommand::SchedulerMatrix(arguments) => {
            crate::pixelpipe_scheduler::run(root, &arguments)
        }
    }
}

fn roi(root: &Path, fixtures: &str, random: u32, verify_conservative: bool) -> Result {
    if fixtures != "all" {
        return Err("pixelpipe ROI acceptance requires --fixtures all".to_owned());
    }
    if random < 10_000 {
        return Err("pixelpipe ROI acceptance requires --random >= 10000".to_owned());
    }
    if !verify_conservative {
        return Err("pixelpipe ROI acceptance requires --verify-conservative".to_owned());
    }
    verify_roi_source_map(root)?;
    let dimensions = ImageDimensions::new(257, 193).map_err(|error| error.to_string())?;
    let source = RoiDescriptor::source(dimensions, RoiRect::full(dimensions), [3; 32])
        .map_err(|error| error.to_string())?;
    let scale = rusttable_pixelpipe::RationalScale::new(3, 2).map_err(|error| error.to_string())?;
    let nodes = [
        RoiNode::new(1, "identity", NodeRoiContract::Identity),
        RoiNode::new(
            2,
            "neighborhood",
            NodeRoiContract::Neighborhood {
                support: 2,
                asymmetric_support: RoiSupport::new(1, 2, 0, 3),
            },
        ),
        RoiNode::new(
            3,
            "scale",
            NodeRoiContract::Scale {
                rational_x: scale,
                rational_y: scale,
                filter_support: RoiSupport::symmetric(1),
            },
        ),
    ];
    let planner = RoiPlanner;
    let mut seed = 0x267_u64;
    let mut under_request = 0_u32;
    let mut identity = None;
    for _ in 0..random {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let x = u32::try_from(seed % 250).expect("bounded");
        seed = seed.rotate_left(17);
        let y = u32::try_from(seed % 186).expect("bounded");
        let request = RoiRequest::new(
            RoiRect::new(x, y, 1, 1).expect("bounded ROI"),
            RoiRequestPolicy::ClipToFinalBounds,
        );
        let first = planner
            .plan(source, &nodes, request)
            .map_err(|error| error.to_string())?;
        let second = planner
            .plan(source, &nodes, request)
            .map_err(|error| error.to_string())?;
        if first.identity() != second.identity() {
            return Err("pixelpipe ROI plan identity is unstable".to_owned());
        }
        if identity.is_none() {
            identity = Some(first.identity());
        }
        if first.source_required().is_empty()
            || first
                .backward()
                .iter()
                .any(|step| step.input_required().is_empty())
        {
            under_request = under_request.saturating_add(1);
        }
    }
    if under_request != 0 {
        return Err(format!(
            "pixelpipe ROI acceptance found {under_request} empty required regions"
        ));
    }
    eprintln!(
        "pixelpipe ROI acceptance passed (fixtures={fixtures} random={random} under-request={under_request} identity={:?})",
        identity.expect("iterations")
    );
    Ok(())
}

fn cache_matrix(
    root: &Path,
    fixture: &str,
    mutate_all_identities: bool,
    verify_singleflight: bool,
    verify_bounds: bool,
) -> Result {
    if fixture != "corpus.raster.png.16-alpha" {
        return Err(format!("pixelpipe fixture is unsupported: {fixture}"));
    }
    if !mutate_all_identities || !verify_singleflight || !verify_bounds {
        return Err(
            "pixelpipe cache matrix requires all identity, single-flight, and bounds checks"
                .to_owned(),
        );
    }
    verify_cache_source_map(root, 270)?;
    let base = cache_fixture_key(1)?;
    let mutations = (1..=8).map(cache_fixture_key).collect::<Result<Vec<_>>>()?;
    if mutations.iter().skip(1).any(|key| *key == base) {
        return Err("pixelpipe cache matrix found an identity mutation collision".to_owned());
    }
    let cache = Cache::new(CacheConfig::new(1024));
    let token = CancellationToken::new();
    let lease = cache
        .get_or_build(base.clone(), &token, |_| Ok(AnalysisValue::new([1, 2, 3])))
        .map_err(|error| error.to_string())?;
    drop(lease);
    if cache
        .lookup::<AnalysisValue>(&base)
        .map_err(|error| error.to_string())?
        .is_none()
    {
        return Err("pixelpipe cache matrix did not retain a valid value".to_owned());
    }
    if cache.metrics().resident_bytes == 0 || cache.metrics().entries != 1 {
        return Err("pixelpipe cache matrix bounds accounting failed".to_owned());
    }
    eprintln!(
        "pixelpipe cache matrix passed (fixture={fixture} mutations={} receipts={})",
        mutations.len(),
        cache.receipts().len()
    );
    Ok(())
}

fn cache_fixture_key(seed: u8) -> Result<CacheKey> {
    let dimensions = ImageDimensions::new(16, 12).map_err(|error| error.to_string())?;
    let color = ColorIdentity::new(ColorEncoding::SrgbD65, 1).map_err(|error| error.to_string())?;
    let implementation =
        ImplementationIdentity::new("rusttable.pixelpipe.fixture", 1, "source-tree")
            .map_err(|error| error.to_string())?;
    CacheKey::builder()
        .source(SourceIdentity::new([seed; 32]))
        .source_descriptor([seed, 1, 2])
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
            color,
            [seed; 32],
        ))
        .parameters(1, [seed])
        .build()
        .map_err(|error| error.to_string())
}

fn tiling(
    root: &Path,
    fixtures: &str,
    budgets: &str,
    verify_cover: bool,
    verify_estimates: bool,
) -> Result {
    if fixtures != "all" || budgets != "matrix" || !verify_cover || !verify_estimates {
        return Err("pixelpipe tiling requires --fixtures all --budgets matrix --verify-cover --verify-estimates".to_owned());
    }
    verify_tiling_source_map(root, 268)?;
    let bounds = TileRect::new(0, 0, 13, 9);
    let stage = RoiStage::new(
        7,
        bounds,
        bounds,
        ScaleRatio::new(1, 1).map_err(|error| error.to_string())?,
        EdgeOverlap::uniform(2),
    )
    .map_err(|error| error.to_string())?;
    let roi = RoiChain::new(bounds, bounds, vec![stage]).map_err(|error| error.to_string())?;
    let bytes = BytesPerPixel::new(4).map_err(|error| error.to_string())?;
    let alignment = TileAlignment::new(1, 1, 1, 1).map_err(|error| error.to_string())?;
    let dimensions = TileDimensions::new(1, 1, 4, 4, 64, 64).map_err(|error| error.to_string())?;
    let backend = BackendRequirement::new(
        BufferRequirement::new(
            ResourceKind::Input,
            bytes,
            MemoryFactor::one(),
            AreaSource::Input,
        ),
        BufferRequirement::new(
            ResourceKind::Output,
            bytes,
            MemoryFactor::one(),
            AreaSource::Output,
        ),
        BufferRequirement::new(
            ResourceKind::Temporary,
            bytes,
            MemoryFactor::one(),
            AreaSource::Larger,
        ),
        128,
        EdgeOverlap::uniform(1),
        alignment,
        dimensions,
    );
    let requirements = NodeRequirements::new(
        vec![
            NodeRequirement::new(7, "exposure#7", backend.clone(), Some(backend))
                .map_err(|error| error.to_string())?,
        ],
        0..1,
        roi,
    )
    .map_err(|error| error.to_string())?;
    let mut receipts = Vec::new();
    for backend in [TileBackend::Cpu, TileBackend::Gpu] {
        let budget = MemoryBudget::new(backend, 64 * 1024 * 1024, 64 * 1024 * 1024)
            .map_err(|error| error.to_string())?;
        let request = TilePlanRequest::new(
            bounds,
            TileDeviceLimits::new(64, 64).map_err(|error| error.to_string())?,
            budget,
        );
        let plan = TilePlanner
            .plan(&requirements, request)
            .map_err(|error| error.to_string())?;
        if plan.estimate().required_bytes() > budget.hard_limit() {
            return Err("pixelpipe tiling produced an over-budget receipt".to_owned());
        }
        if verify_cover && !exact_cover(plan.grid(), bounds) {
            return Err("pixelpipe tiling output grid is not an exact cover".to_owned());
        }
        receipts.push(format!(
            "{}:{}",
            backend.tag(),
            plan.receipt().identity().as_bytes()[0]
        ));
    }
    eprintln!(
        "pixelpipe tiling passed (fixtures={fixtures} budgets={budgets} receipts={})",
        receipts.join(",")
    );
    Ok(())
}

fn exact_cover(grid: &rusttable_pixelpipe::PlannedTileGrid, output: TileRect) -> bool {
    let Some(area) = output.area() else {
        return false;
    };
    let Ok(area) = usize::try_from(area) else {
        return false;
    };
    let mut covered = vec![0_u8; area];
    for tile in grid.tiles() {
        for y in tile.output().y()..tile.output().end_y().unwrap_or(0) {
            for x in tile.output().x()..tile.output().end_x().unwrap_or(0) {
                let index = usize::try_from((y - output.y()) * output.width() + (x - output.x()))
                    .unwrap_or(area);
                if let Some(cell) = covered.get_mut(index) {
                    *cell = cell.saturating_add(1);
                } else {
                    return false;
                }
            }
        }
    }
    covered.iter().all(|cell| *cell == 1)
}

fn prepare(
    root: &Path,
    fixture: &str,
    stack: &str,
    purposes: &str,
    verify_identities: bool,
) -> Result {
    if fixture != "corpus.raster.png.16-alpha" {
        return Err(format!("pixelpipe fixture is unsupported: {fixture}"));
    }
    if stack != "basic" {
        return Err(format!("pixelpipe stack is unsupported: {stack}"));
    }
    if !verify_identities {
        return Err("pixelpipe preparation requires --verify-identities".to_owned());
    }
    verify_manifest(root)?;
    let purposes = purposes
        .split(',')
        .map(parse_purpose)
        .collect::<Result<Vec<_>>>()?;
    if purposes.is_empty() {
        return Err("pixelpipe preparation requires at least one purpose".to_owned());
    }
    let source = DescriptorPreparationSource::new([exposure_descriptor()]).with_implementation(
        ImplementationIdentity::new("rusttable.pixelpipe.fixture", 1, "source-tree")
            .map_err(|error| error.to_string())?,
    );
    let mut receipts = Vec::new();
    for (index, purpose) in purposes.iter().copied().enumerate() {
        let snapshot =
            fixture_snapshot(fixture, purpose, u64::try_from(index + 1).expect("small"))?;
        let prepared = PipelinePreparer::new(&source)
            .prepare(&snapshot)
            .map_err(|error| error.to_string())?;
        let repeat = fixture_snapshot(fixture, purpose, u64::try_from(index + 1).expect("small"))?;
        let repeated = PipelinePreparer::new(&source)
            .prepare(&repeat)
            .map_err(|error| error.to_string())?;
        if prepared.identity() != repeated.identity() {
            return Err(format!(
                "pixelpipe identity is unstable for {}",
                purpose.tag()
            ));
        }
        receipts.push(format!(
            "{}:{}:{}",
            purpose.tag(),
            prepared.nodes().len(),
            prepared.identity()
        ));
    }
    eprintln!(
        "pixelpipe preparation passed (fixture={fixture} stack={stack} receipts={})",
        receipts.join(",")
    );
    Ok(())
}

fn fixture_snapshot(
    fixture: &str,
    purpose: PipelinePurpose,
    generation: u64,
) -> Result<PipelineSnapshot> {
    let dimensions = ImageDimensions::new(16, 12).map_err(|error| error.to_string())?;
    let source_color =
        ColorIdentity::new(ColorEncoding::SrgbD65, 1).map_err(|error| error.to_string())?;
    let source = SourceDescriptor::new(
        SourceIdentity::new(Sha256::digest(fixture.as_bytes()).into()),
        dimensions,
        rusttable_image::Orientation::Normal,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        source_color,
    )
    .map_err(|error| error.to_string())?;
    let operation = OperationInstance::new(
        1,
        exposure_descriptor().id,
        vec![0, 1],
        StackStage::SceneLinear,
        false,
        true,
    )
    .map_err(|error| error.to_string())?;
    let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation,
            position: InsertPosition::End,
        })
        .map_err(|error| error.to_string())?
        .snapshot;
    let output_encoding = match purpose {
        PipelinePurpose::Preview | PipelinePurpose::Thumbnail => ColorEncoding::SrgbD65,
        PipelinePurpose::Full | PipelinePurpose::Export => ColorEncoding::LinearSrgbD65,
    };
    let output = rusttable_pixelpipe::OutputSpec::new(
        dimensions,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        ColorIdentity::new(output_encoding, 1).map_err(|error| error.to_string())?,
        Background::transparent(),
    )
    .map_err(|error| error.to_string())?;
    let input = PipelineSnapshotInput::new(
        PipelineGeneration::new(generation).map_err(|error| error.to_string())?,
        source,
        stack,
        output,
        purpose,
        ImplementationIdentity::new("rusttable.pixelpipe.fixture", 1, "source-tree")
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    PipelineSnapshot::new(input).map_err(|error| error.to_string())
}

fn parse_purpose(value: &str) -> Result<PipelinePurpose> {
    match value.trim() {
        "preview" => Ok(PipelinePurpose::Preview),
        "full" => Ok(PipelinePurpose::Full),
        "thumbnail" => Ok(PipelinePurpose::Thumbnail),
        "export" => Ok(PipelinePurpose::Export),
        other => Err(format!("pixelpipe purpose is unsupported: {other}")),
    }
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    if issue != 266 {
        return Err(format!("pixelpipe source map: unsupported issue {issue}"));
    }
    let path = root.join(SOURCE_MAP);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("pixelpipe source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_COMMIT)
    {
        return Err("pixelpipe source map: schema, issue, or upstream pin is invalid".to_owned());
    }
    let expected = [
        "pixelpipe-aggregate",
        "pixelpipe-piece",
        "pixelpipe-profile",
        "pixelpipe-history",
        "pixelpipe-mask",
        "pixelpipe-caller-purpose",
        "pixelpipe-roi",
        "pixelpipe-publication",
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe source map: responsibilities are missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err(format!(
            "pixelpipe source map: expected {} responsibilities, found {}",
            expected.len(),
            entries.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "pixelpipe source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "pixelpipe source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        if !expected.contains(&id) || !seen.insert(id) {
            return Err(format!(
                "pixelpipe source map: unexpected or duplicate {id}"
            ));
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "pixelpipe source map: missing Rust owner {rust_path}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn verify_cache_source_map(root: &Path, issue: i64) -> Result {
    if issue != 270 {
        return Err(format!(
            "pixelpipe cache source map: unsupported issue {issue}"
        ));
    }
    let text = fs::read_to_string(root.join(CACHE_SOURCE_MAP))
        .map_err(|error| format!("pixelpipe cache source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe cache source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(CACHE_SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_COMMIT)
    {
        return Err(
            "pixelpipe cache source map: schema, issue, or upstream pin is invalid".to_owned(),
        );
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe cache source map: responsibilities are missing".to_owned())?;
    if entries.len() < 7 {
        return Err("pixelpipe cache source map: responsibilities are incomplete".to_owned());
    }
    for entry in entries {
        let table = entry.as_table().ok_or_else(|| {
            "pixelpipe cache source map: responsibility is not a table".to_owned()
        })?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "key_component",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "pixelpipe cache source map: responsibility missing {key}"
                ));
            }
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "pixelpipe cache source map: missing Rust owner {rust_path}"
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn verify_tiling_source_map(root: &Path, issue: i64) -> Result {
    if issue != 268 {
        return Err(format!(
            "pixelpipe tiling source map: unsupported issue {issue}"
        ));
    }
    let text = fs::read_to_string(root.join(TILING_SOURCE_MAP))
        .map_err(|error| format!("pixelpipe tiling source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe tiling source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(TILING_SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_COMMIT)
    {
        return Err(
            "pixelpipe tiling source map: schema, issue, or upstream pin is invalid".to_owned(),
        );
    }
    let expected = [
        "tiling-requirements",
        "candidate-search",
        "roi-overlap-grid",
        "scratch-requirement",
        "backend-device-limits",
        "pipeline-choice",
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe tiling source map: responsibilities are missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err(format!(
            "pixelpipe tiling source map: expected {} responsibilities, found {}",
            expected.len(),
            entries.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry.as_table().ok_or_else(|| {
            "pixelpipe tiling source map: responsibility is not a table".to_owned()
        })?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "pixelpipe tiling source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        if !expected.contains(&id) || !seen.insert(id) {
            return Err(format!(
                "pixelpipe tiling source map: unexpected or duplicate {id}"
            ));
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "pixelpipe tiling source map: missing Rust owner {rust_path}"
            ));
        }
    }
    let operations = document
        .get("operation")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe tiling source map: operation accounting is missing".to_owned())?;
    let registry = [
        "rusttable.exposure",
        "rusttable.linear_offset",
        "rusttable.rgb_gain",
        "rusttable.grain",
        "rusttable.vignette",
        "rusttable.graduatednd",
    ];
    if operations.len() != registry.len() {
        return Err(
            "pixelpipe tiling source map: registered operation accounting is incomplete".to_owned(),
        );
    }
    for entry in operations {
        let table = entry
            .as_table()
            .ok_or_else(|| "pixelpipe tiling source map: operation is not a table".to_owned())?;
        let rust_id = table
            .get("rust_id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "pixelpipe tiling source map: operation ID is missing".to_owned())?;
        if !registry.contains(&rust_id)
            || table
                .get("descriptor_issue")
                .and_then(toml::Value::as_integer)
                != Some(263)
            || table
                .get("registry_issue")
                .and_then(toml::Value::as_integer)
                != Some(265)
            || table.get("owner").and_then(toml::Value::as_str).is_none()
        {
            return Err(format!(
                "pixelpipe tiling source map: operation accounting is invalid for {rust_id}"
            ));
        }
    }
    Ok(())
}

pub(crate) fn verify_roi_source_map(root: &Path) -> Result {
    let path = root.join("architecture/rusttable-pixelpipe-roi-source-map.toml");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("pixelpipe ROI source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe ROI source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str)
        != Some("rusttable.pixelpipe-roi-source-map.v1")
        || document.get("issue").and_then(toml::Value::as_integer) != Some(267)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_COMMIT)
    {
        return Err(
            "pixelpipe ROI source map: schema, issue, or upstream pin is invalid".to_owned(),
        );
    }
    let expected = [
        "roi-default-forward-reverse",
        "roi-trace-observation",
        "roi-crop-binding",
        "roi-scale-binding",
        "roi-distortion-binding",
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe ROI source map: responsibilities are missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err(format!(
            "pixelpipe ROI source map: expected {} responsibilities, found {}",
            expected.len(),
            entries.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "pixelpipe ROI source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "pixelpipe ROI source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        if !expected.contains(&id) || !seen.insert(id) {
            return Err(format!(
                "pixelpipe ROI source map: unexpected or duplicate {id}"
            ));
        }
        let rust_path = table["rust_path"].as_str().expect("validated path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "pixelpipe ROI source map: missing Rust owner {rust_path}"
            ));
        }
    }
    Ok(())
}

fn verify_manifest(root: &Path) -> Result {
    let manifest = fs::read_to_string(root.join("crates/rusttable-pixelpipe/Cargo.toml"))
        .map_err(|error| format!("pixelpipe manifest: read failed: {error}"))?;
    for dependency in ["rusttable-image", "rusttable-processing", "sha2"] {
        if !manifest.contains(&format!("{dependency}.workspace = true")) {
            return Err(format!(
                "pixelpipe manifest: missing workspace dependency {dependency}"
            ));
        }
    }
    Ok(())
}
