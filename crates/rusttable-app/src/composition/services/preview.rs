use std::sync::{Once, OnceLock, RwLock};
use std::thread;

use rusttable_core::Edit;
use rusttable_gpu::{GpuRuntime, GpuRuntimeConfig};
use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodedFrame, DecodedImage, ImageInputError, InputFormat,
    SampleType, SourceColor, SourceColorEvidence, SourceColorFallback,
};
use rusttable_image_io::{
    FileImageInput, RawDecodeError, RawDecodeLimits, RawDecodeRequest, RawlerRawDecoder,
};
use rusttable_pixelpipe::{
    CancellationScope, CancellationStage, CpuPixelpipeError, CpuPixelpipeOutputMode,
    CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, PixelpipeExecutionResult,
    PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, RgbaF32SourceRepresentation,
};
use rusttable_processing::{
    CompiledOperationGraph, DemosaicAlgorithm, FiniteF32, LinearRgb, RasterDimensions,
    RawPipelinePlan, WorkingRgbImage, pre_demosaic_temperature,
};
use rusttable_render::{
    PreparedCpuPixelpipeResult, PreparedCpuPixelpipeResultError, PreviewBounds, RenderOutput,
    RenderProvenance, RenderTarget, SourceColorDecision, SrgbFallbackContract,
    render_prepared_cpu_pixelpipe,
};

static PRODUCTION_PIXELPIPE: OnceLock<RwLock<PixelpipeExecutionService>> = OnceLock::new();
static PRODUCTION_GPU_INITIALIZATION: Once = Once::new();

/// Production preview boundary used by the application composition.
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
        self.render_bytes_with_scope(source, edit, None)
    }

    /// Renders bytes while honoring one generation-owned cancellation scope.
    ///
    /// # Errors
    ///
    /// Returns a typed cancellation, decode, processing, or render failure.
    pub(crate) fn render_bytes_with_cancellation(
        &self,
        source: &[u8],
        edit: &Edit,
        cancellation: &CancellationScope,
    ) -> Result<RenderOutput, PreviewError> {
        self.render_bytes_with_scope(source, edit, Some(cancellation))
    }

    fn render_bytes_with_scope(
        &self,
        source: &[u8],
        edit: &Edit,
        cancellation: Option<&CancellationScope>,
    ) -> Result<RenderOutput, PreviewError> {
        let _span = tracing::info_span!(
            target: "rusttable.preview",
            "render_bytes",
            operation = "preview_render",
            source_bytes = source.len(),
            width = tracing::field::Empty,
            height = tracing::field::Empty
        )
        .entered();
        check_preview_cancellation(cancellation, CancellationStage::SourceDecode)?;
        let input = decode_preview_frame(source, self.limits)?;
        check_preview_cancellation(cancellation, CancellationStage::SourceDecode)?;
        Self::render_decoded_frame_for_target_with_scope(
            &input,
            edit,
            RenderTarget::PreviewFit(self.bounds),
            cancellation,
        )
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
        let input = decode_preview_frame(source, self.limits)?;
        Self::render_decoded_frame_for_target_with_scope(&input, edit, target, None)
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
        render_with_target(input, edit, target, None)
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
        Self::render_decoded_frame_for_target_with_scope(input, edit, target, None)
    }

    fn render_decoded_frame_for_target_with_scope(
        input: &DecodedFrame,
        edit: &Edit,
        target: RenderTarget,
        cancellation: Option<&CancellationScope>,
    ) -> Result<RenderOutput, PreviewError> {
        let prepared = prepare_frame(input, edit, cancellation)?;
        check_preview_cancellation(cancellation, CancellationStage::Publication)?;
        let output =
            render_prepared_cpu_pixelpipe(&prepared, target).map_err(PreviewError::Render)?;
        check_preview_cancellation(cancellation, CancellationStage::Publication)?;
        Ok(output)
    }

    /// Prepares the complete graph at the decoded source/intermediate
    /// resolution without applying a display-sized target.
    ///
    /// The returned frame is the shared full-output reference used by both
    /// preview and export presentation. Callers must pass it through
    /// [`render_prepared_cpu_pixelpipe`] for the requested target.
    ///
    /// # Errors
    ///
    /// Returns a typed frame-adaptation, graph, pixelpipe, or preparation
    /// error.
    pub fn prepare_decoded_frame(
        &self,
        input: &DecodedFrame,
        edit: &Edit,
    ) -> Result<PreparedCpuPixelpipeResult, PreviewError> {
        prepare_frame(input, edit, None)
    }
}

/// Starts the one production WGPU initialization without blocking GTK startup.
///
/// Canonical CPU publication is installed before the worker is spawned, so a
/// preview requested before device discovery completes remains renderable. The
/// initialized runtime then upgrades that same process-long service in place.
pub(crate) fn initialize_production_pixelpipe() {
    let _ = production_pixelpipe();
    PRODUCTION_GPU_INITIALIZATION.call_once(|| {
        if let Err(error) = thread::Builder::new()
            .name("rusttable-gpu-init".to_owned())
            .spawn(initialize_production_gpu)
        {
            tracing::warn!(
                target: "rusttable.preview",
                operation = "gpu_initialization",
                cause = %error,
                "GPU initializer could not start; retaining canonical CPU pixelpipe"
            );
        }
    });
}

fn initialize_production_gpu() {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::warn!(
                target: "rusttable.preview",
                operation = "gpu_initialization",
                cause = %error,
                "GPU runtime bootstrap failed; retaining canonical CPU pixelpipe"
            );
            return;
        }
    };
    match runtime.block_on(GpuRuntime::initialize(GpuRuntimeConfig::default())) {
        Ok(gpu) => {
            let cpu_only = gpu.is_cpu_only();
            let mut service = production_pixelpipe()
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *service = PixelpipeExecutionService::with_gpu(gpu);
            tracing::info!(
                target: "rusttable.preview",
                operation = "gpu_initialization",
                cpu_only,
                "production pixelpipe initialized"
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "rusttable.preview",
                operation = "gpu_initialization",
                cause = %error,
                "GPU initialization failed; retaining canonical CPU pixelpipe"
            );
        }
    }
}

fn production_pixelpipe() -> &'static RwLock<PixelpipeExecutionService> {
    PRODUCTION_PIXELPIPE.get_or_init(|| RwLock::new(PixelpipeExecutionService::cpu_only()))
}

fn execute_production_snapshot(
    snapshot: &CpuPixelpipeSnapshot,
    cancellation: Option<&CancellationScope>,
) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
    let service = production_pixelpipe()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    execute_snapshot(&service, snapshot, cancellation)
}

fn execute_snapshot(
    service: &PixelpipeExecutionService,
    snapshot: &CpuPixelpipeSnapshot,
    cancellation: Option<&CancellationScope>,
) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
    cancellation.map_or_else(
        || service.execute(snapshot),
        |scope| service.execute_with_cancellation(snapshot, scope),
    )
}

fn check_preview_cancellation(
    cancellation: Option<&CancellationScope>,
    stage: CancellationStage,
) -> Result<(), PreviewError> {
    if let Some(scope) = cancellation {
        scope
            .child(stage)
            .check()
            .map_err(|error| PreviewError::Pixelpipe(CpuPixelpipeError::Cancelled(error)))?;
    }
    Ok(())
}

fn decode_preview_frame(source: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, PreviewError> {
    FileImageInput::new(limits)
        .decode_linear_frame_bytes(source)
        .map_err(|error| {
            let failure = recover_raw_decode_error(source, limits, &error).map_or_else(
                || PreviewError::Decode(error),
                |raw| PreviewError::RawDecode(Box::new(raw)),
            );
            tracing::error!(
                target: "rusttable.preview",
                stage = "decode",
                cause = failure.diagnostic_cause()
            );
            failure
        })
}

fn recover_raw_decode_error(
    source: &[u8],
    limits: DecodeLimits,
    error: &ImageInputError,
) -> Option<RawDecodeError> {
    if !matches!(
        error,
        ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            ..
        }
    ) {
        return None;
    }
    let raw_limits = RawDecodeLimits::new(
        limits.max_source_bytes(),
        limits.max_width(),
        limits.max_height(),
        limits.max_pixel_count(),
        limits.max_decoded_bytes(),
    )
    .ok()?;
    RawlerRawDecoder::new()
        .decode_bytes(source, &RawDecodeRequest::new(raw_limits))
        .err()
}

fn prepare_frame(
    input: &DecodedFrame,
    edit: &Edit,
    cancellation: Option<&CancellationScope>,
) -> Result<PreparedCpuPixelpipeResult, PreviewError> {
    let presentation = if input.raw_source().is_some() {
        SrgbFallbackContract::SceneReferredRawV1
    } else {
        SrgbFallbackContract::Colorimetric
    };
    let mut graph = CompiledOperationGraph::compile(edit).map_err(PreviewError::Graph)?;
    let (source_pixels, dimensions, representation) = source_frame_pixels(input, &mut graph)?;
    let source_color = input.source_color();
    let source_color_decision = source_color_decision(source_color).inspect_err(|_| {
        tracing::error!(
            target: "rusttable.preview",
            stage = "processing",
            cause = "unsupported_frame_color"
        );
    })?;
    let encoding = pixelpipe_encoding(source_color);
    let pixelpipe_input = RgbaF32Image::new(
        RgbaF32Descriptor::with_source_representation(dimensions, encoding, representation)
            .with_source_orientation(input.image().descriptor().orientation())
            .with_source_color(source_color),
        source_pixels
            .iter()
            .copied()
            .map(|pixel| RgbaF32Pixel::new(pixel[0], pixel[1], pixel[2], pixel[3]))
            .collect(),
    )
    .map_err(PreviewError::PixelpipeInput)?;
    let snapshot =
        CpuPixelpipeSnapshot::try_new(pixelpipe_input, graph, CpuPixelpipeOutputMode::FullExport)
            .map_err(PreviewError::PixelpipeSnapshot)?;
    let result =
        execute_production_snapshot(&snapshot, cancellation).map_err(PreviewError::Pixelpipe)?;
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
    let output_dimensions = result.image().descriptor().dimensions();
    debug_assert_eq!(
        result.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65,
        "FullExport must publish canonical linear sRGB"
    );
    // FullExport has already converted any intermediate ColorIn working frame
    // to canonical linear sRGB. Retagging these pixels with that intermediate
    // frame would make the render boundary convert them a second time.
    let working = WorkingRgbImage::new(output_dimensions, pixels)
        .expect("CPU pixelpipe result dimensions match its validated pixels");
    let alpha = result
        .image()
        .pixels()
        .iter()
        .map(|pixel| pixel.alpha())
        .collect();
    let prepared = PreparedCpuPixelpipeResult::new_with_source_color(
        working,
        alpha,
        source_color_decision,
        source_color,
        RenderProvenance::new(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
        ),
    )
    .map_err(PreviewError::Prepared)?
    .with_presentation(presentation);
    Ok(prepared)
}

fn source_frame_pixels(
    input: &DecodedFrame,
    graph: &mut CompiledOperationGraph,
) -> Result<(Vec<[f32; 4]>, RasterDimensions, RgbaF32SourceRepresentation), PreviewError> {
    let temperature = pre_demosaic_temperature(graph).map_err(PreviewError::RawPipeline)?;
    // The native RAW decoder has already run raw color calibration while
    // constructing `DecodedFrame::image()`. Re-demosaicing that frame here
    // would discard the decoder's camera matrix/white-balance work and yields
    // the green/cyan cast seen on camera RAWs. Keep the native RAW path only
    // when an edit explicitly requires a pre-demosaic temperature operation;
    // otherwise use the calibrated linear frame as the pixelpipe input.
    if let (Some(raw_source), Some(temperature)) = (input.raw_source(), temperature) {
        let plan = RawPipelinePlan::new(raw_source, Some(temperature), DemosaicAlgorithm::Bilinear)
            .map_err(PreviewError::RawPipeline)?;
        let execution = plan
            .execute(raw_source)
            .map_err(PreviewError::RawPipeline)?;
        if plan.receipt().temperature_applied_once() {
            *graph = graph.without_pre_demosaic_temperature();
        }
        let dimensions = execution.image().dimensions();
        let source_pixels = execution
            .image()
            .pixels()
            .iter()
            .map(|pixel| {
                [
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    1.0,
                ]
            })
            .collect();
        return Ok((source_pixels, dimensions, RgbaF32SourceRepresentation::F32));
    }

    let source_pixels = input
        .rgba_f32_pixels()
        .map_err(|_| PreviewError::DecodedFrame)?;
    let dimensions = RasterDimensions::new(
        input.image().descriptor().dimensions().width(),
        input.image().descriptor().dimensions().height(),
    )
    .expect("decoded images have nonzero dimensions");
    let representation = match input.sample_type() {
        SampleType::U8 => RgbaF32SourceRepresentation::U8,
        SampleType::U16 => RgbaF32SourceRepresentation::U16,
        SampleType::F16 => RgbaF32SourceRepresentation::F16,
        SampleType::F32 => RgbaF32SourceRepresentation::F32,
    };
    Ok((source_pixels, dimensions, representation))
}

fn render_with_target(
    input: &DecodedImage,
    edit: &Edit,
    target: RenderTarget,
    cancellation: Option<&CancellationScope>,
) -> Result<RenderOutput, PreviewError> {
    render_decoded_with_target(input, edit, target, cancellation)
}

fn render_decoded_with_target(
    input: &DecodedImage,
    edit: &Edit,
    target: RenderTarget,
    cancellation: Option<&CancellationScope>,
) -> Result<RenderOutput, PreviewError> {
    let source_color = if input.color_encoding() == ColorEncoding::Unspecified {
        SourceColor::fallback(SourceColorFallback::EncodedSrgb)
    } else {
        SourceColor::from_encoding(input.color_encoding()).map_err(|_| {
            PreviewError::UnsupportedPixelpipeColor {
                actual: input.color_encoding(),
            }
        })?
    };
    let source_color_decision = source_color_decision(source_color).inspect_err(|_| {
        tracing::error!(target: "rusttable.preview", stage = "processing", cause = "unsupported_color");
    })?;
    let dimensions = RasterDimensions::new(input.dimensions().width(), input.dimensions().height())
        .expect("decoded images have nonzero dimensions");
    let (source_pixels, remainder) = input.pixels().as_chunks::<4>();
    debug_assert!(remainder.is_empty(), "decoded images are packed RGBA8");
    let pixelpipe_input = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, pixelpipe_encoding(source_color))
            .with_source_orientation(input.source_orientation())
            .with_source_color(source_color),
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
    let result = execute_production_snapshot(&snapshot, cancellation).map_err(|error| {
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
    let output_dimensions = result.image().descriptor().dimensions();
    debug_assert_eq!(
        result.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65,
        "FullExport must publish canonical linear sRGB"
    );
    // FullExport has already converted any intermediate ColorIn working frame
    // to canonical linear sRGB. Retagging these pixels with that intermediate
    // frame would make the render boundary convert them a second time.
    let working = WorkingRgbImage::new(output_dimensions, pixels)
        .expect("CPU pixelpipe result dimensions match its validated pixels");
    let alpha = result
        .image()
        .pixels()
        .iter()
        .map(|pixel| pixel.alpha())
        .collect();
    let prepared = PreparedCpuPixelpipeResult::new_with_source_color(
        working,
        alpha,
        source_color_decision,
        source_color,
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
    check_preview_cancellation(cancellation, CancellationStage::Publication)?;
    let output = render_prepared_cpu_pixelpipe(&prepared, target).map_err(|error| {
        tracing::error!(target: "rusttable.preview", stage = "render", cause = "target_adaptation");
        PreviewError::Render(error)
    })?;
    check_preview_cancellation(cancellation, CancellationStage::Publication)?;
    Ok(output)
}

fn source_color_decision(source: SourceColor) -> Result<SourceColorDecision, PreviewError> {
    match source.evidence() {
        SourceColorEvidence::EmbeddedIcc => Ok(SourceColorDecision::EmbeddedProfile),
        SourceColorEvidence::EmbeddedChromaticities
        | SourceColorEvidence::EmbeddedContainerMetadata => {
            Ok(SourceColorDecision::EmbeddedChromaticities)
        }
        SourceColorEvidence::Fallback(SourceColorFallback::EncodedSrgb) => {
            Ok(SourceColorDecision::AssumedSrgb)
        }
        SourceColorEvidence::Fallback(SourceColorFallback::LinearRec709) => {
            Ok(SourceColorDecision::AssumedLinearRec709)
        }
        SourceColorEvidence::DeclaredEncoding => match source.encoding() {
            ColorEncoding::SrgbD65 | ColorEncoding::LinearSrgbD65 => {
                Ok(SourceColorDecision::DeclaredSrgb)
            }
            ColorEncoding::DisplayP3D65 | ColorEncoding::LinearDisplayP3D65 => {
                Ok(SourceColorDecision::DeclaredDisplayP3)
            }
            actual => Err(PreviewError::UnsupportedPixelpipeColor { actual }),
        },
    }
}

fn pixelpipe_encoding(source: SourceColor) -> RgbaF32ColorEncoding {
    match source.encoding() {
        ColorEncoding::LinearSrgbD65 => RgbaF32ColorEncoding::LinearSrgbD65,
        ColorEncoding::DisplayP3D65 => RgbaF32ColorEncoding::DisplayP3D65,
        ColorEncoding::LinearDisplayP3D65 => RgbaF32ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::External(profile) => RgbaF32ColorEncoding::External(profile),
        _ => RgbaF32ColorEncoding::SrgbD65,
    }
}

#[derive(Debug)]
pub enum PreviewError {
    Decode(rusttable_image::ImageInputError),
    RawDecode(Box<RawDecodeError>),
    DecodedFrame,
    UnsupportedPixelpipeColor { actual: ColorEncoding },
    PixelpipeInput(RgbaF32ImageError),
    PixelpipeSnapshot(CpuPixelpipeSnapshotError),
    Graph(rusttable_processing::OperationGraphCompileError),
    RawPipeline(rusttable_processing::RawPipelineError),
    Pixelpipe(CpuPixelpipeError),
    Prepared(PreparedCpuPixelpipeResultError),
    Render(rusttable_render::RenderError),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "preview decode failed: {error}"),
            Self::RawDecode(error) => write!(formatter, "preview RAW decode failed: {error}"),
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
            Self::RawPipeline(error) => write!(formatter, "preview RAW pipeline failed: {error}"),
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
            Self::RawDecode(error) => Some(error.as_ref()),
            Self::DecodedFrame | Self::UnsupportedPixelpipeColor { .. } => None,
            Self::PixelpipeInput(error) => Some(error),
            Self::PixelpipeSnapshot(error) => Some(error),
            Self::Graph(error) => Some(error),
            Self::RawPipeline(error) => Some(error),
            Self::Pixelpipe(error) => Some(error),
            Self::Prepared(error) => Some(error),
            Self::Render(error) => Some(error),
        }
    }
}

impl PreviewError {
    fn diagnostic_cause(&self) -> &'static str {
        match self {
            Self::RawDecode(error) if matches!(error.as_ref(), RawDecodeError::Capability(_)) => {
                "raw_capability"
            }
            Self::RawDecode(_) => "raw_decode",
            Self::Decode(_) => "decode_failed",
            Self::DecodedFrame
            | Self::UnsupportedPixelpipeColor { .. }
            | Self::PixelpipeInput(_)
            | Self::PixelpipeSnapshot(_)
            | Self::Graph(_)
            | Self::RawPipeline(_)
            | Self::Pixelpipe(_)
            | Self::Prepared(_)
            | Self::Render(_) => "render_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
        ParameterName, ParameterText, ParameterValue, PhotoId, Revision,
    };
    use rusttable_gpu::{BilateralGridError, GpuRuntime, GpuRuntimeConfig};
    use rusttable_image::{ColorEncoding, DecodeLimits, DecodedImage, ImageDimensions};
    use rusttable_pixelpipe::{
        CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeOutputMode,
        CpuPixelpipeSnapshot, PipelineGeneration, PixelpipeBackend, PixelpipeExecutionService,
        PixelpipeGpuFallback, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
    };
    use rusttable_processing::{CompiledOperationGraph, RasterDimensions, WorkingFrameDescriptor};
    use rusttable_render::{PreviewBounds, RenderTarget};

    use super::{PreviewService, execute_snapshot};

    fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).expect("operation ID"),
            OperationKey::new(key).expect("operation key"),
            true,
            OperationOpacity::new(1.0).expect("operation opacity"),
            parameters.iter().map(|(name, value)| {
                (
                    ParameterName::new(*name).expect("parameter name"),
                    ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
                )
            }),
        )
        .expect("operation")
    }

    fn shadhi_operation(id: u128) -> Operation {
        operation(
            id,
            "rusttable.shadhi",
            &[
                ("order", 0.0),
                ("radius", 2.0),
                ("shadows", 35.0),
                ("whitepoint", 0.0),
                ("highlights", -30.0),
                ("reserved2", 0.0),
                ("compress", 50.0),
                ("shadows_ccorrect", 100.0),
                ("highlights_ccorrect", 50.0),
                ("flags", 127.0),
                ("low_approximation", 0.000_001),
                ("shadhi_algo", 1.0),
            ],
        )
    }

    fn colorin_operation(id: u128, enabled: bool, opacity: f64) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).expect("operation ID"),
            OperationKey::new("rusttable.colorin").expect("operation key"),
            enabled,
            OperationOpacity::new(opacity).expect("operation opacity"),
            [
                (
                    ParameterName::new("input_profile").expect("parameter name"),
                    ParameterValue::Text(ParameterText::new("builtin:srgb").expect("text")),
                ),
                (
                    ParameterName::new("working_profile").expect("parameter name"),
                    ParameterValue::Text(
                        ParameterText::new("builtin:linear-rec2020").expect("text"),
                    ),
                ),
                (
                    ParameterName::new("intent").expect("parameter name"),
                    ParameterValue::Integer(0),
                ),
                (
                    ParameterName::new("normalize").expect("parameter name"),
                    ParameterValue::Integer(0),
                ),
                (
                    ParameterName::new("blue_mapping").expect("parameter name"),
                    ParameterValue::Bool(true),
                ),
            ],
        )
        .expect("ColorIn operation")
    }

    fn edit(operations: Vec<Operation>) -> Edit {
        Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(3),
            operations,
        )
        .expect("edit")
    }

    fn snapshot(operations: Vec<Operation>) -> CpuPixelpipeSnapshot {
        let graph =
            CompiledOperationGraph::compile(&edit(operations)).expect("compiled operation graph");
        let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
        let pixels = (0_u16..16)
            .map(|index| {
                let x = f32::from(index % 4);
                let y = f32::from(index / 4);
                RgbaF32Pixel::new(
                    0.08 + x * 0.12,
                    0.12 + y * 0.11,
                    0.16 + (x + y) * 0.06,
                    0.25 + f32::from(index % 4) * 0.2,
                )
            })
            .collect();
        let image = RgbaF32Image::new(
            RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
            pixels,
        )
        .expect("image");
        CpuPixelpipeSnapshot::new(image, graph, CpuPixelpipeOutputMode::FullExport)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn app_boundary_routes_eligible_bilateral_and_publishes_canonical_fallbacks() {
        let mut config = GpuRuntimeConfig::default();
        config.policy.backends.clear();
        let runtime = GpuRuntime::initialize(config)
            .await
            .expect("CPU-only runtime");
        let unavailable_gpu = PixelpipeExecutionService::with_gpu(runtime);
        let canonical_cpu = PixelpipeExecutionService::cpu_only();

        let bilateral = snapshot(vec![shadhi_operation(10)]);
        let canonical_bilateral =
            execute_snapshot(&canonical_cpu, &bilateral, None).expect("canonical bilateral");
        let selected_bilateral =
            execute_snapshot(&unavailable_gpu, &bilateral, None).expect("bilateral fallback");
        assert_eq!(
            selected_bilateral.receipt().backend(),
            PixelpipeBackend::CpuCanonical
        );
        assert_eq!(
            selected_bilateral.receipt().gpu_fallback(),
            Some(&PixelpipeGpuFallback::Bilateral(
                BilateralGridError::CpuOnly
            ))
        );
        assert_eq!(selected_bilateral.image(), canonical_bilateral.image());

        let unsupported = snapshot(vec![
            operation(9, "rusttable.linear_offset", &[("value", 0.01)]),
            shadhi_operation(10),
        ]);
        let canonical_unsupported =
            execute_snapshot(&canonical_cpu, &unsupported, None).expect("canonical mixed graph");
        let selected_unsupported =
            execute_snapshot(&unavailable_gpu, &unsupported, None).expect("mixed graph fallback");
        assert_eq!(
            selected_unsupported.receipt().backend(),
            PixelpipeBackend::CpuCanonical
        );
        assert!(selected_unsupported.receipt().gpu_fallback().is_none());
        assert_eq!(selected_unsupported.image(), canonical_unsupported.image());
    }

    #[test]
    fn full_export_colorin_output_remains_canonical_linear_srgb() {
        let service = PreviewService::new(
            DecodeLimits::new(4096, 16, 16, 256, 1024).expect("decode limits"),
            PreviewBounds::new(16, 16).expect("preview bounds"),
        );
        let input = DecodedImage::new_with_color_encoding(
            ImageDimensions::new(2, 1).expect("image dimensions"),
            vec![32, 96, 224, 51, 240, 112, 16, 204],
            ColorEncoding::Srgb,
        )
        .expect("decoded image");
        let identity = service
            .render_decoded_for_target(&input, &edit(vec![]), RenderTarget::FullResolution)
            .expect("identity render");

        for (enabled, opacity) in [(true, 1.0), (false, 1.0), (true, 0.0)] {
            let output = service
                .render_decoded_for_target(
                    &input,
                    &edit(vec![colorin_operation(10, enabled, opacity)]),
                    RenderTarget::FullResolution,
                )
                .expect("ColorIn render");
            assert_eq!(
                output.working_profile(),
                WorkingFrameDescriptor::srgb(),
                "FullExport pixels must not be retagged with the intermediate ColorIn frame"
            );
            assert_eq!(output.image().color_encoding(), ColorEncoding::Srgb);
            if !enabled || opacity == 0.0 {
                assert_eq!(output.image(), identity.image());
            }
        }
    }

    #[test]
    fn application_execution_boundary_observes_generation_cancellation() {
        let service = PixelpipeExecutionService::cpu_only();
        let request = snapshot(vec![]);
        let first_generation = PipelineGeneration::new(1).expect("first generation");
        let second_generation = PipelineGeneration::new(2).expect("second generation");
        let superseded = CancellationScope::root(first_generation);
        superseded.cancel(CancellationReason::SupersededGeneration(second_generation));
        let error = execute_snapshot(&service, &request, Some(&superseded))
            .expect_err("superseded render must stop");
        assert!(matches!(
            error,
            CpuPixelpipeError::Cancelled(error)
                if error.reason() == CancellationReason::SupersededGeneration(second_generation)
        ));

        execute_snapshot(
            &service,
            &request,
            Some(&CancellationScope::root(second_generation)),
        )
        .expect("current generation publishes");
    }

    #[test]
    fn production_preview_only_executes_snapshots_through_the_shared_service() {
        let production_source = include_str!("preview.rs")
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .expect("production source");
        assert!(
            !production_source.contains(concat!("CpuPixelpipe", "Executor")),
            "production preview must not construct the canonical executor directly"
        );
        assert_eq!(
            production_source
                .matches("execute_production_snapshot(&snapshot,")
                .count(),
            2,
            "both production snapshot paths must use the shared execution service"
        );
    }
}
