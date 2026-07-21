use std::fmt;

/// A checked half-open rectangle in one logical pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl TileRect {
    /// Creates a rectangle. Zero width or height represents an explicit empty request.
    #[must_use]
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
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
    pub fn end_x(self) -> Option<u32> {
        self.x.checked_add(self.width)
    }

    #[must_use]
    pub fn end_y(self) -> Option<u32> {
        self.y.checked_add(self.height)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[must_use]
    pub fn area(self) -> Option<u64> {
        u64::from(self.width).checked_mul(u64::from(self.height))
    }

    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        let Some(self_end_x) = self.end_x() else {
            return false;
        };
        let Some(self_end_y) = self.end_y() else {
            return false;
        };
        let Some(other_end_x) = other.end_x() else {
            return false;
        };
        let Some(other_end_y) = other.end_y() else {
            return false;
        };
        other.x >= self.x
            && other.y >= self.y
            && other_end_x <= self_end_x
            && other_end_y <= self_end_y
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let end_x = self.end_x()?.min(other.end_x()?);
        let end_y = self.end_y()?.min(other.end_y()?);
        (x < end_x && y < end_y).then(|| Self::new(x, y, end_x - x, end_y - y))
    }

    #[must_use]
    pub fn expand(self, overlap: EdgeOverlap) -> Option<Self> {
        let x = self.x.checked_sub(overlap.left)?;
        let y = self.y.checked_sub(overlap.top)?;
        let width = self
            .width
            .checked_add(overlap.left)?
            .checked_add(overlap.right)?;
        let height = self
            .height
            .checked_add(overlap.top)?
            .checked_add(overlap.bottom)?;
        Some(Self::new(x, y, width, height))
    }
}

/// Per-edge overlap in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EdgeOverlap {
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
}

impl EdgeOverlap {
    /// Creates checked per-edge overlap values.
    #[must_use]
    pub const fn new(left: u32, top: u32, right: u32, bottom: u32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    #[must_use]
    pub const fn uniform(value: u32) -> Self {
        Self::new(value, value, value, value)
    }

    #[must_use]
    pub const fn left(self) -> u32 {
        self.left
    }

    #[must_use]
    pub const fn top(self) -> u32 {
        self.top
    }

    #[must_use]
    pub const fn right(self) -> u32 {
        self.right
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.bottom
    }

    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self::new(
            self.left.saturating_add(other.left),
            self.top.saturating_add(other.top),
            self.right.saturating_add(other.right),
            self.bottom.saturating_add(other.bottom),
        )
    }
}

/// Origin and internal extent alignment requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileAlignment {
    origin_x: u32,
    origin_y: u32,
    extent_x: u32,
    extent_y: u32,
}

impl TileAlignment {
    /// Creates nonzero alignment requirements.
    ///
    /// # Errors
    ///
    /// Returns an error when any alignment is zero.
    pub const fn new(
        origin_x: u32,
        origin_y: u32,
        extent_x: u32,
        extent_y: u32,
    ) -> Result<Self, GeometryError> {
        if origin_x == 0 || origin_y == 0 || extent_x == 0 || extent_y == 0 {
            return Err(GeometryError::ZeroAlignment);
        }
        Ok(Self {
            origin_x,
            origin_y,
            extent_x,
            extent_y,
        })
    }

    #[must_use]
    pub const fn origin_x(self) -> u32 {
        self.origin_x
    }

    #[must_use]
    pub const fn origin_y(self) -> u32 {
        self.origin_y
    }

    #[must_use]
    pub const fn extent_x(self) -> u32 {
        self.extent_x
    }

    #[must_use]
    pub const fn extent_y(self) -> u32 {
        self.extent_y
    }
}

/// Rational input-pixels-per-output-pixel scale used by an ROI adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScaleRatio {
    numerator: u32,
    denominator: u32,
}

impl ScaleRatio {
    /// Creates a positive rational scale.
    ///
    /// # Errors
    ///
    /// Returns an error when the numerator or denominator is zero.
    pub const fn new(numerator: u32, denominator: u32) -> Result<Self, GeometryError> {
        if numerator == 0 || denominator == 0 {
            return Err(GeometryError::InvalidScale);
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }

    #[must_use]
    fn floor_mul(self, value: u32) -> Option<u32> {
        u64::from(value)
            .checked_mul(u64::from(self.numerator))
            .map(|value| value / u64::from(self.denominator))
            .and_then(|value| u32::try_from(value).ok())
    }

    #[must_use]
    fn ceil_mul(self, value: u32) -> Option<u32> {
        let denominator = u64::from(self.denominator);
        u64::from(value)
            .checked_mul(u64::from(self.numerator))?
            .checked_add(denominator - 1)
            .map(|value| value / denominator)
            .and_then(|value| u32::try_from(value).ok())
    }
}

/// A bounded ROI stage supplied by the #267 ROI planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoiStage {
    operation_id: u128,
    input_bounds: TileRect,
    output_bounds: TileRect,
    input_per_output: ScaleRatio,
    overlap: EdgeOverlap,
}

impl RoiStage {
    /// Creates a stage mapping an output rectangle into its required input rectangle.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero operation ID or empty bounds.
    pub const fn new(
        operation_id: u128,
        input_bounds: TileRect,
        output_bounds: TileRect,
        input_per_output: ScaleRatio,
        overlap: EdgeOverlap,
    ) -> Result<Self, GeometryError> {
        if operation_id == 0 || input_bounds.is_empty() || output_bounds.is_empty() {
            return Err(GeometryError::InvalidRoiStage);
        }
        Ok(Self {
            operation_id,
            input_bounds,
            output_bounds,
            input_per_output,
            overlap,
        })
    }

    #[must_use]
    pub const fn operation_id(self) -> u128 {
        self.operation_id
    }

    #[must_use]
    pub const fn input_bounds(self) -> TileRect {
        self.input_bounds
    }

    #[must_use]
    pub const fn output_bounds(self) -> TileRect {
        self.output_bounds
    }

    #[must_use]
    pub const fn overlap(self) -> EdgeOverlap {
        self.overlap
    }

    /// Maps an output tile to a clipped, conservative input ROI.
    ///
    /// # Errors
    ///
    /// Returns an error when the output tile is outside the stage or checked geometry arithmetic overflows.
    pub fn required_input(self, output: TileRect) -> Result<TileRect, GeometryError> {
        if !self.output_bounds.contains(output) {
            return Err(GeometryError::RoiOutsideBounds);
        }
        let relative_x = output
            .x()
            .checked_sub(self.output_bounds.x())
            .ok_or(GeometryError::RoiOutsideBounds)?;
        let relative_y = output
            .y()
            .checked_sub(self.output_bounds.y())
            .ok_or(GeometryError::RoiOutsideBounds)?;
        let end_x = relative_x
            .checked_add(output.width())
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let end_y = relative_y
            .checked_add(output.height())
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let input_x = self
            .input_bounds
            .x()
            .checked_add(
                self.input_per_output
                    .floor_mul(relative_x)
                    .ok_or(GeometryError::ArithmeticOverflow)?,
            )
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let input_y = self
            .input_bounds
            .y()
            .checked_add(
                self.input_per_output
                    .floor_mul(relative_y)
                    .ok_or(GeometryError::ArithmeticOverflow)?,
            )
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let input_end_x = self
            .input_bounds
            .x()
            .checked_add(
                self.input_per_output
                    .ceil_mul(end_x)
                    .ok_or(GeometryError::ArithmeticOverflow)?,
            )
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let input_end_y = self
            .input_bounds
            .y()
            .checked_add(
                self.input_per_output
                    .ceil_mul(end_y)
                    .ok_or(GeometryError::ArithmeticOverflow)?,
            )
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let expanded_x = input_x.saturating_sub(self.overlap.left());
        let expanded_y = input_y.saturating_sub(self.overlap.top());
        let expanded_end_x = input_end_x
            .checked_add(self.overlap.right())
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let expanded_end_y = input_end_y
            .checked_add(self.overlap.bottom())
            .ok_or(GeometryError::ArithmeticOverflow)?;
        let expanded = TileRect::new(
            expanded_x,
            expanded_y,
            expanded_end_x
                .checked_sub(expanded_x)
                .ok_or(GeometryError::ArithmeticOverflow)?,
            expanded_end_y
                .checked_sub(expanded_y)
                .ok_or(GeometryError::ArithmeticOverflow)?,
        );
        expanded
            .intersect(self.input_bounds)
            .ok_or(GeometryError::RoiOutsideBounds)
    }
}

/// The immutable ROI chain produced by #267 and consumed by tile planning.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoiChain {
    input_bounds: TileRect,
    output_bounds: TileRect,
    stages: Vec<RoiStage>,
}

impl RoiChain {
    /// Creates a nonempty, ordered ROI chain.
    ///
    /// # Errors
    ///
    /// Returns an error when bounds or stage count are invalid, or the chain endpoints do not match.
    pub fn new(
        input_bounds: TileRect,
        output_bounds: TileRect,
        stages: Vec<RoiStage>,
    ) -> Result<Self, GeometryError> {
        if input_bounds.is_empty() || output_bounds.is_empty() || stages.is_empty() {
            return Err(GeometryError::InvalidRoiChain);
        }
        if stages
            .first()
            .is_none_or(|stage| stage.input_bounds() != input_bounds)
            || stages
                .last()
                .is_none_or(|stage| stage.output_bounds() != output_bounds)
        {
            return Err(GeometryError::InvalidRoiChain);
        }
        Ok(Self {
            input_bounds,
            output_bounds,
            stages,
        })
    }

    #[must_use]
    pub const fn input_bounds(&self) -> TileRect {
        self.input_bounds
    }

    #[must_use]
    pub const fn output_bounds(&self) -> TileRect {
        self.output_bounds
    }

    #[must_use]
    pub fn stages(&self) -> &[RoiStage] {
        &self.stages
    }

    /// Returns the required input ROI for every enabled node in order.
    ///
    /// # Errors
    ///
    /// Returns an error when the output tile is outside the chain or a stage cannot map it conservatively.
    pub fn required_inputs(
        &self,
        output: TileRect,
    ) -> Result<Vec<(u128, TileRect)>, GeometryError> {
        if !self.output_bounds.contains(output) {
            return Err(GeometryError::RoiOutsideBounds);
        }
        let mut current = output;
        let mut inputs = vec![TileRect::new(0, 0, 0, 0); self.stages.len()];
        for (index, stage) in self.stages.iter().enumerate().rev() {
            let input = stage.required_input(current)?;
            inputs[index] = input;
            current = input;
        }
        self.stages
            .iter()
            .zip(inputs)
            .map(|(stage, input)| Ok((stage.operation_id(), input)))
            .collect()
    }
}

/// Geometry rejection from the pure ROI adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    ZeroAlignment,
    InvalidScale,
    InvalidRoiStage,
    InvalidRoiChain,
    RoiOutsideBounds,
    ArithmeticOverflow,
}

impl fmt::Display for GeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroAlignment => "tile alignment must be nonzero",
            Self::InvalidScale => "ROI scale must be positive",
            Self::InvalidRoiStage => "ROI stage is invalid",
            Self::InvalidRoiChain => "ROI chain is invalid",
            Self::RoiOutsideBounds => "ROI is outside its declared bounds",
            Self::ArithmeticOverflow => "ROI arithmetic overflowed",
        })
    }
}

impl std::error::Error for GeometryError {}
