//! Darktable-compatible liquify geometry and inverse resampling.
//!
//! The persistent representation is deliberately independent from Rust's
//! layout.  A plan snapshots the ordered path nodes, expands paths into the
//! same fixed arc-length stamps used by the reference, and freezes one
//! output-to-source field before any pixel or mask is sampled.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    reason = "liquify uses checked f32 image coordinates and compact compatibility codecs"
)]

use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image::Roi;

#[path = "liquify_descriptor.rs"]
mod descriptor_impl;
#[path = "liquify_math.rs"]
mod liquify_math;
#[path = "liquify_types.rs"]
mod liquify_types;

pub use descriptor_impl::liquify_descriptor;
use liquify_math::{
    ensure_roi, expand_stamps, forward_point, inverse_point, lerp, plan_identity, sample_plane,
    sample_rgb, transform_points,
};
pub use liquify_types::{
    LiquifyCodecError, LiquifyExecution, LiquifyExecutionError, LiquifyValidationError,
};

pub const LIQUIFY_COMPATIBILITY_ID: &str = "liquify";
pub const LIQUIFY_RUST_ID: &str = "rusttable.liquify";
pub const LIQUIFY_SCHEMA_VERSION: u16 = 1;
pub const LIQUIFY_PARAMETER_BYTES: usize = 60;
pub const LIQUIFY_MAX_NODES: usize = 100;
pub const LIQUIFY_MAX_PARAMETER_BYTES: usize = 8192;
pub const LIQUIFY_INTERPOLATION_POINTS: usize = 100;
pub const LIQUIFY_LOOKUP_OVERSAMPLE: usize = 10;
pub const LIQUIFY_STAMP_RELOCATION: f32 = 0.1;
const MAX_FIELD_BYTES: usize = 256 * 1024 * 1024;
const MAX_COORDINATE: f32 = 1.0e7;
const INVERSE_TOLERANCE: f32 = 1.0e-4;
const INVERSE_ITERATIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LiquifyPoint {
    x: FiniteF32,
    y: FiniteF32,
}

impl LiquifyPoint {
    pub fn new(x: f32, y: f32) -> Result<Self, LiquifyValidationError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(LiquifyValidationError::NonFiniteCoordinate);
        }
        if x.abs() > MAX_COORDINATE || y.abs() > MAX_COORDINATE {
            return Err(LiquifyValidationError::CoordinateOutOfRange);
        }
        Ok(Self {
            x: FiniteF32::new(x).map_err(|_| LiquifyValidationError::NonFiniteCoordinate)?,
            y: FiniteF32::new(y).map_err(|_| LiquifyValidationError::NonFiniteCoordinate)?,
        })
    }

    /// # Panics
    ///
    /// Never panics because zero is a finite coordinate.
    pub fn zero() -> Self {
        Self {
            x: FiniteF32::new(0.0).unwrap(),
            y: FiniteF32::new(0.0).unwrap(),
        }
    }

    pub const fn x(self) -> f32 {
        self.x.get()
    }

    pub const fn y(self) -> f32 {
        self.y.get()
    }

    fn add(self, other: Self) -> Result<Self, LiquifyValidationError> {
        Self::new(self.x() + other.x(), self.y() + other.y())
    }

    fn sub(self, other: Self) -> Result<Self, LiquifyValidationError> {
        Self::new(self.x() - other.x(), self.y() - other.y())
    }

    fn scale(self, factor: f32) -> Result<Self, LiquifyValidationError> {
        Self::new(self.x() * factor, self.y() * factor)
    }

    fn distance(self, other: Self) -> f32 {
        (self.x() - other.x()).hypot(self.y() - other.y())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LiquifyWarpType {
    Linear = 0,
    RadialGrow = 1,
    RadialShrink = 2,
}

impl TryFrom<u8> for LiquifyWarpType {
    type Error = LiquifyValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Linear),
            1 => Ok(Self::RadialGrow),
            2 => Ok(Self::RadialShrink),
            _ => Err(LiquifyValidationError::UnknownWarpType(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LiquifyNodeType {
    Cusp = 0,
    Smooth = 1,
    Symmetrical = 2,
    AutoSmooth = 3,
}

impl TryFrom<u8> for LiquifyNodeType {
    type Error = LiquifyValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Cusp),
            1 => Ok(Self::Smooth),
            2 => Ok(Self::Symmetrical),
            3 => Ok(Self::AutoSmooth),
            _ => Err(LiquifyValidationError::UnknownNodeType(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LiquifyPathKind {
    Invalidated = 0,
    MoveToV1 = 1,
    LineToV1 = 2,
    CurveToV1 = 3,
}

impl TryFrom<u8> for LiquifyPathKind {
    type Error = LiquifyValidationError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Invalidated),
            1 => Ok(Self::MoveToV1),
            2 => Ok(Self::LineToV1),
            3 => Ok(Self::CurveToV1),
            _ => Err(LiquifyValidationError::UnknownPathType(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LiquifyStatus(u8);

impl LiquifyStatus {
    pub const NONE: Self = Self(0);
    pub const NEW: Self = Self(1);
    pub const INTERPOLATED: Self = Self(2);
    pub const PREVIEW: Self = Self(4);

    pub const fn new(bits: u8) -> Result<Self, LiquifyValidationError> {
        if bits & !7 == 0 {
            Ok(Self(bits))
        } else {
            Err(LiquifyValidationError::UnknownStatus(bits))
        }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiquifyNode {
    path: LiquifyPathKind,
    node_type: LiquifyNodeType,
    selected: u8,
    hovered: u8,
    prev: i8,
    index: i8,
    next: i8,
    point: LiquifyPoint,
    strength: LiquifyPoint,
    radius: LiquifyPoint,
    control1: FiniteF32,
    control2: FiniteF32,
    warp_type: LiquifyWarpType,
    status: LiquifyStatus,
    ctrl1: LiquifyPoint,
    ctrl2: LiquifyPoint,
}

impl LiquifyNode {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: LiquifyPathKind,
        node_type: LiquifyNodeType,
        point: LiquifyPoint,
        strength: LiquifyPoint,
        radius: LiquifyPoint,
        control1: f32,
        control2: f32,
        warp_type: LiquifyWarpType,
        status: LiquifyStatus,
        ctrl1: LiquifyPoint,
        ctrl2: LiquifyPoint,
    ) -> Result<Self, LiquifyValidationError> {
        if !control1.is_finite()
            || !control2.is_finite()
            || !(0.0..=1.0).contains(&control1)
            || !(0.0..=1.0).contains(&control2)
        {
            return Err(LiquifyValidationError::InvalidFalloff);
        }
        if radius.distance(point) <= f32::EPSILON {
            return Err(LiquifyValidationError::ZeroRadius);
        }
        Ok(Self {
            path,
            node_type,
            selected: 0,
            hovered: 0,
            prev: -1,
            index: -1,
            next: -1,
            point,
            strength,
            radius,
            control1: FiniteF32::new(control1)
                .map_err(|_| LiquifyValidationError::InvalidFalloff)?,
            control2: FiniteF32::new(control2)
                .map_err(|_| LiquifyValidationError::InvalidFalloff)?,
            warp_type,
            status,
            ctrl1,
            ctrl2,
        })
    }

    pub const fn path(&self) -> LiquifyPathKind {
        self.path
    }
    pub const fn node_type(&self) -> LiquifyNodeType {
        self.node_type
    }
    pub const fn point(&self) -> LiquifyPoint {
        self.point
    }
    pub const fn strength(&self) -> LiquifyPoint {
        self.strength
    }
    pub const fn radius(&self) -> LiquifyPoint {
        self.radius
    }
    pub const fn ctrl1(&self) -> LiquifyPoint {
        self.ctrl1
    }
    pub const fn ctrl2(&self) -> LiquifyPoint {
        self.ctrl2
    }
    pub const fn control1(&self) -> f32 {
        self.control1.get()
    }
    pub const fn control2(&self) -> f32 {
        self.control2.get()
    }
    pub const fn warp_type(&self) -> LiquifyWarpType {
        self.warp_type
    }
    pub const fn status(&self) -> LiquifyStatus {
        self.status
    }

    #[must_use]
    pub const fn with_links(mut self, prev: i8, index: i8, next: i8) -> Self {
        self.prev = prev;
        self.index = index;
        self.next = next;
        self
    }

    #[must_use]
    pub const fn with_ui_flags(mut self, selected: u8, hovered: u8) -> Self {
        self.selected = selected;
        self.hovered = hovered;
        self
    }

    fn link_values(&self) -> (i8, i8, i8, u8, u8) {
        (
            self.prev,
            self.index,
            self.next,
            self.selected,
            self.hovered,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiquifyParametersV1 {
    nodes: Vec<LiquifyNode>,
    opaque_source: Option<Vec<u8>>,
    blocked_version: Option<u16>,
}

impl LiquifyParametersV1 {
    pub fn new(nodes: Vec<LiquifyNode>) -> Result<Self, LiquifyValidationError> {
        let value = Self {
            nodes,
            opaque_source: None,
            blocked_version: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LiquifyCodecError> {
        if bytes.len() > LIQUIFY_MAX_PARAMETER_BYTES {
            return Err(LiquifyCodecError::Oversized {
                actual: bytes.len(),
                maximum: LIQUIFY_MAX_PARAMETER_BYTES,
            });
        }
        if bytes.len() < 8 || &bytes[0..4] != b"LTQY" {
            return Err(LiquifyCodecError::InvalidHeader);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != LIQUIFY_SCHEMA_VERSION {
            return Err(LiquifyCodecError::UnsupportedVersion(version));
        }
        let count = usize::from(u16::from_le_bytes([bytes[6], bytes[7]]));
        if count > LIQUIFY_MAX_NODES || bytes.len() != 8 + count * LIQUIFY_PARAMETER_BYTES {
            return Err(LiquifyCodecError::InvalidLength {
                actual: bytes.len(),
            });
        }
        let mut nodes = Vec::with_capacity(count);
        for chunk in bytes[8..].as_chunks::<LIQUIFY_PARAMETER_BYTES>().0 {
            nodes.push(decode_node(chunk)?);
        }
        let value = Self {
            nodes,
            opaque_source: Some(bytes.to_vec()),
            blocked_version: None,
        };
        value.validate()?;
        Ok(value)
    }

    fn from_opaque(version: u16, bytes: &[u8]) -> Result<Self, LiquifyCodecError> {
        if bytes.len() > LIQUIFY_MAX_PARAMETER_BYTES {
            return Err(LiquifyCodecError::Oversized {
                actual: bytes.len(),
                maximum: LIQUIFY_MAX_PARAMETER_BYTES,
            });
        }
        Ok(Self {
            nodes: Vec::new(),
            opaque_source: Some(bytes.to_vec()),
            blocked_version: Some(version),
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, LiquifyCodecError> {
        self.validate().map_err(LiquifyCodecError::Validation)?;
        if let Some(bytes) = &self.opaque_source {
            return Ok(bytes.clone());
        }
        let count =
            u16::try_from(self.nodes.len()).map_err(|_| LiquifyCodecError::InvalidLength {
                actual: self.nodes.len(),
            })?;
        let mut bytes = Vec::with_capacity(8 + self.nodes.len() * LIQUIFY_PARAMETER_BYTES);
        bytes.extend_from_slice(b"LTQY");
        bytes.extend_from_slice(&LIQUIFY_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        for node in &self.nodes {
            encode_node(node, &mut bytes);
        }
        Ok(bytes)
    }

    pub fn nodes(&self) -> &[LiquifyNode] {
        &self.nodes
    }
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }

    pub const fn blocked_version(&self) -> Option<u16> {
        self.blocked_version
    }

    fn validate(&self) -> Result<(), LiquifyValidationError> {
        if self.nodes.len() > LIQUIFY_MAX_NODES {
            return Err(LiquifyValidationError::TooManyNodes(self.nodes.len()));
        }
        let mut saw_end = false;
        for (position, node) in self.nodes.iter().enumerate() {
            if saw_end || node.path == LiquifyPathKind::Invalidated {
                saw_end = true;
                continue;
            }
            if position == 0 && node.path != LiquifyPathKind::MoveToV1 {
                return Err(LiquifyValidationError::PathMustStartWithMove);
            }
            if node.path == LiquifyPathKind::CurveToV1 && position == 0 {
                return Err(LiquifyValidationError::InvalidBezierTopology);
            }
            if node.index >= 0 && usize::from(node.index as u8) != position {
                return Err(LiquifyValidationError::InvalidLinks);
            }
            if node.prev >= 0 && usize::from(node.prev as u8) >= self.nodes.len()
                || node.next >= 0 && usize::from(node.next as u8) >= self.nodes.len()
            {
                return Err(LiquifyValidationError::InvalidLinks);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiquifyConfig {
    parameters: LiquifyParametersV1,
}

impl LiquifyConfig {
    pub fn new(nodes: Vec<LiquifyNode>) -> Result<Self, LiquifyValidationError> {
        Ok(Self {
            parameters: LiquifyParametersV1::new(nodes)?,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LiquifyCodecError> {
        match LiquifyParametersV1::from_bytes(bytes) {
            Ok(parameters) => Ok(Self { parameters }),
            Err(LiquifyCodecError::UnsupportedVersion(version)) => Ok(Self {
                parameters: LiquifyParametersV1::from_opaque(version, bytes)?,
            }),
            Err(LiquifyCodecError::Validation(error)) if is_unknown_type(&error) => Ok(Self {
                parameters: LiquifyParametersV1::from_opaque(LIQUIFY_SCHEMA_VERSION, bytes)?,
            }),
            Err(error) => Err(error),
        }
    }

    pub fn from_hex(hex: &str) -> Result<Self, LiquifyCodecError> {
        if hex.is_empty() {
            return Self::new(Vec::new()).map_err(LiquifyCodecError::Validation);
        }
        if !hex.len().is_multiple_of(2) {
            return Err(LiquifyCodecError::InvalidHex);
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().as_chunks::<2>().0 {
            let high = hex_digit(pair[0]).ok_or(LiquifyCodecError::InvalidHex)?;
            let low = hex_digit(pair[1]).ok_or(LiquifyCodecError::InvalidHex)?;
            bytes.push(high << 4 | low);
        }
        Self::from_bytes(&bytes)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, LiquifyCodecError> {
        self.parameters.to_bytes()
    }
    pub fn nodes(&self) -> &[LiquifyNode] {
        self.parameters.nodes()
    }
    pub fn parameters(&self) -> &LiquifyParametersV1 {
        &self.parameters
    }

    pub const fn is_blocking(&self) -> bool {
        self.parameters.blocked_version().is_some()
    }

    /// # Panics
    ///
    /// Never panics because an empty node list is a valid identity config.
    pub fn identity() -> Self {
        Self::new(Vec::new()).expect("identity liquify config")
    }
}

fn is_unknown_type(error: &LiquifyValidationError) -> bool {
    matches!(
        error,
        LiquifyValidationError::UnknownWarpType(_)
            | LiquifyValidationError::UnknownNodeType(_)
            | LiquifyValidationError::UnknownPathType(_)
            | LiquifyValidationError::UnknownStatus(_)
            | LiquifyValidationError::UnknownFlags
    )
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn put_f32(value: f32, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn encode_node(node: &LiquifyNode, output: &mut Vec<u8>) {
    let (prev, index, next, selected, hovered) = node.link_values();
    output.extend_from_slice(&[node.path as u8, node.node_type as u8, selected, hovered]);
    output.extend_from_slice(&[prev as u8, index as u8, next as u8, 0]);
    for point in [node.point, node.strength, node.radius] {
        put_f32(point.x(), output);
        put_f32(point.y(), output);
    }
    put_f32(node.control1(), output);
    put_f32(node.control2(), output);
    output.extend_from_slice(&[node.warp_type as u8, node.status.bits(), 0, 0]);
    for point in [node.ctrl1, node.ctrl2] {
        put_f32(point.x(), output);
        put_f32(point.y(), output);
    }
}

fn read_f32(bytes: &[u8]) -> f32 {
    f32::from_le_bytes(bytes.try_into().expect("fixed codec slice"))
}

fn decode_node(bytes: &[u8]) -> Result<LiquifyNode, LiquifyCodecError> {
    if bytes[7] != 0 || bytes[42] != 0 || bytes[43] != 0 {
        return Err(LiquifyCodecError::Validation(
            LiquifyValidationError::UnknownFlags,
        ));
    }
    let path = LiquifyPathKind::try_from(bytes[0]).map_err(LiquifyCodecError::Validation)?;
    let node_type = LiquifyNodeType::try_from(bytes[1]).map_err(LiquifyCodecError::Validation)?;
    let point = LiquifyPoint::new(read_f32(&bytes[8..12]), read_f32(&bytes[12..16]))
        .map_err(LiquifyCodecError::Validation)?;
    let strength = LiquifyPoint::new(read_f32(&bytes[16..20]), read_f32(&bytes[20..24]))
        .map_err(LiquifyCodecError::Validation)?;
    let radius = LiquifyPoint::new(read_f32(&bytes[24..28]), read_f32(&bytes[28..32]))
        .map_err(LiquifyCodecError::Validation)?;
    let status = LiquifyStatus::new(bytes[41]).map_err(LiquifyCodecError::Validation)?;
    let warp_type = LiquifyWarpType::try_from(bytes[40]).map_err(LiquifyCodecError::Validation)?;
    let ctrl1 = LiquifyPoint::new(read_f32(&bytes[44..48]), read_f32(&bytes[48..52]))
        .map_err(LiquifyCodecError::Validation)?;
    let ctrl2 = LiquifyPoint::new(read_f32(&bytes[52..56]), read_f32(&bytes[56..60]))
        .map_err(LiquifyCodecError::Validation)?;
    let node = LiquifyNode::new(
        path,
        node_type,
        point,
        strength,
        radius,
        read_f32(&bytes[32..36]),
        read_f32(&bytes[36..40]),
        warp_type,
        status,
        ctrl1,
        ctrl2,
    )?
    .with_links(
        i8::from_ne_bytes([bytes[4]]),
        i8::from_ne_bytes([bytes[5]]),
        i8::from_ne_bytes([bytes[6]]),
    )
    .with_ui_flags(bytes[2], bytes[3]);
    Ok(node)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiquifyInterpolation {
    Bilinear,
    Bicubic,
    Lanczos2,
    Lanczos3,
}

impl LiquifyInterpolation {
    pub const fn support(self) -> u32 {
        match self {
            Self::Bilinear => 1,
            Self::Bicubic | Self::Lanczos2 => 2,
            Self::Lanczos3 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stamp {
    center: LiquifyPoint,
    strength: LiquifyPoint,
    radius: f32,
    control1: f32,
    control2: f32,
    warp_type: LiquifyWarpType,
    relocated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquifyGpuDispatch {
    field_dimensions: RasterDimensions,
    workgroups: (u32, u32),
    field_bytes: usize,
    sampling_support: u32,
    cpu_fallback: bool,
}

impl LiquifyGpuDispatch {
    pub const fn field_dimensions(&self) -> RasterDimensions {
        self.field_dimensions
    }
    pub const fn workgroups(&self) -> (u32, u32) {
        self.workgroups
    }
    pub const fn field_bytes(&self) -> usize {
        self.field_bytes
    }
    pub const fn sampling_support(&self) -> u32 {
        self.sampling_support
    }
    pub const fn uses_cpu_fallback(&self) -> bool {
        self.cpu_fallback
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LiquifyPlan {
    config: LiquifyConfig,
    dimensions: RasterDimensions,
    interpolation: LiquifyInterpolation,
    stamps: Vec<Stamp>,
    field: Vec<LiquifyPoint>,
    maximum_displacement: u32,
    identity: [u8; 32],
}

impl LiquifyPlan {
    pub fn new(
        config: LiquifyConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, LiquifyExecutionError> {
        Self::new_with_interpolation(config, dimensions, LiquifyInterpolation::Lanczos3)
    }

    pub fn new_with_interpolation(
        config: LiquifyConfig,
        dimensions: RasterDimensions,
        interpolation: LiquifyInterpolation,
    ) -> Result<Self, LiquifyExecutionError> {
        let field_pixels = usize::try_from(dimensions.pixel_count())
            .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)?;
        if let Some(version) = config.parameters.blocked_version() {
            return Err(LiquifyExecutionError::UnsupportedOpaquePayload(version));
        }
        let field_bytes = field_pixels
            .checked_mul(8)
            .ok_or(LiquifyExecutionError::ArithmeticOverflow)?;
        if field_bytes > MAX_FIELD_BYTES {
            return Err(LiquifyExecutionError::FieldTooLarge(field_bytes));
        }
        let stamps = expand_stamps(config.nodes())?;
        let mut field = Vec::with_capacity(field_pixels);
        let mut maximum_displacement = 0.0_f32;
        for y in 0..dimensions.height() {
            for x in 0..dimensions.width() {
                let target = LiquifyPoint::new(x as f32, y as f32)
                    .map_err(LiquifyExecutionError::Validation)?;
                let source = inverse_point(target, &stamps)?;
                maximum_displacement = maximum_displacement.max(target.distance(source));
                field.push(source);
            }
        }
        if !maximum_displacement.is_finite() || maximum_displacement > MAX_COORDINATE {
            return Err(LiquifyExecutionError::NonFiniteField);
        }
        let identity = plan_identity(&config, dimensions, interpolation, &field);
        Ok(Self {
            config,
            dimensions,
            interpolation,
            stamps,
            field,
            maximum_displacement: maximum_displacement.ceil() as u32,
            identity,
        })
    }

    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    pub const fn interpolation(&self) -> LiquifyInterpolation {
        self.interpolation
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub const fn maximum_displacement(&self) -> u32 {
        self.maximum_displacement
    }
    pub fn field(&self) -> &[LiquifyPoint] {
        &self.field
    }
    pub fn stamps(&self) -> usize {
        self.stamps.len()
    }
    pub fn config(&self) -> &LiquifyConfig {
        &self.config
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), LiquifyExecutionError> {
        transform_points(points, |point| forward_point(point, &self.stamps))
    }

    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), LiquifyExecutionError> {
        transform_points(points, |point| self.inverse_from_field(point))
    }

    pub fn input_roi(&self, output: Roi) -> Result<Roi, LiquifyExecutionError> {
        ensure_roi(output, self.dimensions)?;
        let support = self.interpolation.support();
        let margin = self.maximum_displacement.saturating_add(support);
        let x = output.x().saturating_sub(margin);
        let y = output.y().saturating_sub(margin);
        let right = output
            .right()
            .saturating_add(margin)
            .min(self.dimensions.width());
        let bottom = output
            .bottom()
            .saturating_add(margin)
            .min(self.dimensions.height());
        Roi::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
            .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)
    }

    pub fn gpu_dispatch(&self) -> Result<LiquifyGpuDispatch, LiquifyExecutionError> {
        let field_bytes = self
            .field
            .len()
            .checked_mul(8)
            .ok_or(LiquifyExecutionError::ArithmeticOverflow)?;
        Ok(LiquifyGpuDispatch {
            field_dimensions: self.dimensions,
            workgroups: (
                self.dimensions.width().div_ceil(8),
                self.dimensions.height().div_ceil(8),
            ),
            field_bytes,
            sampling_support: self.interpolation.support(),
            cpu_fallback: true,
        })
    }

    pub fn execute<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<LiquifyExecution, LiquifyExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(LiquifyExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        let mut output = Vec::with_capacity(expected);
        for y in 0..self.dimensions.height() {
            if cancelled() {
                return Err(LiquifyExecutionError::Cancelled);
            }
            for x in 0..self.dimensions.width() {
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| {
                        usize::try_from(self.dimensions.width())
                            .ok()
                            .and_then(|width| row.checked_mul(width))
                    })
                    .and_then(|row| {
                        usize::try_from(x)
                            .ok()
                            .and_then(|column| row.checked_add(column))
                    })
                    .ok_or(LiquifyExecutionError::ArithmeticOverflow)?;
                output.push(sample_rgb(
                    input,
                    self.dimensions,
                    self.field[index],
                    self.interpolation,
                )?);
            }
        }
        Ok(LiquifyExecution {
            pixels: output,
            identity: self.identity,
        })
    }

    pub fn execute_mask<F: Fn() -> bool>(
        &self,
        input: &[f32],
        cancelled: F,
    ) -> Result<Vec<f32>, LiquifyExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(LiquifyExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        if input.iter().any(|value| !value.is_finite()) {
            return Err(LiquifyExecutionError::NonFiniteMask);
        }
        let mut output = Vec::with_capacity(expected);
        for y in 0..self.dimensions.height() {
            if cancelled() {
                return Err(LiquifyExecutionError::Cancelled);
            }
            for x in 0..self.dimensions.width() {
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| {
                        usize::try_from(self.dimensions.width())
                            .ok()
                            .and_then(|width| row.checked_mul(width))
                    })
                    .and_then(|row| {
                        usize::try_from(x)
                            .ok()
                            .and_then(|column| row.checked_add(column))
                    })
                    .ok_or(LiquifyExecutionError::ArithmeticOverflow)?;
                output.push(sample_plane(input, self.dimensions, self.field[index]));
            }
        }
        Ok(output)
    }

    fn inverse_from_field(
        &self,
        point: LiquifyPoint,
    ) -> Result<LiquifyPoint, LiquifyExecutionError> {
        let x = point.x();
        let y = point.y();
        if x < 0.0
            || y < 0.0
            || x >= self.dimensions.width() as f32
            || y >= self.dimensions.height() as f32
        {
            return inverse_point(point, &self.stamps);
        }
        let x0 = x.floor() as u32;
        let y0 = y.floor() as u32;
        let x1 = (x0 + 1).min(self.dimensions.width().saturating_sub(1));
        let y1 = (y0 + 1).min(self.dimensions.height().saturating_sub(1));
        let index = |xx: u32, yy: u32| -> Result<usize, LiquifyExecutionError> {
            usize::try_from(yy * self.dimensions.width() + xx)
                .map_err(|_| LiquifyExecutionError::ArithmeticOverflow)
        };
        let top_left = self.field[index(x0, y0)?];
        let top_right = self.field[index(x1, y0)?];
        let bottom_left = self.field[index(x0, y1)?];
        let bottom_right = self.field[index(x1, y1)?];
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let top = LiquifyPoint::new(
            lerp(top_left.x(), top_right.x(), tx),
            lerp(top_left.y(), top_right.y(), tx),
        )
        .map_err(LiquifyExecutionError::Validation)?;
        let bottom = LiquifyPoint::new(
            lerp(bottom_left.x(), bottom_right.x(), tx),
            lerp(bottom_left.y(), bottom_right.y(), tx),
        )
        .map_err(LiquifyExecutionError::Validation)?;
        LiquifyPoint::new(lerp(top.x(), bottom.x(), ty), lerp(top.y(), bottom.y(), ty))
            .map_err(LiquifyExecutionError::Validation)
    }
}

pub const fn wgpu_passes() -> [&'static str; 2] {
    ["liquify.field", "liquify.resample"]
}
