#![forbid(unsafe_code)]
#![doc = "Pure decoded-image render coordination for `RustTable`."]

use std::fmt;

use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{ColorEncoding, DecodedImage, DecodedImageError};
use rusttable_processing::{
    CompiledPipeline, DisplayP3Channel, DisplayP3Rgb, DisplayP3RgbImage, EvaluationError,
    GamutClipReport, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel, encode_linear_srgb,
    evaluate, to_linear_srgb, to_linear_srgb_from_display_p3,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceColorPolicy {
    RequireDeclaredSrgb,
    RequireDeclaredSupported,
    AssumeSrgbWhenUnspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceColorDecision {
    DeclaredSrgb,
    DeclaredDisplayP3,
    AssumedSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderProvenance {
    source_edit_id: EditId,
    source_photo_id: PhotoId,
    base_photo_revision: Revision,
    edit_revision: Revision,
}

impl RenderProvenance {
    #[must_use]
    pub const fn new(
        source_edit_id: EditId,
        source_photo_id: PhotoId,
        base_photo_revision: Revision,
        edit_revision: Revision,
    ) -> Self {
        Self {
            source_edit_id,
            source_photo_id,
            base_photo_revision,
            edit_revision,
        }
    }

    #[must_use]
    pub const fn source_edit_id(self) -> EditId {
        self.source_edit_id
    }

    #[must_use]
    pub const fn source_photo_id(self) -> PhotoId {
        self.source_photo_id
    }

    #[must_use]
    pub const fn base_photo_revision(self) -> Revision {
        self.base_photo_revision
    }

    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOutput {
    image: DecodedImage,
    source_color_decision: SourceColorDecision,
    clipping: GamutClipReport,
    provenance: RenderProvenance,
}

impl RenderOutput {
    #[must_use]
    pub const fn image(&self) -> &DecodedImage {
        &self.image
    }

    #[must_use]
    pub const fn source_color_decision(&self) -> SourceColorDecision {
        self.source_color_decision
    }

    #[must_use]
    pub const fn clipping(&self) -> GamutClipReport {
        self.clipping
    }

    #[must_use]
    pub const fn provenance(&self) -> RenderProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    SourceColor {
        actual: ColorEncoding,
    },
    Pipeline {
        source: Box<rusttable_processing::PipelineCompileError>,
    },
    Evaluation {
        source: EvaluationError,
    },
    Image {
        source: DecodedImageError,
    },
}

/// Renders one already-decoded image through one already-selected immutable edit.
///
/// # Errors
///
/// Returns a typed error when the source-color policy, pipeline compilation,
/// evaluation, or final checked image construction fails.
pub fn render_edit(
    edit: &Edit,
    input: &DecodedImage,
    policy: SourceColorPolicy,
) -> Result<RenderOutput, RenderError> {
    let source_color_decision = source_color_decision(input.color_encoding(), policy)?;
    let pipeline = CompiledPipeline::compile(edit).map_err(|source| RenderError::Pipeline {
        source: Box::new(source),
    })?;
    let (source, alpha) = source_image(input);
    let working = match source {
        SourceImage::Srgb(source) => to_linear_srgb(&source),
        SourceImage::DisplayP3(source) => to_linear_srgb_from_display_p3(&source),
    };
    let evaluated =
        evaluate(&pipeline, &working).map_err(|source| RenderError::Evaluation { source })?;
    let encoded = encode_linear_srgb(&evaluated);
    let pixels = quantized_pixels(&encoded, &alpha);
    let image =
        DecodedImage::new_with_color_encoding(input.dimensions(), pixels, ColorEncoding::Srgb)
            .map_err(|source| RenderError::Image { source })?;

    Ok(RenderOutput {
        image,
        source_color_decision,
        clipping: encoded.clipping(),
        provenance: RenderProvenance::new(
            pipeline.source_edit_id(),
            pipeline.source_photo_id(),
            pipeline.base_photo_revision(),
            pipeline.revision(),
        ),
    })
}

fn source_color_decision(
    actual: ColorEncoding,
    policy: SourceColorPolicy,
) -> Result<SourceColorDecision, RenderError> {
    match (actual, policy) {
        (ColorEncoding::Srgb, _) => Ok(SourceColorDecision::DeclaredSrgb),
        (ColorEncoding::DisplayP3, SourceColorPolicy::RequireDeclaredSrgb) => {
            Err(RenderError::SourceColor { actual })
        }
        (ColorEncoding::DisplayP3, _) => Ok(SourceColorDecision::DeclaredDisplayP3),
        (ColorEncoding::Unspecified, SourceColorPolicy::AssumeSrgbWhenUnspecified) => {
            Ok(SourceColorDecision::AssumedSrgb)
        }
        (
            ColorEncoding::Unspecified,
            SourceColorPolicy::RequireDeclaredSrgb | SourceColorPolicy::RequireDeclaredSupported,
        ) => Err(RenderError::SourceColor { actual }),
    }
}

enum SourceImage {
    Srgb(SourceRgbImage),
    DisplayP3(DisplayP3RgbImage),
}

fn source_image(input: &DecodedImage) -> (SourceImage, Vec<u8>) {
    let dimensions = RasterDimensions::new(input.dimensions().width(), input.dimensions().height())
        .expect("validated decoded dimensions are nonzero");
    let pixel_count = usize::try_from(
        input
            .dimensions()
            .pixel_count()
            .expect("validated dimensions have a representable pixel count"),
    )
    .expect("decoded pixel count fits the host allocation");
    let mut alpha = Vec::with_capacity(pixel_count);
    match input.color_encoding() {
        ColorEncoding::DisplayP3 => {
            let mut source_pixels = Vec::with_capacity(pixel_count);
            for pixel in input.pixels().chunks_exact(4) {
                source_pixels.push(DisplayP3Rgb::new(
                    DisplayP3Channel::new(f32::from(pixel[0]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                    DisplayP3Channel::new(f32::from(pixel[1]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                    DisplayP3Channel::new(f32::from(pixel[2]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                ));
                alpha.push(pixel[3]);
            }
            (
                SourceImage::DisplayP3(
                    DisplayP3RgbImage::new(dimensions, source_pixels)
                        .expect("decoded pixels match dimensions"),
                ),
                alpha,
            )
        }
        ColorEncoding::Unspecified | ColorEncoding::Srgb => {
            let mut source_pixels = Vec::with_capacity(pixel_count);
            for pixel in input.pixels().chunks_exact(4) {
                source_pixels.push(SourceRgb::new(
                    SrgbChannel::new(f32::from(pixel[0]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                    SrgbChannel::new(f32::from(pixel[1]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                    SrgbChannel::new(f32::from(pixel[2]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                ));
                alpha.push(pixel[3]);
            }
            (
                SourceImage::Srgb(
                    SourceRgbImage::new(dimensions, source_pixels)
                        .expect("decoded pixels match dimensions"),
                ),
                alpha,
            )
        }
    }
}

fn quantized_pixels(encoded: &rusttable_processing::EncodedSrgbOutput, alpha: &[u8]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(alpha.len() * 4);
    for (encoded_pixel, &alpha) in encoded.image().pixels().zip(alpha) {
        pixels.extend([
            quantize(encoded_pixel.red()),
            quantize(encoded_pixel.green()),
            quantize(encoded_pixel.blue()),
            alpha,
        ]);
    }
    pixels
}

fn quantize(channel: SrgbChannel) -> u8 {
    let scaled = channel.get() * 255.0;
    let rounded = (scaled + 0.5).floor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "SrgbChannel proves rounded is in the inclusive 0..=255 code range"
    )]
    {
        rounded as u8
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceColor { actual } => {
                write!(
                    formatter,
                    "source color encoding {actual:?} is not supported by the selected policy"
                )
            }
            Self::Pipeline { source } => {
                write!(formatter, "render pipeline compilation failed: {source}")
            }
            Self::Evaluation { source } => write!(formatter, "render evaluation failed: {source}"),
            Self::Image { source } => write!(
                formatter,
                "render output image construction failed: {source}"
            ),
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SourceColor { .. } => None,
            Self::Pipeline { source } => Some(source.as_ref()),
            Self::Evaluation { source } => Some(source),
            Self::Image { source } => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::quantize;
    use rusttable_processing::SrgbChannel;

    #[test]
    fn quantization_uses_round_half_up() {
        let below = SrgbChannel::new(10.499 / 255.0).expect("normalized");
        let half = SrgbChannel::new(10.5 / 255.0).expect("normalized");
        let above = SrgbChannel::new(10.501 / 255.0).expect("normalized");

        assert_eq!(quantize(below), 10);
        assert_eq!(quantize(half), 11);
        assert_eq!(quantize(above), 11);
    }
}
