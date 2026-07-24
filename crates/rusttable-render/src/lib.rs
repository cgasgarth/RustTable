#![forbid(unsafe_code)]
#![doc = "Pure decoded-image render coordination for `RustTable`."]

use std::fmt;

use rusttable_core::{Edit, EditId, PhotoId, Revision};
use rusttable_image::{
    ColorEncoding, DecodedImage, DecodedImageError, ImageDimensions, SourceColor,
    SourceColorFallback,
};
use rusttable_processing::{
    CompiledPipeline, DisplayP3Channel, DisplayP3Rgb, DisplayP3RgbImage, EvaluationError,
    GamutClipReport, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel, evaluate,
    linear_display_p3_to_working, linear_srgb_to_working, to_linear_srgb,
    to_linear_srgb_from_display_p3,
};

pub mod diagnostics;
mod plan;
mod prepared_pixelpipe;
mod presentation;
mod provenance;
mod resampling;
mod thumbnail;

pub use diagnostics::{
    DiagnosticBackend, DiagnosticDescriptor, DiagnosticFinding, DiagnosticFrame,
    DiagnosticFrameError, DiagnosticGeometry, DiagnosticPath, OverexposedColorScheme,
    OverexposedMode, OverexposedPlan, OverexposedReceipt, OverexposedResult, OverexposedState,
    RawOverexposedPlan, RawOverexposedReceipt, RawOverexposedResult, RawOverexposedState,
    RawOverlayMode, RawSolidColor, RgbaPixel,
};
pub use plan::{
    PreviewBounds, PreviewBoundsError, RenderAlphaPolicy, RenderBorderPolicy, RenderPlan,
    RenderResampling, RenderSampling, RenderTarget,
};
pub use prepared_pixelpipe::{
    PreparedCpuPixelpipeResult, PreparedCpuPixelpipeResultError, render_prepared_cpu_pixelpipe,
};
pub use presentation::{
    SCENE_REFERRED_RAW_EXPOSURE_STOPS, SCENE_REFERRED_RAW_LINEAR_GAIN, SrgbFallbackContract,
};
pub use provenance::{
    ProvenancedRenderError, ProvenancedRenderErrorKind, ProvenancedRenderOutput,
    RenderFailureStage, RenderReceipt, RenderRequestContext, RenderSourceProvenance,
};
pub use resampling::{ResamplingChannel, ResamplingError};
pub use thumbnail::cache::{
    CURRENT_CACHE_SCHEMA, CacheEntry, CacheError, CacheLease, CacheLimits, CachePin, CacheStore,
    CacheTime, ReconciliationReport, ThumbnailMemoryCache,
};
pub use thumbnail::lifecycle::{
    CacheChangeEvent, CacheInvalidationReport, CacheLifecycle, CacheLifecycleError,
    CacheResolution, CacheResolutionError, CacheResolutionSource,
};
pub use thumbnail::scheduler::{
    PrefetchCancellation, PrefetchCompletion, PrefetchError, PrefetchHandle, PrefetchJob,
    PrefetchPriority, PrefetchRequest, PrefetchScheduler,
};
pub use thumbnail::{
    MipmapLevel, ResamplingQuality, ThumbnailError, ThumbnailGenerator, ThumbnailKey,
    ThumbnailKeyError, ThumbnailProvenance, ThumbnailRequest, ThumbnailSize,
    thumbnail_edit_operations_identity,
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
    EmbeddedProfile,
    EmbeddedChromaticities,
    AssumedSrgb,
    AssumedLinearRec709,
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
    plan: RenderPlan,
    source_color_decision: SourceColorDecision,
    source_color: SourceColor,
    clipping: GamutClipReport,
    provenance: RenderProvenance,
    working_profile: rusttable_processing::WorkingFrameDescriptor,
    presentation: SrgbFallbackContract,
}

impl RenderOutput {
    #[must_use]
    pub const fn image(&self) -> &DecodedImage {
        &self.image
    }

    #[must_use]
    pub const fn plan(&self) -> RenderPlan {
        self.plan
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
    pub const fn clipping(&self) -> GamutClipReport {
        self.clipping
    }

    #[must_use]
    pub const fn provenance(&self) -> RenderProvenance {
        self.provenance
    }

    #[must_use]
    pub const fn working_profile(&self) -> rusttable_processing::WorkingFrameDescriptor {
        self.working_profile
    }

    #[must_use]
    pub const fn presentation(&self) -> SrgbFallbackContract {
        self.presentation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    PlanSourceDimensions {
        planned: ImageDimensions,
        actual: ImageDimensions,
    },
    SourceColor {
        actual: ColorEncoding,
    },
    Pipeline {
        source: Box<rusttable_processing::PipelineCompileError>,
    },
    Evaluation {
        source: EvaluationError,
    },
    Resampling {
        source: ResamplingError,
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
    let plan = RenderPlan::for_source(input.dimensions(), RenderTarget::FullResolution);
    render_edit_with_plan(edit, input, policy, plan)
}

/// Renders one decoded image through an explicit deterministic render plan.
///
/// # Errors
///
/// Returns a plan mismatch before source policy, then preserves the existing
/// typed source-color, pipeline, evaluation, and output errors.
pub fn render_edit_with_plan(
    edit: &Edit,
    input: &DecodedImage,
    policy: SourceColorPolicy,
    plan: RenderPlan,
) -> Result<RenderOutput, RenderError> {
    if plan.source_dimensions() != input.dimensions() {
        return Err(RenderError::PlanSourceDimensions {
            planned: plan.source_dimensions(),
            actual: input.dimensions(),
        });
    }
    let source_color_decision = source_color_decision(input.color_encoding(), policy)?;
    let source_color = if input.color_encoding() == ColorEncoding::Unspecified {
        SourceColor::fallback(SourceColorFallback::EncodedSrgb)
    } else {
        SourceColor::from_encoding(input.color_encoding()).map_err(|_| {
            RenderError::SourceColor {
                actual: input.color_encoding(),
            }
        })?
    };
    let pipeline = CompiledPipeline::compile(edit).map_err(|source| RenderError::Pipeline {
        source: Box::new(source),
    })?;
    let (source, alpha) = source_image(input);
    let working = match source {
        SourceImage::Srgb(source) => to_linear_srgb(&source),
        SourceImage::LinearSrgb(source) => linear_srgb_to_working(&source),
        SourceImage::DisplayP3(source) => to_linear_srgb_from_display_p3(&source),
        SourceImage::LinearDisplayP3(source) => linear_display_p3_to_working(&source),
    };
    let evaluated =
        evaluate(&pipeline, &working).map_err(|source| RenderError::Evaluation { source })?;
    // Operations must observe the decoded source/intermediate frame.  The
    // render target is a presentation boundary, so final preview scaling is
    // applied only after the complete graph has run.  This keeps dimension-
    // and scale-sensitive operations aligned with full-resolution export and
    // leaves the shared resampler as the sole final-downsampling path.
    let (evaluated, alpha) = resampling::resample_working(&evaluated, &alpha, plan)
        .map_err(|source| RenderError::Resampling { source })?;
    let presentation = SrgbFallbackContract::Colorimetric;
    let presented = presentation.apply(&evaluated);
    let encoded = rusttable_processing::encode_working_to_srgb(&presented);
    let pixels = quantized_pixels(&encoded, &alpha);
    let image = DecodedImage::new_with_color_encoding(
        plan.output_dimensions(),
        pixels,
        ColorEncoding::Srgb,
    )
    .map_err(|source| RenderError::Image { source })?;

    Ok(RenderOutput {
        image,
        plan,
        source_color_decision,
        source_color,
        clipping: encoded.clipping(),
        provenance: RenderProvenance::new(
            pipeline.source_edit_id(),
            pipeline.source_photo_id(),
            pipeline.base_photo_revision(),
            pipeline.revision(),
        ),
        working_profile: evaluated.frame(),
        presentation,
    })
}

/// Renders with caller-supplied imported source provenance and returns an
/// owned success receipt or an owned stage-attributed failure context.
///
/// # Errors
///
/// Returns a source-preflight or delegated render failure with its complete
/// immutable request context and typed nested cause when applicable.
pub fn render_edit_with_provenance(
    edit: &Edit,
    input: &DecodedImage,
    policy: SourceColorPolicy,
    plan: RenderPlan,
    source: RenderSourceProvenance,
) -> Result<ProvenancedRenderOutput, ProvenancedRenderError> {
    let context = RenderRequestContext::new(source, edit, policy, plan);
    if source.photo_id() != edit.photo_id() {
        return Err(ProvenancedRenderError::new(
            context,
            RenderFailureStage::SourcePhoto,
            ProvenancedRenderErrorKind::SourcePhoto {
                source_photo_id: source.photo_id(),
                edit_photo_id: edit.photo_id(),
            },
        ));
    }
    if source.probe().dimensions() != input.dimensions() {
        return Err(ProvenancedRenderError::new(
            context,
            RenderFailureStage::SourceDimensions,
            ProvenancedRenderErrorKind::SourceDimensions {
                probed: source.probe().dimensions(),
                decoded: input.dimensions(),
            },
        ));
    }
    let output = render_edit_with_plan(edit, input, policy, plan).map_err(|source| {
        ProvenancedRenderError::new(
            context,
            failure_stage(&source),
            ProvenancedRenderErrorKind::Render {
                source: Box::new(source),
            },
        )
    })?;
    let receipt = RenderReceipt::new(context, &output);
    Ok(ProvenancedRenderOutput::new(output, receipt))
}

const fn failure_stage(error: &RenderError) -> RenderFailureStage {
    match error {
        RenderError::PlanSourceDimensions { .. } => RenderFailureStage::Plan,
        RenderError::SourceColor { .. } => RenderFailureStage::SourceColor,
        RenderError::Pipeline { .. } => RenderFailureStage::Pipeline,
        RenderError::Evaluation { .. } => RenderFailureStage::Evaluation,
        RenderError::Resampling { .. } => RenderFailureStage::Resampling,
        RenderError::Image { .. } => RenderFailureStage::Image,
    }
}

fn source_color_decision(
    actual: ColorEncoding,
    policy: SourceColorPolicy,
) -> Result<SourceColorDecision, RenderError> {
    match (actual, policy) {
        (ColorEncoding::Srgb | ColorEncoding::LinearSrgb, _) => {
            Ok(SourceColorDecision::DeclaredSrgb)
        }
        (
            ColorEncoding::DisplayP3D65 | ColorEncoding::LinearDisplayP3D65,
            SourceColorPolicy::RequireDeclaredSrgb,
        )
        | (
            ColorEncoding::Unspecified,
            SourceColorPolicy::RequireDeclaredSrgb | SourceColorPolicy::RequireDeclaredSupported,
        ) => Err(RenderError::SourceColor { actual }),
        (ColorEncoding::DisplayP3D65 | ColorEncoding::LinearDisplayP3D65, _) => {
            Ok(SourceColorDecision::DeclaredDisplayP3)
        }
        (ColorEncoding::Unspecified, SourceColorPolicy::AssumeSrgbWhenUnspecified) => {
            Ok(SourceColorDecision::AssumedSrgb)
        }
        _ => Err(RenderError::SourceColor { actual }),
    }
}

enum SourceImage {
    Srgb(SourceRgbImage),
    LinearSrgb(SourceRgbImage),
    DisplayP3(DisplayP3RgbImage),
    LinearDisplayP3(DisplayP3RgbImage),
}

fn source_image(input: &DecodedImage) -> (SourceImage, Vec<f32>) {
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
        ColorEncoding::DisplayP3D65 | ColorEncoding::LinearDisplayP3D65 => {
            let mut source_pixels = Vec::with_capacity(pixel_count);
            for pixel in input.pixels().as_chunks::<4>().0 {
                source_pixels.push(DisplayP3Rgb::new(
                    DisplayP3Channel::new(f32::from(pixel[0]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                    DisplayP3Channel::new(f32::from(pixel[1]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                    DisplayP3Channel::new(f32::from(pixel[2]) / 255.0)
                        .expect("normalized byte is valid Display P3"),
                ));
                alpha.push(f32::from(pixel[3]) / 255.0);
            }
            let source = DisplayP3RgbImage::new(dimensions, source_pixels)
                .expect("decoded pixels match dimensions");
            let source = if input.color_encoding() == ColorEncoding::LinearDisplayP3D65 {
                SourceImage::LinearDisplayP3(source)
            } else {
                SourceImage::DisplayP3(source)
            };
            (source, alpha)
        }
        ColorEncoding::Unspecified | ColorEncoding::Srgb | ColorEncoding::LinearSrgb => {
            let mut source_pixels = Vec::with_capacity(pixel_count);
            for pixel in input.pixels().as_chunks::<4>().0 {
                source_pixels.push(SourceRgb::new(
                    SrgbChannel::new(f32::from(pixel[0]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                    SrgbChannel::new(f32::from(pixel[1]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                    SrgbChannel::new(f32::from(pixel[2]) / 255.0)
                        .expect("normalized byte is valid sRGB"),
                ));
                alpha.push(f32::from(pixel[3]) / 255.0);
            }
            let source = SourceRgbImage::new(dimensions, source_pixels)
                .expect("decoded pixels match dimensions");
            let source = if input.color_encoding() == ColorEncoding::LinearSrgb {
                SourceImage::LinearSrgb(source)
            } else {
                SourceImage::Srgb(source)
            };
            (source, alpha)
        }
        _ => unreachable!("source_image is called only after source-color validation"),
    }
}

fn quantized_pixels(encoded: &rusttable_processing::EncodedSrgbOutput, alpha: &[f32]) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(alpha.len() * 4);
    for (encoded_pixel, &alpha) in encoded.image().pixels().zip(alpha) {
        pixels.extend([
            quantize(encoded_pixel.red()),
            quantize(encoded_pixel.green()),
            quantize(encoded_pixel.blue()),
            quantize_alpha(alpha),
        ]);
    }
    pixels
}

fn quantize_alpha(value: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "alpha is bounded at the presentation boundary"
    )]
    {
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    }
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
            Self::PlanSourceDimensions { planned, actual } => write!(
                formatter,
                "render plan expects {planned:?} but input is {actual:?}"
            ),
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
            Self::Resampling { source } => {
                write!(formatter, "render resampling failed: {source:?}")
            }
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
            Self::PlanSourceDimensions { .. } | Self::SourceColor { .. } => None,
            Self::Pipeline { source } => Some(source.as_ref()),
            Self::Evaluation { source } => Some(source),
            Self::Resampling { source } => Some(source),
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
