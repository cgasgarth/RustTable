#![forbid(unsafe_code)]
#![doc = "Compiled operation data for the `RustTable` processing pipeline."]
#![doc = "Source RGB images and linear-light working RGB images are distinct types. Conversion is explicit via [`to_linear_srgb`]."]
#![doc = ""]
#![doc = "```compile_fail"]
#![doc = "use rusttable_processing::{SourceRgbImage, WorkingRgbImage};"]
#![doc = "fn takes_working(_: WorkingRgbImage) {}"]
#![doc = "fn takes_source(source: SourceRgbImage) { takes_working(source); }"]
#![doc = "```"]
#![doc = "```compile_fail"]
#![doc = "use rusttable_processing::{to_linear_srgb, DisplayP3RgbImage};"]
#![doc = "fn converts(source: &DisplayP3RgbImage) { let _ = to_linear_srgb(source); }"]
#![doc = "```"]
#![doc = "```compile_fail"]
#![doc = "use rusttable_processing::{to_linear_srgb_from_display_p3, SourceRgbImage};"]
#![doc = "fn converts(source: &SourceRgbImage) { let _ = to_linear_srgb_from_display_p3(source); }"]
#![doc = "```"]

mod color;
mod evaluate;
mod graph;
mod operation;
mod output;
mod pipeline;
mod scalar;
mod window;

pub use color::{
    DisplayP3Channel, DisplayP3ChannelError, DisplayP3Rgb, DisplayP3RgbImage, ImageBuildError,
    LinearRgb, RasterDimensions, RasterDimensionsError, RgbChannel, SourceColorSpace, SourceRgb,
    SourceRgbImage, SrgbChannel, SrgbChannelError, WorkingColorSpace, WorkingRgbImage,
    to_linear_srgb, to_linear_srgb_from_display_p3,
};
pub use evaluate::{BlendArithmeticStage, EvaluationError, evaluate};
pub use graph::{
    CompiledOperationGraph, OperationGraphCompileError, OperationGraphInput, OperationGraphNode,
    OperationGraphNodeIndex, OperationGraphOutput,
};
pub use operation::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
pub use output::{
    ChannelCounts, EncodedSrgb, EncodedSrgbImage, EncodedSrgbOutput, GamutClipReport,
    encode_linear_srgb,
};
pub use pipeline::{CompiledPipeline, PipelineCompileError, PipelineStep, PipelineStepIndex};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
pub use window::{
    EvaluatedRowWindow, GraphWindowEvaluationError, RasterRowWindow, RasterRowWindowError,
    evaluate_graph, evaluate_graph_window,
};
