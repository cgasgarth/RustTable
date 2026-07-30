//! Dynamic neighborhood and tiled-ROI contract ported from Darktable's
//! `src/iop/sharpen.c` `tiling_callback()` and `process()` callbacks.
//!
//! The retained callback resolves the committed radius against
//! `roi_in.scale / piece->iscale`, rounds it upward, caps it at `MAXR`, and
//! requests that many pixels of overlap. GPU execution deliberately remains
//! unavailable in this CPU-only milestone.

use std::fmt;

/// Native `MAXR` support cap.
pub const SHARPEN_MAX_RADIUS: u32 = 12;
/// `commit_params()` expands the UI radius to contain 2.5 sigma.
pub const SHARPEN_RADIUS_MULTIPLIER: f32 = 2.5;
/// Native CPU tiling factor: input + output + one temporary row.
pub const SHARPEN_CPU_MEMORY_FACTOR: f32 = 2.1;
pub const SHARPEN_MAX_BUFFER_FACTOR: f32 = 1.0;
pub const SHARPEN_TILING_OVERHEAD_BYTES: usize = 0;
pub const SHARPEN_TILE_ALIGNMENT: u32 = 1;

/// One checked rectangle in the image's logical pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharpenRoi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl SharpenRoi {
    /// Creates a nonempty ROI after checking its half-open endpoints.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, SharpenTilingError> {
        if width == 0 || height == 0 {
            return Err(SharpenTilingError::EmptyRoi);
        }
        x.checked_add(width)
            .ok_or(SharpenTilingError::ArithmeticOverflow)?;
        y.checked_add(height)
            .ok_or(SharpenTilingError::ArithmeticOverflow)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
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

    fn end_x(self) -> u32 {
        self.x + self.width
    }

    fn end_y(self) -> u32 {
        self.y + self.height
    }
}

/// Immutable native-equivalent radius and tiling requirements for one ROI scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenTilingPlan {
    parameter_radius: f32,
    committed_radius: f32,
    roi_scale: f32,
    input_scale: f32,
    radius: u32,
}

impl SharpenTilingPlan {
    /// Resolves the user-facing radius through `commit_params()` and
    /// `tiling_callback()`.
    pub fn new(
        parameter_radius: f32,
        roi_scale: f32,
        input_scale: f32,
    ) -> Result<Self, SharpenTilingError> {
        if !parameter_radius.is_finite() {
            return Err(SharpenTilingError::NonFiniteRadius);
        }
        if !(0.0..=99.0).contains(&parameter_radius) {
            return Err(SharpenTilingError::RadiusOutOfRange);
        }
        let committed_radius = parameter_radius * SHARPEN_RADIUS_MULTIPLIER;
        Self::resolve(parameter_radius, committed_radius, roi_scale, input_scale)
    }

    /// Resolves an already-committed radius. This is the direct seam used by
    /// CPU execution after parameters have been committed.
    pub fn from_committed_radius(
        committed_radius: f32,
        roi_scale: f32,
        input_scale: f32,
    ) -> Result<Self, SharpenTilingError> {
        if !committed_radius.is_finite() {
            return Err(SharpenTilingError::NonFiniteRadius);
        }
        if !(0.0..=99.0 * SHARPEN_RADIUS_MULTIPLIER).contains(&committed_radius) {
            return Err(SharpenTilingError::RadiusOutOfRange);
        }
        Self::resolve(
            committed_radius / SHARPEN_RADIUS_MULTIPLIER,
            committed_radius,
            roi_scale,
            input_scale,
        )
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn resolve(
        parameter_radius: f32,
        committed_radius: f32,
        roi_scale: f32,
        input_scale: f32,
    ) -> Result<Self, SharpenTilingError> {
        if !roi_scale.is_finite() || roi_scale <= 0.0 {
            return Err(SharpenTilingError::InvalidRoiScale);
        }
        if !input_scale.is_finite() || input_scale <= 0.0 {
            return Err(SharpenTilingError::InvalidInputScale);
        }

        // Preserve sharpen.c's operation order: d->radius * roi_in->scale /
        // piece->iscale, followed by ceilf and MIN(MAXR, ...).
        let scaled_radius = committed_radius * roi_scale / input_scale;
        if !scaled_radius.is_finite() {
            return Err(SharpenTilingError::ArithmeticOverflow);
        }
        let radius = if scaled_radius >= 12.0 {
            SHARPEN_MAX_RADIUS
        } else {
            // The validated value is finite, nonnegative, and below 12.
            scaled_radius.ceil() as u32
        };

        Ok(Self {
            parameter_radius,
            committed_radius,
            roi_scale,
            input_scale,
            radius,
        })
    }

    #[must_use]
    pub const fn parameter_radius(self) -> f32 {
        self.parameter_radius
    }

    #[must_use]
    pub const fn committed_radius(self) -> f32 {
        self.committed_radius
    }

    #[must_use]
    pub const fn roi_scale(self) -> f32 {
        self.roi_scale
    }

    #[must_use]
    pub const fn input_scale(self) -> f32 {
        self.input_scale
    }

    /// Quantized native support radius and per-edge tile overlap.
    #[must_use]
    pub const fn radius(self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn overlap(self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn kernel_width(self) -> u32 {
        self.radius * 2 + 1
    }

    /// Mirrors `process()`'s pass-through cases for the supplied processing
    /// buffer dimensions.
    #[must_use]
    pub const fn is_identity_for(self, width: u32, height: u32) -> bool {
        self.radius == 0 || width < self.kernel_width() || height < self.kernel_width()
    }

    /// Expands an output tile by the dynamic overlap and clips it to the full
    /// image. The returned crop offset identifies the requested output inside
    /// the expanded input buffer.
    pub fn tile(
        self,
        full_width: u32,
        full_height: u32,
        output: SharpenRoi,
    ) -> Result<SharpenTile, SharpenTilingError> {
        if full_width == 0 || full_height == 0 {
            return Err(SharpenTilingError::InvalidImageDimensions);
        }
        if output.end_x() > full_width || output.end_y() > full_height {
            return Err(SharpenTilingError::RoiOutsideImage);
        }

        let input_x = output.x.saturating_sub(self.radius);
        let input_y = output.y.saturating_sub(self.radius);
        let input_end_x = output
            .end_x()
            .checked_add(self.radius)
            .ok_or(SharpenTilingError::ArithmeticOverflow)?
            .min(full_width);
        let input_end_y = output
            .end_y()
            .checked_add(self.radius)
            .ok_or(SharpenTilingError::ArithmeticOverflow)?
            .min(full_height);
        let input = SharpenRoi::new(
            input_x,
            input_y,
            input_end_x - input_x,
            input_end_y - input_y,
        )?;

        Ok(SharpenTile {
            output,
            input,
            crop_x: output.x - input_x,
            crop_y: output.y - input_y,
        })
    }
}

/// Expanded input and crop geometry for one output tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharpenTile {
    output: SharpenRoi,
    input: SharpenRoi,
    crop_x: u32,
    crop_y: u32,
}

impl SharpenTile {
    #[must_use]
    pub const fn output(self) -> SharpenRoi {
        self.output
    }

    #[must_use]
    pub const fn input(self) -> SharpenRoi {
        self.input
    }

    #[must_use]
    pub const fn crop_x(self) -> u32 {
        self.crop_x
    }

    #[must_use]
    pub const fn crop_y(self) -> u32 {
        self.crop_y
    }

    /// Whether the native processor must pass this expanded input through.
    #[must_use]
    pub const fn is_identity(self, plan: SharpenTilingPlan) -> bool {
        plan.is_identity_for(self.input.width, self.input.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenTilingError {
    NonFiniteRadius,
    RadiusOutOfRange,
    InvalidRoiScale,
    InvalidInputScale,
    InvalidImageDimensions,
    EmptyRoi,
    RoiOutsideImage,
    ArithmeticOverflow,
}

impl fmt::Display for SharpenTilingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteRadius => "sharpen radius must be finite",
            Self::RadiusOutOfRange => "sharpen radius is outside the native parameter range",
            Self::InvalidRoiScale => "sharpen ROI scale must be finite and positive",
            Self::InvalidInputScale => "sharpen input scale must be finite and positive",
            Self::InvalidImageDimensions => "sharpen image dimensions must be nonzero",
            Self::EmptyRoi => "sharpen ROI must be nonempty",
            Self::RoiOutsideImage => "sharpen ROI is outside the full image",
            Self::ArithmeticOverflow => "sharpen tiling arithmetic overflowed",
        })
    }
}

impl std::error::Error for SharpenTilingError {}
