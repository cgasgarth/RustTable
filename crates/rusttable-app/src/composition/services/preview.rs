use rusttable_core::Edit;
use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodedFrame, DecodedImage, ImageInput, SampleType,
};
use rusttable_image_io::FileImageInput;
use rusttable_pixelpipe::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot,
    CpuPixelpipeSnapshotError, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, RgbaF32SourceRepresentation,
};
use rusttable_processing::{
    CompiledOperationGraph, FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage,
};
use rusttable_render::{
    PreparedCpuPixelpipeResult, PreparedCpuPixelpipeResultError, PreviewBounds, RenderOutput,
    RenderProvenance, RenderTarget, SourceColorDecision, render_prepared_cpu_pixelpipe,
};

/// Production CPU preview boundary used by the application composition.
#[derive(Debug, Clone, Copy)]
pub struct PreviewService {
    limits: DecodeLimits,
    bounds: PreviewBounds,
}

impl PreviewService {
    #[must_use]
    pub const fn new(limits: DecodeLimits, bounds: PreviewBounds) -> Self {
        Self { limits, bounds }
    }

    /// Decodes immutable snapshot bytes and renders the exact edit.
    ///
    /// # Errors
    ///
    /// Returns a typed decode or CPU-render failure.
    pub fn render_bytes(&self, source: &[u8], edit: &Edit) -> Result<RenderOutput, PreviewError> {
        let _span = tracing::info_span!(
            target: "rusttable.preview",
            "render_bytes",
            operation = "preview_render",
            source_bytes = source.len(),
            width = tracing::field::Empty,
            height = tracing::field::Empty
        )
        .entered();
        let input = FileImageInput::new(self.limits)
            .decode_frame_bytes(source)
            .map_err(|error| {
                tracing::error!(target: "rusttable.preview", stage = "decode", cause = "decode_failed");
                PreviewError::Decode(error)
            })?;
        self.render_decoded_frame_for_target(&input, edit, RenderTarget::PreviewFit(self.bounds))
    }

    /// Decodes immutable snapshot bytes and renders the exact edit at source resolution.
    ///
    /// This is deliberately separate from [`Self::render_bytes`]: preview callers
    /// retain their bounded display target while export callers share the same
    /// production decode, edit, and color path without a second renderer.
    ///
    /// # Errors
    ///
    /// Returns a typed decode or CPU-render failure.
    pub fn render_full_resolution_bytes(
        &self,
        source: &[u8],
        edit: &Edit,
    ) -> Result<RenderOutput, PreviewError> {
        self.render_bytes_for_target(source, edit, RenderTarget::FullResolution)
    }

    /// Decodes immutable snapshot bytes and renders them through one target.
    ///
    /// Export callers use this to keep final scaling in the production render
    /// plan instead of applying a second resize after processing.
    ///
    /// # Errors
    ///
    /// Returns a typed decode or CPU-render failure.
    pub fn render_bytes_for_target(
        &self,
        source: &[u8],
        edit: &Edit,
        target: RenderTarget,
    ) -> Result<RenderOutput, PreviewError> {
        let input = FileImageInput::new(self.limits)
            .decode_frame_bytes(source)
            .map_err(|error| {
                tracing::error!(target: "rusttable.preview", stage = "decode", cause = "decode_failed");
                PreviewError::Decode(error)
            })?;
        self.render_decoded_frame_for_target(&input, edit, target)
    }

    /// Renders an image that was already decoded by the composition's input
    /// port. This keeps catalog-preview orchestration independent of a
    /// concrete decoder while retaining the canonical CPU path.
    ///
    /// # Errors
    ///
    /// Returns a typed CPU-render failure.
    pub fn render_decoded_for_target(
        &self,
        input: &DecodedImage,
        edit: &Edit,
        target: RenderTarget,
    ) -> Result<RenderOutput, PreviewError> {
        render_with_target(input, edit, target)
    }

    /// Renders one native decoded frame without an RGBA8 compatibility
    /// projection before the first CPU pixelpipe input.
    ///
    /// # Errors
    ///
    /// Returns a typed frame-adaptation, pixelpipe, or presentation error.
    pub fn render_decoded_frame_for_target(
        &self,
        input: &DecodedFrame,
        edit: &Edit,
        target: RenderTarget,
    ) -> Result<RenderOutput, PreviewError> {
        render_frame_with_target(input, edit, target)
    }
}

fn render_frame_with_target(
    input: &DecodedFrame,
    edit: &Edit,
    target: RenderTarget,
) -> Result<RenderOutput, PreviewError> {
    let source_pixels = input
        .rgba_f32_pixels()
        .map_err(|_| PreviewError::DecodedFrame)?;
    let dimensions = RasterDimensions::new(
        input.image().descriptor().dimensions().width(),
        input.image().descriptor().dimensions().height(),
    )
    .expect("decoded images have nonzero dimensions");
    let encoding = match input.sample_type() {
        SampleType::F16 | SampleType::F32 => RgbaF32ColorEncoding::LinearSrgbD65,
        SampleType::U8 | SampleType::U16 => RgbaF32ColorEncoding::SrgbD65,
    };
    let representation = match input.sample_type() {
        SampleType::U8 => RgbaF32SourceRepresentation::U8,
        SampleType::U16 => RgbaF32SourceRepresentation::U16,
        SampleType::F16 => RgbaF32SourceRepresentation::F16,
        SampleType::F32 => RgbaF32SourceRepresentation::F32,
    };
    let pixelpipe_input = RgbaF32Image::new(
        RgbaF32Descriptor::with_source_representation(dimensions, encoding, representation),
        source_pixels
            .iter()
            .copied()
            .map(|pixel| RgbaF32Pixel::new(pixel[0], pixel[1], pixel[2], pixel[3]))
            .collect(),
    )
    .map_err(PreviewError::PixelpipeInput)?;
    let graph = CompiledOperationGraph::compile(edit).map_err(PreviewError::Graph)?;
    let snapshot =
        CpuPixelpipeSnapshot::try_new(pixelpipe_input, graph, CpuPixelpipeOutputMode::FullExport)
            .map_err(PreviewError::PixelpipeSnapshot)?;
    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .map_err(PreviewError::Pixelpipe)?;
    let pixels = result
        .image()
        .pixels()
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                FiniteF32::new(pixel.red()).expect("CPU pixelpipe output is finite"),
                FiniteF32::new(pixel.green()).expect("CPU pixelpipe output is finite"),
                FiniteF32::new(pixel.blue()).expect("CPU pixelpipe output is finite"),
            )
        })
        .collect();
    let working = WorkingRgbImage::new(dimensions, pixels)
        .expect("CPU pixelpipe preserves its validated image dimensions");
    let alpha = source_pixels.iter().map(|pixel| pixel[3]).collect();
    let prepared = PreparedCpuPixelpipeResult::new(
        working,
        alpha,
        SourceColorDecision::DeclaredSrgb,
        RenderProvenance::new(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
        ),
    )
    .map_err(PreviewError::Prepared)?;
    render_prepared_cpu_pixelpipe(&prepared, target).map_err(PreviewError::Render)
}

fn render_with_target(
    input: &DecodedImage,
    edit: &Edit,
    target: RenderTarget,
) -> Result<RenderOutput, PreviewError> {
    render_decoded_with_target(input, edit, target)
}

fn render_decoded_with_target(
    input: &DecodedImage,
    edit: &Edit,
    target: RenderTarget,
) -> Result<RenderOutput, PreviewError> {
    let source_color_decision = source_color_decision(input.color_encoding()).inspect_err(|_| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "unsupported_color");
    })?;
    let dimensions = RasterDimensions::new(input.dimensions().width(), input.dimensions().height())
        .expect("decoded images have nonzero dimensions");
    let (source_pixels, remainder) = input.pixels().as_chunks::<4>();
    debug_assert!(remainder.is_empty(), "decoded images are packed RGBA8");
    let pixelpipe_input = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        source_pixels
            .iter()
            .map(|pixel| {
                RgbaF32Pixel::new(
                    f32::from(pixel[0]) / 255.0,
                    f32::from(pixel[1]) / 255.0,
                    f32::from(pixel[2]) / 255.0,
                    f32::from(pixel[3]) / 255.0,
                )
            })
            .collect(),
    )
    .map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "input_adaptation");
        PreviewError::PixelpipeInput(error)
    })?;
    let graph = CompiledOperationGraph::compile(edit).map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "graph_compile");
        PreviewError::Graph(error)
    })?;
    let snapshot = CpuPixelpipeSnapshot::try_new(
        pixelpipe_input,
        graph,
        // The prepared-render boundary owns deterministic target encoding.
        CpuPixelpipeOutputMode::FullExport,
    )
    .map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "snapshot");
        PreviewError::PixelpipeSnapshot(error)
    })?;
    let result = CpuPixelpipeExecutor.execute(&snapshot).map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "pixelpipe");
        PreviewError::Pixelpipe(error)
    })?;
    let pixels = result
        .image()
        .pixels()
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                FiniteF32::new(pixel.red()).expect("CPU pixelpipe output is finite"),
                FiniteF32::new(pixel.green()).expect("CPU pixelpipe output is finite"),
                FiniteF32::new(pixel.blue()).expect("CPU pixelpipe output is finite"),
            )
        })
        .collect();
    let working = WorkingRgbImage::new(dimensions, pixels)
        .expect("CPU pixelpipe preserves its validated image dimensions");
    let alpha = source_pixels
        .iter()
        .map(|pixel| f32::from(pixel[3]) / 255.0)
        .collect();
    let prepared = PreparedCpuPixelpipeResult::new(
        working,
        alpha,
        source_color_decision,
        RenderProvenance::new(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
        ),
    )
    .map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "prepare");
        PreviewError::Prepared(error)
    })?;
    render_prepared_cpu_pixelpipe(&prepared, target).map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "render", cause = "target_adaptation");
        PreviewError::Render(error)
    })
}

fn source_color_decision(encoding: ColorEncoding) -> Result<SourceColorDecision, PreviewError> {
    match encoding {
        ColorEncoding::Srgb | ColorEncoding::LinearSrgb => Ok(SourceColorDecision::DeclaredSrgb),
        ColorEncoding::Unspecified => Ok(SourceColorDecision::AssumedSrgb),
        ColorEncoding::DisplayP3D65 | ColorEncoding::LinearDisplayP3D65 => {
            Err(PreviewError::UnsupportedPixelpipeColor { actual: encoding })
        }
        _ => Err(PreviewError::UnsupportedPixelpipeColor { actual: encoding }),
    }
}

#[derive(Debug)]
pub enum PreviewError {
    Decode(rusttable_image::ImageInputError),
    DecodedFrame,
    UnsupportedPixelpipeColor { actual: ColorEncoding },
    PixelpipeInput(RgbaF32ImageError),
    PixelpipeSnapshot(CpuPixelpipeSnapshotError),
    Graph(rusttable_processing::OperationGraphCompileError),
    Pixelpipe(CpuPixelpipeError),
    Prepared(PreparedCpuPixelpipeResultError),
    Render(rusttable_render::RenderError),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "preview decode failed: {error}"),
            Self::DecodedFrame => formatter.write_str("preview decoded frame adaptation failed"),
            Self::UnsupportedPixelpipeColor { actual } => write!(
                formatter,
                "preview CPU pixelpipe does not support {actual:?} source color"
            ),
            Self::PixelpipeInput(error) => write!(formatter, "preview CPU input failed: {error}"),
            Self::PixelpipeSnapshot(error) => {
                write!(formatter, "preview CPU snapshot failed: {error}")
            }
            Self::Graph(error) => write!(formatter, "preview graph compilation failed: {error}"),
            Self::Pixelpipe(error) => write!(formatter, "preview CPU pixelpipe failed: {error}"),
            Self::Prepared(error) => {
                write!(formatter, "preview CPU render preparation failed: {error}")
            }
            Self::Render(error) => write!(formatter, "preview render failed: {error}"),
        }
    }
}

impl std::error::Error for PreviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::DecodedFrame | Self::UnsupportedPixelpipeColor { .. } => None,
            Self::PixelpipeInput(error) => Some(error),
            Self::PixelpipeSnapshot(error) => Some(error),
            Self::Graph(error) => Some(error),
            Self::Pixelpipe(error) => Some(error),
            Self::Prepared(error) => Some(error),
            Self::Render(error) => Some(error),
        }
    }
}
