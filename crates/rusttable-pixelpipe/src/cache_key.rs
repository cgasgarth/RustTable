#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::hash::{Hash, Hasher};

use rusttable_color::Precision;
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDimensions, Orientation, PixelFormat, Roi,
    SampleType, StorageLayout,
};
use sha2::{Digest, Sha256};

use crate::{
    BlendStatus, ColorIdentity, ImplementationIdentity, MaskStatus, ModePlan, PipelineGeneration,
    PipelinePurpose, PipelineQuality, PipelineSnapshot, PipelineSnapshotIdentity, SourceIdentity,
};

/// Version of the structured in-memory cache identity.
pub const CACHE_KEY_SCHEMA_VERSION: u16 = 2;

/// The precision identity used by a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CachePrecision {
    F16,
    F32,
    F64,
    U16,
    U32,
}

impl From<Precision> for CachePrecision {
    fn from(value: Precision) -> Self {
        match value {
            Precision::F32 => Self::F32,
            Precision::F64 => Self::F64,
            Precision::U16 => Self::U16,
            Precision::U32 => Self::U32,
        }
    }
}

/// Quality is explicit because preview quality changes output even when the
/// source and edit snapshot do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheQuality {
    Draft,
    Normal,
    High,
    Maximum,
}

impl From<PipelineQuality> for CacheQuality {
    fn from(value: PipelineQuality) -> Self {
        match value {
            PipelineQuality::Draft => Self::Draft,
            PipelineQuality::Normal => Self::Normal,
            PipelineQuality::High => Self::High,
            PipelineQuality::Maximum => Self::Maximum,
        }
    }
}

/// A prepared node boundary and range. The optional boundary allows a whole
/// pipeline result while retaining the same identity shape for tiled work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeBoundary {
    boundary: Option<[u8; 32]>,
    first: u32,
    last: u32,
    implementation: ImplementationIdentity,
}

impl NodeBoundary {
    #[must_use]
    pub fn whole(implementation: ImplementationIdentity) -> Self {
        Self {
            boundary: None,
            first: 0,
            last: u32::MAX,
            implementation,
        }
    }

    #[must_use]
    pub fn range(
        boundary: [u8; 32],
        first: u32,
        last: u32,
        implementation: ImplementationIdentity,
    ) -> Self {
        Self {
            boundary: Some(boundary),
            first,
            last,
            implementation,
        }
    }

    #[must_use]
    pub const fn boundary(&self) -> Option<[u8; 32]> {
        self.boundary
    }
    #[must_use]
    pub const fn first(&self) -> u32 {
        self.first
    }
    #[must_use]
    pub const fn last(&self) -> u32 {
        self.last
    }
    #[must_use]
    pub const fn implementation(&self) -> &ImplementationIdentity {
        &self.implementation
    }
}

/// The output identity includes geometry, format, alpha, color and transform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputIdentity {
    dimensions: ImageDimensions,
    roi: Roi,
    format: PixelFormat,
    color: ColorIdentity,
    transform: [u8; 32],
}

impl OutputIdentity {
    #[must_use]
    pub fn new(
        dimensions: ImageDimensions,
        roi: Roi,
        format: PixelFormat,
        color: ColorIdentity,
        transform: [u8; 32],
    ) -> Self {
        Self {
            dimensions,
            roi,
            format,
            color,
            transform,
        }
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
    pub const fn transform(&self) -> [u8; 32] {
        self.transform
    }
}

/// A diagnostic-only digest. Equality and lookup always use [`CacheKey`]
/// equality, so a SHA-256 collision cannot alias a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheKeyDigest([u8; 32]);

impl CacheKeyDigest {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Display for CacheKeyDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Named identity portions emitted in privacy-safe cache receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheKeyComponent {
    Source,
    SourceDescriptor,
    Snapshot,
    Node,
    Inputs,
    Output,
    Parameters,
    MaskBlendRaster,
    Backend,
    Schema,
    Mode,
}

/// A complete output-affecting pixelpipe cache identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    schema_version: u16,
    source: SourceIdentity,
    source_descriptor: Vec<u8>,
    snapshot: PipelineSnapshotIdentity,
    generation: PipelineGeneration,
    purpose: PipelinePurpose,
    quality: CacheQuality,
    precision: CachePrecision,
    node: NodeBoundary,
    inputs: Vec<CacheKeyDigest>,
    output: OutputIdentity,
    params: Vec<u8>,
    params_version: u16,
    enabled: bool,
    opacity_basis_points: u16,
    mask: MaskStatus,
    blend: BlendStatus,
    raster_identity: [u8; 32],
    analysis_identity: [u8; 32],
    backend_identity: [u8; 32],
    mode_identity: [u8; 32],
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_bytes().hash(state);
    }
}

impl CacheKey {
    #[must_use]
    pub fn builder() -> CacheKeyBuilder {
        CacheKeyBuilder::default()
    }

    /// Creates the whole-pipeline key directly from an immutable snapshot.
    #[must_use]
    pub fn from_snapshot(snapshot: &PipelineSnapshot) -> Self {
        let source_descriptor = source_descriptor_bytes(snapshot);
        let output = OutputIdentity::new(
            snapshot.output().dimensions(),
            snapshot.output().roi(),
            snapshot.output().format(),
            snapshot.output().color(),
            transform_identity(snapshot.output().color()),
        );
        Self {
            schema_version: CACHE_KEY_SCHEMA_VERSION,
            source: snapshot.source().identity(),
            source_descriptor,
            snapshot: snapshot.identity(),
            generation: snapshot.generation(),
            purpose: snapshot.purpose(),
            quality: snapshot.quality().into(),
            precision: snapshot.precision().into(),
            node: NodeBoundary::whole(snapshot.implementation().clone()),
            inputs: Vec::new(),
            output,
            params: snapshot.canonical_bytes(),
            params_version: snapshot.schema_version(),
            enabled: true,
            opacity_basis_points: 10_000,
            mask: snapshot.mask_status(),
            blend: snapshot.blend_status(),
            raster_identity: snapshot.source().identity().as_bytes(),
            analysis_identity: [0; 32],
            backend_identity: implementation_identity(snapshot.implementation()),
            mode_identity: [0; 32],
        }
    }

    /// Creates a key whose mode component is the complete immutable mode plan.
    #[must_use]
    pub fn from_mode_plan(plan: &ModePlan) -> Self {
        let mut key = Self::from_snapshot_identity(plan);
        key.mode_identity = plan.identity().as_bytes();
        key.purpose = plan.request().purpose();
        key.quality = match plan.request().quality() {
            crate::ModeQuality::Interactive => CacheQuality::Draft,
            crate::ModeQuality::Balanced => CacheQuality::Normal,
            crate::ModeQuality::High => CacheQuality::High,
            crate::ModeQuality::Exact => CacheQuality::Maximum,
        };
        key.output = OutputIdentity::new(
            plan.request().output().dimensions(),
            plan.request().roi(),
            plan.request().output().format(),
            plan.request().output().color(),
            plan.request().target().as_bytes(),
        );
        key
    }

    fn from_snapshot_identity(plan: &ModePlan) -> Self {
        Self {
            schema_version: CACHE_KEY_SCHEMA_VERSION,
            source: plan.source_identity(),
            source_descriptor: Vec::new(),
            snapshot: plan.snapshot_identity(),
            generation: plan.generation(),
            purpose: plan.request().purpose(),
            quality: CacheQuality::Normal,
            precision: plan.request().precision().into(),
            node: NodeBoundary::whole(
                ImplementationIdentity::new("rusttable.pixelpipe.mode", 1, "planner")
                    .expect("constant identity"),
            ),
            inputs: Vec::new(),
            output: OutputIdentity::new(
                plan.request().output().dimensions(),
                plan.request().roi(),
                plan.request().output().format(),
                plan.request().output().color(),
                plan.request().target().as_bytes(),
            ),
            params: plan.canonical_bytes(),
            params_version: crate::MODE_SCHEMA_VERSION,
            enabled: true,
            opacity_basis_points: 10_000,
            mask: MaskStatus::NotReferenced,
            blend: BlendStatus::NotReferenced,
            raster_identity: plan.source_identity().as_bytes(),
            analysis_identity: [0; 32],
            backend_identity: [0; 32],
            mode_identity: plan.identity().as_bytes(),
        }
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }
    #[must_use]
    pub const fn source(&self) -> SourceIdentity {
        self.source
    }
    #[must_use]
    pub const fn snapshot(&self) -> PipelineSnapshotIdentity {
        self.snapshot
    }
    #[must_use]
    pub const fn generation(&self) -> PipelineGeneration {
        self.generation
    }
    #[must_use]
    pub const fn purpose(&self) -> PipelinePurpose {
        self.purpose
    }
    #[must_use]
    pub const fn quality(&self) -> CacheQuality {
        self.quality
    }
    #[must_use]
    pub const fn precision(&self) -> CachePrecision {
        self.precision
    }
    #[must_use]
    pub const fn node(&self) -> &NodeBoundary {
        &self.node
    }
    #[must_use]
    pub fn output(&self) -> &OutputIdentity {
        &self.output
    }
    #[must_use]
    pub const fn mask(&self) -> MaskStatus {
        self.mask
    }
    #[must_use]
    pub const fn blend(&self) -> BlendStatus {
        self.blend
    }
    #[must_use]
    pub const fn backend_identity(&self) -> [u8; 32] {
        self.backend_identity
    }
    #[must_use]
    pub const fn mode_identity(&self) -> [u8; 32] {
        self.mode_identity
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"rusttable.pixelpipe.cache-key.v2");
        bytes.extend_from_slice(&self.schema_version.to_le_bytes());
        bytes.extend_from_slice(&self.source.as_bytes());
        write_bytes(&self.source_descriptor, &mut bytes);
        bytes.extend_from_slice(&self.snapshot.as_bytes());
        bytes.extend_from_slice(&self.generation.get().to_le_bytes());
        bytes.push(purpose_tag(self.purpose));
        bytes.push(quality_tag(self.quality));
        bytes.push(precision_tag(self.precision));
        write_node(&self.node, &mut bytes);
        write_bytes(
            &self
                .inputs
                .iter()
                .flat_map(|input| input.as_bytes())
                .collect::<Vec<_>>(),
            &mut bytes,
        );
        write_output(&self.output, &mut bytes);
        write_bytes(&self.params, &mut bytes);
        bytes.extend_from_slice(&self.params_version.to_le_bytes());
        bytes.push(u8::from(self.enabled));
        bytes.extend_from_slice(&self.opacity_basis_points.to_le_bytes());
        bytes.push(mask_tag(self.mask));
        bytes.push(blend_tag(self.blend));
        bytes.extend_from_slice(&self.raster_identity);
        bytes.extend_from_slice(&self.analysis_identity);
        bytes.extend_from_slice(&self.backend_identity);
        bytes.extend_from_slice(&self.mode_identity);
        bytes
    }

    #[must_use]
    pub fn diagnostic_sha256(&self) -> CacheKeyDigest {
        CacheKeyDigest(Sha256::digest(self.canonical_bytes()).into())
    }

    #[must_use]
    pub fn components(&self) -> &'static [CacheKeyComponent] {
        &[
            CacheKeyComponent::Source,
            CacheKeyComponent::SourceDescriptor,
            CacheKeyComponent::Snapshot,
            CacheKeyComponent::Node,
            CacheKeyComponent::Inputs,
            CacheKeyComponent::Output,
            CacheKeyComponent::Parameters,
            CacheKeyComponent::MaskBlendRaster,
            CacheKeyComponent::Backend,
            CacheKeyComponent::Mode,
            CacheKeyComponent::Schema,
        ]
    }

    #[must_use]
    pub fn matches(&self, scope: &crate::CacheScope) -> bool {
        match scope {
            crate::CacheScope::Source(source) => self.source == *source,
            crate::CacheScope::Snapshot(generation) => self.generation == *generation,
            crate::CacheScope::Implementation(identity) => self.node.implementation() == identity,
            crate::CacheScope::Backend(backend) => self.backend_identity == *backend,
            crate::CacheScope::MemoryPressure | crate::CacheScope::All => true,
        }
    }
}

/// Checked construction for callers that need node/tile identities.
#[derive(Debug, Clone)]
pub struct CacheKeyBuilder {
    source: Option<SourceIdentity>,
    source_descriptor: Vec<u8>,
    snapshot: Option<PipelineSnapshotIdentity>,
    generation: Option<PipelineGeneration>,
    purpose: Option<PipelinePurpose>,
    quality: Option<CacheQuality>,
    precision: Option<CachePrecision>,
    node: Option<NodeBoundary>,
    inputs: Vec<CacheKeyDigest>,
    output: Option<OutputIdentity>,
    params: Vec<u8>,
    params_version: u16,
    enabled: bool,
    opacity_basis_points: u16,
    mask: MaskStatus,
    blend: BlendStatus,
    raster_identity: [u8; 32],
    analysis_identity: [u8; 32],
    backend_identity: [u8; 32],
    mode_identity: [u8; 32],
}

impl Default for CacheKeyBuilder {
    fn default() -> Self {
        Self {
            source: None,
            source_descriptor: Vec::new(),
            snapshot: None,
            generation: None,
            purpose: None,
            quality: None,
            precision: None,
            node: None,
            inputs: Vec::new(),
            output: None,
            params: Vec::new(),
            params_version: 0,
            enabled: true,
            opacity_basis_points: 10_000,
            mask: MaskStatus::NotReferenced,
            blend: BlendStatus::NotReferenced,
            raster_identity: [0; 32],
            analysis_identity: [0; 32],
            backend_identity: [0; 32],
            mode_identity: [0; 32],
        }
    }
}

impl CacheKeyBuilder {
    #[must_use]
    pub fn source(mut self, value: SourceIdentity) -> Self {
        self.source = Some(value);
        self
    }
    #[must_use]
    pub fn source_descriptor(mut self, value: impl Into<Vec<u8>>) -> Self {
        self.source_descriptor = value.into();
        self
    }
    #[must_use]
    pub fn snapshot(mut self, value: PipelineSnapshotIdentity) -> Self {
        self.snapshot = Some(value);
        self
    }
    #[must_use]
    pub fn generation(mut self, value: PipelineGeneration) -> Self {
        self.generation = Some(value);
        self
    }
    #[must_use]
    pub fn purpose(mut self, value: PipelinePurpose) -> Self {
        self.purpose = Some(value);
        self
    }
    #[must_use]
    pub fn quality(mut self, value: CacheQuality) -> Self {
        self.quality = Some(value);
        self
    }
    #[must_use]
    pub fn precision(mut self, value: CachePrecision) -> Self {
        self.precision = Some(value);
        self
    }
    #[must_use]
    pub fn node(mut self, value: NodeBoundary) -> Self {
        self.node = Some(value);
        self
    }
    #[must_use]
    pub fn inputs(mut self, value: impl IntoIterator<Item = CacheKeyDigest>) -> Self {
        self.inputs = value.into_iter().collect();
        self
    }
    #[must_use]
    pub fn output(mut self, value: OutputIdentity) -> Self {
        self.output = Some(value);
        self
    }
    #[must_use]
    pub fn parameters(mut self, version: u16, value: impl Into<Vec<u8>>) -> Self {
        self.params_version = version;
        self.params = value.into();
        self
    }
    #[must_use]
    pub const fn enabled(mut self, value: bool) -> Self {
        self.enabled = value;
        self
    }
    #[must_use]
    pub const fn opacity_basis_points(mut self, value: u16) -> Self {
        self.opacity_basis_points = value;
        self
    }
    #[must_use]
    pub const fn mask(mut self, value: MaskStatus) -> Self {
        self.mask = value;
        self
    }
    #[must_use]
    pub const fn blend(mut self, value: BlendStatus) -> Self {
        self.blend = value;
        self
    }
    #[must_use]
    pub const fn raster_identity(mut self, value: [u8; 32]) -> Self {
        self.raster_identity = value;
        self
    }
    #[must_use]
    pub const fn analysis_identity(mut self, value: [u8; 32]) -> Self {
        self.analysis_identity = value;
        self
    }
    #[must_use]
    pub const fn backend_identity(mut self, value: [u8; 32]) -> Self {
        self.backend_identity = value;
        self
    }
    #[must_use]
    pub const fn mode_identity(mut self, value: [u8; 32]) -> Self {
        self.mode_identity = value;
        self
    }

    /// Builds a complete key and rejects incomplete or malformed identity.
    pub fn build(self) -> Result<CacheKey, CacheKeyError> {
        let key = CacheKey {
            schema_version: CACHE_KEY_SCHEMA_VERSION,
            source: self.source.ok_or(CacheKeyError::Missing("source"))?,
            source_descriptor: self.source_descriptor,
            snapshot: self.snapshot.ok_or(CacheKeyError::Missing("snapshot"))?,
            generation: self
                .generation
                .ok_or(CacheKeyError::Missing("generation"))?,
            purpose: self.purpose.ok_or(CacheKeyError::Missing("purpose"))?,
            quality: self.quality.ok_or(CacheKeyError::Missing("quality"))?,
            precision: self.precision.ok_or(CacheKeyError::Missing("precision"))?,
            node: self.node.ok_or(CacheKeyError::Missing("node"))?,
            inputs: self.inputs,
            output: self.output.ok_or(CacheKeyError::Missing("output"))?,
            params: self.params,
            params_version: self.params_version,
            enabled: self.enabled,
            opacity_basis_points: self.opacity_basis_points,
            mask: self.mask,
            blend: self.blend,
            raster_identity: self.raster_identity,
            analysis_identity: self.analysis_identity,
            backend_identity: self.backend_identity,
            mode_identity: self.mode_identity,
        };
        if key.params_version == 0 || key.node.first > key.node.last {
            return Err(CacheKeyError::Invalid("version or node range"));
        }
        Ok(key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheKeyError {
    Missing(&'static str),
    Invalid(&'static str),
}

impl fmt::Display for CacheKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(value) => write!(formatter, "pixelpipe cache key is missing {value}"),
            Self::Invalid(value) => write!(formatter, "pixelpipe cache key is invalid: {value}"),
        }
    }
}

impl std::error::Error for CacheKeyError {}

fn source_descriptor_bytes(snapshot: &PipelineSnapshot) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_dimensions(snapshot.source().dimensions(), &mut bytes);
    bytes.push(orientation_tag(snapshot.source().orientation()));
    write_roi(snapshot.source().bounds(), &mut bytes);
    write_format(snapshot.source().format(), &mut bytes);
    write_color(snapshot.source().color(), &mut bytes);
    bytes
}

fn write_node(node: &NodeBoundary, output: &mut Vec<u8>) {
    match node.boundary() {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&value);
        }
        None => output.push(0),
    }
    output.extend_from_slice(&node.first().to_le_bytes());
    output.extend_from_slice(&node.last().to_le_bytes());
    write_identity(node.implementation(), output);
}

fn write_output(output: &OutputIdentity, bytes: &mut Vec<u8>) {
    write_dimensions(output.dimensions, bytes);
    write_roi(output.roi, bytes);
    write_format(output.format, bytes);
    write_color(output.color, bytes);
    bytes.extend_from_slice(&output.transform);
}

fn write_identity(identity: &ImplementationIdentity, output: &mut Vec<u8>) {
    write_bytes(identity.name().as_bytes(), output);
    output.extend_from_slice(&identity.version().to_le_bytes());
    write_bytes(identity.build().as_bytes(), output);
}

fn write_dimensions(value: ImageDimensions, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.width().to_le_bytes());
    output.extend_from_slice(&value.height().to_le_bytes());
}

fn write_roi(value: Roi, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.x().to_le_bytes());
    output.extend_from_slice(&value.y().to_le_bytes());
    output.extend_from_slice(&value.width().to_le_bytes());
    output.extend_from_slice(&value.height().to_le_bytes());
}

fn write_format(value: PixelFormat, output: &mut Vec<u8>) {
    output.extend_from_slice(&[
        sample_tag(value.sample_type()),
        channel_tag(value.channels()),
        alpha_tag(value.alpha()),
        byte_order_tag(value.byte_order()),
        storage_tag(value.storage()),
    ]);
}

fn write_color(value: ColorIdentity, output: &mut Vec<u8>) {
    output.extend_from_slice(
        &postcard::to_allocvec(&value.encoding()).expect("color is serializable"),
    );
    output.extend_from_slice(&value.planner_version().to_le_bytes());
}

fn write_bytes(value: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(
        &(u64::try_from(value.len()).expect("key component fits")).to_le_bytes(),
    );
    output.extend_from_slice(value);
}

fn transform_identity(color: ColorIdentity) -> [u8; 32] {
    Sha256::digest(postcard::to_allocvec(&color.encoding()).expect("color is serializable")).into()
}

fn implementation_identity(value: &ImplementationIdentity) -> [u8; 32] {
    let mut bytes = Vec::new();
    write_identity(value, &mut bytes);
    Sha256::digest(bytes).into()
}

const fn purpose_tag(value: PipelinePurpose) -> u8 {
    match value {
        PipelinePurpose::Preview => 0,
        PipelinePurpose::Full => 1,
        PipelinePurpose::Thumbnail => 2,
        PipelinePurpose::Export => 3,
    }
}
const fn quality_tag(value: CacheQuality) -> u8 {
    match value {
        CacheQuality::Draft => 0,
        CacheQuality::Normal => 1,
        CacheQuality::High => 2,
        CacheQuality::Maximum => 3,
    }
}
const fn precision_tag(value: CachePrecision) -> u8 {
    match value {
        CachePrecision::F16 => 0,
        CachePrecision::F32 => 1,
        CachePrecision::F64 => 2,
        CachePrecision::U16 => 3,
        CachePrecision::U32 => 4,
    }
}
const fn mask_tag(value: MaskStatus) -> u8 {
    match value {
        MaskStatus::NotReferenced => 0,
        MaskStatus::Referenced => 1,
        MaskStatus::Deferred => 2,
    }
}
const fn blend_tag(value: BlendStatus) -> u8 {
    match value {
        BlendStatus::NotReferenced => 0,
        BlendStatus::Referenced => 1,
        BlendStatus::Deferred => 2,
    }
}
const fn orientation_tag(value: Orientation) -> u8 {
    match value {
        Orientation::Normal => 0,
        Orientation::FlipHorizontal => 1,
        Orientation::Rotate180 => 2,
        Orientation::FlipVertical => 3,
        Orientation::Transpose => 4,
        Orientation::Rotate90 => 5,
        Orientation::Transverse => 6,
        Orientation::Rotate270 => 7,
    }
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
const fn byte_order_tag(value: ByteOrder) -> u8 {
    match value {
        ByteOrder::Native => 0,
        ByteOrder::Little => 1,
        ByteOrder::Big => 2,
    }
}
const fn storage_tag(value: StorageLayout) -> u8 {
    match value {
        StorageLayout::Interleaved => 0,
        StorageLayout::Planar => 1,
    }
}
