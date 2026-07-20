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
pub mod descriptor;
mod evaluate;
mod exposure;
mod graph;
mod operation;
pub mod operation_stack;
pub mod operations;
mod output;
mod pipeline;
pub mod registry;
mod registry_closure;
mod registry_color;
mod registry_reconstruction;
mod scalar;
mod window;

pub use color::{
    DisplayP3Channel, DisplayP3ChannelError, DisplayP3Rgb, DisplayP3RgbImage, ImageBuildError,
    LinearRgb, RasterDimensions, RasterDimensionsError, RgbChannel, SourceColorSpace, SourceRgb,
    SourceRgbImage, SrgbChannel, SrgbChannelError, WorkingColorSpace, WorkingRgbImage,
    to_linear_srgb, to_linear_srgb_from_display_p3,
};
pub use descriptor::{
    color_reconstruction_descriptor, colorin_descriptor, exposure_descriptor,
    highlights_descriptor, linear_offset_descriptor, primaries_descriptor, rgb_gain_descriptor,
};
pub use evaluate::{BlendArithmeticStage, EvaluationError, evaluate};
pub use exposure::{
    BLACK_LEVEL_MAXIMUM, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_SOFT_MAXIMUM, BLACK_LEVEL_SOFT_MINIMUM,
    DEFAULT_BLACK_LEVEL, DEFAULT_EXPOSURE_EV, EXPOSURE_EV_MAXIMUM, EXPOSURE_EV_MINIMUM,
    EXPOSURE_EV_SOFT_MAXIMUM, EXPOSURE_EV_SOFT_MINIMUM, ExposureAction, ExposureActionError,
    ExposureMode, ExposureModuleState,
};
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
pub use registry::{
    BUILTIN_OPERATIONS, CpuFactory, DefinitionAvailability, DeviceCapabilitySnapshot,
    ExecutionBackend, FactoryError, GpuBinding, ImplementationIdentity, MigrationBinding,
    OperationCapability, OperationDefinition, OperationDefinitionFactory, PreparedCpuOperation,
    RegistryBuildError, RegistryLookupError, RegistrySnapshot, RegistryValidationError,
    builtin_registry,
};
pub use registry_closure::{
    OperationClassification, REGISTRY_CLOSURE_SCHEMA, RegistryClosure, RegistryClosureEntry,
    RegistryClosureError,
};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
pub use window::{
    EvaluatedRowWindow, GraphWindowEvaluationError, RasterRowWindow, RasterRowWindowError,
    evaluate_graph, evaluate_graph_window,
};

pub use operation_stack::{
    CommandReceipt, InsertPosition, MigrationFinding, MigrationOutcome, MoveTarget,
    OpaqueOperation, OperationInstance, OperationStackError, OperationStackResult,
    OperationStackSnapshot, OperationStackTemplate, StackCommand, StackStage, StackStageFence,
};
