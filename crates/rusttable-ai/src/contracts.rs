use std::collections::BTreeSet;
use std::fmt;

use rusttable_ai_native::{Dimension, GraphMetadata, TensorDataType, TensorDescriptor};
use serde::{Deserialize, Serialize};

use crate::{AlphaPolicy, EdgePadding, ModelTask, Provider, TensorLayout, TransferFunction};

pub const MODEL_PACKAGE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryManifest {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub task: ModelTask,
    pub onnx: OnnxContract,
    pub tensors: TensorContract,
    pub color: ColorContract,
    pub tile: TileContract,
    pub providers: BTreeSet<Provider>,
    pub resources: ResourceContract,
    #[serde(default)]
    pub data_assets: Vec<DataAsset>,
    pub hashes: ManifestHashes,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OnnxContract {
    pub ir_version: u32,
    pub opset: u32,
    pub required_operators: BTreeSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TensorContract {
    pub input: TensorSpec,
    pub output: TensorSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TensorSpec {
    pub name: String,
    pub dtype: TensorDtype,
    pub dimensions: Vec<DimensionSpec>,
    pub layout: TensorLayout,
    pub channels: u32,
    pub channel_meaning: String,
    pub batch: BatchPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DimensionSpec {
    pub static_size: Option<u32>,
    pub dynamic_axis: Option<String>,
}

impl DimensionSpec {
    pub fn static_size(value: u32) -> Result<Self, ContractError> {
        if value == 0 {
            return Err(ContractError::ZeroDimension);
        }
        Ok(Self {
            static_size: Some(value),
            dynamic_axis: None,
        })
    }

    #[must_use]
    pub fn dynamic(name: impl Into<String>) -> Self {
        Self {
            static_size: None,
            dynamic_axis: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TensorDtype {
    F32,
    F16,
    U8,
    I8,
    I16,
    I32,
    I64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchPolicy {
    FixedOne,
    DynamicOneOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ColorContract {
    pub source_primaries: ColorPrimaries,
    pub target_primaries: ColorPrimaries,
    pub input_transfer: TransferFunction,
    pub output_transfer: TransferFunction,
    pub range: ColorRange,
    pub alpha: AlphaPolicy,
    pub normalization_scale: f32,
    pub normalization_offset: f32,
    pub finite_only: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorPrimaries {
    Srgb,
    DisplayP3,
    Rec2020,
    CameraLinear,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorRange {
    Full,
    Limited,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TileContract {
    pub width: u32,
    pub height: u32,
    pub overlap: u32,
    pub alignment: u32,
    pub valid_crop: TileCropContract,
    pub scale: u32,
    pub edge_padding: EdgePadding,
    pub blend_window: BlendWindow,
    pub minimum_width: u32,
    pub minimum_height: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TileCropContract {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlendWindow {
    Linear,
    Uniform,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceContract {
    pub estimated_model_bytes: u64,
    pub estimated_session_bytes: u64,
    pub estimated_tile_bytes: u64,
    pub max_concurrency: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataAsset {
    pub name: String,
    pub sha256: String,
    pub max_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestHashes {
    pub model_sha256: String,
    #[serde(default)]
    pub data_sha256: String,
}

impl RegistryManifest {
    pub fn from_toml(text: &str) -> Result<Self, ContractError> {
        let manifest: Self = toml::from_str(text).map_err(|_| ContractError::MalformedManifest)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema != MODEL_PACKAGE_SCHEMA {
            return Err(ContractError::UnsupportedSchema(self.schema));
        }
        validate_identifier(&self.id, 64).map_err(|()| ContractError::InvalidIdentifier)?;
        if self.version.is_empty() || self.version.len() > 64 {
            return Err(ContractError::InvalidVersion);
        }
        if self.onnx.ir_version == 0
            || self.onnx.opset == 0
            || self.onnx.required_operators.is_empty()
        {
            return Err(ContractError::InvalidOnnxContract);
        }
        if self
            .onnx
            .required_operators
            .iter()
            .any(|operator| operator.is_empty() || operator.contains("::"))
        {
            return Err(ContractError::UnsupportedOperator);
        }
        validate_tensor(&self.tensors.input)?;
        validate_tensor(&self.tensors.output)?;
        if self.tensors.input.name == self.tensors.output.name {
            return Err(ContractError::DuplicateTensorName);
        }
        if !self.color.normalization_scale.is_finite()
            || !self.color.normalization_offset.is_finite()
            || self.color.normalization_scale == 0.0
        {
            return Err(ContractError::NonFiniteColorContract);
        }
        validate_tile(&self.tile, self.task.scale_factor())?;
        if self.providers.is_empty() || !self.providers.contains(&Provider::Cpu) {
            return Err(ContractError::CpuProviderRequired);
        }
        if self.providers.iter().any(|provider| {
            !matches!(
                provider,
                Provider::Cpu | Provider::CoreMl | Provider::DirectMl | Provider::Cuda
            )
        }) {
            return Err(ContractError::UnsupportedProvider);
        }
        if self.resources.estimated_model_bytes == 0
            || self.resources.estimated_session_bytes == 0
            || self.resources.estimated_tile_bytes == 0
            || self.resources.max_concurrency == 0
            || self.resources.max_concurrency > 64
        {
            return Err(ContractError::InvalidResourceContract);
        }
        for asset in &self.data_assets {
            validate_asset(asset)?;
        }
        if self.data_assets.is_empty() && !self.hashes.data_sha256.is_empty()
            || !self.data_assets.is_empty() && self.hashes.data_sha256.is_empty()
        {
            return Err(ContractError::InvalidHash);
        }
        validate_hash(&self.hashes.model_sha256)?;
        if !self.hashes.data_sha256.is_empty() {
            validate_hash(&self.hashes.data_sha256)?;
        }
        Ok(())
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| ContractError::Canonicalization)
    }

    pub fn validate_graph(&self, graph: &GraphMetadata) -> Result<(), ContractError> {
        if graph.ir_version() != self.onnx.ir_version || graph.opset() != self.onnx.opset {
            return Err(ContractError::GraphVersionMismatch);
        }
        if graph.has_external_data() {
            return Err(ContractError::ExternalModelData);
        }
        let actual: BTreeSet<_> = graph.operators().iter().cloned().collect();
        if !self.onnx.required_operators.is_subset(&actual)
            || actual.iter().any(|operator| operator.contains("::"))
        {
            return Err(ContractError::GraphOperatorMismatch);
        }
        validate_graph_tensor(&self.tensors.input, graph.inputs())?;
        validate_graph_tensor(&self.tensors.output, graph.outputs())?;
        Ok(())
    }
}

fn validate_identifier(value: &str, max: usize) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > max
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
        || !value.as_bytes()[0].is_ascii_lowercase() && !value.as_bytes()[0].is_ascii_digit()
    {
        return Err(());
    }
    Ok(())
}

fn validate_tensor(tensor: &TensorSpec) -> Result<(), ContractError> {
    if tensor.name.is_empty()
        || tensor.name.len() > 128
        || tensor.channels == 0
        || tensor.channel_meaning.is_empty()
        || tensor.dimensions.is_empty()
        || tensor.dimensions.len() > 5
    {
        return Err(ContractError::InvalidTensorContract);
    }
    for dimension in &tensor.dimensions {
        match (&dimension.static_size, &dimension.dynamic_axis) {
            (Some(size), None) if *size > 0 => {}
            (None, Some(name))
                if !name.is_empty()
                    && name.len() <= 32
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') => {}
            _ => return Err(ContractError::InvalidTensorDimension),
        }
    }
    Ok(())
}

fn validate_tile(tile: &TileContract, scale: u32) -> Result<(), ContractError> {
    if tile.width == 0
        || tile.height == 0
        || tile.alignment == 0
        || tile.scale != scale
        || tile.minimum_width == 0
        || tile.minimum_height == 0
    {
        return Err(ContractError::InvalidTileContract);
    }
    let horizontal = tile
        .valid_crop
        .left
        .checked_add(tile.valid_crop.right)
        .ok_or(ContractError::TileArithmetic)?;
    let vertical = tile
        .valid_crop
        .top
        .checked_add(tile.valid_crop.bottom)
        .ok_or(ContractError::TileArithmetic)?;
    if horizontal >= tile.width
        || vertical >= tile.height
        || tile.overlap >= tile.width.min(tile.height)
    {
        return Err(ContractError::InvalidTileContract);
    }
    Ok(())
}

fn validate_asset(asset: &DataAsset) -> Result<(), ContractError> {
    if asset.name.is_empty()
        || asset.name == "model.toml"
        || asset.name == "model.onnx"
        || asset.name.starts_with('/')
        || asset.name.contains("..")
        || asset.name.contains('\\')
        || asset.max_bytes == 0
    {
        return Err(ContractError::InvalidAsset);
    }
    validate_hash(&asset.sha256)
}

fn validate_hash(value: &str) -> Result<(), ContractError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ContractError::InvalidHash);
    }
    Ok(())
}

fn validate_graph_tensor(
    expected: &TensorSpec,
    actual: &[TensorDescriptor],
) -> Result<(), ContractError> {
    if actual.len() != 1 {
        return Err(ContractError::GraphTensorMismatch);
    }
    let Some(tensor) = actual.iter().find(|tensor| tensor.name() == expected.name) else {
        return Err(ContractError::GraphTensorMismatch);
    };
    if tensor.dtype() != native_dtype(expected.dtype)
        || tensor.dimensions().len() != expected.dimensions.len()
    {
        return Err(ContractError::GraphTensorMismatch);
    }
    for (expected, actual) in expected.dimensions.iter().zip(tensor.dimensions()) {
        match (expected.static_size, &expected.dynamic_axis, actual) {
            (Some(size), _, Dimension::Static(actual)) if size == *actual => {}
            (None, Some(name), Dimension::Dynamic(actual)) if name == actual => {}
            _ => return Err(ContractError::GraphTensorMismatch),
        }
    }
    Ok(())
}

const fn native_dtype(dtype: TensorDtype) -> TensorDataType {
    match dtype {
        TensorDtype::F32 => TensorDataType::F32,
        TensorDtype::F16 => TensorDataType::F16,
        TensorDtype::U8 => TensorDataType::U8,
        TensorDtype::I8 => TensorDataType::I8,
        TensorDtype::I16 => TensorDataType::I16,
        TensorDtype::I32 => TensorDataType::I32,
        TensorDtype::I64 => TensorDataType::I64,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractError {
    MalformedManifest,
    UnsupportedSchema(u32),
    InvalidIdentifier,
    InvalidVersion,
    InvalidOnnxContract,
    UnsupportedOperator,
    InvalidTensorContract,
    InvalidTensorDimension,
    DuplicateTensorName,
    NonFiniteColorContract,
    InvalidTileContract,
    TileArithmetic,
    CpuProviderRequired,
    UnsupportedProvider,
    InvalidResourceContract,
    InvalidAsset,
    InvalidHash,
    GraphVersionMismatch,
    ExternalModelData,
    GraphOperatorMismatch,
    GraphTensorMismatch,
    ZeroDimension,
    Canonicalization,
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid AI model contract: {self:?}")
    }
}

impl std::error::Error for ContractError {}
