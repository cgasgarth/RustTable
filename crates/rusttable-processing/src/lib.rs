#![forbid(unsafe_code)]
#![doc = "Compiled operation data for the `RustTable` processing pipeline."]

mod operation;
mod scalar;

pub use operation::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
