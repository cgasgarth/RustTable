use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_RENDER_EDGE: u32 = 65_535;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RenderSizeRequest {
    Source,
    Exact { width: u32, height: u32 },
    Fit { max_width: u32, max_height: u32 },
    LongEdge(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSizeError {
    ZeroWidth,
    ZeroHeight,
    EdgeTooLarge,
    ArithmeticOverflow,
}

impl RenderSizeRequest {
    pub const MAX_EDGE: u32 = MAX_RENDER_EDGE;

    #[must_use]
    pub const fn source() -> Self {
        Self::Source
    }

    /// Creates an exact output size bounded for safe allocation.
    ///
    /// # Errors
    ///
    /// Returns an error when either dimension is zero or exceeds the supported edge length.
    pub fn exact(width: u32, height: u32) -> Result<Self, RenderSizeError> {
        validate_dimensions(width, height)?;
        Ok(Self::Exact { width, height })
    }

    /// Creates a fit-to-bounds request.
    ///
    /// # Errors
    ///
    /// Returns an error when either bound is zero or exceeds the supported edge length.
    pub fn fit(max_width: u32, max_height: u32) -> Result<Self, RenderSizeError> {
        validate_dimensions(max_width, max_height)?;
        Ok(Self::Fit {
            max_width,
            max_height,
        })
    }

    /// Creates a request constrained by the longest output edge.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested edge is zero or exceeds the supported edge length.
    pub const fn long_edge(edge: u32) -> Result<Self, RenderSizeError> {
        if edge == 0 {
            return Err(RenderSizeError::ZeroWidth);
        }
        if edge > MAX_RENDER_EDGE {
            return Err(RenderSizeError::EdgeTooLarge);
        }
        Ok(Self::LongEdge(edge))
    }

    /// Resolves this request against a nonzero source size.
    ///
    /// # Errors
    ///
    /// Returns an error when source dimensions or requested dimensions are invalid, or when
    /// aspect-preserving arithmetic overflows.
    pub fn resolve(
        self,
        source_width: u32,
        source_height: u32,
    ) -> Result<(u32, u32), RenderSizeError> {
        validate_dimensions(source_width, source_height)?;
        match self {
            Self::Source => Ok((source_width, source_height)),
            Self::Exact { width, height } => Ok((width, height)),
            Self::Fit {
                max_width,
                max_height,
            } => fit(source_width, source_height, max_width, max_height),
            Self::LongEdge(edge) => fit(source_width, source_height, edge, edge),
        }
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), RenderSizeError> {
    if width == 0 {
        return Err(RenderSizeError::ZeroWidth);
    }
    if height == 0 {
        return Err(RenderSizeError::ZeroHeight);
    }
    if width > MAX_RENDER_EDGE || height > MAX_RENDER_EDGE {
        return Err(RenderSizeError::EdgeTooLarge);
    }
    Ok(())
}

fn fit(
    source_width: u32,
    source_height: u32,
    max_width: u32,
    max_height: u32,
) -> Result<(u32, u32), RenderSizeError> {
    validate_dimensions(max_width, max_height)?;
    if source_width <= max_width && source_height <= max_height {
        return Ok((source_width, source_height));
    }
    let source_width = u64::from(source_width);
    let source_height = u64::from(source_height);
    let max_width = u64::from(max_width);
    let max_height = u64::from(max_height);
    let (width, height) = if source_width * max_height >= source_height * max_width {
        (max_width, (source_height * max_width / source_width).max(1))
    } else {
        (
            (source_width * max_height / source_height).max(1),
            max_height,
        )
    };
    let width = u32::try_from(width).map_err(|_| RenderSizeError::ArithmeticOverflow)?;
    let height = u32::try_from(height).map_err(|_| RenderSizeError::ArithmeticOverflow)?;
    Ok((width, height))
}

impl fmt::Display for RenderSizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroWidth => "render width must be nonzero",
            Self::ZeroHeight => "render height must be nonzero",
            Self::EdgeTooLarge => "render edge exceeds the safe maximum",
            Self::ArithmeticOverflow => "render dimensions overflowed",
        })
    }
}

impl std::error::Error for RenderSizeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_preserves_aspect_ratio_and_never_returns_zero() {
        let request = RenderSizeRequest::fit(100, 100).expect("bounds");
        assert_eq!(request.resolve(400, 200).expect("resolution"), (100, 50));
        assert_eq!(request.resolve(1, 1).expect("resolution"), (1, 1));
    }

    #[test]
    fn invalid_sizes_are_rejected_before_allocation() {
        assert_eq!(
            RenderSizeRequest::exact(0, 1),
            Err(RenderSizeError::ZeroWidth)
        );
        assert_eq!(
            RenderSizeRequest::long_edge(0),
            Err(RenderSizeError::ZeroWidth)
        );
        assert_eq!(
            RenderSizeRequest::exact(MAX_RENDER_EDGE + 1, 1),
            Err(RenderSizeError::EdgeTooLarge)
        );
    }
}
