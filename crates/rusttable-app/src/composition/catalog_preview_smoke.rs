use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rusttable_catalog::{EditRepository, ImportRepository, RepositoryError};
use rusttable_core::{ContentHash, Edit, EditId, ParameterName, ParameterValue, PhotoId, Revision};
use rusttable_image::{DecodedImage, DurableImageOutput, DurableImageOutputError, ImageInput};
use rusttable_image_io::{DecoderDescriptor, ImageDecoderRegistry};
use rusttable_import::{
    ImportSourceLimits, SourceSnapshotError, SourceSnapshotReadError, SourceSnapshotReader,
    decode_reference_source,
};
use rusttable_render::{PreviewBounds, RenderOutput, RenderTarget};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{PreviewError, PreviewService};

const RECEIPT_VERSION: u8 = 1;
const OUTPUT_NAME: &str = "preview.png";
const RECEIPT_NAME: &str = "preview.receipt.json";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The production ports used by the persisted catalog-to-preview slice.
///
/// The coordinator deliberately accepts ports instead of constructing redb,
/// filesystem, decoder, or output adapters. A desktop composition and a
/// repository command can therefore share this exact path.
#[derive(Clone, Copy)]
pub struct CatalogPreviewSmokePorts<'a> {
    pub imports: &'a dyn ImportRepository,
    pub edits: &'a dyn EditRepository,
    pub sources: &'a dyn SourceSnapshotReader,
    pub images: &'a dyn ImageInput,
    pub output: &'a dyn DurableImageOutput,
}

/// One deterministic persisted catalog-preview invocation.
#[derive(Debug, Clone)]
pub struct CatalogPreviewSmokeRequest {
    source_root: PathBuf,
    output_root: PathBuf,
    photo_id: PhotoId,
    edit_id: EditId,
    catalog_revision: Revision,
    max_width: u32,
    max_height: u32,
    max_source_bytes: u64,
    output_name: String,
    receipt_name: String,
}

impl CatalogPreviewSmokeRequest {
    #[must_use]
    pub fn new(
        source_root: PathBuf,
        output_root: PathBuf,
        photo_id: PhotoId,
        edit_id: EditId,
    ) -> Self {
        Self {
            source_root,
            output_root,
            photo_id,
            edit_id,
            catalog_revision: Revision::from_u64(2),
            max_width: 1024,
            max_height: 1024,
            max_source_bytes: 64 * 1024 * 1024,
            output_name: OUTPUT_NAME.to_owned(),
            receipt_name: RECEIPT_NAME.to_owned(),
        }
    }

    #[must_use]
    pub fn with_catalog_revision(mut self, revision: Revision) -> Self {
        self.catalog_revision = revision;
        self
    }

    #[must_use]
    pub fn with_preview_bounds(mut self, max_width: u32, max_height: u32) -> Self {
        self.max_width = max_width;
        self.max_height = max_height;
        self
    }

    #[must_use]
    pub fn with_source_limit(mut self, max_source_bytes: u64) -> Self {
        self.max_source_bytes = max_source_bytes;
        self
    }

    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    #[must_use]
    pub fn output_root(&self) -> &Path {
        &self.output_root
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn edit_id(&self) -> EditId {
        self.edit_id
    }
}

/// Ordered progress stages emitted by the smoke service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogPreviewSmokeStage {
    CatalogOpen,
    Resolve,
    SourceSnapshot,
    Probe,
    Decode,
    EditValidation,
    Render,
    Publish,
    Verify,
    Receipt,
}

/// A bounded progress event suitable for logs or a UI progress adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogPreviewSmokeStatus {
    stage: CatalogPreviewSmokeStage,
    complete: bool,
}

/// Cooperative cancellation shared by every stage of one smoke invocation.
#[derive(Debug, Clone, Default)]
pub struct CatalogPreviewSmokeCancellation {
    cancelled: Arc<AtomicBool>,
}

impl CatalogPreviewSmokeCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl CatalogPreviewSmokeStatus {
    #[must_use]
    pub const fn stage(self) -> CatalogPreviewSmokeStage {
        self.stage
    }

    #[must_use]
    pub const fn complete(self) -> bool {
        self.complete
    }
}

/// Stable, path-redacted evidence for one successful preview publication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPreviewSmokeReceipt {
    pub schema_version: u8,
    pub workload: String,
    pub catalog_revision: u64,
    pub photo_id: u128,
    pub edit_id: u128,
    pub edit_revision: u64,
    pub source_alias: String,
    pub source_byte_length: u64,
    pub source_sha256: String,
    pub decoder_id: String,
    pub decoder_version: u32,
    pub decoder_implementation: String,
    pub source_format: String,
    pub source_width: u32,
    pub source_height: u32,
    pub preview_width: u32,
    pub preview_height: u32,
    pub sampling: String,
    pub decoded_pixel_sha256: String,
    pub rendered_pixel_sha256: String,
    pub png_sha256: String,
    pub png_byte_length: u64,
    pub operations: Vec<CatalogPreviewOperationReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPreviewOperationReceipt {
    pub id: u128,
    pub key: String,
    pub enabled: bool,
    pub opacity: String,
    pub parameters: Vec<CatalogPreviewParameterReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPreviewParameterReceipt {
    pub name: String,
    pub value: String,
}

/// Output paths and receipt for one successful invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPreviewSmokeResult {
    receipt: CatalogPreviewSmokeReceipt,
    output_path: PathBuf,
    receipt_path: PathBuf,
}

impl CatalogPreviewSmokeResult {
    #[must_use]
    pub const fn receipt(&self) -> &CatalogPreviewSmokeReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    #[must_use]
    pub fn receipt_path(&self) -> &Path {
        &self.receipt_path
    }
}

/// Safe application composition for the first persisted catalog-preview slice.
#[derive(Debug, Clone, Copy)]
pub struct CatalogPreviewSmokeService {
    preview: PreviewService,
}

impl CatalogPreviewSmokeService {
    #[must_use]
    pub const fn new(preview: PreviewService) -> Self {
        Self { preview }
    }

    /// Resolves one persisted photo/edit, renders it on the canonical CPU
    /// path, publishes a create-new PNG, verifies it through the input port,
    /// and atomically writes a redacted receipt.
    ///
    /// # Errors
    ///
    /// Returns a stage-specific error. A failed publication removes only the
    /// output owned by this invocation and never overwrites an existing file.
    pub fn run(
        &self,
        request: &CatalogPreviewSmokeRequest,
        ports: CatalogPreviewSmokePorts<'_>,
        progress: &mut dyn FnMut(CatalogPreviewSmokeStatus),
    ) -> Result<CatalogPreviewSmokeResult, CatalogPreviewSmokeError> {
        self.run_with_cancellation(
            request,
            ports,
            &CatalogPreviewSmokeCancellation::new(),
            progress,
        )
    }

    /// Runs the slice with cooperative cancellation at each ordered stage.
    ///
    /// # Errors
    ///
    /// Returns [`CatalogPreviewSmokeError::Cancelled`] without leaving a
    /// final PNG or success receipt behind.
    #[expect(
        clippy::too_many_lines,
        reason = "the persisted vertical slice keeps its ordered stage boundaries visible"
    )]
    pub fn run_with_cancellation(
        &self,
        request: &CatalogPreviewSmokeRequest,
        ports: CatalogPreviewSmokePorts<'_>,
        cancellation: &CatalogPreviewSmokeCancellation,
        progress: &mut dyn FnMut(CatalogPreviewSmokeStatus),
    ) -> Result<CatalogPreviewSmokeResult, CatalogPreviewSmokeError> {
        emit(progress, CatalogPreviewSmokeStage::CatalogOpen, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::CatalogOpen)?;
        let record = ports
            .imports
            .find_by_photo_id(request.photo_id)
            .map_err(CatalogPreviewSmokeError::ImportRepository)?
            .ok_or(CatalogPreviewSmokeError::UnknownPhoto(request.photo_id))?;
        let edit = ports
            .edits
            .find_by_edit_id(request.edit_id)
            .map_err(CatalogPreviewSmokeError::EditRepository)?
            .ok_or(CatalogPreviewSmokeError::UnknownEdit(request.edit_id))?;
        emit(progress, CatalogPreviewSmokeStage::Resolve, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Resolve)?;
        validate_edit(&edit, record.photo().id())?;
        let source = decode_reference_source(record.source())
            .unwrap_or_else(|_| request.source_root.join(record.source().as_str()));
        let source_limits = ImportSourceLimits::new(request.max_source_bytes)
            .map_err(|_| CatalogPreviewSmokeError::InvalidSourceLimit)?;
        let snapshot = ports
            .sources
            .read_snapshot(&source, source_limits)
            .map_err(CatalogPreviewSmokeError::Snapshot)?;
        emit(progress, CatalogPreviewSmokeStage::SourceSnapshot, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::SourceSnapshot)?;
        let bytes = snapshot
            .materialize(source_limits)
            .map_err(CatalogPreviewSmokeError::SnapshotRead)?;
        let source_hash = sha256_hex(&bytes);
        let asset = record.photo().primary_asset();
        if asset.byte_length().get() != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            || asset.content_hash() != ContentHash::Sha256(hex_to_array(&source_hash))
        {
            return Err(CatalogPreviewSmokeError::SourceEvidenceMismatch);
        }
        let selected_decoder = ImageDecoderRegistry::standard()
            .select(&bytes)
            .map_err(CatalogPreviewSmokeError::ImageInput)?;
        let probe = ports
            .images
            .probe_bytes(&bytes)
            .map_err(CatalogPreviewSmokeError::ImageInput)?;
        emit(progress, CatalogPreviewSmokeStage::Probe, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Probe)?;
        if probe != record.probe() || selected_decoder.format() != probe.format() {
            return Err(CatalogPreviewSmokeError::ProbeEvidenceMismatch);
        }
        let decoded_image = ports
            .images
            .decode_bytes(&bytes)
            .map_err(CatalogPreviewSmokeError::ImageInput)?;
        emit(progress, CatalogPreviewSmokeStage::Decode, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Decode)?;
        if decoded_image.dimensions() != probe.dimensions() {
            return Err(CatalogPreviewSmokeError::DecodeEvidenceMismatch);
        }
        let target = RenderTarget::PreviewFit(
            PreviewBounds::new(request.max_width, request.max_height)
                .map_err(|_| CatalogPreviewSmokeError::InvalidPreviewBounds)?,
        );
        let output = self
            .preview
            .render_decoded_for_target(&decoded_image, &edit, target)
            .map_err(CatalogPreviewSmokeError::Preview)?;
        emit(progress, CatalogPreviewSmokeStage::Render, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Render)?;
        ports
            .sources
            .revalidate(&snapshot, source_limits)
            .map_err(CatalogPreviewSmokeError::Snapshot)?;
        let output_path = checked_output_path(request.output_root(), &request.output_name)?;
        let receipt_path = checked_output_path(request.output_root(), &request.receipt_name)?;
        let durable = ports
            .output
            .write_new_durable(
                output.image(),
                &output_path,
                rusttable_image::OutputOptions::Png,
            )
            .map_err(CatalogPreviewSmokeError::Output)?;
        emit(progress, CatalogPreviewSmokeStage::Publish, false);
        if cancellation.is_cancelled() {
            let _ = fs::remove_file(&output_path);
            return Err(CatalogPreviewSmokeError::Cancelled(
                CatalogPreviewSmokeStage::Publish,
            ));
        }
        let published = fs::read(&output_path).map_err(CatalogPreviewSmokeError::OutputRead)?;
        let reopened = ports
            .images
            .decode_path(&output_path)
            .map_err(CatalogPreviewSmokeError::ImageInput)?;
        if reopened.dimensions() != output.image().dimensions()
            || reopened.pixels() != output.image().pixels()
        {
            let _ = fs::remove_file(&output_path);
            return Err(CatalogPreviewSmokeError::OutputVerificationMismatch);
        }
        emit(progress, CatalogPreviewSmokeStage::Verify, false);
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Verify).inspect_err(|_| {
            let _ = fs::remove_file(&output_path);
        })?;
        let receipt = make_receipt(
            request,
            ReceiptInputs {
                source_alias: record.source().as_str(),
                source_bytes: &bytes,
                source_hash: &source_hash,
                selected_decoder,
                probe,
                decoded_image: &decoded_image,
                output: &output,
                png: &published,
                png_byte_length: durable.encoded_byte_length(),
                edit: &edit,
            },
        );
        if let Err(error) = write_receipt(&receipt_path, &receipt) {
            let _ = fs::remove_file(&output_path);
            return Err(error);
        }
        check_cancelled(cancellation, CatalogPreviewSmokeStage::Receipt).inspect_err(|_| {
            let _ = fs::remove_file(&output_path);
            let _ = fs::remove_file(&receipt_path);
        })?;
        emit(progress, CatalogPreviewSmokeStage::Receipt, true);
        Ok(CatalogPreviewSmokeResult {
            receipt,
            output_path,
            receipt_path,
        })
    }
}

#[derive(Debug)]
pub enum CatalogPreviewSmokeError {
    ImportRepository(RepositoryError),
    EditRepository(rusttable_catalog::EditRepositoryError),
    UnknownPhoto(PhotoId),
    UnknownEdit(EditId),
    InvalidEdit,
    InvalidSourceLimit,
    InvalidPreviewBounds,
    InvalidOutputName,
    SourceEvidenceMismatch,
    ProbeEvidenceMismatch,
    DecodeEvidenceMismatch,
    Snapshot(SourceSnapshotError),
    SnapshotRead(SourceSnapshotReadError),
    ImageInput(rusttable_image::ImageInputError),
    Preview(PreviewError),
    Output(DurableImageOutputError),
    OutputRead(std::io::Error),
    OutputVerificationMismatch,
    Cancelled(CatalogPreviewSmokeStage),
    ReceiptIo(std::io::Error),
    ReceiptSerialization(serde_json::Error),
}

impl std::fmt::Display for CatalogPreviewSmokeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "catalog-preview smoke failed: {self:?}")
    }
}

impl std::error::Error for CatalogPreviewSmokeError {}

fn emit(
    progress: &mut dyn FnMut(CatalogPreviewSmokeStatus),
    stage: CatalogPreviewSmokeStage,
    complete: bool,
) {
    progress(CatalogPreviewSmokeStatus { stage, complete });
}

fn check_cancelled(
    cancellation: &CatalogPreviewSmokeCancellation,
    stage: CatalogPreviewSmokeStage,
) -> Result<(), CatalogPreviewSmokeError> {
    if cancellation.is_cancelled() {
        Err(CatalogPreviewSmokeError::Cancelled(stage))
    } else {
        Ok(())
    }
}

fn validate_edit(edit: &Edit, photo_id: PhotoId) -> Result<(), CatalogPreviewSmokeError> {
    if edit.photo_id() != photo_id {
        return Err(CatalogPreviewSmokeError::InvalidEdit);
    }
    let operations = edit.operations().collect::<Vec<_>>();
    if operations.len() != 2
        || !operation_matches(operations[0], "rusttable.exposure", &[("stops", 0.5)])
        || !operation_matches(
            operations[1],
            "rusttable.rgb_gain",
            &[("red", 1.0), ("green", 0.75), ("blue", 0.5)],
        )
    {
        return Err(CatalogPreviewSmokeError::InvalidEdit);
    }
    Ok(())
}

fn operation_matches(
    operation: &rusttable_core::Operation,
    key: &str,
    expected: &[(&str, f64)],
) -> bool {
    operation.is_enabled()
        && operation.opacity().get().to_bits() == 1.0_f64.to_bits()
        && operation.key().as_str() == key
        && expected.iter().all(|(name, value)| {
            let name = ParameterName::new(*name).expect("smoke operation parameter is valid");
            matches!(
                operation.parameter(&name),
                Some(ParameterValue::Scalar(actual))
                    if actual.get().to_bits() == value.to_bits()
            )
        })
        && operation.parameters().count() == expected.len()
}

#[derive(Clone, Copy)]
struct ReceiptInputs<'a> {
    source_alias: &'a str,
    source_bytes: &'a [u8],
    source_hash: &'a str,
    selected_decoder: DecoderDescriptor,
    probe: rusttable_image::ImageProbe,
    decoded_image: &'a DecodedImage,
    output: &'a RenderOutput,
    png: &'a [u8],
    png_byte_length: u64,
    edit: &'a Edit,
}

fn make_receipt(
    request: &CatalogPreviewSmokeRequest,
    inputs: ReceiptInputs<'_>,
) -> CatalogPreviewSmokeReceipt {
    CatalogPreviewSmokeReceipt {
        schema_version: RECEIPT_VERSION,
        workload: "catalog-to-preview-v1".to_owned(),
        catalog_revision: request.catalog_revision.get(),
        photo_id: inputs.edit.photo_id().get(),
        edit_id: inputs.edit.id().get(),
        edit_revision: inputs.edit.revision().get(),
        source_alias: inputs.source_alias.replace('\\', "/"),
        source_byte_length: u64::try_from(inputs.source_bytes.len()).unwrap_or(u64::MAX),
        source_sha256: inputs.source_hash.to_owned(),
        decoder_id: inputs.selected_decoder.identity().id().to_owned(),
        decoder_version: inputs.selected_decoder.identity().version(),
        decoder_implementation: inputs
            .selected_decoder
            .identity()
            .implementation()
            .to_owned(),
        source_format: format!("{:?}", inputs.probe.format()),
        source_width: inputs.probe.dimensions().width(),
        source_height: inputs.probe.dimensions().height(),
        preview_width: inputs.output.image().dimensions().width(),
        preview_height: inputs.output.image().dimensions().height(),
        sampling: format!("{:?}", inputs.output.plan().sampling()),
        decoded_pixel_sha256: sha256_hex(inputs.decoded_image.pixels()),
        rendered_pixel_sha256: sha256_hex(inputs.output.image().pixels()),
        png_sha256: sha256_hex(inputs.png),
        png_byte_length: inputs.png_byte_length,
        operations: inputs
            .edit
            .operations()
            .map(|operation| CatalogPreviewOperationReceipt {
                id: operation.id().get(),
                key: operation.key().as_str().to_owned(),
                enabled: operation.is_enabled(),
                opacity: operation.opacity().get().to_string(),
                parameters: operation
                    .parameters()
                    .map(|(name, value)| CatalogPreviewParameterReceipt {
                        name: name.as_str().to_owned(),
                        value: format!("{value:?}"),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn checked_output_path(root: &Path, name: &str) -> Result<PathBuf, CatalogPreviewSmokeError> {
    let path = Path::new(name);
    if path.file_name().and_then(|value| value.to_str()) != Some(name) {
        return Err(CatalogPreviewSmokeError::InvalidOutputName);
    }
    Ok(root.join(path))
}

fn write_receipt(
    destination: &Path,
    receipt: &CatalogPreviewSmokeReceipt,
) -> Result<(), CatalogPreviewSmokeError> {
    if destination.exists() {
        return Err(CatalogPreviewSmokeError::Output(
            DurableImageOutputError::BeforePublication {
                source: rusttable_image::ImageOutputError::DestinationExists {
                    path: destination.to_owned(),
                },
            },
        ));
    }
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(CatalogPreviewSmokeError::ReceiptSerialization)?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary =
        destination.with_extension(format!("json.tmp-{}-{sequence}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(CatalogPreviewSmokeError::ReceiptIo)?;
    if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(CatalogPreviewSmokeError::ReceiptIo(error));
    }
    if let Err(error) = fs::hard_link(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(CatalogPreviewSmokeError::ReceiptIo(error));
    }
    fs::remove_file(temporary).map_err(CatalogPreviewSmokeError::ReceiptIo)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn hex_to_array(value: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .expect("sha256_hex always emits valid hexadecimal");
    }
    bytes
}
