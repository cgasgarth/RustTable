#![forbid(unsafe_code)]
#![doc = "Compiled operation data for the `RustTable` processing pipeline."]

mod operation;
mod pipeline;
mod scalar;

pub use operation::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
pub use pipeline::{CompiledPipeline, PipelineCompileError, PipelineStep, PipelineStepIndex};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
