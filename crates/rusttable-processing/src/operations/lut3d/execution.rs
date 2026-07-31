//! Cancellation-aware transactional CPU execution ported from `process` in
//! `src/iop/lut3d.c`.
//!
//! The operation accepts an explicit profile context.  Shared ICC profile
//! acquisition, operation routing, blending, tiling, GPU dispatch, and UI
//! availability are intentionally deferred.

use std::fmt;

use super::codec::{Lut3dColorspace, Lut3dInterpolation, Lut3dParameters};
use super::parser::{Lut3d, Lut3dParseError};
use super::profile::{Lut3dProfileContext, Lut3dProfileError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameDimensions {
    pub width: usize,
    pub height: usize,
}

impl FrameDimensions {
    pub fn new(width: usize, height: usize) -> Result<Self, Lut3dExecutionError> {
        if width == 0 || height == 0 || width.checked_mul(height).is_none() {
            return Err(Lut3dExecutionError::InvalidDimensions { width, height });
        }
        Ok(Self { width, height })
    }

    fn pixels(self) -> Result<usize, Lut3dExecutionError> {
        self.width
            .checked_mul(self.height)
            .ok_or(Lut3dExecutionError::InvalidDimensions {
                width: self.width,
                height: self.height,
            })
    }
}

#[derive(Debug, Clone)]
pub struct Lut3dPlan {
    lut: Lut3d,
    colorspace: Lut3dColorspace,
    interpolation: Lut3dInterpolation,
}

impl Lut3dPlan {
    #[must_use]
    pub fn new(lut: Lut3d, colorspace: Lut3dColorspace, interpolation: Lut3dInterpolation) -> Self {
        Self {
            lut,
            colorspace,
            interpolation,
        }
    }

    /// Builds a plan from v3 parameters only for an already-loaded text LUT.
    /// Compressed keypoints are not decoded by this leaf.
    pub fn from_parameters(
        lut: Lut3d,
        parameters: &Lut3dParameters,
    ) -> Result<Self, Lut3dExecutionError> {
        if parameters.nb_keypoints > 0 {
            return Err(Lut3dExecutionError::CompressedLutUnsupported);
        }
        Ok(Self::new(
            lut,
            parameters.colorspace,
            parameters.interpolation,
        ))
    }

    #[must_use]
    pub const fn lut(&self) -> &Lut3d {
        &self.lut
    }

    #[must_use]
    pub const fn colorspace(&self) -> Lut3dColorspace {
        self.colorspace
    }

    #[must_use]
    pub const fn interpolation(&self) -> Lut3dInterpolation {
        self.interpolation
    }

    pub fn execute(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: Option<&Lut3dProfileContext>,
    ) -> Result<Vec<[f32; 4]>, Lut3dExecutionError> {
        self.execute_with_cancellation(input, dimensions, profile, || false)
    }

    /// Every output is written into a private vector.  Cancellation returns an
    /// error before that vector is exposed, so no partial pixels are published.
    pub fn execute_with_cancellation<F>(
        &self,
        input: &[[f32; 4]],
        dimensions: FrameDimensions,
        profile: Option<&Lut3dProfileContext>,
        mut cancelled: F,
    ) -> Result<Vec<[f32; 4]>, Lut3dExecutionError>
    where
        F: FnMut() -> bool,
    {
        let pixel_count = dimensions.pixels()?;
        if input.len() != pixel_count {
            return Err(Lut3dExecutionError::InputLength {
                expected: pixel_count,
                actual: input.len(),
            });
        }
        let profile = profile.ok_or(Lut3dExecutionError::MissingProfileContext)?;
        profile.validate()?;
        if profile.colorspace() != self.colorspace {
            return Err(Lut3dExecutionError::ProfileMismatch {
                expected: self.colorspace,
                actual: profile.colorspace(),
            });
        }
        validate_input(input, &mut cancelled)?;
        if cancelled() {
            return Err(Lut3dExecutionError::Cancelled);
        }

        let mut output = Vec::new();
        output
            .try_reserve_exact(pixel_count)
            .map_err(|_| Lut3dExecutionError::AllocationFailure)?;
        for pixel in input.iter().copied() {
            if cancelled() {
                return Err(Lut3dExecutionError::Cancelled);
            }
            let lut_rgb = profile.transform_working_to_lut([pixel[0], pixel[1], pixel[2]]);
            let sampled = self.lut.sample(
                [lut_rgb[0], lut_rgb[1], lut_rgb[2], pixel[3]],
                self.interpolation,
            );
            let working_rgb =
                profile.transform_lut_to_working([sampled[0], sampled[1], sampled[2]]);
            output.push([working_rgb[0], working_rgb[1], working_rgb[2], pixel[3]]);
        }
        if cancelled() {
            return Err(Lut3dExecutionError::Cancelled);
        }
        Ok(output)
    }
}

fn validate_input<F>(input: &[[f32; 4]], cancelled: &mut F) -> Result<(), Lut3dExecutionError>
where
    F: FnMut() -> bool,
{
    for (index, pixel) in input.iter().enumerate() {
        if cancelled() {
            return Err(Lut3dExecutionError::Cancelled);
        }
        for (channel, value) in pixel.iter().copied().enumerate() {
            if !value.is_finite() {
                return Err(Lut3dExecutionError::NonFiniteInput { index, channel });
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum Lut3dExecutionError {
    InvalidDimensions {
        width: usize,
        height: usize,
    },
    InputLength {
        expected: usize,
        actual: usize,
    },
    MissingProfileContext,
    ProfileMismatch {
        expected: Lut3dColorspace,
        actual: Lut3dColorspace,
    },
    NonFiniteInput {
        index: usize,
        channel: usize,
    },
    CompressedLutUnsupported,
    AllocationFailure,
    Cancelled,
    Profile(Lut3dProfileError),
    Parse(Lut3dParseError),
}

impl fmt::Display for Lut3dExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(formatter, "invalid LUT3D frame dimensions {width}x{height}")
            }
            Self::InputLength { expected, actual } => {
                write!(
                    formatter,
                    "LUT3D input has {actual} pixels; expected {expected}"
                )
            }
            Self::MissingProfileContext => {
                formatter.write_str("LUT3D requires explicit profile evidence")
            }
            Self::ProfileMismatch { expected, actual } => {
                write!(
                    formatter,
                    "LUT3D profile is {actual:?}; expected {expected:?}"
                )
            }
            Self::NonFiniteInput { index, channel } => {
                write!(
                    formatter,
                    "LUT3D input pixel {index} channel {channel} is non-finite"
                )
            }
            Self::CompressedLutUnsupported => {
                formatter.write_str("compressed GMIC LUTs are unavailable in this CPU leaf")
            }
            Self::AllocationFailure => formatter.write_str("LUT3D output allocation failed"),
            Self::Cancelled => formatter.write_str("LUT3D execution was cancelled"),
            Self::Profile(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Lut3dExecutionError {}

impl From<Lut3dProfileError> for Lut3dExecutionError {
    fn from(error: Lut3dProfileError) -> Self {
        Self::Profile(error)
    }
}

impl From<Lut3dParseError> for Lut3dExecutionError {
    fn from(error: Lut3dParseError) -> Self {
        Self::Parse(error)
    }
}
