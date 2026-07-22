use std::fmt;

use rusttable_core::{RenderSizeError, RenderSizeRequest};
use rusttable_image::ImageDimensions;
use rusttable_processing::operations::finalscale::FinalScaleKernel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewBounds {
    max_width: u32,
    max_height: u32,
}

impl PreviewBounds {
    /// Creates nonzero maximum preview dimensions.
    ///
    /// # Errors
    ///
    /// Returns a distinct error for a zero width or height.
    pub const fn new(max_width: u32, max_height: u32) -> Result<Self, PreviewBoundsError> {
        if max_width == 0 {
            return Err(PreviewBoundsError::ZeroWidth);
        }
        if max_height == 0 {
            return Err(PreviewBoundsError::ZeroHeight);
        }
        Ok(Self {
            max_width,
            max_height,
        })
    }

    #[must_use]
    pub const fn max_width(self) -> u32 {
        self.max_width
    }

    #[must_use]
    pub const fn max_height(self) -> u32 {
        self.max_height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewBoundsError {
    ZeroWidth,
    ZeroHeight,
}

impl fmt::Display for PreviewBoundsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroWidth => "preview width must be nonzero",
            Self::ZeroHeight => "preview height must be nonzero",
        })
    }
}

impl std::error::Error for PreviewBoundsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderTarget {
    FullResolution,
    PreviewFit(PreviewBounds),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderSampling {
    Identity,
    Filtered,
}

/// Border policy used by the preview resampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderBorderPolicy {
    Reflect,
}

/// Alpha policy used by filtered preview resampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderAlphaPolicy {
    Premultiplied,
}

/// Explicit preview resampling policy carried by every non-identity plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderResampling {
    filter: FinalScaleKernel,
    border: RenderBorderPolicy,
    alpha: RenderAlphaPolicy,
}

impl RenderResampling {
    #[must_use]
    pub const fn preview() -> Self {
        Self {
            filter: FinalScaleKernel::Bicubic,
            border: RenderBorderPolicy::Reflect,
            alpha: RenderAlphaPolicy::Premultiplied,
        }
    }

    #[must_use]
    pub const fn new(
        filter: FinalScaleKernel,
        border: RenderBorderPolicy,
        alpha: RenderAlphaPolicy,
    ) -> Self {
        Self {
            filter,
            border,
            alpha,
        }
    }

    #[must_use]
    pub const fn filter(self) -> FinalScaleKernel {
        self.filter
    }

    #[must_use]
    pub const fn support(self) -> u32 {
        self.filter.support()
    }

    #[must_use]
    pub const fn border(self) -> RenderBorderPolicy {
        self.border
    }

    #[must_use]
    pub const fn alpha(self) -> RenderAlphaPolicy {
        self.alpha
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderPlan {
    source_dimensions: ImageDimensions,
    target: RenderTarget,
    output_dimensions: ImageDimensions,
    sampling: RenderSampling,
    resampling: Option<RenderResampling>,
}

impl RenderPlan {
    /// Builds a render plan from the shared export/render size contract.
    ///
    /// # Errors
    ///
    /// Returns an error when the source dimensions or requested output size is invalid.
    pub fn for_source_with_size_request(
        source_dimensions: ImageDimensions,
        request: RenderSizeRequest,
    ) -> Result<Self, RenderSizeError> {
        let (width, height) =
            request.resolve(source_dimensions.width(), source_dimensions.height())?;
        let output_dimensions =
            ImageDimensions::new(width, height).map_err(|_| RenderSizeError::ArithmeticOverflow)?;
        let target = if output_dimensions == source_dimensions {
            RenderTarget::FullResolution
        } else {
            PreviewBounds::new(width, height)
                .map(RenderTarget::PreviewFit)
                .map_err(|_| RenderSizeError::ArithmeticOverflow)?
        };
        Ok(Self {
            source_dimensions,
            target,
            output_dimensions,
            sampling: if output_dimensions == source_dimensions {
                RenderSampling::Identity
            } else {
                RenderSampling::Filtered
            },
            resampling: (output_dimensions != source_dimensions)
                .then_some(RenderResampling::preview()),
        })
    }

    #[must_use]
    pub fn for_source(source_dimensions: ImageDimensions, target: RenderTarget) -> Self {
        let output_dimensions = match target {
            RenderTarget::FullResolution => source_dimensions,
            RenderTarget::PreviewFit(bounds) => fit_dimensions(source_dimensions, bounds),
        };
        let sampling = if output_dimensions == source_dimensions {
            RenderSampling::Identity
        } else {
            RenderSampling::Filtered
        };
        Self {
            source_dimensions,
            target,
            output_dimensions,
            sampling,
            resampling: (output_dimensions != source_dimensions)
                .then_some(RenderResampling::preview()),
        }
    }

    #[must_use]
    pub const fn source_dimensions(self) -> ImageDimensions {
        self.source_dimensions
    }

    #[must_use]
    pub const fn target(self) -> RenderTarget {
        self.target
    }

    #[must_use]
    pub const fn output_dimensions(self) -> ImageDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn sampling(self) -> RenderSampling {
        self.sampling
    }

    #[must_use]
    pub const fn resampling(self) -> Option<RenderResampling> {
        self.resampling
    }

    /// Horizontal output-to-source scale propagated to scale-sensitive work.
    #[must_use]
    pub fn scale_x(self) -> f64 {
        f64::from(self.output_dimensions.width()) / f64::from(self.source_dimensions.width())
    }

    /// Vertical output-to-source scale propagated to scale-sensitive work.
    #[must_use]
    pub fn scale_y(self) -> f64 {
        f64::from(self.output_dimensions.height()) / f64::from(self.source_dimensions.height())
    }
}

fn fit_dimensions(source: ImageDimensions, bounds: PreviewBounds) -> ImageDimensions {
    if source.width() <= bounds.max_width() && source.height() <= bounds.max_height() {
        return source;
    }
    let source_width = u64::from(source.width());
    let source_height = u64::from(source.height());
    let bound_width = u64::from(bounds.max_width());
    let bound_height = u64::from(bounds.max_height());
    let (width, height) = if source_width * bound_height >= source_height * bound_width {
        (
            bound_width,
            (source_height * bound_width / source_width).max(1),
        )
    } else {
        (
            (source_width * bound_height / source_height).max(1),
            bound_height,
        )
    };
    ImageDimensions::new(
        u32::try_from(width).expect("preview bound width fits u32"),
        u32::try_from(height).expect("preview bound height fits u32"),
    )
    .expect("preview dimensions remain nonzero")
}
