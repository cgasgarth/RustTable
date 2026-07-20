use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};
use rusttable_pixelpipe::{
    Background, ColorIdentity, DescriptorPreparationSource, ImplementationIdentity,
    PipelineGeneration, PipelinePreparer, PipelinePurpose, PipelineSnapshot, PipelineSnapshotInput,
    SourceDescriptor, SourceIdentity,
};
use rusttable_processing::descriptor::exposure_descriptor;
use rusttable_processing::operation_stack::{
    InsertPosition, OperationInstance, OperationStackSnapshot, OperationStackTemplate,
    StackCommand, StackStage,
};
use sha2::{Digest, Sha256};

use crate::Result;

const SOURCE_MAP: &str = "architecture/rusttable-pixelpipe-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.pixelpipe-source-map.v1";
const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

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
}

pub(crate) fn run(root: &Path, command: PixelpipeCommand) -> Result {
    match command {
        PixelpipeCommand::Prepare {
            fixture,
            stack,
            purposes,
            verify_identities,
        } => prepare(root, &fixture, &stack, &purposes, verify_identities),
    }
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
