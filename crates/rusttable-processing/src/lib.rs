#![forbid(unsafe_code)]
#![doc = "Compiled operation data for the `RustTable` processing pipeline."]
#![doc = "Source RGB images and linear-light working RGB images are distinct types. Conversion is explicit via [`to_linear_srgb`]."]
#![doc = ""]
#![doc = "```compile_fail"]
#![doc = "use rusttable_processing::{SourceRgbImage, WorkingRgbImage};"]
#![doc = "fn takes_working(_: WorkingRgbImage) {}"]
#![doc = "fn takes_source(source: SourceRgbImage) { takes_working(source); }"]
#![doc = "```"]

mod color;
mod evaluate;
mod operation;
mod output;
mod pipeline;
mod scalar;

pub use color::{
    ImageBuildError, LinearRgb, RasterDimensions, RasterDimensionsError, RgbChannel,
    SourceColorSpace, SourceRgb, SourceRgbImage, SrgbChannel, SrgbChannelError, WorkingColorSpace,
    WorkingRgbImage, to_linear_srgb,
};
pub use evaluate::{BlendArithmeticStage, EvaluationError, evaluate};
pub use operation::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
pub use output::{
    ChannelCounts, EncodedSrgb, EncodedSrgbImage, EncodedSrgbOutput, GamutClipReport,
    encode_linear_srgb,
};
pub use pipeline::{CompiledPipeline, PipelineCompileError, PipelineStep, PipelineStepIndex};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
