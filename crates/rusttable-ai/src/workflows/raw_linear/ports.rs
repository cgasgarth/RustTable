use std::fmt;

use super::types::{LinearRawRgbU16, RawLinearDenoiseRequest, RawLinearReceipt, RawLinearTile};
use crate::{CancellationToken, Provider, ProviderPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLinearModelDescriptor {
    pub identity: [u8; 32],
    pub task: crate::ModelTask,
    pub input_color: rusttable_color::ColorEncoding,
    pub output_color: rusttable_color::ColorEncoding,
    pub layout: crate::TensorLayout,
    pub full_range: bool,
    pub finite_only: bool,
    pub tile_width: u32,
    pub tile_height: u32,
    pub overlap: u32,
    pub valid_crop: crate::TileCrop,
    pub minimum_width: u32,
    pub minimum_height: u32,
    pub providers: &'static [Provider],
    pub qualified_providers: &'static [Provider],
    pub estimated_session_bytes: u64,
}

pub struct RawLinearTileInput<'a> {
    tile: RawLinearTile,
    width: u32,
    height: u32,
    nchw_rgb: &'a [f32],
}

impl<'a> RawLinearTileInput<'a> {
    pub(crate) const fn new(
        tile: RawLinearTile,
        width: u32,
        height: u32,
        nchw_rgb: &'a [f32],
    ) -> Self {
        Self {
            tile,
            width,
            height,
            nchw_rgb,
        }
    }
    #[must_use]
    pub const fn tile(&self) -> RawLinearTile {
        self.tile
    }
    #[must_use]
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    #[must_use]
    pub fn nchw_rgb(&self) -> &[f32] {
        self.nchw_rgb
    }
}

pub trait RawLinearDenoiseModel: Send + Sync {
    fn descriptor(&self) -> &RawLinearModelDescriptor;
    fn infer(
        &self,
        provider: Provider,
        input: &RawLinearTileInput<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<f32>, RawLinearModelError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLinearModelError {
    Unavailable,
    Execution,
    Cancelled,
    InvalidOutput,
}

pub trait RawLinearDngPublisher {
    fn publish(
        &mut self,
        request: &RawLinearDenoiseRequest,
        output: &LinearRawRgbU16,
        cancellation: &CancellationToken,
    ) -> Result<PublishedLinearRaw, RawLinearPublishError>;
    fn probe(
        &self,
        artifact: &PublishedLinearRaw,
    ) -> Result<LinearRawRgbU16, RawLinearPublishError>;
    fn discard(&mut self, artifact: &PublishedLinearRaw);
}

/// Explicit adapter used until `rusttable-image-io::dng_output` (#497) is available.
#[derive(Debug, Default)]
pub struct BlockingLinearRawDngPublisher;

impl RawLinearDngPublisher for BlockingLinearRawDngPublisher {
    fn publish(
        &mut self,
        _request: &RawLinearDenoiseRequest,
        _output: &LinearRawRgbU16,
        _cancellation: &CancellationToken,
    ) -> Result<PublishedLinearRaw, RawLinearPublishError> {
        Err(RawLinearPublishError::DngWriterUnavailable)
    }

    fn probe(
        &self,
        _artifact: &PublishedLinearRaw,
    ) -> Result<LinearRawRgbU16, RawLinearPublishError> {
        Err(RawLinearPublishError::DngWriterUnavailable)
    }

    fn discard(&mut self, _artifact: &PublishedLinearRaw) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedLinearRaw {
    pub destination: String,
    pub artifact_identity: [u8; 32],
}

pub trait RawLinearCatalogPort {
    fn reconcile(
        &mut self,
        request_identity: [u8; 32],
    ) -> Result<Option<RawLinearReceipt>, RawLinearCatalogError>;
    fn import_and_group(
        &mut self,
        request: &RawLinearDenoiseRequest,
        artifact: &PublishedLinearRaw,
    ) -> Result<ImportGroupingOutcome, RawLinearCatalogError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportGroupingOutcome {
    Imported { grouped: bool },
    AlreadyPresent { grouped: bool },
}

pub trait RawLinearObserver {
    fn progress(&self, progress: RawLinearProgress);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLinearProgress {
    pub stage: RawLinearStage,
    pub completed: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLinearStage {
    Validate,
    Prepare,
    Color,
    Inference,
    Quantize,
    Publish,
    Probe,
    Import,
    Group,
}

pub trait RawLinearControl {
    fn is_cancelled(&self, stage: RawLinearStage) -> bool;
}

#[derive(Debug, Default)]
pub struct NoopRawLinearObserver;
impl RawLinearObserver for NoopRawLinearObserver {
    fn progress(&self, _progress: RawLinearProgress) {}
}

#[derive(Debug, Default)]
pub struct NoopRawLinearControl;
impl RawLinearControl for NoopRawLinearControl {
    fn is_cancelled(&self, _stage: RawLinearStage) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLinearPublishError {
    DngWriterUnavailable,
    Destination,
    Probe,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLinearCatalogError {
    Unavailable,
    RevisionConflict,
    Pending,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawLinearWorkflowError {
    Request(super::types::RequestError),
    Plan(super::planning::RawLinearPlanError),
    Model(RawLinearModelError),
    Publish(RawLinearPublishError),
    Catalog(RawLinearCatalogError),
    Cancelled(RawLinearStage),
    NonFiniteOutput,
    QuantizationRange,
    RoundTripMismatch,
}

impl fmt::Display for RawLinearWorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "RAW-linear denoise workflow failed: {self:?}")
    }
}
impl std::error::Error for RawLinearWorkflowError {}

impl fmt::Display for RawLinearPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LinearRaw DNG publication failed: {self:?}")
    }
}
impl std::error::Error for RawLinearPublishError {}

impl fmt::Display for RawLinearCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "derived RAW catalog operation failed: {self:?}")
    }
}
impl std::error::Error for RawLinearCatalogError {}

impl fmt::Display for RawLinearModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "qualified RAW-linear model failed: {self:?}")
    }
}
impl std::error::Error for RawLinearModelError {}

pub(crate) fn selected_provider(
    policy: ProviderPolicy,
    descriptor: &RawLinearModelDescriptor,
) -> Result<Provider, RawLinearWorkflowError> {
    let provider = match policy {
        ProviderPolicy::Cpu => Provider::Cpu,
        ProviderPolicy::Explicit(provider) => provider,
        ProviderPolicy::Auto => descriptor
            .qualified_providers
            .iter()
            .copied()
            .find(|provider| *provider != Provider::Cpu)
            .unwrap_or(Provider::Cpu),
    };
    if !descriptor.providers.contains(&provider)
        || !descriptor.qualified_providers.contains(&provider)
    {
        return Err(RawLinearWorkflowError::Plan(
            super::planning::RawLinearPlanError::ProviderUnqualified,
        ));
    }
    Ok(provider)
}
