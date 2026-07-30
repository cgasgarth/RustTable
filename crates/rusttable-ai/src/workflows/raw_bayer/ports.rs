use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;

use super::types::{CfaBayerU16, RawBayerDenoiseRequest, RawBayerReceipt, RawBayerTile};
use crate::{CancellationToken, Provider, ProviderPolicy, TileCrop};
use rusttable_image::{CfaColor, CfaPattern};
use rusttable_image_io::dng_output::{
    DngCfaColor, DngCfaDescriptor, DngCfaPattern, DngCollisionPolicy, DngError, DngLimits,
    DngOutput, DngOutputRequest, DngRawLayout,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawBayerModelDescriptor {
    pub identity: [u8; 32],
    pub task: crate::ModelTask,
    pub tile_width: u32,
    pub tile_height: u32,
    pub overlap: u32,
    pub valid_crop: TileCrop,
    pub minimum_width: u32,
    pub minimum_height: u32,
    pub scale: f32,
    pub offset: f32,
    pub domain_min: f32,
    pub domain_max: f32,
    pub white_balanced_input: bool,
    pub providers: &'static [Provider],
    pub qualified_providers: &'static [Provider],
    pub estimated_session_bytes: u64,
}

pub struct RawBayerTileInput<'a> {
    tile: RawBayerTile,
    tensor: &'a [f32],
}

impl<'a> RawBayerTileInput<'a> {
    pub(crate) const fn new(tile: RawBayerTile, tensor: &'a [f32]) -> Self {
        Self { tile, tensor }
    }
    #[must_use]
    pub const fn tile(&self) -> &RawBayerTile {
        &self.tile
    }
    #[must_use]
    pub fn tensor(&self) -> &[f32] {
        self.tensor
    }
}

pub trait RawBayerDenoiseModel: Send + Sync {
    fn descriptor(&self) -> &RawBayerModelDescriptor;
    fn infer(
        &self,
        provider: Provider,
        input: &RawBayerTileInput<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<f32>, RawBayerModelError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawBayerModelError {
    Unavailable,
    Execution,
    Cancelled,
    InvalidOutput,
}

pub trait RawBayerDngPublisher {
    fn publish(
        &mut self,
        request: &RawBayerDenoiseRequest,
        output: &CfaBayerU16,
        cancellation: &CancellationToken,
    ) -> Result<PublishedCfaBayer, RawBayerPublishError>;
    fn probe(&self, artifact: &PublishedCfaBayer) -> Result<CfaBayerU16, RawBayerPublishError>;
    fn discard(&mut self, artifact: &PublishedCfaBayer);
}

/// The production #498 publisher. It delegates the file boundary to the
/// checked, deterministic pure-Rust #497 writer and probes the written bytes
/// before the workflow can import them.
#[derive(Debug)]
pub struct FileCfaBayerDngPublisher {
    limits: DngLimits,
    last_output: Mutex<Option<(PathBuf, CfaBayerU16)>>,
}

impl FileCfaBayerDngPublisher {
    #[must_use]
    pub const fn with_limits(limits: DngLimits) -> Self {
        Self {
            limits,
            last_output: Mutex::new(None),
        }
    }
}

impl Default for FileCfaBayerDngPublisher {
    fn default() -> Self {
        Self::with_limits(DngLimits::default())
    }
}

impl RawBayerDngPublisher for FileCfaBayerDngPublisher {
    fn publish(
        &mut self,
        request: &RawBayerDenoiseRequest,
        output: &CfaBayerU16,
        cancellation: &CancellationToken,
    ) -> Result<PublishedCfaBayer, RawBayerPublishError> {
        let descriptor = descriptor_for(request, output)?;
        let mut dng_request = DngOutputRequest::new(
            request.output().destination().to_owned(),
            DngRawLayout::CfaBayerU16(descriptor),
        )
        .map_err(RawBayerPublishError::Dng)?;
        dng_request.limits = self.limits;
        dng_request.collision = match request.output().collision() {
            super::types::CollisionPolicy::Fail => DngCollisionPolicy::Fail,
            super::types::CollisionPolicy::UniqueSuffix => DngCollisionPolicy::Suffix,
        };
        let published = DngOutput::publish(&dng_request, || cancellation.is_cancelled())
            .map_err(RawBayerPublishError::Dng)?;
        self.last_output
            .lock()
            .map_err(|_| {
                RawBayerPublishError::Dng(DngError::Probe("publisher lock poisoned".to_owned()))
            })?
            .replace((published.destination.clone(), output.clone()));
        Ok(PublishedCfaBayer {
            destination: published.destination.to_string_lossy().into_owned(),
            artifact_identity: published.receipt.artifact_identity,
        })
    }

    fn probe(&self, artifact: &PublishedCfaBayer) -> Result<CfaBayerU16, RawBayerPublishError> {
        let path = PathBuf::from(&artifact.destination);
        let probe = DngOutput::probe(&path, self.limits.max_encoded_bytes)
            .map_err(RawBayerPublishError::Dng)?;
        if probe.artifact_identity != artifact.artifact_identity {
            return Err(RawBayerPublishError::Dng(DngError::RoundTripMismatch));
        }
        let guard = self.last_output.lock().map_err(|_| {
            RawBayerPublishError::Dng(DngError::Probe("publisher lock poisoned".to_owned()))
        })?;
        let Some((expected_path, output)) = guard.as_ref() else {
            return Err(RawBayerPublishError::Dng(DngError::RoundTripMismatch));
        };
        if expected_path != &path || compact_samples(output) != probe.samples {
            return Err(RawBayerPublishError::Dng(DngError::RoundTripMismatch));
        }
        Ok(output.clone())
    }

    fn discard(&mut self, artifact: &PublishedCfaBayer) {
        let path = PathBuf::from(&artifact.destination);
        let _ = DngOutput::discard(&path);
        if let Ok(mut guard) = self.last_output.lock()
            && guard
                .as_ref()
                .is_some_and(|(expected, _)| expected == &path)
        {
            *guard = None;
        }
    }
}

fn descriptor_for(
    request: &RawBayerDenoiseRequest,
    output: &CfaBayerU16,
) -> Result<DngCfaDescriptor, RawBayerPublishError> {
    let CfaPattern::Bayer(colors) = output.pattern() else {
        return Err(RawBayerPublishError::Dng(DngError::InvalidCfa));
    };
    let mut converted = [[DngCfaColor::Red; 2]; 2];
    for (target_row, source_row) in converted.iter_mut().zip(colors) {
        for (target, source) in target_row.iter_mut().zip(source_row) {
            *target = match source {
                CfaColor::Red => DngCfaColor::Red,
                CfaColor::Green => DngCfaColor::Green,
                CfaColor::Blue => DngCfaColor::Blue,
                CfaColor::Clear => {
                    return Err(RawBayerPublishError::Dng(DngError::InvalidCfa));
                }
            };
        }
    }
    DngCfaDescriptor::new(
        output.dimensions(),
        output.row_stride_samples(),
        output.samples().to_vec(),
        DngCfaPattern::new(
            converted,
            (output.phase().x() as u8, output.phase().y() as u8),
        ),
        output.orientation(),
        output.active_area(),
        output.default_crop(),
        output.masked_areas().to_vec(),
        output.calibration().black(),
        output.calibration().white(),
        output.calibration().white_balance(),
        output.calibration().camera_to_xyz(),
        request.output().camera_identity(),
        output.source_identity(),
        output.output_identity(),
    )
    .map_err(RawBayerPublishError::Dng)
}

fn compact_samples(output: &CfaBayerU16) -> Vec<u16> {
    let width = usize::try_from(output.dimensions().width()).unwrap_or(0);
    output
        .samples()
        .chunks(output.row_stride_samples())
        .flat_map(|row| row[..width].iter().copied())
        .collect()
}

/// Typed boundary for #497. It deliberately blocks until the Rust DNG writer is installed.
#[derive(Debug, Default)]
pub struct BlockingCfaBayerDngPublisher;

impl RawBayerDngPublisher for BlockingCfaBayerDngPublisher {
    fn publish(
        &mut self,
        _request: &RawBayerDenoiseRequest,
        _output: &CfaBayerU16,
        _cancellation: &CancellationToken,
    ) -> Result<PublishedCfaBayer, RawBayerPublishError> {
        Err(RawBayerPublishError::DngWriterUnavailable)
    }
    fn probe(&self, _artifact: &PublishedCfaBayer) -> Result<CfaBayerU16, RawBayerPublishError> {
        Err(RawBayerPublishError::DngWriterUnavailable)
    }
    fn discard(&mut self, _artifact: &PublishedCfaBayer) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedCfaBayer {
    pub destination: String,
    pub artifact_identity: [u8; 32],
}

pub trait RawBayerCatalogPort {
    fn reconcile(
        &mut self,
        request_identity: [u8; 32],
    ) -> Result<Option<RawBayerReceipt>, RawBayerCatalogError>;
    fn import_and_group(
        &mut self,
        request: &RawBayerDenoiseRequest,
        artifact: &PublishedCfaBayer,
    ) -> Result<ImportGroupingOutcome, RawBayerCatalogError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportGroupingOutcome {
    Imported { grouped: bool },
    AlreadyPresent { grouped: bool },
}

pub trait RawBayerObserver {
    fn progress(&self, progress: RawBayerProgress);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawBayerProgress {
    pub stage: RawBayerStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawBayerStage {
    Validate,
    Pack,
    Inference,
    Blend,
    Inverse,
    Publish,
    Probe,
    Import,
    Group,
}

pub trait RawBayerControl {
    fn is_cancelled(&self, stage: RawBayerStage) -> bool;
}

#[derive(Debug, Default)]
pub struct NoopRawBayerObserver;
impl RawBayerObserver for NoopRawBayerObserver {
    fn progress(&self, _progress: RawBayerProgress) {}
}
#[derive(Debug, Default)]
pub struct NoopRawBayerControl;
impl RawBayerControl for NoopRawBayerControl {
    fn is_cancelled(&self, _stage: RawBayerStage) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBayerPublishError {
    DngWriterUnavailable,
    Destination,
    Probe,
    Cancelled,
    Dng(DngError),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawBayerCatalogError {
    Unavailable,
    RevisionConflict,
    Pending,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawBayerWorkflowError {
    Request(super::types::RequestError),
    Frame(super::types::RawFrameError),
    Plan(super::planning::RawBayerPlanError),
    Model(RawBayerModelError),
    Publish(RawBayerPublishError),
    Catalog(RawBayerCatalogError),
    Cancelled(RawBayerStage),
    NonFiniteOutput,
    QuantizationRange,
    RoundTripMismatch,
}

impl fmt::Display for RawBayerWorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bayer RAW denoise workflow failed: {self:?}")
    }
}
impl std::error::Error for RawBayerWorkflowError {}
impl fmt::Display for RawBayerPublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CFA DNG publication failed: {self:?}")
    }
}
impl std::error::Error for RawBayerPublishError {}
impl fmt::Display for RawBayerCatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "derived RAW catalog operation failed: {self:?}")
    }
}
impl std::error::Error for RawBayerCatalogError {}
impl fmt::Display for RawBayerModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "qualified Bayer model failed: {self:?}")
    }
}
impl std::error::Error for RawBayerModelError {}

pub(crate) fn selected_provider(
    policy: ProviderPolicy,
    descriptor: &RawBayerModelDescriptor,
) -> Result<Provider, super::planning::RawBayerPlanError> {
    let provider = match policy {
        ProviderPolicy::Cpu => Provider::Cpu,
        ProviderPolicy::Explicit(value) => value,
        ProviderPolicy::Auto => descriptor
            .qualified_providers
            .iter()
            .copied()
            .find(|value| *value != Provider::Cpu)
            .unwrap_or(Provider::Cpu),
    };
    if descriptor.providers.contains(&provider)
        && descriptor.qualified_providers.contains(&provider)
    {
        Ok(provider)
    } else {
        Err(super::planning::RawBayerPlanError::ProviderUnqualified)
    }
}
