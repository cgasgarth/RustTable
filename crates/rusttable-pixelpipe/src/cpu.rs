use std::fmt;

use rusttable_processing::{
    CompiledOperationGraph, EvaluationError, SourceRgb, SourceRgbImage, SrgbChannel,
    evaluate_graph, to_linear_srgb,
};

use crate::{
    CpuNodeReceipt, CpuPipelineReceipt, PixelIdentity, RgbaF32Channel, RgbaF32ColorEncoding,
    RgbaF32Descriptor, RgbaF32Image, RgbaF32ImageError, RgbaF32Pixel,
};

/// Immutable prepared CPU execution input.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuPixelpipeRequest {
    input: RgbaF32Image,
    graph: CompiledOperationGraph,
}

impl CpuPixelpipeRequest {
    #[must_use]
    pub fn new(input: RgbaF32Image, graph: CompiledOperationGraph) -> Self {
        Self { input, graph }
    }

    #[must_use]
    pub const fn input(&self) -> &RgbaF32Image {
        &self.input
    }

    #[must_use]
    pub const fn graph(&self) -> &CompiledOperationGraph {
        &self.graph
    }
}

/// Immutable output from the registered scalar CPU executor.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuPixelpipeResult {
    image: RgbaF32Image,
    receipt: CpuPipelineReceipt,
}

impl CpuPixelpipeResult {
    #[must_use]
    pub const fn image(&self) -> &RgbaF32Image {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> &CpuPipelineReceipt {
        &self.receipt
    }
}

/// Failure from the narrow, full-frame CPU pixelpipe executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuPixelpipeError {
    UnsupportedInputEncoding { actual: RgbaF32ColorEncoding },
    InputBridge { source: RgbaF32ImageError },
    Evaluation { source: EvaluationError },
    OutputBoundary { source: RgbaF32ImageError },
}

/// The canonical scalar CPU executor for registered processing operations.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuPixelpipeExecutor;

impl CpuPixelpipeExecutor {
    /// Executes a prepared graph in authored order without interpreting operation names.
    ///
    /// The executor accepts normalized transfer-encoded sRGB, converts it once
    /// to linear sRGB, delegates registered nodes to `rusttable-processing`,
    /// and preserves straight alpha through the RGB-only operation boundary.
    ///
    /// # Errors
    ///
    /// Returns a typed failure before exposing a partial output image.
    pub fn execute(
        &self,
        request: &CpuPixelpipeRequest,
    ) -> Result<CpuPixelpipeResult, CpuPixelpipeError> {
        let input_descriptor = request.input().descriptor();
        if input_descriptor.color_encoding() != RgbaF32ColorEncoding::SrgbD65 {
            return Err(CpuPixelpipeError::UnsupportedInputEncoding {
                actual: input_descriptor.color_encoding(),
            });
        }

        let source = to_processing_source(request.input())?;
        let linear_input = to_linear_srgb(&source);
        let evaluated = evaluate_graph(request.graph(), &linear_input)
            .map_err(|source| CpuPixelpipeError::Evaluation { source })?;
        let output_descriptor = RgbaF32Descriptor::new(
            input_descriptor.dimensions(),
            RgbaF32ColorEncoding::LinearSrgbD65,
        );
        let output_pixels = evaluated
            .pixels()
            .zip(request.input().pixels())
            .map(|(rgb, source)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    source.alpha(),
                )
            })
            .collect();
        let image = RgbaF32Image::new(output_descriptor, output_pixels)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })?;
        let receipt = CpuPipelineReceipt::new(
            input_descriptor,
            output_descriptor,
            pixel_identity(request.input()),
            pixel_identity(&image),
            request
                .graph()
                .nodes()
                .map(|node| {
                    CpuNodeReceipt::new(node.index().get(), node.operation().operation_id())
                })
                .collect(),
        );
        Ok(CpuPixelpipeResult { image, receipt })
    }
}

fn to_processing_source(input: &RgbaF32Image) -> Result<SourceRgbImage, CpuPixelpipeError> {
    let pixels = input
        .pixels()
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel_index, pixel)| {
            let red = SrgbChannel::new(pixel.red())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Red))?;
            let green = SrgbChannel::new(pixel.green())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Green))?;
            let blue = SrgbChannel::new(pixel.blue())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Blue))?;
            Ok(SourceRgb::new(red, green, blue))
        })
        .collect::<Result<Vec<_>, _>>()?;
    SourceRgbImage::new(input.descriptor().dimensions(), pixels).map_err(|_| {
        CpuPixelpipeError::InputBridge {
            source: RgbaF32ImageError::PixelCountMismatch {
                expected: input.descriptor().dimensions().pixel_count(),
                actual: input.pixels().len(),
            },
        }
    })
}

const fn input_component_error(pixel_index: usize, channel: RgbaF32Channel) -> CpuPixelpipeError {
    CpuPixelpipeError::InputBridge {
        source: RgbaF32ImageError::ComponentOutsideUnitInterval {
            pixel_index,
            channel,
        },
    }
}

fn pixel_identity(image: &RgbaF32Image) -> PixelIdentity {
    PixelIdentity::from_components(
        image
            .pixels()
            .iter()
            .flat_map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]),
    )
}

impl fmt::Display for CpuPixelpipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedInputEncoding { actual } => {
                write!(formatter, "CPU pixelpipe does not accept {actual:?} input")
            }
            Self::InputBridge { source } => write!(formatter, "invalid CPU input bridge: {source}"),
            Self::Evaluation { source } => {
                write!(formatter, "CPU operation evaluation failed: {source}")
            }
            Self::OutputBoundary { source } => {
                write!(formatter, "invalid CPU output boundary: {source}")
            }
        }
    }
}

impl std::error::Error for CpuPixelpipeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::UnsupportedInputEncoding { .. } => None,
            Self::InputBridge { source } | Self::OutputBoundary { source } => Some(source),
            Self::Evaluation { source } => Some(source),
        }
    }
}
