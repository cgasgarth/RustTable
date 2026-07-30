use std::{fmt, num::NonZeroU64, ops::Range};

use crate::{EdgeOverlap, RoiChain, TileAlignment};

/// Backend whose immutable requirements and budget are being planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TileBackend {
    Cpu,
    Gpu,
}

impl TileBackend {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// A resource included in a candidate estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKind {
    Input,
    Output,
    Temporary,
    Mask,
    Blend,
    Analysis,
    Transfer,
    Command,
    Fixed,
}

impl ResourceKind {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Temporary => "temporary",
            Self::Mask => "mask",
            Self::Blend => "blend",
            Self::Analysis => "analysis",
            Self::Transfer => "transfer",
            Self::Command => "command",
            Self::Fixed => "fixed",
        }
    }
}

/// A nonnegative, checked resource multiplier. Unknown and unbounded remain
/// typed so they cannot silently become a finite estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MemoryFactor {
    Rational {
        numerator: u64,
        denominator: NonZeroU64,
    },
    Float(f64),
    Unknown,
    Unbounded,
}

impl MemoryFactor {
    /// Creates a checked rational factor.
    ///
    /// # Errors
    ///
    /// Returns an error when the denominator is zero.
    pub fn rational(numerator: u64, denominator: u64) -> Result<Self, RequirementError> {
        let denominator = NonZeroU64::new(denominator).ok_or(RequirementError::ZeroDenominator)?;
        let divisor = gcd(numerator, denominator.get());
        let denominator = NonZeroU64::new(denominator.get() / divisor)
            .ok_or(RequirementError::ZeroDenominator)?;
        Ok(Self::Rational {
            numerator: numerator / divisor,
            denominator,
        })
    }

    /// Creates a checked finite f64 factor.
    ///
    /// # Errors
    ///
    /// Returns an error for NaN, infinity, or a negative value.
    pub fn from_f64(value: f64) -> Result<Self, RequirementError> {
        if !value.is_finite() || value < 0.0 {
            return Err(RequirementError::InvalidFactor);
        }
        Ok(Self::Float(value))
    }

    #[must_use]
    pub const fn one() -> Self {
        Self::Rational {
            numerator: 1,
            denominator: NonZeroU64::MIN,
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub(crate) fn checked_bytes(self, base: u64) -> Result<u64, EstimateError> {
        match self {
            Self::Rational {
                numerator,
                denominator,
            } => base
                .checked_mul(numerator)
                .and_then(|value| value.checked_add(denominator.get() - 1))
                .map(|value| value / denominator.get())
                .ok_or(EstimateError::ArithmeticOverflow),
            Self::Float(value) => {
                let bytes = (base as f64) * value;
                if !bytes.is_finite() || bytes > u64::MAX as f64 {
                    return Err(EstimateError::ArithmeticOverflow);
                }
                Ok(bytes.ceil() as u64)
            }
            Self::Unknown => Err(EstimateError::UnknownResource),
            Self::Unbounded => Err(EstimateError::UnboundedResource),
        }
    }
}

/// Checked bytes-per-pixel descriptor for one live buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BytesPerPixel(u64);

impl BytesPerPixel {
    /// Creates a nonzero bytes-per-pixel descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is zero.
    pub const fn new(value: u64) -> Result<Self, RequirementError> {
        if value == 0 {
            return Err(RequirementError::ZeroBytesPerPixel);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One resource plane and the factor by which it is live for a candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferRequirement {
    kind: ResourceKind,
    bytes_per_pixel: BytesPerPixel,
    factor: MemoryFactor,
    area: AreaSource,
}

impl BufferRequirement {
    /// Creates a resource whose area is chosen from the candidate tile.
    #[must_use]
    pub const fn new(
        kind: ResourceKind,
        bytes_per_pixel: BytesPerPixel,
        factor: MemoryFactor,
        area: AreaSource,
    ) -> Self {
        Self {
            kind,
            bytes_per_pixel,
            factor,
            area,
        }
    }

    #[must_use]
    pub const fn kind(self) -> ResourceKind {
        self.kind
    }

    #[must_use]
    pub const fn bytes_per_pixel(self) -> BytesPerPixel {
        self.bytes_per_pixel
    }

    #[must_use]
    pub const fn factor(self) -> MemoryFactor {
        self.factor
    }

    #[must_use]
    pub const fn area(self) -> AreaSource {
        self.area
    }
}

/// Candidate area used by an auxiliary resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AreaSource {
    Input,
    Output,
    Larger,
}

/// Full-frame constraints attached to one backend record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FullFrameRequirements {
    full_input: bool,
    full_output: bool,
    full_pipeline: bool,
}

impl FullFrameRequirements {
    /// Creates full-frame flags for input, output, and the complete pipeline.
    #[must_use]
    pub const fn new(full_input: bool, full_output: bool, full_pipeline: bool) -> Self {
        Self {
            full_input,
            full_output,
            full_pipeline,
        }
    }

    #[must_use]
    pub const fn full_input(self) -> bool {
        self.full_input
    }

    #[must_use]
    pub const fn full_output(self) -> bool {
        self.full_output
    }

    #[must_use]
    pub const fn full_pipeline(self) -> bool {
        self.full_pipeline
    }

    #[must_use]
    pub const fn requires_full_frame(self) -> bool {
        self.full_input || self.full_output || self.full_pipeline
    }
}

/// All immutable memory and geometry requirements for one backend.
#[derive(Debug, Clone, PartialEq)]
pub struct BackendRequirement {
    input: BufferRequirement,
    output: BufferRequirement,
    temporary: BufferRequirement,
    auxiliary: Vec<BufferRequirement>,
    fixed_bytes: u64,
    overlap: EdgeOverlap,
    alignment: TileAlignment,
    dimensions: TileDimensions,
    full_frame: FullFrameRequirements,
    max_allocation: Option<u64>,
}

impl BackendRequirement {
    /// Creates the required input/output/temporary records for one backend.
    #[must_use]
    pub const fn new(
        input: BufferRequirement,
        output: BufferRequirement,
        temporary: BufferRequirement,
        fixed_bytes: u64,
        overlap: EdgeOverlap,
        alignment: TileAlignment,
        dimensions: TileDimensions,
    ) -> Self {
        Self {
            input,
            output,
            temporary,
            auxiliary: Vec::new(),
            fixed_bytes,
            overlap,
            alignment,
            dimensions,
            full_frame: FullFrameRequirements::new(false, false, false),
            max_allocation: None,
        }
    }

    #[must_use]
    pub fn with_auxiliary(mut self, requirement: BufferRequirement) -> Self {
        self.auxiliary.push(requirement);
        self
    }

    #[must_use]
    pub const fn with_full_frame(mut self, full_frame: FullFrameRequirements) -> Self {
        self.full_frame = full_frame;
        self
    }

    #[must_use]
    pub const fn with_max_allocation(mut self, bytes: u64) -> Self {
        self.max_allocation = Some(bytes);
        self
    }

    #[must_use]
    pub const fn input(&self) -> BufferRequirement {
        self.input
    }

    #[must_use]
    pub const fn output(&self) -> BufferRequirement {
        self.output
    }

    #[must_use]
    pub const fn temporary(&self) -> BufferRequirement {
        self.temporary
    }

    #[must_use]
    pub fn auxiliary(&self) -> &[BufferRequirement] {
        &self.auxiliary
    }

    #[must_use]
    pub const fn fixed_bytes(&self) -> u64 {
        self.fixed_bytes
    }

    #[must_use]
    pub const fn overlap(&self) -> EdgeOverlap {
        self.overlap
    }

    #[must_use]
    pub const fn alignment(&self) -> TileAlignment {
        self.alignment
    }

    #[must_use]
    pub const fn dimensions(&self) -> TileDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn full_frame(&self) -> FullFrameRequirements {
        self.full_frame
    }

    #[must_use]
    pub const fn max_allocation(&self) -> Option<u64> {
        self.max_allocation
    }
}

/// Minimum, preferred, and maximum output tile dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileDimensions {
    minimum_width: u32,
    minimum_height: u32,
    preferred_width: u32,
    preferred_height: u32,
    maximum_width: u32,
    maximum_height: u32,
}

impl TileDimensions {
    /// Creates checked tile dimension policy.
    ///
    /// # Errors
    ///
    /// Returns an error when minimum, preferred, or maximum dimensions conflict.
    pub const fn new(
        minimum_width: u32,
        minimum_height: u32,
        preferred_width: u32,
        preferred_height: u32,
        maximum_width: u32,
        maximum_height: u32,
    ) -> Result<Self, RequirementError> {
        if minimum_width == 0
            || minimum_height == 0
            || preferred_width < minimum_width
            || preferred_height < minimum_height
            || maximum_width < preferred_width
            || maximum_height < preferred_height
        {
            return Err(RequirementError::InvalidDimensions);
        }
        Ok(Self {
            minimum_width,
            minimum_height,
            preferred_width,
            preferred_height,
            maximum_width,
            maximum_height,
        })
    }

    #[must_use]
    pub const fn minimum_width(self) -> u32 {
        self.minimum_width
    }
    #[must_use]
    pub const fn minimum_height(self) -> u32 {
        self.minimum_height
    }
    #[must_use]
    pub const fn preferred_width(self) -> u32 {
        self.preferred_width
    }
    #[must_use]
    pub const fn preferred_height(self) -> u32 {
        self.preferred_height
    }
    #[must_use]
    pub const fn maximum_width(self) -> u32 {
        self.maximum_width
    }
    #[must_use]
    pub const fn maximum_height(self) -> u32 {
        self.maximum_height
    }
}

/// One enabled operation's immutable CPU/GPU tile records.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRequirement {
    operation_id: u128,
    operation_name: String,
    cpu: BackendRequirement,
    gpu: Option<BackendRequirement>,
}

impl NodeRequirement {
    /// Creates a requirement with a stable operation instance identity.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero instance ID or an empty/oversized operation name.
    pub fn new(
        operation_id: u128,
        operation_name: impl Into<String>,
        cpu: BackendRequirement,
        gpu: Option<BackendRequirement>,
    ) -> Result<Self, RequirementError> {
        let operation_name = operation_name.into();
        if operation_id == 0 || operation_name.is_empty() || operation_name.len() > 128 {
            return Err(RequirementError::InvalidOperationIdentity);
        }
        Ok(Self {
            operation_id,
            operation_name,
            cpu,
            gpu,
        })
    }

    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }

    #[must_use]
    pub fn operation_name(&self) -> &str {
        &self.operation_name
    }

    #[must_use]
    pub const fn cpu(&self) -> &BackendRequirement {
        &self.cpu
    }

    #[must_use]
    pub const fn gpu(&self) -> Option<&BackendRequirement> {
        self.gpu.as_ref()
    }

    #[must_use]
    pub fn backend(&self, backend: TileBackend) -> Option<&BackendRequirement> {
        match backend {
            TileBackend::Cpu => Some(&self.cpu),
            TileBackend::Gpu => self.gpu.as_ref(),
        }
    }
}

/// Ordered, exact enabled-node range consumed by the planner.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRequirements {
    nodes: Vec<NodeRequirement>,
    enabled_range: Range<usize>,
    roi_chain: RoiChain,
}

impl NodeRequirements {
    /// Creates a requirement set and binds it to the exact #267 ROI chain.
    ///
    /// # Errors
    ///
    /// Returns an error when the enabled range is empty, out of bounds, or does not match the ROI stages.
    pub fn new(
        nodes: Vec<NodeRequirement>,
        enabled_range: Range<usize>,
        roi_chain: RoiChain,
    ) -> Result<Self, RequirementError> {
        if enabled_range.start > enabled_range.end
            || enabled_range.end > nodes.len()
            || enabled_range.is_empty()
            || roi_chain.stages().len() != enabled_range.len()
        {
            return Err(RequirementError::InvalidNodeRange);
        }
        Ok(Self {
            nodes,
            enabled_range,
            roi_chain,
        })
    }

    #[must_use]
    pub fn nodes(&self) -> &[NodeRequirement] {
        &self.nodes
    }

    #[must_use]
    pub const fn enabled_range(&self) -> &Range<usize> {
        &self.enabled_range
    }

    #[must_use]
    pub const fn roi_chain(&self) -> &RoiChain {
        &self.roi_chain
    }

    pub fn enabled_nodes(&self) -> impl Iterator<Item = (usize, &NodeRequirement)> {
        self.enabled_range
            .clone()
            .map(|index| (index, &self.nodes[index]))
    }

    /// Returns true when any enabled node requires a whole-frame boundary.
    #[must_use]
    pub fn requires_full_frame(&self, backend: TileBackend) -> bool {
        self.enabled_nodes().any(|(_, node)| {
            node.backend(backend)
                .is_some_and(|requirement| requirement.full_frame().requires_full_frame())
        })
    }
}

/// Typed validation failures before candidate planning begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequirementError {
    ZeroDenominator,
    InvalidFactor,
    ZeroBytesPerPixel,
    InvalidDimensions,
    InvalidOperationIdentity,
    InvalidNodeRange,
}

impl fmt::Display for RequirementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroDenominator => "memory factor denominator must be nonzero",
            Self::InvalidFactor => "memory factor must be finite and nonnegative",
            Self::ZeroBytesPerPixel => "bytes per pixel must be nonzero",
            Self::InvalidDimensions => "tile dimensions are contradictory",
            Self::InvalidOperationIdentity => "operation identity is invalid",
            Self::InvalidNodeRange => "enabled node range does not match the ROI chain",
        })
    }
}

impl std::error::Error for RequirementError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateError {
    UnknownResource,
    UnboundedResource,
    ArithmeticOverflow,
}

impl fmt::Display for EstimateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnknownResource => "a required resource factor is unknown",
            Self::UnboundedResource => "a required resource is unbounded",
            Self::ArithmeticOverflow => "resource estimate arithmetic overflowed",
        })
    }
}

impl std::error::Error for EstimateError {}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}
