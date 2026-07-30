use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

use crate::model::{
    AlphaPolicy, AuxiliaryTensor, ModelInferenceError, ModelManifest, ModelTileContract,
    ModelValidationError, Provider, ProviderPolicy, SuperResolutionModel,
};
use crate::tiff::{TiffError, TiffProbe, encode_tiff, verify_tiff_artifact};
use crate::types::{
    CancellationToken, ImageDimensions, LinearRgbaImage, ShadowPolicy, SuperResolutionRequest,
    SuperResolutionScale,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLimits {
    pub max_output_bytes: u64,
    pub max_session_bytes: u64,
    pub max_provider_bytes: u64,
}

impl Default for MemoryLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 512 * 1024 * 1024,
            max_session_bytes: 512 * 1024 * 1024,
            max_provider_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedTile {
    origin_x: u32,
    origin_y: u32,
    valid_width: u32,
    valid_height: u32,
}

impl PlannedTile {
    #[must_use]
    pub const fn origin(self) -> (u32, u32) {
        (self.origin_x, self.origin_y)
    }
    #[must_use]
    pub const fn valid_dimensions(self) -> (u32, u32) {
        (self.valid_width, self.valid_height)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperResolutionPlan {
    source_dimensions: ImageDimensions,
    output_dimensions: ImageDimensions,
    provider: Provider,
    model_identity: String,
    tiles: Vec<PlannedTile>,
    tile_contract: ModelTileContract,
    alpha_policy: AlphaPolicy,
    shadow_boost: bool,
    estimated_bytes: u64,
}

impl SuperResolutionPlan {
    #[must_use]
    pub const fn source_dimensions(&self) -> ImageDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> ImageDimensions {
        self.output_dimensions
    }
    #[must_use]
    pub const fn provider(&self) -> Provider {
        self.provider
    }
    #[must_use]
    pub fn model_identity(&self) -> &str {
        &self.model_identity
    }
    #[must_use]
    pub fn tiles(&self) -> &[PlannedTile] {
        &self.tiles
    }
    #[must_use]
    pub const fn shadow_boost(&self) -> bool {
        self.shadow_boost
    }
    #[must_use]
    pub const fn estimated_bytes(&self) -> u64 {
        self.estimated_bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInput {
    tile: PlannedTile,
    width: u32,
    height: u32,
    scale: u32,
    planar_rgb: Vec<f32>,
    auxiliary: Vec<Vec<f32>>,
}

impl ModelInput {
    #[must_use]
    pub const fn tile(&self) -> PlannedTile {
        self.tile
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn scale(&self) -> u32 {
        self.scale
    }
    #[must_use]
    pub fn planar_rgb(&self) -> &[f32] {
        &self.planar_rgb
    }
    #[must_use]
    pub fn auxiliary(&self) -> &[Vec<f32>] {
        &self.auxiliary
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelOutput {
    width: u32,
    height: u32,
    planar_rgb: Vec<f32>,
}

impl ModelOutput {
    pub fn new(width: u32, height: u32, planar_rgb: Vec<f32>) -> Self {
        Self {
            width,
            height,
            planar_rgb,
        }
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub fn planar_rgb(&self) -> &[f32] {
        &self.planar_rgb
    }
}

pub trait SuperResolutionPublisher {
    fn publish(
        &mut self,
        destination: Option<&Path>,
        artifact: &[u8],
        probe: &TiffProbe,
    ) -> Result<PublicationReceipt, PublicationError>;
}

pub trait CatalogPort {
    fn register_derived(
        &mut self,
        source_photo_id: u64,
        source_edit_revision: u64,
        publication: &PublicationReceipt,
    ) -> Result<CatalogReceipt, CatalogError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationReceipt {
    destination: Option<PathBuf>,
    artifact_sha256: String,
    bytes: u64,
    dimensions: ImageDimensions,
}

impl PublicationReceipt {
    #[doc(hidden)]
    #[must_use]
    pub fn from_verified_artifact(dimensions: ImageDimensions, artifact: &[u8]) -> Self {
        Self {
            destination: None,
            artifact_sha256: sha256(artifact),
            bytes: u64::try_from(artifact.len()).unwrap_or(u64::MAX),
            dimensions,
        }
    }

    #[must_use]
    pub fn destination(&self) -> Option<&Path> {
        self.destination.as_deref()
    }
    #[must_use]
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogReceipt {
    pub derived_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperResolutionReceipt {
    pub plan: SuperResolutionPlan,
    pub publication: PublicationReceipt,
    pub catalog: Option<CatalogReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperResolutionProgress {
    Planned,
    Inference { completed: usize, total: usize },
    Encoding,
    Published,
    Cataloged,
}

#[derive(Debug)]
pub enum PublicationError {
    MissingDestination,
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Verification(TiffError),
}

impl fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "TIFF publication failed: {self:?}")
    }
}
impl std::error::Error for PublicationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    Rejected,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("derived artifact was not registered")
    }
}
impl std::error::Error for CatalogError {}

#[derive(Debug)]
pub enum SuperResolutionError {
    Model(ModelValidationError),
    Dimensions,
    UnsupportedWideGamut,
    MemoryLimit {
        estimated: u64,
        limit: u64,
    },
    Tiff(TiffError),
    ModelInference(ModelInferenceError),
    OutputShape {
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    OutputBytes {
        expected: usize,
        actual: usize,
    },
    NonFiniteOutput,
    Cancelled(SuperResolutionProgress),
    Publication(PublicationError),
    Catalog(CatalogError),
}

impl fmt::Display for SuperResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "super-resolution failed: {self:?}")
    }
}
impl std::error::Error for SuperResolutionError {}

pub fn plan_super_resolution<M: SuperResolutionModel>(
    request: &SuperResolutionRequest,
    model: &M,
    provider_policy: ProviderPolicy,
    limits: MemoryLimits,
) -> Result<SuperResolutionPlan, SuperResolutionError> {
    if request.settings().preserve_wide_gamut {
        return Err(SuperResolutionError::UnsupportedWideGamut);
    }
    let provider = provider_policy.selected();
    model
        .manifest()
        .validate_for(request.scale(), provider)
        .map_err(SuperResolutionError::Model)?;
    let source = request.source().dimensions();
    let scale = u64::from(request.scale().integer());
    let output_width = u32::try_from(
        u64::from(source.width())
            .checked_mul(scale)
            .ok_or(SuperResolutionError::Dimensions)?,
    )
    .map_err(|_| SuperResolutionError::Dimensions)?;
    let output_height = u32::try_from(
        u64::from(source.height())
            .checked_mul(scale)
            .ok_or(SuperResolutionError::Dimensions)?,
    )
    .map_err(|_| SuperResolutionError::Dimensions)?;
    let output_dimensions = ImageDimensions::new(output_width, output_height)
        .map_err(|_| SuperResolutionError::Dimensions)?;
    let tile = model.manifest().tile();
    let step_x = tile
        .width
        .checked_sub(
            tile.valid_crop
                .left
                .checked_add(tile.valid_crop.right)
                .ok_or(SuperResolutionError::Dimensions)?,
        )
        .ok_or(SuperResolutionError::Dimensions)?;
    let step_y = tile
        .height
        .checked_sub(
            tile.valid_crop
                .top
                .checked_add(tile.valid_crop.bottom)
                .ok_or(SuperResolutionError::Dimensions)?,
        )
        .ok_or(SuperResolutionError::Dimensions)?;
    if step_x == 0 || step_y == 0 {
        return Err(SuperResolutionError::Dimensions);
    }
    let cols = (source.width() - 1) / step_x + 1;
    let rows = (source.height() - 1) / step_y + 1;
    let tile_count = usize::try_from(
        u64::from(cols)
            .checked_mul(u64::from(rows))
            .ok_or(SuperResolutionError::Dimensions)?,
    )
    .map_err(|_| SuperResolutionError::Dimensions)?;
    let mut tiles = Vec::with_capacity(tile_count);
    for row in 0..rows {
        for column in 0..cols {
            let origin_x = column * step_x;
            let origin_y = row * step_y;
            tiles.push(PlannedTile {
                origin_x,
                origin_y,
                valid_width: step_x.min(source.width() - origin_x),
                valid_height: step_y.min(source.height() - origin_y),
            });
        }
    }
    let output_pixels = u64::from(output_width)
        .checked_mul(u64::from(output_height))
        .ok_or(SuperResolutionError::Dimensions)?;
    let tile_pixels = u64::from(tile.width)
        .checked_mul(u64::from(tile.height))
        .ok_or(SuperResolutionError::Dimensions)?;
    let output_bytes = output_pixels
        .checked_mul(16)
        .ok_or(SuperResolutionError::Dimensions)?;
    let tile_bytes = tile_pixels
        .checked_mul(3 * 4)
        .ok_or(SuperResolutionError::Dimensions)?;
    let estimated = output_bytes
        .checked_add(
            tile_bytes
                .checked_mul(2)
                .ok_or(SuperResolutionError::Dimensions)?,
        )
        .ok_or(SuperResolutionError::Dimensions)?;
    if estimated > limits.max_output_bytes
        || estimated > limits.max_session_bytes
        || estimated > limits.max_provider_bytes
    {
        return Err(SuperResolutionError::MemoryLimit {
            estimated,
            limit: limits
                .max_output_bytes
                .min(limits.max_session_bytes)
                .min(limits.max_provider_bytes),
        });
    }
    let tiff_channels = if request.settings().tiff().alpha() {
        4u64
    } else {
        3u64
    };
    let tiff_bytes = output_pixels
        .checked_mul(tiff_channels)
        .and_then(|value| {
            value.checked_mul(u64::try_from(request.settings().tiff().bit_depth().bytes()).ok()?)
        })
        .and_then(|value| {
            value.checked_add(u64::try_from(request.settings().output_profile().icc().len()).ok()?)
        })
        .ok_or(SuperResolutionError::Dimensions)?;
    if tiff_bytes > request.settings().tiff().max_bytes() || tiff_bytes > u64::from(u32::MAX) {
        return Err(SuperResolutionError::Tiff(TiffError::ByteLimit));
    }
    let shadow_boost = model.manifest().shadow_boost()
        && request.settings().shadow_policy == ShadowPolicy::Auto
        && has_deep_shadows(request.source());
    Ok(SuperResolutionPlan {
        source_dimensions: source,
        output_dimensions,
        provider,
        model_identity: model.manifest().identity(),
        tiles,
        tile_contract: tile,
        alpha_policy: model.manifest().alpha(),
        shadow_boost,
        estimated_bytes: estimated,
    })
}

pub fn execute_super_resolution<M, P, C, F>(
    request: &SuperResolutionRequest,
    model: &mut M,
    publisher: &mut P,
    catalog: Option<&mut C>,
    provider_policy: ProviderPolicy,
    limits: MemoryLimits,
    cancel: &CancellationToken,
    mut progress: F,
) -> Result<SuperResolutionReceipt, SuperResolutionError>
where
    M: SuperResolutionModel,
    P: SuperResolutionPublisher,
    C: CatalogPort,
    F: FnMut(SuperResolutionProgress),
{
    let plan = plan_super_resolution(request, model, provider_policy, limits)?;
    progress(SuperResolutionProgress::Planned);
    if cancel.is_cancelled() {
        return Err(SuperResolutionError::Cancelled(
            SuperResolutionProgress::Planned,
        ));
    }
    let mut output = vec![
        [0.0; 4];
        plan.output_dimensions
            .pixels()
            .ok_or(SuperResolutionError::Dimensions)?
    ];
    for (index, planned) in plan.tiles.iter().copied().enumerate() {
        if cancel.is_cancelled() {
            return Err(SuperResolutionError::Cancelled(
                SuperResolutionProgress::Inference {
                    completed: index,
                    total: plan.tiles.len(),
                },
            ));
        }
        let input = build_model_input(
            request.source(),
            request.source_profile(),
            model.manifest(),
            planned,
            plan.shadow_boost,
            request.scale(),
        );
        let model_output = model
            .infer(&input, cancel)
            .map_err(SuperResolutionError::ModelInference)?;
        let expected_width = model
            .manifest()
            .tile()
            .width
            .checked_mul(request.scale().integer())
            .ok_or(SuperResolutionError::Dimensions)?;
        let expected_height = model
            .manifest()
            .tile()
            .height
            .checked_mul(request.scale().integer())
            .ok_or(SuperResolutionError::Dimensions)?;
        if model_output.width() != expected_width || model_output.height() != expected_height {
            return Err(SuperResolutionError::OutputShape {
                expected_width,
                expected_height,
                actual_width: model_output.width(),
                actual_height: model_output.height(),
            });
        }
        let expected = usize::try_from(
            u64::from(expected_width)
                .checked_mul(u64::from(expected_height))
                .ok_or(SuperResolutionError::Dimensions)?
                .checked_mul(3)
                .ok_or(SuperResolutionError::Dimensions)?,
        )
        .map_err(|_| SuperResolutionError::Dimensions)?;
        if model_output.planar_rgb().len() != expected {
            return Err(SuperResolutionError::OutputBytes {
                expected,
                actual: model_output.planar_rgb().len(),
            });
        }
        if model_output
            .planar_rgb()
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(SuperResolutionError::NonFiniteOutput);
        }
        assemble_tile(
            &mut output,
            request,
            &plan,
            planned,
            model_output.planar_rgb(),
        );
        progress(SuperResolutionProgress::Inference {
            completed: index + 1,
            total: plan.tiles.len(),
        });
    }
    if cancel.is_cancelled() {
        return Err(SuperResolutionError::Cancelled(
            SuperResolutionProgress::Encoding,
        ));
    }
    progress(SuperResolutionProgress::Encoding);
    let artifact = encode_tiff(
        plan.output_dimensions,
        &output,
        request.settings().output_profile(),
        request.settings().tiff(),
    )
    .map_err(SuperResolutionError::Tiff)?;
    let probe = verify_tiff_artifact(&artifact).map_err(SuperResolutionError::Tiff)?;
    if probe.dimensions() != plan.output_dimensions
        || probe.icc_sha256() != sha256(request.settings().output_profile().icc())
    {
        return Err(SuperResolutionError::Tiff(TiffError::ProfileMismatch));
    }
    let publication = publisher
        .publish(request.settings().destination.as_deref(), &artifact, &probe)
        .map_err(SuperResolutionError::Publication)?;
    progress(SuperResolutionProgress::Published);
    let catalog = if let Some(catalog) = catalog {
        let receipt = catalog
            .register_derived(
                request.settings().source_photo_id,
                request.settings().edit_revision,
                &publication,
            )
            .map_err(SuperResolutionError::Catalog)?;
        progress(SuperResolutionProgress::Cataloged);
        Some(receipt)
    } else {
        None
    };
    Ok(SuperResolutionReceipt {
        plan,
        publication,
        catalog,
    })
}

fn build_model_input(
    source: &LinearRgbaImage,
    profile: &crate::ColorProfile,
    manifest: &ModelManifest,
    tile: PlannedTile,
    shadow_boost: bool,
    scale: SuperResolutionScale,
) -> ModelInput {
    let contract = manifest.tile();
    let plane = usize::try_from(u64::from(contract.width) * u64::from(contract.height))
        .expect("validated tile dimensions");
    let mut planar_rgb = vec![0.0; plane * 3];
    for y in 0..contract.height {
        for x in 0..contract.width {
            let source_x = mirror(
                i64::from(tile.origin_x) - i64::from(contract.valid_crop.left) + i64::from(x),
                source.dimensions().width(),
            );
            let source_y = mirror(
                i64::from(tile.origin_y) - i64::from(contract.valid_crop.top) + i64::from(y),
                source.dimensions().height(),
            );
            let pixel = source
                .pixel(source_x, source_y)
                .expect("mirror stays inside source");
            let mut rgb = profile.to_model_srgb([pixel[0], pixel[1], pixel[2]]);
            if shadow_boost {
                for channel in &mut rgb {
                    *channel = channel.max(0.0).sqrt();
                }
            }
            for channel in &mut rgb {
                *channel = srgb_encode(channel.clamp(0.0, 1.0));
            }
            let index = usize::try_from(y * contract.width + x).expect("validated tile dimensions");
            planar_rgb[index] = rgb[0];
            planar_rgb[plane + index] = rgb[1];
            planar_rgb[2 * plane + index] = rgb[2];
        }
    }
    let auxiliary = manifest
        .auxiliary()
        .iter()
        .map(|tensor| match tensor {
            AuxiliaryTensor::Constant {
                channels, value, ..
            } => vec![
                *value;
                plane * usize::try_from(*channels).expect("validated auxiliary channels")
            ],
        })
        .collect();
    ModelInput {
        tile,
        width: contract.width,
        height: contract.height,
        scale: scale.integer(),
        planar_rgb,
        auxiliary,
    }
}

fn assemble_tile(
    output: &mut [[f32; 4]],
    request: &SuperResolutionRequest,
    plan: &SuperResolutionPlan,
    tile: PlannedTile,
    planar: &[f32],
) {
    let contract = plan.tile_contract;
    let scale = request.scale().integer();
    let tile_width = contract.width * scale;
    let plane = usize::try_from(u64::from(tile_width) * u64::from(contract.height * scale))
        .expect("validated tile dimensions");
    for dy in 0..tile.valid_height * scale {
        for dx in 0..tile.valid_width * scale {
            let source_index = usize::try_from(
                (contract.valid_crop.top * scale + dy) * tile_width
                    + contract.valid_crop.left * scale
                    + dx,
            )
            .expect("validated tile dimensions");
            let model_rgb = [
                srgb_decode(planar[source_index]),
                srgb_decode(planar[plane + source_index]),
                srgb_decode(planar[2 * plane + source_index]),
            ];
            let mut rgb = request
                .settings()
                .output_profile()
                .from_model_srgb(model_rgb);
            for channel in &mut rgb {
                if plan.shadow_boost {
                    *channel *= *channel;
                }
            }
            let source_x = tile.origin_x + dx / scale;
            let source_y = tile.origin_y + dy / scale;
            let alpha = match plan.alpha_policy {
                AlphaPolicy::PreserveNearest => model_alpha(request, source_x, source_y),
                AlphaPolicy::Opaque => 1.0,
            };
            let output_x = source_x * scale + dx % scale;
            let output_y = source_y * scale + dy % scale;
            let index = usize::try_from(
                u64::from(output_y) * u64::from(plan.output_dimensions.width())
                    + u64::from(output_x),
            )
            .expect("validated output dimensions");
            output[index] = [rgb[0], rgb[1], rgb[2], alpha];
        }
    }
}

fn model_alpha(request: &SuperResolutionRequest, x: u32, y: u32) -> f32 {
    if request.settings().tiff().alpha() {
        request
            .source()
            .pixel(x, y)
            .map_or(1.0, |pixel| pixel[3].clamp(0.0, 1.0))
    } else {
        1.0
    }
}

fn has_deep_shadows(source: &LinearRgbaImage) -> bool {
    let mut dark = 0u64;
    let mut total = 0u64;
    for y in (0..source.dimensions().height()).step_by(16) {
        for x in (0..source.dimensions().width()).step_by(16) {
            let pixel = source.pixel(x, y).expect("sample is inside source");
            let luma = 0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2];
            if luma < 0.005 {
                dark += 1;
            }
            total += 1;
        }
    }
    total > 0 && (dark as f64 / total as f64) >= 0.10
}

fn mirror(value: i64, limit: u32) -> u32 {
    let limit = i64::from(limit);
    if limit == 1 {
        return 0;
    }
    let period = 2 * limit - 2;
    let wrapped = value.rem_euclid(period);
    u32::try_from(if wrapped < limit {
        wrapped
    } else {
        period - wrapped
    })
    .expect("mirror is in range")
}

fn srgb_encode(value: f32) -> f32 {
    if value <= 0.0031308 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
fn srgb_decode(value: f32) -> f32 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}
fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Default)]
pub struct FileTiffPublisher;

impl SuperResolutionPublisher for FileTiffPublisher {
    fn publish(
        &mut self,
        destination: Option<&Path>,
        artifact: &[u8],
        probe: &TiffProbe,
    ) -> Result<PublicationReceipt, PublicationError> {
        let destination = destination.ok_or(PublicationError::MissingDestination)?;
        let destination =
            unique_destination(destination).map_err(|source| PublicationError::Io {
                operation: "choose TIFF destination",
                source,
            })?;
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(PublicationError::Io {
                operation: "open TIFF parent",
                source: io::Error::from(io::ErrorKind::NotFound),
            });
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = destination.with_extension(format!("rt-tmp-{}", sequence));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|source| PublicationError::Io {
                    operation: "create TIFF staging file",
                    source,
                })?;
            file.write_all(artifact)
                .map_err(|source| PublicationError::Io {
                    operation: "write TIFF staging file",
                    source,
                })?;
            file.sync_all().map_err(|source| PublicationError::Io {
                operation: "sync TIFF staging file",
                source,
            })?;
            fs::rename(&temporary, &destination).map_err(|source| PublicationError::Io {
                operation: "publish TIFF",
                source,
            })?;
            Ok(PublicationReceipt {
                destination: Some(destination.to_owned()),
                artifact_sha256: sha256(artifact),
                bytes: u64::try_from(artifact.len()).unwrap_or(u64::MAX),
                dimensions: probe.dimensions(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn unique_destination(destination: &Path) -> io::Result<PathBuf> {
    if !destination.exists() {
        return Ok(destination.to_owned());
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let stem = destination
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let extension = destination
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("tiff");
    for suffix in 1..=10_000u32 {
        let candidate = parent.join(format!("{stem}-{suffix}.{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no unique TIFF destination suffix remains",
    ))
}
