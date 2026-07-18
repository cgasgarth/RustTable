use std::fmt;

use rusttable_image::ImageDimensions;

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
    CenterPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderPlan {
    source_dimensions: ImageDimensions,
    target: RenderTarget,
    output_dimensions: ImageDimensions,
    sampling: RenderSampling,
}

impl RenderPlan {
    #[must_use]
    pub fn for_source(source_dimensions: ImageDimensions, target: RenderTarget) -> Self {
        let output_dimensions = match target {
            RenderTarget::FullResolution => source_dimensions,
            RenderTarget::PreviewFit(bounds) => fit_dimensions(source_dimensions, bounds),
        };
        let sampling = if output_dimensions == source_dimensions {
            RenderSampling::Identity
        } else {
            RenderSampling::CenterPoint
        };
        Self {
            source_dimensions,
            target,
            output_dimensions,
            sampling,
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
