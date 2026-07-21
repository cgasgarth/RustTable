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
pub mod defringe_compatibility;
pub mod demosaic;
pub mod descriptor;
mod evaluate;
mod exposure;
mod graph;
mod operation;
pub mod operation_stack;
pub mod operations;
mod output;
mod pipeline;
pub mod rawprepare;
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
pub use demosaic::{DemosaicAlgorithm, DemosaicError, DemosaicPlan, DemosaicedImage};
pub use descriptor::{
    basicadj_descriptor, bloom_descriptor, censorize_descriptor, clahe_descriptor,
    clipping_descriptor, color_reconstruction_descriptor, colorin_descriptor, crop_descriptor,
    defringe_descriptor, dither_descriptor, enlargecanvas_descriptor, exposure_descriptor,
    finalscale_descriptor, flip_descriptor, graduatednd_descriptor, grain_descriptor,
    highlights_descriptor, invert_descriptor, lenscorrection_descriptor, linear_offset_descriptor,
    liquify_descriptor, mask_manager_descriptor, perspective_descriptor, primaries_descriptor,
    retouch_descriptor, rgb_gain_descriptor, rotatepixels_descriptor, scalepixels_descriptor,
    soften_descriptor, temperature_descriptor, vignette_descriptor,
};
pub use evaluate::{
    BasicAdjPlanSet, BlendArithmeticStage, EvaluationError, evaluate, prepare_basicadj_plans,
};
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
pub use operations::basicadj::{
    BasicAdjAutoControls, BasicAdjConfig, BasicAdjConfigError, BasicAdjExecutionReceipt,
    BasicAdjGpuParameters, BasicAdjParametersV1, BasicAdjParametersV2, BasicAdjPlan,
    BasicAdjPlanError, BasicAdjustmentsPlan, PreserveColors, migrate_v1_to_v2,
};
pub use operations::basicadj_analysis::{
    BASICADJ_HISTOGRAM_BINS, BASICADJ_HISTOGRAM_MAXIMUM, BASICADJ_HISTOGRAM_MINIMUM,
    BASICADJ_MAX_ANALYSIS_PIXELS, BasicAdjAnalysisError, BasicAdjAnalysisPlan,
    BasicAdjAnalysisRaster, BasicAdjAnalysisResult, BasicAdjAnalysisRoi, BasicAdjResolvedValues,
};
pub use operations::censorize::{
    CENSORIZE_COMPATIBILITY_ID, CENSORIZE_PARAMETER_BYTES, CENSORIZE_RNG_VERSION,
    CENSORIZE_SCHEMA_VERSION, CensorizeBackend, CensorizeBlend, CensorizeCodecError,
    CensorizeConfig, CensorizeExecutionError, CensorizeHistory, CensorizeMask,
    CensorizeParameterError, CensorizeParametersV1, CensorizePixel, CensorizePlan,
    CensorizeReceipt, CensorizeRng, CensorizeStages, gaussian_noise, splitmix32, xoshiro128plus,
};
pub use operations::clahe::{
    CLAHE_ALIAS, CLAHE_BINS, CLAHE_COMPATIBILITY_ID, CLAHE_HISTOGRAM_ENTRIES,
    CLAHE_PARAMETER_BYTES, CLAHE_RADIUS_DEFAULT, CLAHE_RADIUS_MAX, CLAHE_RADIUS_MIN,
    CLAHE_SCHEMA_VERSION, CLAHE_SLOPE_DEFAULT, CLAHE_SLOPE_MAX, CLAHE_SLOPE_MIN, ClaheBackend,
    ClaheBlend, ClaheCodecError, ClaheConfig, ClaheExecutionError, ClaheHistory, ClaheMask,
    ClaheOutcome, ClaheParameterError, ClaheParametersV1, ClahePixel, ClahePlan, ClaheReceipt,
};
pub use operations::defringe::{
    DEFRINGE_ALIAS, DEFRINGE_COMPATIBILITY_ID, DEFRINGE_GAUSSIAN_ORDER,
    DEFRINGE_MAGIC_THRESHOLD_COEFFICIENT, DEFRINGE_PARAMETER_BYTES, DEFRINGE_RADIUS_DEFAULT,
    DEFRINGE_RADIUS_MAX, DEFRINGE_RADIUS_MIN, DEFRINGE_SCHEMA_VERSION, DEFRINGE_THRESHOLD_DEFAULT,
    DEFRINGE_THRESHOLD_MAX, DEFRINGE_THRESHOLD_MIN, DefringeAnalysis, DefringeBackend,
    DefringeBlend, DefringeCodecError, DefringeConfig, DefringeExecutionError, DefringeHistory,
    DefringeMask, DefringeMode, DefringeOutcome, DefringeParameterError, DefringeParametersV1,
    DefringePixel, DefringePlan, DefringeReceipt,
};
pub use operations::grain::{GrainGpuParameters, GrainPlan};
pub use operations::liquify::{
    LIQUIFY_COMPATIBILITY_ID, LIQUIFY_PARAMETER_BYTES, LIQUIFY_SCHEMA_VERSION, LiquifyConfig,
    LiquifyExecution, LiquifyExecutionError, LiquifyGpuDispatch, LiquifyInterpolation, LiquifyNode,
    LiquifyNodeType, LiquifyParametersV1, LiquifyPathKind, LiquifyPlan, LiquifyPoint,
    LiquifyStatus, LiquifyWarpType,
};
pub use operations::mask_manager::{MaskManagerError, MaskManagerParameters};
pub use operations::retouch::{
    RETOUCH_MAX_SCALES, RETOUCH_SCHEMA_VERSION, RetouchAlgorithm, RetouchBlurType, RetouchConfig,
    RetouchConfigError, RetouchExecutionError, RetouchFillMode, RetouchForm, RetouchParameters,
    RetouchPixel, RetouchPlan, RetouchReceipt, RetouchScale,
};
pub use operations::spots::{
    SPOTS_COMPATIBILITY_ID, SPOTS_IMPLEMENTATION_VERSION, SPOTS_MAX_ENTRIES,
    SPOTS_PARAMETER_BYTES_V1, SPOTS_PARAMETER_BYTES_V2, SPOTS_RUST_ID, SPOTS_SCHEMA_VERSION,
    SpotsCodecError, SpotsConfig, SpotsEntry, SpotsExecutionError, SpotsForm, SpotsFormKind,
    SpotsHistory, SpotsLegacySpot, SpotsMode, SpotsParametersV1, SpotsParametersV2, SpotsPlan,
    SpotsReceipt,
};
pub use output::{
    ChannelCounts, EncodedSrgb, EncodedSrgbImage, EncodedSrgbOutput, GamutClipReport,
    encode_linear_srgb,
};
pub use pipeline::{CompiledPipeline, PipelineCompileError, PipelineStep, PipelineStepIndex};
pub use rawprepare::{NormalizedRaw, RawPrepareConfig, RawPrepareError, RawPreparePlan};
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
    evaluate_graph, evaluate_graph_window, evaluate_graph_with_basicadj_plans,
};

pub use operation_stack::{
    CommandReceipt, InsertPosition, MigrationFinding, MigrationOutcome, MoveTarget,
    OpaqueOperation, OperationInstance, OperationStackError, OperationStackResult,
    OperationStackSnapshot, OperationStackTemplate, StackCommand, StackStage, StackStageFence,
};
