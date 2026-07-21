use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{CancellationToken, SuperResolutionScale};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTask {
    RawBayerDenoise,
    RawLinearDenoise,
    RgbDenoise,
    SuperResolution2x,
    SuperResolution4x,
}

impl ModelTask {
    #[must_use]
    pub const fn scale_factor(self) -> u32 {
        match self {
            Self::RawBayerDenoise | Self::RawLinearDenoise | Self::RgbDenoise => 1,
            Self::SuperResolution2x => 2,
            Self::SuperResolution4x => 4,
        }
    }

    #[must_use]
    pub const fn super_resolution_scale(self) -> Option<SuperResolutionScale> {
        match self {
            Self::SuperResolution2x => Some(SuperResolutionScale::X2),
            Self::SuperResolution4x => Some(SuperResolutionScale::X4),
            Self::RawBayerDenoise | Self::RawLinearDenoise | Self::RgbDenoise => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Cpu,
    CoreMl,
    DirectMl,
    Cuda,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderPolicy {
    Cpu,
    Auto,
    Explicit(Provider),
}

impl ProviderPolicy {
    #[must_use]
    pub const fn selected(self) -> Provider {
        match self {
            Self::Cpu | Self::Auto => Provider::Cpu,
            Self::Explicit(provider) => provider,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaPolicy {
    PreserveNearest,
    Opaque,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgePadding {
    Mirror,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorLayout {
    PlanarNchwRgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferFunction {
    ExtendedSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelColorContract {
    pub input_transfer: TransferFunction,
    pub output_transfer: TransferFunction,
    pub layout: TensorLayout,
}

impl Default for ModelColorContract {
    fn default() -> Self {
        Self {
            input_transfer: TransferFunction::ExtendedSrgb,
            output_transfer: TransferFunction::ExtendedSrgb,
            layout: TensorLayout::PlanarNchwRgb,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCrop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl TileCrop {
    #[must_use]
    pub const fn all(value: u32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelTileContract {
    pub width: u32,
    pub height: u32,
    pub overlap: u32,
    pub valid_crop: TileCrop,
    pub scale: u32,
    pub edge_padding: EdgePadding,
}

impl ModelTileContract {
    pub const fn new(width: u32, height: u32, overlap: u32, scale: u32) -> Self {
        Self {
            width,
            height,
            overlap,
            valid_crop: TileCrop::all(overlap),
            scale,
            edge_padding: EdgePadding::Mirror,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuxiliaryTensor {
    Constant {
        name: &'static str,
        channels: u32,
        value: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelManifest {
    id: String,
    version: String,
    task: ModelTask,
    color: ModelColorContract,
    tile: ModelTileContract,
    alpha: AlphaPolicy,
    shadow_boost: bool,
    auxiliary: Vec<AuxiliaryTensor>,
    providers: Vec<Provider>,
    model_bytes: Vec<u8>,
}

impl ModelManifest {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        task: ModelTask,
        tile: ModelTileContract,
        model_bytes: Vec<u8>,
    ) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            task,
            color: ModelColorContract::default(),
            tile,
            alpha: AlphaPolicy::PreserveNearest,
            shadow_boost: false,
            auxiliary: Vec::new(),
            providers: vec![Provider::Cpu],
            model_bytes,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn task(&self) -> ModelTask {
        self.task
    }

    #[must_use]
    pub const fn color(&self) -> ModelColorContract {
        self.color
    }

    #[must_use]
    pub const fn tile(&self) -> ModelTileContract {
        self.tile
    }

    #[must_use]
    pub const fn alpha(&self) -> AlphaPolicy {
        self.alpha
    }

    #[must_use]
    pub const fn shadow_boost(&self) -> bool {
        self.shadow_boost
    }

    #[must_use]
    pub fn auxiliary(&self) -> &[AuxiliaryTensor] {
        &self.auxiliary
    }

    #[must_use]
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    #[must_use]
    pub fn identity(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable-model-v1\0");
        for part in [&self.id, &self.version] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        hasher.update(
            format!(
                "{:?}|{:?}|{:?}|{:?}|{}|{:?}",
                self.task, self.color, self.tile, self.alpha, self.shadow_boost, self.providers
            )
            .as_bytes(),
        );
        hasher.update(&self.model_bytes);
        hex_digest(hasher.finalize())
    }

    #[must_use]
    pub fn with_color_contract(mut self, color: ModelColorContract) -> Self {
        self.color = color;
        self
    }

    #[must_use]
    pub fn with_alpha_policy(mut self, alpha: AlphaPolicy) -> Self {
        self.alpha = alpha;
        self
    }

    #[must_use]
    pub fn with_shadow_boost(mut self, enabled: bool) -> Self {
        self.shadow_boost = enabled;
        self
    }

    #[must_use]
    pub fn with_auxiliary(mut self, auxiliary: Vec<AuxiliaryTensor>) -> Self {
        self.auxiliary = auxiliary;
        self
    }

    #[must_use]
    pub fn with_providers(mut self, providers: Vec<Provider>) -> Self {
        self.providers = providers;
        self
    }

    pub fn validate_for(
        &self,
        scale: SuperResolutionScale,
        provider: Provider,
    ) -> Result<(), ModelValidationError> {
        if self.task.super_resolution_scale() != Some(scale) {
            return Err(ModelValidationError::TaskScaleMismatch);
        }
        if self.tile.scale != scale.integer() {
            return Err(ModelValidationError::TileScaleMismatch);
        }
        if self.color != ModelColorContract::default() {
            return Err(ModelValidationError::UnsupportedColorContract);
        }
        if self.tile.width == 0 || self.tile.height == 0 {
            return Err(ModelValidationError::InvalidTile);
        }
        let crop = self.tile.valid_crop;
        if self.tile.width <= crop.left.saturating_add(crop.right)
            || self.tile.height <= crop.top.saturating_add(crop.bottom)
        {
            return Err(ModelValidationError::InvalidCrop);
        }
        if !self.providers.contains(&provider) {
            return Err(ModelValidationError::ProviderUnavailable);
        }
        if self.auxiliary.iter().any(|tensor| match tensor {
            AuxiliaryTensor::Constant {
                channels, value, ..
            } => *channels == 0 || !value.is_finite(),
        }) {
            return Err(ModelValidationError::InvalidAuxiliary);
        }
        Ok(())
    }
}

pub trait SuperResolutionModel {
    fn manifest(&self) -> &ModelManifest;
    fn infer(
        &mut self,
        input: &crate::workflow::ModelInput,
        cancel: &CancellationToken,
    ) -> Result<crate::workflow::ModelOutput, ModelInferenceError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelValidationError {
    TaskScaleMismatch,
    TileScaleMismatch,
    UnsupportedColorContract,
    InvalidTile,
    InvalidCrop,
    ProviderUnavailable,
    InvalidAuxiliary,
}

impl fmt::Display for ModelValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "model contract validation failed: {self:?}")
    }
}

impl std::error::Error for ModelValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInferenceError {
    pub code: &'static str,
}

impl fmt::Display for ModelInferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}
impl std::error::Error for ModelInferenceError {}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
