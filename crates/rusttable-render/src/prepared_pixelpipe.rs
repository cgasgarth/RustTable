use std::fmt;

use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_processing::{WorkingRgbImage, encode_linear_srgb};

use crate::{
    RenderError, RenderOutput, RenderPlan, RenderProvenance, RenderSampling, RenderTarget,
    SourceColorDecision, quantized_pixels, sample_center_point,
};

/// Immutable canonical CPU pixelpipe output ready for final rendering.
///
/// The executor owns operation evaluation. This boundary accepts only its
/// completed linear-light pixels, alpha channel, and immutable provenance; it
/// deliberately has no operation registry or operation-name dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCpuPixelpipeResult {
    pixels: WorkingRgbImage,
    alpha: Vec<u8>,
    source_color_decision: SourceColorDecision,
    provenance: RenderProvenance,
}

impl PreparedCpuPixelpipeResult {
    /// Creates a validated completed CPU pixelpipe result.
    ///
    /// # Errors
    ///
    /// Returns an error when alpha does not provide exactly one value for each
    /// already-validated working pixel.
    pub fn new(
        pixels: WorkingRgbImage,
        alpha: Vec<u8>,
        source_color_decision: SourceColorDecision,
        provenance: RenderProvenance,
    ) -> Result<Self, PreparedCpuPixelpipeResultError> {
        let expected = pixels.dimensions().pixel_count();
        if u64::try_from(alpha.len()) != Ok(expected) {
            return Err(PreparedCpuPixelpipeResultError::AlphaLength {
                expected,
                actual: alpha.len(),
            });
        }
        Ok(Self {
            pixels,
            alpha,
            source_color_decision,
            provenance,
        })
    }

    #[must_use]
    pub const fn pixels(&self) -> &WorkingRgbImage {
        &self.pixels
    }

    #[must_use]
    pub fn alpha(&self) -> &[u8] {
        &self.alpha
    }

    #[must_use]
    pub const fn source_color_decision(&self) -> SourceColorDecision {
        self.source_color_decision
    }

    #[must_use]
    pub const fn provenance(&self) -> RenderProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparedCpuPixelpipeResultError {
    AlphaLength { expected: u64, actual: usize },
}

/// Renders a completed canonical CPU pixelpipe result for preview or full output.
///
/// # Errors
///
/// Returns a typed image error if final output construction or deterministic
/// preview sampling cannot represent the completed result.
///
/// # Panics
///
/// Panics only if a `WorkingRgbImage` violates its own nonzero-dimensions
/// invariant.
pub fn render_prepared_cpu_pixelpipe(
    prepared: &PreparedCpuPixelpipeResult,
    target: RenderTarget,
) -> Result<RenderOutput, RenderError> {
    let dimensions = prepared.pixels().dimensions();
    let source_dimensions = ImageDimensions::new(dimensions.width(), dimensions.height())
        .expect("processing dimensions are nonzero by type");
    let plan = RenderPlan::for_source(source_dimensions, target);
    let encoded = encode_linear_srgb(prepared.pixels());
    let full = DecodedImage::new_with_color_encoding(
        source_dimensions,
        quantized_pixels(&encoded, prepared.alpha()),
        ColorEncoding::Srgb,
    )
    .map_err(|source| RenderError::Image { source })?;
    let image = match plan.sampling() {
        RenderSampling::Identity => full,
        RenderSampling::CenterPoint => sample_center_point(&full, plan.output_dimensions())
            .map_err(|source| RenderError::Image { source })?,
    };

    Ok(RenderOutput {
        image,
        plan,
        source_color_decision: prepared.source_color_decision(),
        clipping: encoded.clipping(),
        provenance: prepared.provenance(),
    })
}

impl fmt::Display for PreparedCpuPixelpipeResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlphaLength { expected, actual } => write!(
                formatter,
                "prepared CPU pixelpipe alpha length is {actual}, expected {expected}"
            ),
        }
    }
}

impl std::error::Error for PreparedCpuPixelpipeResultError {}
