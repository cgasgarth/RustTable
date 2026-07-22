use std::fmt;

use rusttable_image::{
    ColorEncoding, DecodedImage, ImageDimensions, SourceColor, SourceColorFallback,
};
use rusttable_processing::{WorkingRgbImage, encode_working_to_srgb};

use crate::{
    RenderError, RenderOutput, RenderPlan, RenderProvenance, RenderTarget, SourceColorDecision,
    quantized_pixels, resampling,
};

/// Immutable canonical CPU pixelpipe output ready for final rendering.
///
/// The executor owns operation evaluation. This boundary accepts only its
/// completed linear-light pixels, alpha channel, and immutable provenance; it
/// deliberately has no operation registry or operation-name dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCpuPixelpipeResult {
    pixels: WorkingRgbImage,
    alpha: Vec<f32>,
    source_color_decision: SourceColorDecision,
    source_color: SourceColor,
    provenance: RenderProvenance,
}

impl PreparedCpuPixelpipeResult {
    /// Creates a validated completed CPU pixelpipe result.
    ///
    /// # Errors
    ///
    /// Returns an error when alpha does not provide exactly one value for each
    /// already-validated working pixel.
    ///
    /// # Panics
    ///
    /// Panics only if the hard-coded Display P3 color encoding is rejected by
    /// the color contract, which would indicate an internal invariant failure.
    pub fn new(
        pixels: WorkingRgbImage,
        alpha: Vec<f32>,
        source_color_decision: SourceColorDecision,
        provenance: RenderProvenance,
    ) -> Result<Self, PreparedCpuPixelpipeResultError> {
        let source_color = match source_color_decision {
            SourceColorDecision::DeclaredDisplayP3 => {
                SourceColor::declared(ColorEncoding::DisplayP3D65)
                    .expect("published Display P3 is valid")
            }
            SourceColorDecision::AssumedLinearRec709 => {
                SourceColor::fallback(SourceColorFallback::LinearRec709)
            }
            SourceColorDecision::DeclaredSrgb
            | SourceColorDecision::EmbeddedProfile
            | SourceColorDecision::EmbeddedChromaticities
            | SourceColorDecision::AssumedSrgb => {
                SourceColor::fallback(SourceColorFallback::EncodedSrgb)
            }
        };
        Self::new_with_source_color(
            pixels,
            alpha,
            source_color_decision,
            source_color,
            provenance,
        )
    }

    /// Creates a prepared result with the exact decoded source-color evidence.
    ///
    /// # Errors
    ///
    /// Returns an error when alpha does not provide exactly one value for each
    /// already-validated working pixel.
    pub fn new_with_source_color(
        pixels: WorkingRgbImage,
        alpha: Vec<f32>,
        source_color_decision: SourceColorDecision,
        source_color: SourceColor,
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
            source_color,
            provenance,
        })
    }

    #[must_use]
    pub const fn pixels(&self) -> &WorkingRgbImage {
        &self.pixels
    }

    #[must_use]
    pub fn alpha(&self) -> &[f32] {
        &self.alpha
    }

    #[must_use]
    pub const fn source_color_decision(&self) -> SourceColorDecision {
        self.source_color_decision
    }

    #[must_use]
    pub const fn source_color(&self) -> SourceColor {
        self.source_color
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
    let (pixels, alpha) = resampling::resample_working(prepared.pixels(), prepared.alpha(), plan)
        .map_err(|source| RenderError::Resampling { source })?;
    let encoded = encode_working_to_srgb(&pixels);
    let image = DecodedImage::new_with_color_encoding(
        plan.output_dimensions(),
        quantized_pixels(&encoded, &alpha),
        ColorEncoding::Srgb,
    )
    .map_err(|source| RenderError::Image { source })?;

    Ok(RenderOutput {
        image,
        plan,
        source_color_decision: prepared.source_color_decision(),
        source_color: prepared.source_color(),
        clipping: encoded.clipping(),
        provenance: prepared.provenance(),
        working_profile: pixels.frame(),
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
