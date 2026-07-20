use std::fs;
use std::path::Path;

use clap::Args;
use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, Orientation, PixelFormat, Roi};
use rusttable_pixelpipe::{
    Background, BasicStackFixture, CacheKey, ColorIdentity, ModePlanner, ModeQuality, ModeRequest,
    PipelineGeneration, PipelinePurpose, PipelineSnapshot, PipelineSnapshotInput, SourceDescriptor,
    SourceIdentity, TargetIdentity,
};
use rusttable_processing::descriptor::{exposure_descriptor, rgb_gain_descriptor};
use rusttable_processing::operation_stack::{
    InsertPosition, OperationInstance, OperationStackSnapshot, OperationStackTemplate,
    StackCommand, StackStage,
};
use sha2::{Digest, Sha256};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

pub(crate) const SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-mode-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-mode-source-map.v1";

#[derive(Debug, Args)]
pub(crate) struct ModeMatrixArgs {
    #[arg(long)]
    pub(crate) fixture: String,
    #[arg(long)]
    pub(crate) all_purposes: bool,
    #[arg(long)]
    pub(crate) all_quality: bool,
    #[arg(long)]
    pub(crate) verify_no_export_degradation: bool,
}

pub(crate) fn run(root: &Path, arguments: &ModeMatrixArgs) -> Result {
    if arguments.fixture != "corpus.raster.png.16-alpha"
        || !arguments.all_purposes
        || !arguments.all_quality
        || !arguments.verify_no_export_degradation
    {
        return Err("pixelpipe mode matrix requires the real raster fixture, all purposes, all quality, and export exactness checks".to_owned());
    }
    verify_fixture(root, &arguments.fixture)?;
    verify_mode_source_map(root, 271)?;
    let mut plan_count = 0_usize;
    let mut identities = std::collections::BTreeSet::new();
    for purpose in [
        PipelinePurpose::Preview,
        PipelinePurpose::Full,
        PipelinePurpose::Thumbnail,
        PipelinePurpose::Export,
    ] {
        let qualities = if matches!(purpose, PipelinePurpose::Full | PipelinePurpose::Export) {
            vec![ModeQuality::Exact]
        } else {
            vec![
                ModeQuality::Interactive,
                ModeQuality::Balanced,
                ModeQuality::High,
                ModeQuality::Exact,
            ]
        };
        for quality in qualities {
            let snapshot = fixture_snapshot(root, &arguments.fixture, purpose)?;
            let dimensions = if purpose == PipelinePurpose::Thumbnail {
                ImageDimensions::new(2, 2).map_err(|error| error.to_string())?
            } else {
                ImageDimensions::new(4, 3).map_err(|error| error.to_string())?
            };
            let output_color = if matches!(
                purpose,
                PipelinePurpose::Preview | PipelinePurpose::Thumbnail
            ) {
                ColorEncoding::SrgbD65
            } else {
                ColorEncoding::LinearSrgbD65
            };
            let output = rusttable_pixelpipe::OutputSpec::new(
                dimensions,
                Roi::full(dimensions),
                PixelFormat::rgba8(),
                ColorIdentity::new(output_color, 1).map_err(|error| error.to_string())?,
                Background::transparent(),
            )
            .map_err(|error| error.to_string())?;
            let request = ModeRequest::new(
                purpose,
                quality,
                TargetIdentity::consumer(purpose.tag()),
                output,
            );
            let plan = ModePlanner
                .plan(&snapshot, request, BasicStackFixture::raster().operations())
                .map_err(|error| error.to_string())?;
            if matches!(purpose, PipelinePurpose::Full | PipelinePurpose::Export)
                && (!plan.approximations().is_empty() || plan.receipt().degraded())
            {
                return Err(format!("{purpose:?} plan degraded"));
            }
            identities.insert(plan.identity().as_bytes());
            plan_count += 1;
            let _ = CacheKey::from_mode_plan(&plan);
        }
    }
    if identities.len() != plan_count {
        return Err("mode plan identity collision".to_owned());
    }
    eprintln!(
        "pixelpipe mode matrix passed (fixture={} plans={} identities={})",
        arguments.fixture,
        plan_count,
        identities.len()
    );
    Ok(())
}

fn verify_fixture(root: &Path, fixture: &str) -> Result {
    let path = root.join("fixtures/corpus/assets/raster-png-16-alpha.png");
    let bytes = fs::read(&path).map_err(|error| format!("read {fixture}: {error}"))?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let expected = "0aefb09ec3e28fc3bcb60470bad9c22322e4e4ada1fd0f34d32e6894b0615557";
    let matches = digest
        .iter()
        .zip(expected.as_bytes().chunks(2))
        .all(|(byte, chunk)| {
            std::str::from_utf8(chunk)
                .ok()
                .and_then(|text| u8::from_str_radix(text, 16).ok())
                == Some(*byte)
        });
    if !matches {
        return Err("real raster fixture checksum mismatch".to_owned());
    }
    Ok(())
}

fn fixture_snapshot(
    root: &Path,
    fixture: &str,
    purpose: PipelinePurpose,
) -> Result<PipelineSnapshot> {
    let bytes = fs::read(root.join("fixtures/corpus/assets/raster-png-16-alpha.png"))
        .map_err(|error| format!("read {fixture}: {error}"))?;
    let dimensions = ImageDimensions::new(4, 3).map_err(|error| error.to_string())?;
    let color = ColorIdentity::new(ColorEncoding::SrgbD65, 1).map_err(|error| error.to_string())?;
    let source = SourceDescriptor::new(
        SourceIdentity::new(Sha256::digest(bytes).into()),
        dimensions,
        Orientation::Normal,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        color,
    )
    .map_err(|error| error.to_string())?;
    let output = rusttable_pixelpipe::OutputSpec::new(
        dimensions,
        Roi::full(dimensions),
        PixelFormat::rgba8(),
        color,
        Background::transparent(),
    )
    .map_err(|error| error.to_string())?;
    let stack = OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation: OperationInstance::new(
                1,
                exposure_descriptor().id,
                vec![0, 1],
                StackStage::SceneLinear,
                false,
                true,
            )
            .map_err(|error| error.to_string())?,
            position: InsertPosition::End,
        })
        .map_err(|error| error.to_string())?
        .snapshot
        .apply(StackCommand::Insert {
            operation: OperationInstance::new(
                2,
                rgb_gain_descriptor().id,
                vec![1, 0],
                StackStage::CreativeAndTone,
                false,
                true,
            )
            .map_err(|error| error.to_string())?,
            position: InsertPosition::End,
        })
        .map_err(|error| error.to_string())?
        .snapshot;
    PipelineSnapshot::new(
        PipelineSnapshotInput::new(
            PipelineGeneration::new(1).map_err(|error| error.to_string())?,
            source,
            stack,
            output,
            purpose,
            rusttable_pixelpipe::ImplementationIdentity::new(
                "rusttable.pixelpipe.mode-matrix",
                1,
                "source-tree",
            )
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn verify_mode_source_map(root: &Path, issue: i64) -> Result {
    if issue != 271 {
        return Err(format!(
            "pixelpipe mode source map: unsupported issue {issue}"
        ));
    }
    let path = root.join(SOURCE_MAP);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("pixelpipe mode source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("pixelpipe mode source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_DARKTABLE_COMMIT)
    {
        return Err("pixelpipe mode source map header is invalid".to_owned());
    }
    let expected = [
        (
            "purpose",
            "src/develop/pixelpipe.h",
            "DT_DEV_PIXELPIPE_FULL",
        ),
        (
            "preview-quality",
            "src/develop/pixelpipe_hb.c",
            "DT_DEV_PIXELPIPE_PREVIEW",
        ),
        (
            "thumbnail-size-class",
            "src/common/mipmap_cache.h",
            "DT_MIPMAP_0",
        ),
        ("interactive-request", "src/views/darkroom.c", "expose"),
        (
            "export-quality",
            "src/imageio/imageio.c",
            "dt_imageio_export",
        ),
        (
            "operation-inclusion",
            "src/develop/pixelpipe_hb.c",
            "operation inclusion and fast branch",
        ),
        (
            "approximation-identity",
            "src/develop/pixelpipe_hb.c",
            "preview fast-path quality branch",
        ),
        (
            "output-accounting",
            "src/imageio/imageio.c",
            "export output encoding and precision",
        ),
        (
            "snapshot-identity",
            "src/develop/pixelpipe_hb.c",
            "pixelpipe generation",
        ),
        (
            "cache-identity",
            "src/common/mipmap_cache.h",
            "full preview thumbnail cache mode",
        ),
        (
            "basic-stack-fixture",
            "src/develop/pixelpipe_hb.c",
            "representative operation stack",
        ),
        (
            "architecture-boundary",
            "src/views/darkroom.c",
            "caller/window driven pipeline selection",
        ),
    ];
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "pixelpipe mode source map responsibilities are missing".to_owned())?;
    if entries.len() != expected.len() {
        return Err("pixelpipe mode source map is incomplete".to_owned());
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "mode source-map entry is not a table".to_owned())?;
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "mode source-map ID is missing".to_owned())?;
        let Some((_, expected_path, expected_symbol)) = expected.iter().find(|entry| entry.0 == id)
        else {
            return Err(format!("unexpected mode source-map ID {id}"));
        };
        if !seen.insert(id) {
            return Err(format!("unexpected mode source-map ID {id}"));
        }
        for key in ["upstream_path", "upstream_symbol", "rust_path", "status"] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!("mode source-map {id} missing {key}"));
            }
        }
        let owner = table["rust_path"].as_str().expect("validated owner");
        if !root.join(owner).is_file() {
            return Err(format!("mode source-map owner missing {owner}"));
        }
        if table["upstream_path"].as_str() != Some(*expected_path)
            || table["upstream_symbol"].as_str() != Some(*expected_symbol)
            || table["status"].as_str() != Some("implemented")
        {
            return Err(format!(
                "mode source-map {id} does not match its pinned anchor"
            ));
        }
    }
    let mode_source = fs::read_to_string(root.join("crates/rusttable-pixelpipe/src/mode.rs"))
        .map_err(|error| format!("read mode architecture source: {error}"))?
        .to_ascii_lowercase();
    for forbidden in [
        "iced",
        "gtk",
        "caller_mode",
        "window_mode",
        "post_scale",
        "resize_after_pipeline",
    ] {
        if mode_source.contains(forbidden) {
            return Err(format!(
                "implicit mode architecture token found: {forbidden}"
            ));
        }
    }
    Ok(())
}
