#![allow(clippy::missing_errors_doc)]

use std::{fmt, num::NonZeroU32};

use rusttable_image::{ImageDimensions, Orientation, Roi};

use super::roi_distortion::{
    DistortionError, Point, invert_affine, invert_homography, map_point, validate_mapping,
};

pub const ROI_SCHEMA_VERSION: u16 = 1;

/// A checked integer half-open rectangle in a node's logical pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoiRect {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl RoiRect {
    /// Creates a rectangle. Zero-area rectangles are retained for explicit empty requests.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, RoiError> {
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return Err(RoiError::Overflow);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn full(dimensions: ImageDimensions) -> Self {
        Self {
            x: 0,
            y: 0,
            width: dimensions.width(),
            height: dimensions.height(),
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
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[must_use]
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    pub(super) fn within(self, bounds: Self) -> Result<Self, RoiError> {
        if self.right() > bounds.right() || self.bottom() > bounds.bottom() {
            return Err(RoiError::OutOfBounds);
        }
        Ok(self)
    }
}

impl From<Roi> for RoiRect {
    fn from(value: Roi) -> Self {
        Self {
            x: value.x(),
            y: value.y(),
            width: value.width(),
            height: value.height(),
        }
    }
}

impl From<RoiRect> for Roi {
    fn from(value: RoiRect) -> Self {
        Roi::new(value.x, value.y, value.width, value.height).expect("checked ROI")
    }
}

/// Validation failure for integer ROI geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiError {
    Overflow,
    OutOfBounds,
    EmptyNotAllowed,
    InvalidDimensions,
}

impl fmt::Display for RoiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Overflow => "ROI coordinate arithmetic overflowed",
            Self::OutOfBounds => "ROI is outside its logical bounds",
            Self::EmptyNotAllowed => "this ROI contract does not accept an empty ROI",
            Self::InvalidDimensions => "ROI descriptor dimensions are invalid",
        })
    }
}

impl std::error::Error for RoiError {}

/// Identity for the logical descriptor at one planner boundary.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiDescriptorIdentity(pub(super) [u8; 32]);

impl RoiDescriptorIdentity {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for RoiDescriptorIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Full logical bounds and identity for a node boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiDescriptor {
    pub(super) dimensions: ImageDimensions,
    pub(super) bounds: RoiRect,
    pub(super) identity: RoiDescriptorIdentity,
}

impl RoiDescriptor {
    /// Creates a checked logical descriptor.
    pub fn new(
        dimensions: ImageDimensions,
        bounds: RoiRect,
        identity: RoiDescriptorIdentity,
    ) -> Result<Self, RoiError> {
        bounds.within(RoiRect::full(dimensions))?;
        Ok(Self {
            dimensions,
            bounds,
            identity,
        })
    }

    /// Creates a source descriptor whose identity is the immutable source digest.
    pub fn source(
        dimensions: ImageDimensions,
        bounds: RoiRect,
        source_identity: [u8; 32],
    ) -> Result<Self, RoiError> {
        Self::new(
            dimensions,
            bounds,
            RoiDescriptorIdentity::new(source_identity),
        )
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn bounds(self) -> RoiRect {
        self.bounds
    }
    #[must_use]
    pub const fn identity(self) -> RoiDescriptorIdentity {
        self.identity
    }
}

/// A checked positive rational scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RationalScale {
    pub(super) numerator: NonZeroU32,
    pub(super) denominator: NonZeroU32,
}

impl RationalScale {
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, RoiError> {
        Ok(Self {
            numerator: NonZeroU32::new(numerator).ok_or(RoiError::InvalidDimensions)?,
            denominator: NonZeroU32::new(denominator).ok_or(RoiError::InvalidDimensions)?,
        })
    }
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator.get()
    }
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }
}

/// Per-edge neighborhood support, in the input node's logical pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiSupport {
    pub(super) left: u32,
    pub(super) right: u32,
    pub(super) top: u32,
    pub(super) bottom: u32,
}

impl RoiSupport {
    #[must_use]
    pub const fn new(left: u32, right: u32, top: u32, bottom: u32) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }
    #[must_use]
    pub const fn symmetric(radius: u32) -> Self {
        Self::new(radius, radius, radius, radius)
    }
    #[must_use]
    pub const fn left(self) -> u32 {
        self.left
    }
    #[must_use]
    pub const fn right(self) -> u32 {
        self.right
    }
    #[must_use]
    pub const fn top(self) -> u32 {
        self.top
    }
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.bottom
    }

    pub(super) fn add(self, other: Self) -> Result<Self, RoiError> {
        Ok(Self::new(
            self.left
                .checked_add(other.left)
                .ok_or(RoiError::Overflow)?,
            self.right
                .checked_add(other.right)
                .ok_or(RoiError::Overflow)?,
            self.top.checked_add(other.top).ok_or(RoiError::Overflow)?,
            self.bottom
                .checked_add(other.bottom)
                .ok_or(RoiError::Overflow)?,
        ))
    }
}

/// Policy applied when a requested final rectangle exceeds the final descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoiRequestPolicy {
    RejectOutOfBounds,
    ClipToFinalBounds,
}

/// A requested output rectangle and its explicit clipping policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiRequest {
    pub(super) output: RoiRect,
    pub(super) policy: RoiRequestPolicy,
}

impl RoiRequest {
    #[must_use]
    pub const fn new(output: RoiRect, policy: RoiRequestPolicy) -> Self {
        Self { output, policy }
    }
    #[must_use]
    pub const fn output(self) -> RoiRect {
        self.output
    }
    #[must_use]
    pub const fn policy(self) -> RoiRequestPolicy {
        self.policy
    }
}

/// Prepared point transform used by the bounded distortion enclosure.
#[derive(Debug, Clone, PartialEq)]
pub enum DistortionMapping {
    Affine {
        matrix: [f64; 6],
    },
    Homography {
        matrix: [f64; 9],
    },
    Radial {
        center_x: f64,
        center_y: f64,
        k1: f64,
        k2: f64,
    },
}

/// A validated distortion binding with deterministic forward/inverse evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct DistortionBinding {
    pub(super) identity: String,
    pub(super) mapping: DistortionMapping,
    pub(super) inverse: DistortionMapping,
    pub(super) output_dimensions: Option<ImageDimensions>,
    pub(super) tolerance: f64,
}

impl DistortionBinding {
    /// Creates an affine binding and computes its checked inverse.
    pub fn affine(
        identity: impl Into<String>,
        matrix: [f64; 6],
        output_dimensions: Option<ImageDimensions>,
        tolerance: f64,
    ) -> Result<Self, DistortionError> {
        let inverse = invert_affine(matrix)?;
        Self::new(
            identity,
            DistortionMapping::Affine { matrix },
            DistortionMapping::Affine { matrix: inverse },
            output_dimensions,
            tolerance,
        )
    }

    /// Creates a projective homography binding and computes its checked inverse.
    pub fn homography(
        identity: impl Into<String>,
        matrix: [f64; 9],
        output_dimensions: Option<ImageDimensions>,
        tolerance: f64,
    ) -> Result<Self, DistortionError> {
        let inverse = invert_homography(matrix)?;
        Self::new(
            identity,
            DistortionMapping::Homography { matrix },
            DistortionMapping::Homography { matrix: inverse },
            output_dimensions,
            tolerance,
        )
    }

    /// Creates a radial barrel or pincushion binding with a numeric inverse.
    pub fn radial(
        identity: impl Into<String>,
        center_x: f64,
        center_y: f64,
        k1: f64,
        k2: f64,
        output_dimensions: Option<ImageDimensions>,
        tolerance: f64,
    ) -> Result<Self, DistortionError> {
        let mapping = DistortionMapping::Radial {
            center_x,
            center_y,
            k1,
            k2,
        };
        validate_mapping(&mapping)?;
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(DistortionError::InvalidTolerance);
        }
        Ok(Self {
            identity: identity.into(),
            mapping: mapping.clone(),
            inverse: mapping,
            output_dimensions,
            tolerance,
        })
    }

    fn new(
        identity: impl Into<String>,
        mapping: DistortionMapping,
        inverse: DistortionMapping,
        output_dimensions: Option<ImageDimensions>,
        tolerance: f64,
    ) -> Result<Self, DistortionError> {
        validate_mapping(&mapping)?;
        validate_mapping(&inverse)?;
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(DistortionError::InvalidTolerance);
        }
        Ok(Self {
            identity: identity.into(),
            mapping,
            inverse,
            output_dimensions,
            tolerance,
        })
    }

    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
    #[must_use]
    pub(super) const fn tolerance(&self) -> f64 {
        self.tolerance
    }

    pub(super) fn output_dimensions(&self, input: ImageDimensions) -> ImageDimensions {
        self.output_dimensions.unwrap_or(input)
    }

    pub(super) fn forward(&self, point: Point) -> Result<Point, DistortionError> {
        map_point(&self.mapping, point, false)
    }
    pub(super) fn inverse(&self, point: Point) -> Result<Point, DistortionError> {
        map_point(&self.inverse, point, true)
    }
}

/// One supported forward/reverse ROI contract.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeRoiContract {
    Identity,
    Neighborhood {
        support: u32,
        asymmetric_support: RoiSupport,
    },
    Crop {
        output_bounds: RoiRect,
        input_offset: (i32, i32),
    },
    Scale {
        rational_x: RationalScale,
        rational_y: RationalScale,
        filter_support: RoiSupport,
    },
    Canvas {
        output_bounds: RoiRect,
        source_offset: (i32, i32),
        fill: FillValue,
    },
    Orientation {
        orientation: Orientation,
    },
    Distortion {
        binding: DistortionBinding,
    },
    FullImage,
    Unsupported {
        reason: String,
    },
}

impl NodeRoiContract {
    #[must_use]
    pub const fn identity() -> Self {
        Self::Identity
    }
    #[must_use]
    pub fn from_kind(kind: rusttable_processing::descriptor::RoiKind) -> Self {
        match kind {
            rusttable_processing::descriptor::RoiKind::Identity
            | rusttable_processing::descriptor::RoiKind::PreparedBinding => Self::Identity,
            rusttable_processing::descriptor::RoiKind::Neighborhood => Self::Neighborhood {
                support: 1,
                asymmetric_support: RoiSupport::symmetric(0),
            },
            rusttable_processing::descriptor::RoiKind::FullImage => Self::FullImage,
            other => Self::Unsupported {
                reason: format!("descriptor ROI kind {other:?} needs a prepared binding"),
            },
        }
    }
}

/// Fill used by a canvas contract; it affects geometry identity but is not pixel execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FillValue {
    Transparent,
    Constant([u32; 4]),
}

/// Operation identity carried into every planner trace step.
#[derive(Debug, Clone, PartialEq)]
pub struct RoiNode {
    pub(super) operation_id: u128,
    pub(super) compatibility_name: String,
    pub(super) contract: NodeRoiContract,
}

impl RoiNode {
    #[must_use]
    pub fn new(
        operation_id: u128,
        compatibility_name: impl Into<String>,
        contract: NodeRoiContract,
    ) -> Self {
        Self {
            operation_id,
            compatibility_name: compatibility_name.into(),
            contract,
        }
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub fn compatibility_name(&self) -> &str {
        &self.compatibility_name
    }
    #[must_use]
    pub fn contract(&self) -> &NodeRoiContract {
        &self.contract
    }
}

/// One forward descriptor/ROI result.
#[derive(Debug, Clone, PartialEq)]
pub struct RoiForwardStep {
    pub(super) node: RoiNode,
    pub(super) input_descriptor: RoiDescriptor,
    pub(super) output_descriptor: RoiDescriptor,
    pub(super) input_roi: RoiRect,
    pub(super) output_roi: RoiRect,
}

impl RoiForwardStep {
    #[must_use]
    pub fn node(&self) -> &RoiNode {
        &self.node
    }
    #[must_use]
    pub const fn input_descriptor(&self) -> RoiDescriptor {
        self.input_descriptor
    }
    #[must_use]
    pub const fn output_descriptor(&self) -> RoiDescriptor {
        self.output_descriptor
    }
    #[must_use]
    pub const fn input_roi(&self) -> RoiRect {
        self.input_roi
    }
    #[must_use]
    pub const fn output_roi(&self) -> RoiRect {
        self.output_roi
    }
}

/// One reverse required-input result.
#[derive(Debug, Clone, PartialEq)]
pub struct RoiBackwardStep {
    pub(super) node: RoiNode,
    pub(super) output_required: RoiRect,
    pub(super) input_required: RoiRect,
}

impl RoiBackwardStep {
    #[must_use]
    pub fn node(&self) -> &RoiNode {
        &self.node
    }
    #[must_use]
    pub const fn output_required(&self) -> RoiRect {
        self.output_required
    }
    #[must_use]
    pub const fn input_required(&self) -> RoiRect {
        self.input_required
    }
}

/// Stable identity of a complete forward/reverse ROI plan.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiPlanIdentity(pub(super) [u8; 32]);

impl RoiPlanIdentity {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for RoiPlanIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Complete deterministic evidence for one request.
#[derive(Debug, Clone, PartialEq)]
pub struct RoiPlan {
    pub(super) source: RoiDescriptor,
    pub(super) request: RoiRequest,
    pub(super) final_descriptor: RoiDescriptor,
    pub(super) requested_output: RoiRect,
    pub(super) forward: Vec<RoiForwardStep>,
    pub(super) backward: Vec<RoiBackwardStep>,
    pub(super) source_required: RoiRect,
    pub(super) identity: RoiPlanIdentity,
}

impl RoiPlan {
    #[must_use]
    pub const fn source(&self) -> RoiDescriptor {
        self.source
    }
    #[must_use]
    pub const fn request(&self) -> RoiRequest {
        self.request
    }
    #[must_use]
    pub const fn final_descriptor(&self) -> RoiDescriptor {
        self.final_descriptor
    }
    #[must_use]
    pub const fn requested_output(&self) -> RoiRect {
        self.requested_output
    }
    #[must_use]
    pub fn forward(&self) -> &[RoiForwardStep] {
        &self.forward
    }
    #[must_use]
    pub fn backward(&self) -> &[RoiBackwardStep] {
        &self.backward
    }
    #[must_use]
    pub const fn source_required(&self) -> RoiRect {
        self.source_required
    }
    #[must_use]
    pub const fn identity(&self) -> RoiPlanIdentity {
        self.identity
    }
}

/// Typed planner failure. No partial plan is returned.
#[derive(Debug, Clone, PartialEq)]
pub enum RoiPlanningError {
    InvalidSource(RoiError),
    Unsupported {
        operation_id: u128,
        compatibility_name: String,
        reason: String,
    },
    InvalidContract {
        operation_id: u128,
        reason: String,
    },
    RequestOutOfBounds {
        requested: RoiRect,
        bounds: RoiRect,
    },
    EmptyRoiRejected {
        operation_id: u128,
    },
    NonFiniteTransform {
        operation_id: u128,
    },
    SingularTransform {
        operation_id: u128,
    },
    DistortionLimit {
        operation_id: u128,
        samples: u32,
    },
    Arithmetic {
        operation_id: u128,
    },
}

impl fmt::Display for RoiPlanningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ROI planning failed: {self:?}")
    }
}
impl std::error::Error for RoiPlanningError {}
