use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_GEOMETRY_STEPS: usize = 128;

/// Stable identity for authored mask data. It is independent of a pixelpipe
/// allocation or a UI selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MaskIdentity {
    photo_id: u128,
    edit_revision: u64,
    mask_id: u128,
    mask_version: u32,
}

impl MaskIdentity {
    #[must_use]
    pub const fn new(photo_id: u128, edit_revision: u64, mask_id: u128, mask_version: u32) -> Self {
        Self {
            photo_id,
            edit_revision,
            mask_id,
            mask_version,
        }
    }
    #[must_use]
    pub const fn photo_id(self) -> u128 {
        self.photo_id
    }
    #[must_use]
    pub const fn edit_revision(self) -> u64 {
        self.edit_revision
    }
    #[must_use]
    pub const fn mask_id(self) -> u128 {
        self.mask_id
    }
    #[must_use]
    pub const fn mask_version(self) -> u32 {
        self.mask_version
    }
}

/// Half-open raster coordinates used by mask publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MaskRoi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl MaskRoi {
    /// Creates a checked ROI.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, MaskModelError> {
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return Err(MaskModelError::RoiOverflow);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
    #[must_use]
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x + self.width
    }
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y + self.height
    }
}

/// Exact ordered equations used by CPU and mirrored backend implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CombinationMode {
    Union,
    Intersection,
    Add,
    Subtract,
    Multiply,
    Screen,
}

impl CombinationMode {
    #[must_use]
    pub const fn combine(self, left: f32, right: f32) -> f32 {
        match self {
            Self::Union => left.max(right),
            Self::Intersection => left.min(right),
            Self::Add => (left + right).min(1.0),
            Self::Subtract => (left - right).max(0.0),
            Self::Multiply => left * right,
            Self::Screen => 1.0 - (1.0 - left) * (1.0 - right),
        }
    }
}

/// Post-processing applied in declaration order to a mask node or group.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MaskModifier {
    Invert,
    Opacity(f32),
    Feather(u32),
}

/// A geometry stack is shared with the operation boundary by identity and
/// order. Opaque steps retain non-affine operation ancestry without inventing
/// a second mask-specific transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GeometryStep {
    Identity,
    Affine {
        coefficients: [f32; 6],
    },
    Opaque {
        operation_id: u128,
        transform_hash: [u8; 32],
    },
}

impl GeometryStep {
    fn validate(self) -> Result<(), MaskModelError> {
        match self {
            Self::Identity | Self::Opaque { .. } => Ok(()),
            Self::Affine { coefficients } if coefficients.iter().all(|value| value.is_finite()) => {
                Ok(())
            }
            Self::Affine { .. } => Err(MaskModelError::NonFiniteGeometry),
        }
    }
    fn map(self, point: (f32, f32)) -> (f32, f32) {
        match self {
            Self::Identity | Self::Opaque { .. } => point,
            Self::Affine {
                coefficients: [a, b, c, d, e, f],
            } => (
                a.mul_add(point.0, b.mul_add(point.1, c)),
                d.mul_add(point.0, e.mul_add(point.1, f)),
            ),
        }
    }
}

/// Ordered forward geometry ancestry. Consumers must use this stack rather
/// than independently reimplementing operation transforms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometryAncestry {
    steps: Vec<GeometryStep>,
}

impl GeometryAncestry {
    pub fn new(steps: impl IntoIterator<Item = GeometryStep>) -> Result<Self, MaskModelError> {
        let steps = steps.into_iter().collect::<Vec<_>>();
        if steps.len() > MAX_GEOMETRY_STEPS {
            return Err(MaskModelError::GeometryLimit);
        }
        for step in steps.iter().copied() {
            step.validate()?;
        }
        Ok(Self { steps })
    }
    #[must_use]
    pub fn identity() -> Self {
        Self {
            steps: vec![GeometryStep::Identity],
        }
    }
    #[must_use]
    pub fn steps(&self) -> &[GeometryStep] {
        &self.steps
    }
    #[must_use]
    pub fn map_point(&self, mut point: (f32, f32)) -> (f32, f32) {
        for step in self.steps.iter().copied() {
            point = step.map(point);
        }
        point
    }
    #[must_use]
    pub fn identity_hash(&self) -> [u8; 32] {
        Sha256::digest(postcard::to_allocvec(self).expect("geometry is serializable")).into()
    }
}

/// Geometry metadata attached to one immutable mask source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskGeometry {
    ancestry: GeometryAncestry,
    source_roi: MaskRoi,
    full_frame: bool,
}

impl MaskGeometry {
    #[must_use]
    pub fn new(ancestry: GeometryAncestry, source_roi: MaskRoi, full_frame: bool) -> Self {
        Self {
            ancestry,
            source_roi,
            full_frame,
        }
    }
    #[must_use]
    pub fn ancestry(&self) -> &GeometryAncestry {
        &self.ancestry
    }
    #[must_use]
    pub const fn source_roi(&self) -> MaskRoi {
        self.source_roi
    }
    #[must_use]
    pub const fn requires_full_frame(&self) -> bool {
        self.full_frame
    }
}

/// A reference is an edge endpoint, never a mutable pointer to a producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MaskReference {
    identity: MaskIdentity,
    consumer_operation: u128,
    consumer_mask_id: u128,
}

impl MaskReference {
    #[must_use]
    pub const fn new(
        identity: MaskIdentity,
        consumer_operation: u128,
        consumer_mask_id: u128,
    ) -> Self {
        Self {
            identity,
            consumer_operation,
            consumer_mask_id,
        }
    }
    #[must_use]
    pub const fn identity(self) -> MaskIdentity {
        self.identity
    }
    #[must_use]
    pub const fn consumer_operation(self) -> u128 {
        self.consumer_operation
    }
    #[must_use]
    pub const fn consumer_mask_id(self) -> u128 {
        self.consumer_mask_id
    }
}

/// Full producer and geometry ancestry for an operation-generated raster.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProducerIdentity {
    operation_instance: u128,
    mask_id: u128,
    parameter_hash: [u8; 32],
    input_hash: [u8; 32],
    geometry_hash: [u8; 32],
    roi: MaskRoi,
    scale_bits: u32,
}

impl ProducerIdentity {
    #[must_use]
    pub fn new(
        operation_instance: u128,
        mask_id: u128,
        parameter_hash: [u8; 32],
        input_hash: [u8; 32],
        geometry_hash: [u8; 32],
        roi: MaskRoi,
        scale: f32,
    ) -> Result<Self, MaskModelError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(MaskModelError::InvalidScale);
        }
        Ok(Self {
            operation_instance,
            mask_id,
            parameter_hash,
            input_hash,
            geometry_hash,
            roi,
            scale_bits: scale.to_bits(),
        })
    }
    #[must_use]
    pub const fn operation_instance(&self) -> u128 {
        self.operation_instance
    }
    #[must_use]
    pub const fn mask_id(&self) -> u128 {
        self.mask_id
    }
    #[must_use]
    pub const fn roi(&self) -> MaskRoi {
        self.roi
    }
    #[must_use]
    pub const fn scale(self) -> f32 {
        f32::from_bits(self.scale_bits)
    }
    #[must_use]
    pub fn cache_identity(&self) -> [u8; 32] {
        Sha256::digest(postcard::to_allocvec(self).expect("producer identity is serializable"))
            .into()
    }
}

/// Canonical descriptor used by publication and exact cache lookups.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RasterMaskDescriptor {
    identity: MaskIdentity,
    producer: ProducerIdentity,
}

impl RasterMaskDescriptor {
    #[must_use]
    pub const fn new(identity: MaskIdentity, producer: ProducerIdentity) -> Self {
        Self { identity, producer }
    }
    #[must_use]
    pub const fn identity(&self) -> MaskIdentity {
        self.identity
    }
    #[must_use]
    pub fn producer(&self) -> &ProducerIdentity {
        &self.producer
    }
    #[must_use]
    pub fn cache_identity(&self) -> [u8; 32] {
        Sha256::digest(postcard::to_allocvec(self).expect("raster descriptor is serializable"))
            .into()
    }
}

/// Source kinds preserve opaque imported values without pretending they are evaluable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MaskSource {
    Raster,
    Generated(RasterMaskDescriptor),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl MaskSource {
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskModelError {
    RoiOverflow,
    GeometryLimit,
    NonFiniteGeometry,
    InvalidScale,
}

impl fmt::Display for MaskModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::RoiOverflow => "mask ROI coordinate arithmetic overflowed",
            Self::GeometryLimit => "mask geometry ancestry exceeds the configured limit",
            Self::NonFiniteGeometry => "mask geometry contains a non-finite coefficient",
            Self::InvalidScale => "mask scale must be finite and positive",
        })
    }
}

impl std::error::Error for MaskModelError {}
