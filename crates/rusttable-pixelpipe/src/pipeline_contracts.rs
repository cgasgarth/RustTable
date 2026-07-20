#![allow(clippy::missing_errors_doc)]

use std::{fmt, num::NonZeroU64};

use rusttable_color::{ColorEncoding, Precision};
use rusttable_image::{
    AlphaMode, ChannelLayout, ImageDimensions, Orientation, PixelFormat, Roi, SampleType,
    StorageLayout,
};
use rusttable_processing::{OperationStackSnapshot, StackStage};
use sha2::{Digest, Sha256};

pub const PIPELINE_SCHEMA_VERSION: u16 = 1;
pub const WORKING_COLOR: ColorEncoding = ColorEncoding::LinearSrgbD65;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PipelineGeneration(NonZeroU64);

impl PipelineGeneration {
    /// Creates a nonzero immutable pipeline generation.
    ///
    /// # Errors
    ///
    /// Returns [`GenerationError::Zero`] for zero.
    pub fn new(value: u64) -> Result<Self, GenerationError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(GenerationError::Zero)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationError {
    Zero,
}

impl fmt::Display for GenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("pipeline generations must be nonzero")
    }
}

impl std::error::Error for GenerationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicationGeneration(u64);

impl PublicationGeneration {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelinePurpose {
    Preview,
    Full,
    Thumbnail,
    Export,
}

impl PipelinePurpose {
    #[must_use]
    pub const fn mode(self) -> PipelineMode {
        match self {
            Self::Preview | Self::Thumbnail => PipelineMode::Interactive,
            Self::Full | Self::Export => PipelineMode::Batch,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Full => "full",
            Self::Thumbnail => "thumbnail",
            Self::Export => "export",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineMode {
    Interactive,
    Batch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineQuality {
    Draft,
    Normal,
    High,
    Maximum,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceIdentity([u8; 32]);

impl SourceIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for SourceIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    identity: SourceIdentity,
    dimensions: ImageDimensions,
    orientation: Orientation,
    bounds: Roi,
    format: PixelFormat,
    color: ColorIdentity,
}

impl SourceDescriptor {
    /// Creates a checked source descriptor and ROI boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError`] for untagged color or an invalid ROI.
    pub fn new(
        identity: SourceIdentity,
        dimensions: ImageDimensions,
        orientation: Orientation,
        bounds: Roi,
        format: PixelFormat,
        color: ColorIdentity,
    ) -> Result<Self, ContractError> {
        if !color.encoding().is_explicit() {
            return Err(ContractError::UntaggedColor);
        }
        bounds
            .within(dimensions)
            .map_err(|_| ContractError::RoiOutOfBounds)?;
        Ok(Self {
            identity,
            dimensions,
            orientation,
            bounds,
            format,
            color,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> SourceIdentity {
        self.identity
    }
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn bounds(&self) -> Roi {
        self.bounds
    }
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }
    #[must_use]
    pub const fn color(&self) -> ColorIdentity {
        self.color
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorIdentity {
    encoding: ColorEncoding,
    planner_version: u16,
}

impl ColorIdentity {
    /// Creates an explicit color identity.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::UntaggedColor`] or
    /// [`ContractError::InvalidVersion`] when the identity is invalid.
    pub fn new(encoding: ColorEncoding, planner_version: u16) -> Result<Self, ContractError> {
        if !encoding.is_explicit() {
            return Err(ContractError::UntaggedColor);
        }
        if planner_version == 0 {
            return Err(ContractError::InvalidVersion);
        }
        Ok(Self {
            encoding,
            planner_version,
        })
    }

    #[must_use]
    pub const fn working() -> Self {
        Self {
            encoding: WORKING_COLOR,
            planner_version: 1,
        }
    }

    #[must_use]
    pub const fn encoding(self) -> ColorEncoding {
        self.encoding
    }
    #[must_use]
    pub const fn planner_version(self) -> u16 {
        self.planner_version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineInput {
    dimensions: ImageDimensions,
    format: PixelFormat,
    color: ColorIdentity,
}

impl PipelineInput {
    /// Creates a checked immutable input boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::UntaggedColor`] for an unspecified color.
    pub fn new(
        dimensions: ImageDimensions,
        format: PixelFormat,
        color: ColorIdentity,
    ) -> Result<Self, ContractError> {
        if !color.encoding().is_explicit() {
            return Err(ContractError::UntaggedColor);
        }
        Ok(Self {
            dimensions,
            format,
            color,
        })
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn format(self) -> PixelFormat {
        self.format
    }
    #[must_use]
    pub const fn color(self) -> ColorIdentity {
        self.color
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Background {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

impl Background {
    /// Creates a finite normalized RGBA background.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidBackground`] for non-finite or out-of-range values.
    pub fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Result<Self, ContractError> {
        let values = [red, green, blue, alpha];
        if values.iter().any(|value| !value.is_finite())
            || values.iter().any(|value| !(0.0..=1.0).contains(value))
        {
            return Err(ContractError::InvalidBackground);
        }
        Ok(Self {
            red,
            green,
            blue,
            alpha,
        })
    }

    #[must_use]
    pub const fn transparent() -> Self {
        Self {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 0.0,
        }
    }

    #[must_use]
    pub const fn rgba(self) -> [f32; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSpec {
    dimensions: ImageDimensions,
    roi: Roi,
    format: PixelFormat,
    color: ColorIdentity,
    background: Background,
}

impl OutputSpec {
    /// Creates a checked immutable output boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError`] for untagged color or an out-of-bounds ROI.
    pub fn new(
        dimensions: ImageDimensions,
        roi: Roi,
        format: PixelFormat,
        color: ColorIdentity,
        background: Background,
    ) -> Result<Self, ContractError> {
        if !color.encoding().is_explicit() {
            return Err(ContractError::UntaggedColor);
        }
        roi.within(dimensions)
            .map_err(|_| ContractError::RoiOutOfBounds)?;
        Ok(Self {
            dimensions,
            roi,
            format,
            color,
            background,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }
    #[must_use]
    pub const fn color(&self) -> ColorIdentity {
        self.color
    }
    #[must_use]
    pub const fn background(&self) -> Background {
        self.background
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplementationIdentity {
    name: String,
    version: u16,
    build: String,
}

impl ImplementationIdentity {
    /// Creates a bounded implementation/build identity.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidIdentity`] for empty, non-ASCII, oversized,
    /// or zero-version values.
    pub fn new(
        name: impl Into<String>,
        version: u16,
        build: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let value = Self {
            name: name.into(),
            version,
            build: build.into(),
        };
        if value.name.is_empty()
            || value.name.len() > 128
            || !value.name.is_ascii()
            || value.build.is_empty()
            || value.build.len() > 128
            || !value.build.is_ascii()
            || version == 0
        {
            return Err(ContractError::InvalidIdentity);
        }
        Ok(value)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn version(&self) -> u16 {
        self.version
    }
    #[must_use]
    pub fn build(&self) -> &str {
        &self.build
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceMetadata {
    input_bytes: u64,
    output_bytes: u64,
    temporary_bytes: u64,
    alignment: u32,
}

impl ResourceMetadata {
    /// Creates checked resource metadata with power-of-two alignment.
    ///
    /// # Errors
    ///
    /// Returns [`ContractError::InvalidResourceMetadata`] for invalid alignment.
    pub fn new(
        input_bytes: u64,
        output_bytes: u64,
        temporary_bytes: u64,
        alignment: u32,
    ) -> Result<Self, ContractError> {
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(ContractError::InvalidResourceMetadata);
        }
        Ok(Self {
            input_bytes,
            output_bytes,
            temporary_bytes,
            alignment,
        })
    }

    #[must_use]
    pub const fn input_bytes(self) -> u64 {
        self.input_bytes
    }
    #[must_use]
    pub const fn output_bytes(self) -> u64 {
        self.output_bytes
    }
    #[must_use]
    pub const fn temporary_bytes(self) -> u64 {
        self.temporary_bytes
    }
    #[must_use]
    pub const fn alignment(self) -> u32 {
        self.alignment
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskStatus {
    NotReferenced,
    Referenced,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendStatus {
    NotReferenced,
    Referenced,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterStatus {
    Validated,
    Prepared,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    UntaggedColor,
    InvalidVersion,
    RoiOutOfBounds,
    InvalidBackground,
    InvalidIdentity,
    InvalidResourceMetadata,
    InvalidFormat,
    InvalidDimensions,
    Stack(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UntaggedColor => formatter.write_str("pixelpipe color encoding is untagged"),
            Self::InvalidVersion => formatter.write_str("pixelpipe version is invalid"),
            Self::RoiOutOfBounds => formatter.write_str("pixelpipe ROI is out of bounds"),
            Self::InvalidBackground => formatter.write_str("pixelpipe background is invalid"),
            Self::InvalidIdentity => {
                formatter.write_str("pixelpipe implementation identity is invalid")
            }
            Self::InvalidResourceMetadata => {
                formatter.write_str("pixelpipe resource metadata is invalid")
            }
            Self::InvalidFormat => formatter.write_str("pixelpipe format continuity is invalid"),
            Self::InvalidDimensions => formatter.write_str("pixelpipe dimensions are invalid"),
            Self::Stack(error) => write!(formatter, "pixelpipe stack is invalid: {error}"),
        }
    }
}

impl std::error::Error for ContractError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineSnapshotIdentity([u8; 32]);

impl PipelineSnapshotIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl fmt::Display for PipelineSnapshotIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotDiffComponent {
    Source,
    Stack,
    Roi,
    InputColor,
    WorkingColor,
    OutputColor,
    OutputGeometry,
    OutputFormat,
    Purpose,
    Quality,
    Precision,
    Implementation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotDiff {
    components: Vec<SnapshotDiffComponent>,
}

impl SnapshotDiff {
    pub(crate) fn new(components: Vec<SnapshotDiffComponent>) -> Self {
        Self { components }
    }

    #[must_use]
    pub fn components(&self) -> &[SnapshotDiffComponent] {
        &self.components
    }

    pub(crate) fn push(&mut self, component: SnapshotDiffComponent) {
        self.components.push(component);
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

pub(crate) fn color_bytes(color: ColorIdentity, output: &mut Vec<u8>) {
    output.extend_from_slice(
        &postcard::to_allocvec(&color.encoding()).expect("color encoding is serializable"),
    );
    output.extend_from_slice(&color.planner_version().to_le_bytes());
}

pub(crate) fn format_bytes(format: PixelFormat, output: &mut Vec<u8>) {
    output.push(sample_tag(format.sample_type()));
    output.push(channel_tag(format.channels()));
    output.push(alpha_tag(format.alpha()));
    output.push(byte_order_tag(format.byte_order()));
    output.push(storage_tag(format.storage()));
}

pub(crate) fn write_stack(stack: &OperationStackSnapshot, output: &mut Vec<u8>) {
    write_text(stack.template().name(), output);
    output.extend_from_slice(&stack.revision().to_le_bytes());
    for operation in stack.operations() {
        output.extend_from_slice(&operation.id().to_le_bytes());
        write_text(&operation.descriptor().compatibility_name, output);
        write_text(&operation.descriptor().rust_id, output);
        output.extend_from_slice(&operation.descriptor().schema_version.to_le_bytes());
        output.extend_from_slice(&operation.descriptor().parameter_version.to_le_bytes());
        output.extend_from_slice(&operation.descriptor().implementation_version.to_le_bytes());
        output.extend_from_slice(&(operation.parameters().len() as u64).to_le_bytes());
        output.extend_from_slice(operation.parameters());
        output.push(u8::from(operation.enabled()));
        output.extend_from_slice(&operation.opacity_basis_points().to_le_bytes());
        output.push(stage_tag(operation.stage()));
        output.push(u8::from(operation.mandatory()));
        output.push(u8::from(operation.multi_instance()));
        output.extend_from_slice(&operation.mask_id().unwrap_or_default().to_le_bytes());
        output.extend_from_slice(&operation.blend_id().unwrap_or_default().to_le_bytes());
    }
}

fn write_text(value: &str, output: &mut Vec<u8>) {
    output.extend_from_slice(&(value.len() as u64).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

const fn sample_tag(value: SampleType) -> u8 {
    match value {
        SampleType::U8 => 0,
        SampleType::U16 => 1,
        SampleType::F16 => 2,
        SampleType::F32 => 3,
    }
}

const fn channel_tag(value: ChannelLayout) -> u8 {
    match value {
        ChannelLayout::Gray => 0,
        ChannelLayout::GrayA => 1,
        ChannelLayout::Rgb => 2,
        ChannelLayout::Rgba => 3,
        ChannelLayout::Bayer => 4,
        ChannelLayout::XTrans => 5,
    }
}

const fn alpha_tag(value: AlphaMode) -> u8 {
    match value {
        AlphaMode::None => 0,
        AlphaMode::Straight => 1,
        AlphaMode::Premultiplied => 2,
    }
}

const fn byte_order_tag(value: rusttable_image::ByteOrder) -> u8 {
    match value {
        rusttable_image::ByteOrder::Native => 0,
        rusttable_image::ByteOrder::Little => 1,
        rusttable_image::ByteOrder::Big => 2,
    }
}

const fn storage_tag(value: StorageLayout) -> u8 {
    match value {
        StorageLayout::Interleaved => 0,
        StorageLayout::Planar => 1,
    }
}

const fn stage_tag(value: StackStage) -> u8 {
    match value {
        StackStage::InputPreparation => 0,
        StackStage::SensorPreparation => 1,
        StackStage::DemosaicAndInputColor => 2,
        StackStage::SceneLinear => 3,
        StackStage::CreativeAndTone => 4,
        StackStage::Geometry => 5,
        StackStage::OutputPreparation => 6,
        StackStage::Diagnostics => 7,
    }
}

pub(crate) fn precision_tag(value: Precision) -> u8 {
    match value {
        Precision::F32 => 0,
        Precision::F64 => 1,
        Precision::U16 => 2,
        Precision::U32 => 3,
    }
}
