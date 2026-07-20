use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use rusttable_app::{
    CatalogPreviewSmokePorts, CatalogPreviewSmokeReceipt, CatalogPreviewSmokeRequest,
    CatalogPreviewSmokeService, PreviewService,
};
use rusttable_catalog::{CatalogState, EditRepository, SourcePath};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{
    AssetId, Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_image::{DecodeLimits, OutputLimits};
use rusttable_image_io::{FileImageInput, FileImageOutput};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceImportRequest, SourceImportService,
};
use rusttable_metadata::{ExifMetadataInput, MetadataLimits};
use rusttable_render::PreviewBounds;
use rusttable_testkit::fixtures::{FixtureManifest, qualify_binary, sha256_hex};

use crate::Result;

const FIXTURE_ID: &str = "corpus.raster.png.16-alpha";
const PHOTO_ID: u128 = 181_001;
const ASSET_ID: u128 = 181_002;
const EDIT_ID: u128 = 181_003;

#[derive(Debug, Subcommand)]
pub(crate) enum FoundationCommand {
    /// Run the real persisted catalog-to-preview vertical slice.
    CatalogPreview {
        /// Parser-qualified fixture ID from fixtures/manifest.toml.
        #[arg(long)]
        fixture: String,
        /// Number of separate output roots to render.
        #[arg(long, default_value_t = 2)]
        repeat: u8,
        /// Reopen the redb catalog and rebuild the ports for every run.
        #[arg(long)]
        verify_restart: bool,
        /// Verify create-new collision behavior after the positive runs.
        #[arg(long)]
        verify_failures: bool,
        /// Root for ephemeral smoke inputs and outputs.
        #[arg(long)]
        work_root: Option<PathBuf>,
        /// Internal worker mode used for a true fresh-process restart check.
        #[arg(long, hide = true)]
        worker: bool,
        /// Internal worker output index.
        #[arg(long, hide = true)]
        output_index: Option<u8>,
    },
}

pub(crate) fn run(root: &Path, command: FoundationCommand) -> Result {
    match command {
        FoundationCommand::CatalogPreview {
            fixture,
            repeat,
            verify_restart,
            verify_failures,
            work_root,
            worker,
            output_index,
        } => run_catalog_preview(
            root,
            &fixture,
            repeat,
            verify_restart,
            verify_failures,
            work_root,
            worker,
            output_index,
        ),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the CLI fields map directly to the documented smoke command"
)]
fn run_catalog_preview(
    root: &Path,
    fixture_id: &str,
    repeat: u8,
    verify_restart: bool,
    verify_failures: bool,
    work_root: Option<PathBuf>,
    worker: bool,
    output_index: Option<u8>,
) -> Result {
    if fixture_id != FIXTURE_ID {
        return Err(format!(
            "catalog-preview accepts only the parser-qualified fixture {FIXTURE_ID}"
        ));
    }
    if repeat == 0 {
        return Err("catalog-preview repeat must be at least one".to_owned());
    }
    let work_root = work_root.unwrap_or_else(|| root.join("target/catalog-preview-smoke"));
    let source_root = work_root.join("source-root");
    let catalog_path = work_root.join("catalog.redb");
    if worker {
        let index =
            output_index.ok_or_else(|| "catalog-preview worker needs --output-index".to_owned())?;
        run_one(&source_root, &catalog_path, &work_root, index)?;
        return Ok(());
    }
    match fs::remove_dir_all(&work_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("clean smoke root: {error}")),
    }
    fs::create_dir_all(source_root.join("imports"))
        .map_err(|error| format!("create smoke source root: {error}"))?;
    prepare_fixture(root, &source_root, &catalog_path)?;

    let mut expected = None;
    for index in 0..repeat {
        let receipt = if verify_restart {
            run_fresh_process(root, &work_root, index)?
        } else {
            run_one(&source_root, &catalog_path, &work_root, index)?
        };
        if expected
            .as_ref()
            .is_some_and(|expected| expected != &receipt)
        {
            return Err(format!("catalog-preview run {index} was not deterministic"));
        }
        expected = Some(receipt.clone());
        eprintln!(
            "catalog-preview: run {index} {}x{} png={} bytes",
            receipt.preview_width, receipt.preview_height, receipt.png_byte_length
        );
    }

    if verify_failures {
        verify_collision(&catalog_path, &source_root, &work_root.join("output-0"))?;
    }
    Ok(())
}

fn run_one(
    source_root: &Path,
    catalog_path: &Path,
    work_root: &Path,
    index: u8,
) -> Result<CatalogPreviewSmokeReceipt> {
    let output_root = work_root.join(format!("output-{index}"));
    fs::create_dir_all(&output_root)
        .map_err(|error| format!("create smoke output root: {error}"))?;
    let repository = RedbCatalogRepository::open(catalog_path)
        .map_err(|error| format!("reopen smoke catalog: {error}"))?;
    let image_input = FileImageInput::new(decode_limits());
    let image_output = FileImageOutput::new(output_limits());
    let service = CatalogPreviewSmokeService::new(PreviewService::new(
        decode_limits(),
        PreviewBounds::new(1024, 1024).expect("smoke bounds are valid"),
    ));
    let request = CatalogPreviewSmokeRequest::new(
        source_root.to_owned(),
        output_root,
        PhotoId::new(PHOTO_ID).expect("smoke photo ID is nonzero"),
        EditId::new(EDIT_ID).expect("smoke edit ID is nonzero"),
    )
    .with_catalog_revision(Revision::from_u64(2));
    let ports = CatalogPreviewSmokePorts {
        imports: &repository,
        edits: &repository,
        sources: &FileSourceSnapshotReader,
        images: &image_input,
        output: &image_output,
    };
    let mut progress = |_| {};
    let result = service
        .run(&request, ports, &mut progress)
        .map_err(|error| format!("catalog-preview run {index}: {error}"))?;
    Ok(result.receipt().clone())
}

fn run_fresh_process(
    root: &Path,
    work_root: &Path,
    index: u8,
) -> Result<CatalogPreviewSmokeReceipt> {
    let executable =
        std::env::current_exe().map_err(|error| format!("resolve xtask executable: {error}"))?;
    let status = std::process::Command::new(executable)
        .args([
            "foundation",
            "catalog-preview",
            "--fixture",
            FIXTURE_ID,
            "--repeat",
            "1",
            "--work-root",
        ])
        .arg(work_root)
        .args(["--worker", "--output-index"])
        .arg(index.to_string())
        .current_dir(root)
        .status()
        .map_err(|error| format!("start catalog-preview worker: {error}"))?;
    if !status.success() {
        return Err(format!("catalog-preview worker exited with {status}"));
    }
    let receipt_path = work_root
        .join(format!("output-{index}"))
        .join("preview.receipt.json");
    serde_json::from_slice(
        &fs::read(&receipt_path).map_err(|error| format!("read worker receipt: {error}"))?,
    )
    .map_err(|error| format!("parse worker receipt: {error}"))
}

fn prepare_fixture(root: &Path, source_root: &Path, catalog_path: &Path) -> Result {
    let manifest = FixtureManifest::parse(
        &fs::read_to_string(root.join("fixtures/manifest.toml"))
            .map_err(|error| format!("read fixture manifest: {error}"))?,
    )
    .map_err(|error| format!("parse fixture manifest: {error}"))?;
    let entry = manifest
        .fixture(FIXTURE_ID)
        .ok_or_else(|| format!("fixture {FIXTURE_ID} is not registered"))?;
    let fixture_path = entry
        .canonical_path(root)
        .map_err(|error| format!("resolve fixture: {error}"))?;
    let fixture_bytes =
        fs::read(&fixture_path).map_err(|error| format!("read fixture: {error}"))?;
    if u64::try_from(fixture_bytes.len()).unwrap_or(u64::MAX) != entry.size
        || sha256_hex(&fixture_bytes) != entry.sha256
    {
        return Err("fixture size or SHA-256 does not match its manifest".to_owned());
    }
    qualify_binary(entry, &fixture_bytes)
        .map_err(|error| format!("qualify parser fixture: {error}"))?;
    let source = source_root.join("imports/corpus.raster.png.16-alpha.png");
    fs::copy(&fixture_path, &source).map_err(|error| format!("copy fixture source: {error}"))?;

    let mut state = CatalogState::new();
    let mut repository = RedbCatalogRepository::open(catalog_path)
        .map_err(|error| format!("create smoke catalog: {error}"))?;
    let request = SourceImportRequest::new(
        PhotoId::new(PHOTO_ID).expect("smoke photo ID is nonzero"),
        AssetId::new(ASSET_ID).expect("smoke asset ID is nonzero"),
        SourcePath::new("imports/corpus.raster.png.16-alpha.png").expect("smoke source key"),
        source,
    );
    SourceImportService::inspect_and_register(
        &mut state,
        Revision::ZERO,
        &request,
        ImportSourceLimits::new(64 * 1024 * 1024).expect("source limit"),
        &mut repository,
        &FileSourceSnapshotReader,
        &FileImageInput::new(decode_limits()),
        &ExifMetadataInput::new(metadata_limits()),
    )
    .map_err(|error| format!("register fixture source: {error}"))?;
    let edit = exact_edit(
        EditId::new(EDIT_ID).expect("smoke edit ID is nonzero"),
        PhotoId::new(PHOTO_ID).expect("smoke photo ID is nonzero"),
    );
    repository
        .commit_new(&edit)
        .map_err(|error| format!("persist smoke edit: {error}"))?;
    Ok(())
}

fn verify_collision(catalog_path: &Path, source_root: &Path, output_root: &Path) -> Result {
    let repository = RedbCatalogRepository::open(catalog_path)
        .map_err(|error| format!("reopen collision catalog: {error}"))?;
    let image_input = FileImageInput::new(decode_limits());
    let image_output = FileImageOutput::new(output_limits());
    let service = CatalogPreviewSmokeService::new(PreviewService::new(
        decode_limits(),
        PreviewBounds::new(1024, 1024).expect("smoke bounds are valid"),
    ));
    let request = CatalogPreviewSmokeRequest::new(
        source_root.to_owned(),
        output_root.to_owned(),
        PhotoId::new(PHOTO_ID).expect("smoke photo ID is nonzero"),
        EditId::new(EDIT_ID).expect("smoke edit ID is nonzero"),
    );
    let ports = CatalogPreviewSmokePorts {
        imports: &repository,
        edits: &repository,
        sources: &FileSourceSnapshotReader,
        images: &image_input,
        output: &image_output,
    };
    let mut progress = |_| {};
    match service.run(&request, ports, &mut progress) {
        Err(rusttable_app::CatalogPreviewSmokeError::Output(_)) => Ok(()),
        Err(error) => Err(format!("collision check returned the wrong error: {error}")),
        Ok(_) => Err("collision check unexpectedly overwrote the published preview".to_owned()),
    }
}

fn exact_edit(edit_id: EditId, photo_id: PhotoId) -> Edit {
    Edit::new(
        edit_id,
        photo_id,
        Revision::ZERO,
        [
            operation(181_101, "rusttable.exposure", &[("stops", 0.5)]),
            operation(
                181_102,
                "rusttable.rgb_gain",
                &[("red", 1.0), ("green", 0.75), ("blue", 0.5)],
            ),
        ],
    )
    .expect("exact smoke edit is valid")
}

fn operation(id: u128, key: &str, values: &[(&str, f64)]) -> Operation {
    Operation::new(
        OperationId::new(id).expect("smoke operation ID is nonzero"),
        OperationKey::new(key).expect("smoke operation key is valid"),
        true,
        values.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("smoke parameter is valid"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("smoke value is finite")),
            )
        }),
    )
    .expect("smoke operation is valid")
}

fn decode_limits() -> DecodeLimits {
    DecodeLimits::new(
        64 * 1024 * 1024,
        16_384,
        16_384,
        16_777_216,
        64 * 1024 * 1024,
    )
    .expect("smoke decode limits are valid")
}

fn output_limits() -> OutputLimits {
    OutputLimits::new(64 * 1024 * 1024).expect("smoke output limits are valid")
}

fn metadata_limits() -> MetadataLimits {
    MetadataLimits::new(64 * 1024 * 1024, 512 * 1024, 512, 512, 8, 2_048, 128 * 1024)
        .expect("smoke metadata limits are valid")
}
