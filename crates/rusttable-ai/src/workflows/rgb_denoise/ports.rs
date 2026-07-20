#![expect(
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    reason = "these traits are explicit bounded adapter ports for workflow failures"
)]

use std::path::Path;

use super::{ModelDescriptor, ModelError, ProviderUsed, RgbDenoiseProgress, RgbDenoiseStage};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTile<'a> {
    pub width: u32,
    pub height: u32,
    pub planar_rgb: &'a [f32],
}

pub trait RgbDenoiseModel: Send + Sync {
    fn descriptor(&self) -> &ModelDescriptor;

    fn infer(&self, provider: ProviderUsed, tile: ModelTile<'_>) -> Result<Vec<f32>, ModelError>;
}

pub trait RgbDenoisePublisher: Send + Sync {
    fn publish(
        &self,
        destination: &Path,
        recipe: &super::TiffRecipe,
        collision: super::CollisionPolicy,
        profile: &super::RgbProfile,
        pixels: &[[f32; 4]],
        dimensions: rusttable_image::ImageDimensions,
        artifact_key: [u8; 32],
    ) -> Result<PublishedArtifact, PublishError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedArtifact {
    pub destination: std::path::PathBuf,
    pub encoded_bytes: u64,
}

pub trait DerivedPhotoImporter: Send {
    fn import_and_group(
        &mut self,
        source_identity: [u8; 32],
        destination: &Path,
        group_with_source: bool,
    ) -> Result<ImportOutcome, ImportError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    AlreadyPresent,
}

pub trait RgbDenoiseObserver: Send + Sync {
    fn progress(&self, progress: RgbDenoiseProgress);
}

pub trait RgbDenoiseControl: Send + Sync {
    fn is_cancelled(&self, stage: RgbDenoiseStage) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopObserver;

impl RgbDenoiseObserver for NoopObserver {
    fn progress(&self, _progress: RgbDenoiseProgress) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopControl;

impl RgbDenoiseControl for NoopControl {
    fn is_cancelled(&self, _stage: RgbDenoiseStage) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    InvalidDestination,
    DestinationExists,
    AllocationLimit { limit: u64 },
    Encode(String),
    Io(String),
    Probe(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    Failed(String),
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "TIFF publication failed: {self:?}")
    }
}

impl std::error::Error for PublishError {}

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "derived photo import failed: {self:?}")
    }
}

impl std::error::Error for ImportError {}
