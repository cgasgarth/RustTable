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

extern crate self as rusttable_processing;

mod color;
pub mod common;
pub mod defringe_compatibility;
pub mod demosaic;
pub mod descriptor;
mod evaluate;
mod exposure;
mod graph;
mod operation;
mod operation_mask;
pub mod operation_stack;
pub mod operations;
mod output;
mod pipeline;
pub mod raw_pipeline;
pub mod rawprepare;
pub mod registry;
mod scalar;
mod window;

pub use color::{
    DisplayP3Channel, DisplayP3ChannelError, DisplayP3Rgb, DisplayP3RgbImage, ImageBuildError,
    LinearRgb, RasterDimensions, RasterDimensionsError, RgbChannel, SourceColorSpace, SourceRgb,
    SourceRgbImage, SrgbChannel, SrgbChannelError, WorkingColorSpace, WorkingFrameDescriptor,
    WorkingProfileProvenance, WorkingRgbImage, linear_display_p3_to_working,
    linear_srgb_to_working, to_linear_srgb, to_linear_srgb_from_display_p3,
};
pub use demosaic::{DemosaicAlgorithm, DemosaicError, DemosaicPlan, DemosaicedImage};
pub use descriptor::{
    agx_descriptor, basicadj_descriptor, bloom_descriptor, censorize_descriptor,
    channelmixer_descriptor, clahe_descriptor, clipping_descriptor,
    color_reconstruction_descriptor, colorcorrection_descriptor, colorin_descriptor,
    colorzones_descriptor, crop_descriptor, defringe_descriptor, dither_descriptor,
    enlargecanvas_descriptor, exposure_descriptor, finalscale_descriptor, flip_descriptor,
    graduatednd_descriptor, grain_descriptor, highlights_descriptor, invert_descriptor,
    lenscorrection_descriptor, levels_descriptor, linear_offset_descriptor, liquify_descriptor,
    mask_manager_descriptor, perspective_descriptor, primaries_descriptor, retouch_descriptor,
    rgb_gain_descriptor, rgblevels_descriptor, rotatepixels_descriptor, scalepixels_descriptor,
    soften_descriptor, temperature_descriptor, velvia_descriptor, vibrance_descriptor,
    vignette_descriptor,
};
pub use evaluate::{
    BasicAdjPlanSet, BlendArithmeticStage, DistortionBorderMode, DistortionInterpolation,
    DistortionPlan, DistortionSamplingPolicy, EvaluatedFrame, EvaluationError, EvaluationOutput,
    FrameBoundaryMode, FrameBoundaryOptions, FrameBoundaryPlan, ShadhiBilateralBoundaryError,
    ShadhiBilateralEvaluationError, evaluate, evaluate_bilateral_shadhi_with,
    evaluate_bilateral_shadhi_with_cancellation, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks, evaluate_output, graph_has_discrete_geometry,
    graph_has_frame_geometry, prepare_basicadj_plans, prepare_basicadj_plans_with_cancellation,
};
pub use exposure::{
    BLACK_LEVEL_MAXIMUM, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_SOFT_MAXIMUM, BLACK_LEVEL_SOFT_MINIMUM,
    DEFAULT_BLACK_LEVEL, DEFAULT_EXPOSURE_EV, EXPOSURE_EV_MAXIMUM, EXPOSURE_EV_MINIMUM,
    EXPOSURE_EV_SOFT_MAXIMUM, EXPOSURE_EV_SOFT_MINIMUM, ExposureAction, ExposureActionError,
    ExposureMode, ExposureModuleState,
};
pub use graph::{
    CompiledOperationGraph, OperationGraphCompileError, OperationGraphInput, OperationGraphNode,
    OperationGraphNodeIndex, OperationGraphOutput, RAW_SOURCE_PREPARATION_SEGMENT,
    RawSourcePreparationSegment,
};
pub use operation::{
    OperationCompileError, ProcessingOperation, ProcessingOperationKind, SourceProcessingOperation,
};
pub use operation_mask::{OperationMaskSet, OperationMaskSetError};
pub use operations::agx::{
    AGX_COMPATIBILITY_ID, AGX_PARAMETER_BYTES_V7, AGX_PARAMETER_FIELD_ORDER,
    AGX_PARAMETER_LAYOUT_HASH, AGX_RUST_ID, AGX_SCHEMA_VERSION, AgxBasePrimaries, AgxCodecError,
    AgxConfig, AgxExecutionError, AgxHistory, AgxParameterError, AgxParametersV7, AgxPixel,
    AgxPlan, AgxPlanError, AgxProfile, AgxProfileError, AgxProfileResolutionError,
    AgxToneMappingParameters, resolve_builtin_working_profile,
};
pub use operations::basicadj::analysis::{
    BASICADJ_HISTOGRAM_BINS, BASICADJ_HISTOGRAM_MAXIMUM, BASICADJ_HISTOGRAM_MINIMUM,
    BASICADJ_MAX_ANALYSIS_PIXELS, BasicAdjAnalysisError, BasicAdjAnalysisPlan,
    BasicAdjAnalysisRaster, BasicAdjAnalysisResult, BasicAdjAnalysisRoi, BasicAdjResolvedValues,
};
pub use operations::basicadj::{
    BasicAdjAutoControls, BasicAdjConfig, BasicAdjConfigError, BasicAdjExecutionReceipt,
    BasicAdjGpuParameters, BasicAdjParametersV1, BasicAdjParametersV2, BasicAdjPlan,
    BasicAdjPlanError, BasicAdjustmentsPlan, PreserveColors, migrate_v1_to_v2,
};
pub use operations::borders::{
    BORDERS_WGSL, BordersAspect, BordersBasis, BordersCodecError, BordersColor, BordersConfig,
    BordersExecution, BordersExecutionError, BordersFrame, BordersGeometry, BordersHistory,
    BordersOrientation, BordersParametersV1, BordersParametersV2, BordersParametersV3,
    BordersParametersV4, BordersPlan, BordersPlanError, decode_history as decode_borders_history,
    migrate_history as migrate_borders_history,
};
pub use operations::censorize::{
    CENSORIZE_COMPATIBILITY_ID, CENSORIZE_PARAMETER_BYTES, CENSORIZE_RNG_VERSION,
    CENSORIZE_SCHEMA_VERSION, CensorizeBackend, CensorizeBlend, CensorizeCodecError,
    CensorizeConfig, CensorizeExecutionError, CensorizeHistory, CensorizeMask,
    CensorizeParameterError, CensorizeParametersV1, CensorizePixel, CensorizePlan,
    CensorizeReceipt, CensorizeRng, CensorizeStages, gaussian_noise, splitmix32, xoshiro128plus,
};
pub use operations::channelmixer::{
    CHANNEL_MIXER_COMPATIBILITY_ID, CHANNEL_MIXER_GUI_MAX, CHANNEL_MIXER_GUI_MIN,
    CHANNEL_MIXER_PARAMETER_MAX, CHANNEL_MIXER_PARAMETER_MIN, CHANNEL_MIXER_RUST_ID,
    CHANNEL_MIXER_SCHEMA_VERSION, CHANNEL_MIXER_V1_PARAMETER_BYTES,
    CHANNEL_MIXER_V2_PARAMETER_BYTES, ChannelMixerAlgorithm, ChannelMixerCodecError,
    ChannelMixerConfig, ChannelMixerExecutionError, ChannelMixerHistory, ChannelMixerOperationMode,
    ChannelMixerParameterError, ChannelMixerParametersV1, ChannelMixerParametersV2,
    ChannelMixerPixel, ChannelMixerPlan, migrate_v1_to_v2 as migrate_channelmixer_v1_to_v2,
};
pub use operations::clahe::{
    CLAHE_ALIAS, CLAHE_BINS, CLAHE_COMPATIBILITY_ID, CLAHE_HISTOGRAM_ENTRIES,
    CLAHE_PARAMETER_BYTES, CLAHE_RADIUS_DEFAULT, CLAHE_RADIUS_MAX, CLAHE_RADIUS_MIN,
    CLAHE_SCHEMA_VERSION, CLAHE_SLOPE_DEFAULT, CLAHE_SLOPE_MAX, CLAHE_SLOPE_MIN, ClaheBackend,
    ClaheBlend, ClaheCodecError, ClaheConfig, ClaheExecutionError, ClaheHistory, ClaheMask,
    ClaheOutcome, ClaheParameterError, ClaheParametersV1, ClahePixel, ClahePlan, ClaheReceipt,
};
pub use operations::colorcontrast::{
    COLOR_CONTRAST_COMPATIBILITY_ID, COLOR_CONTRAST_DEFAULT_A_OFFSET,
    COLOR_CONTRAST_DEFAULT_A_STEEPNESS, COLOR_CONTRAST_DEFAULT_B_OFFSET,
    COLOR_CONTRAST_DEFAULT_B_STEEPNESS, COLOR_CONTRAST_DEFAULT_UNBOUND, COLOR_CONTRAST_RUST_ID,
    COLOR_CONTRAST_SCHEMA_VERSION, COLOR_CONTRAST_V1_PARAMETER_BYTES,
    COLOR_CONTRAST_V2_PARAMETER_BYTES, ColorContrastCodecError, ColorContrastConfig,
    ColorContrastHistory, ColorContrastParameterError, ColorContrastParametersV1,
    ColorContrastParametersV2, ColorContrastPixel, ColorContrastPlan,
    migrate_v1_to_v2 as migrate_colorcontrast_v1_to_v2,
};
pub use operations::colorcorrection::{
    COLORCORRECTION_COMPATIBILITY_ID, COLORCORRECTION_DEFAULT_HIA, COLORCORRECTION_DEFAULT_HIB,
    COLORCORRECTION_DEFAULT_LOA, COLORCORRECTION_DEFAULT_LOB, COLORCORRECTION_DEFAULT_SATURATION,
    COLORCORRECTION_GPU_TIER, COLORCORRECTION_PRESETS, COLORCORRECTION_RUST_ID,
    COLORCORRECTION_SCHEMA_VERSION, COLORCORRECTION_V1_PARAMETER_BYTES,
    COLORCORRECTION_WGPU_PASS_ID, ColorCorrectionCodecError, ColorCorrectionCoefficients,
    ColorCorrectionConfig, ColorCorrectionHistory, ColorCorrectionParameterError,
    ColorCorrectionParametersV1, ColorCorrectionPixel, ColorCorrectionPlan, ColorCorrectionPreset,
    ColorCorrectionPresetBlendColorSpace, presets as colorcorrection_presets,
};
pub use operations::colormapping::{
    COLOR_MAPPING_COMPATIBILITY_ID, COLOR_MAPPING_PARAMETER_BYTES, COLOR_MAPPING_RUST_ID,
    COLOR_MAPPING_SCHEMA_VERSION, ColorMappingCodecError, ColorMappingConfig,
    ColorMappingExecutionError, ColorMappingHistory, ColorMappingParameterError,
    ColorMappingParametersV1, ColorMappingPixel, ColorMappingPlan, ColorMappingPlanError,
    ColorMappingTiling,
};
pub use operations::colorout::{TerminalOutputDescriptor, TerminalOutputFrame};
pub use operations::colortransfer::{
    COLORTRANSFER_COMPATIBILITY_ID, COLORTRANSFER_NATIVE_PARAMETER_BYTES, COLORTRANSFER_RUST_ID,
    COLORTRANSFER_SCHEMA_VERSION, ColorTransferCodecError, ColorTransferFlag,
    ColorTransferParameters, ColorTransferPixel, ColorTransferPlan, PointsRng,
};
pub use operations::colorzones::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_DEFAULT_ENABLED,
    COLORZONES_GPU_TIER, COLORZONES_LEGACY_BANDS, COLORZONES_LUT_RESOLUTION, COLORZONES_MAX_NODES,
    COLORZONES_RUST_ID, COLORZONES_SCHEMA_VERSION, COLORZONES_V1_BANDS,
    COLORZONES_V1_PARAMETER_BYTES, COLORZONES_V2_PARAMETER_BYTES, COLORZONES_V3_PARAMETER_BYTES,
    COLORZONES_V4_PARAMETER_BYTES, COLORZONES_V5_PARAMETER_BYTES, COLORZONES_WGPU_PASS_ID,
    ColorZonesChannel, ColorZonesCodecError, ColorZonesCompileError, ColorZonesConfig,
    ColorZonesCurve, ColorZonesCurveType, ColorZonesHistory, ColorZonesMode, ColorZonesNode,
    ColorZonesParameterError, ColorZonesParametersV1, ColorZonesParametersV2,
    ColorZonesParametersV3, ColorZonesParametersV4, ColorZonesParametersV5, ColorZonesPixel,
    ColorZonesPlan, ColorZonesPoint, ColorZonesSplinesVersion,
    migrate_v1_to_v5 as migrate_colorzones_v1_to_v5,
    migrate_v2_to_v5 as migrate_colorzones_v2_to_v5,
    migrate_v3_to_v5 as migrate_colorzones_v3_to_v5,
    migrate_v4_to_v5 as migrate_colorzones_v4_to_v5,
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
pub use operations::levels::{
    LEVELS_AUTO_HISTOGRAM_BINS, LEVELS_COMPATIBILITY_ID, LEVELS_DEFAULT_LEVELS, LEVELS_LUT_ENTRIES,
    LEVELS_MANUAL_HISTOGRAM_BINS, LEVELS_PARAMETER_BYTES_V1, LEVELS_PARAMETER_BYTES_V2,
    LEVELS_RUST_ID, LEVELS_SCHEMA_VERSION, LevelsCodecError, LevelsConfig, LevelsHistogram,
    LevelsHistogramError, LevelsHistory, LevelsMode, LevelsParameterError, LevelsParametersV1,
    LevelsParametersV2, LevelsPixel, LevelsPlan, LevelsTiling,
    migrate_v1_to_v2 as migrate_levels_v1_to_v2,
};
pub use operations::liquify::{
    LIQUIFY_COMPATIBILITY_ID, LIQUIFY_PARAMETER_BYTES, LIQUIFY_SCHEMA_VERSION, LiquifyConfig,
    LiquifyExecution, LiquifyExecutionError, LiquifyGpuDispatch, LiquifyInterpolation, LiquifyNode,
    LiquifyNodeType, LiquifyParametersV1, LiquifyPathKind, LiquifyPlan, LiquifyPoint,
    LiquifyStatus, LiquifyWarpType,
};
pub use operations::mask_manager::{MaskManagerError, MaskManagerParameters};
pub use operations::overlay::{
    OVERLAY_WGSL, OverlayAlpha, OverlayAnchor, OverlayAsset, OverlayAssetError, OverlayAssetStore,
    OverlayBaseScale, OverlayChannel, OverlayCodecError, OverlayConfig, OverlayEdge,
    OverlayExecution, OverlayExecutionError, OverlayHistory, OverlayImageScale,
    OverlayInterpolation, OverlayParametersV1, OverlayPlan, OverlayProfilePolicy, OverlayReceipt,
    OverlayReference, decode_history as decode_overlay_history,
    migrate_history as migrate_overlay_history,
};
pub use operations::rasterfile::{
    RasterFileChannelMode, RasterFileExecutionError, RasterFileForm, RasterFileHistory,
    RasterFileParametersV1, RasterFilePlan, RasterFileReceipt, RasterFileTile,
    RasterFileVectorizationReceipt, RasterMaskAsset, RasterMaskAssetError, RasterMaskCache,
    RasterMaskFormat, RasterMaskLimits, decode_history, migrate_history,
};
pub use operations::rawprepare::{
    RAWPREPARE_SOURCE_MAP, RAWPREPARE_SOURCE_REGISTRATION, RawPrepareCfa as SourceRawPrepareCfa,
    RawPrepareCrop as SourceRawPrepareCrop,
    RawPrepareImageMetadata as SourceRawPrepareImageMetadata,
    RawPrepareInputKind as SourceRawPrepareInputKind,
    RawPrepareMemoryBudget as SourceRawPrepareMemoryBudget, RawPreparePlan as SourceRawPreparePlan,
    RawPrepareRoute, RawPrepareRouteRejection,
    RawPrepareSampleFormat as SourceRawPrepareSampleFormat, RawPrepareSourceOperation,
    RawPrepareSourceRegistration, RawPrepareTile as SourceRawPrepareTile,
    RawPrepareTiling as SourceRawPrepareTiling, rawprepare_route,
};
pub use operations::retouch::{
    RETOUCH_MAX_SCALES, RETOUCH_SCHEMA_VERSION, RetouchAlgorithm, RetouchBlurType, RetouchConfig,
    RetouchConfigError, RetouchExecutionError, RetouchFillMode, RetouchForm, RetouchParameters,
    RetouchPixel, RetouchPlan, RetouchReceipt, RetouchScale,
};
pub use operations::rgblevels::{
    RGBLEVELS_COMPATIBILITY_ID, RGBLEVELS_PARAMETER_BYTES, RGBLEVELS_RUST_ID,
    RGBLEVELS_SCHEMA_VERSION, RgbLevelsAutoscale, RgbLevelsCodecError, RgbLevelsConfig,
    RgbLevelsExecution, RgbLevelsExecutionError, RgbLevelsHistory, RgbLevelsParameterError,
    RgbLevelsParametersV1, RgbLevelsPixel, RgbLevelsPlan, RgbLevelsPlanError,
    RgbLevelsPreserveColors, RgbLevelsProfileError, RgbLevelsProfileEvidence,
};
pub use operations::shadhi::ShadhiBilateralRequest;
pub use operations::spots::{
    SPOTS_COMPATIBILITY_ID, SPOTS_IMPLEMENTATION_VERSION, SPOTS_MAX_ENTRIES,
    SPOTS_PARAMETER_BYTES_V1, SPOTS_PARAMETER_BYTES_V2, SPOTS_RUST_ID, SPOTS_SCHEMA_VERSION,
    SpotsCodecError, SpotsConfig, SpotsEntry, SpotsExecutionError, SpotsForm, SpotsFormKind,
    SpotsHistory, SpotsLegacySpot, SpotsMode, SpotsParametersV1, SpotsParametersV2, SpotsPlan,
    SpotsReceipt,
};
pub use operations::velvia::{
    VELVIA_COMPATIBILITY_ID, VELVIA_DEFAULT_BIAS, VELVIA_DEFAULT_STRENGTH, VELVIA_RUST_ID,
    VELVIA_SCHEMA_VERSION, VELVIA_V1_PARAMETER_BYTES, VELVIA_V2_PARAMETER_BYTES, VelviaCodecError,
    VelviaConfig, VelviaHistory, VelviaParameterError, VelviaParametersV1, VelviaParametersV2,
    VelviaPixel, VelviaPlan, migrate_v1_to_v2 as migrate_velvia_v1_to_v2,
};
pub use operations::vibrance::{
    VIBRANCE_COMPATIBILITY_ID, VIBRANCE_DEFAULT_AMOUNT, VIBRANCE_GPU_TIER, VIBRANCE_RUST_ID,
    VIBRANCE_SCHEMA_VERSION, VIBRANCE_V2_PARAMETER_BYTES, VIBRANCE_WGPU_PASS_ID,
    VibranceCodecError, VibranceConfig, VibranceHistory, VibranceParameterError,
    VibranceParametersV2, VibrancePixel, VibrancePlan,
};
pub use operations::watermark::{
    ExpandedWatermark, WATERMARK_ALLOWED_FONT_SET_HASH, WATERMARK_COMPATIBILITY_ID,
    WATERMARK_IMPLEMENTATION_VERSION, WATERMARK_PARAMETER_VERSION, WATERMARK_RUST_ID,
    WATERMARK_SCHEMA_VERSION, WatermarkAnchor, WatermarkCodecError, WatermarkContext,
    WatermarkContextError, WatermarkExecutionError, WatermarkHistory, WatermarkParametersV1,
    WatermarkParametersV7, WatermarkPlan, WatermarkReceipt, WatermarkScaleMode,
    decode_history as decode_watermark_history, migrate_history as migrate_watermark_history,
    watermark_descriptor,
};
pub use output::{
    ChannelCounts, EncodedSrgb, EncodedSrgbImage, EncodedSrgbOutput, GamutClipReport,
    convert_working_to_linear_srgb, encode_linear_srgb, encode_working_to_srgb,
};
pub use pipeline::{CompiledPipeline, PipelineCompileError, PipelineStep, PipelineStepIndex};
pub use raw_pipeline::{
    RawPipelineError, RawPipelineExecution, RawPipelinePlan, RawPipelineReceipt,
    RawTemperatureSelection, pre_demosaic_temperature,
};
pub use rawprepare::{NormalizedRaw, RawPrepareConfig, RawPrepareError, RawPreparePlan};
pub use registry::closure::{
    OperationClassification, REGISTRY_CLOSURE_SCHEMA, RegistryClosure, RegistryClosureEntry,
    RegistryClosureError,
};
pub use registry::{
    BUILTIN_OPERATIONS, CpuExecutionRoute, CpuFactory, DefinitionAvailability,
    DeviceCapabilitySnapshot, ExecutionBackend, FactoryError, GpuBinding, ImplementationIdentity,
    MigrationBinding, OperationCapability, OperationDefinition, OperationDefinitionFactory,
    OperationMaterializationError, OperationUiAvailability, PreparedCpuOperation,
    RegistryBuildError, RegistryLookupError, RegistrySnapshot, RegistryValidationError,
    builtin_registry,
};
pub use scalar::{FiniteF32, FiniteF32Error, ScalarNarrowingError};
pub use window::{
    EvaluatedRowWindow, GraphWindowEvaluationError, RasterRowWindow, RasterRowWindowError,
    evaluate_graph, evaluate_graph_node_with_context_and_cancellation,
    evaluate_graph_output_with_basicadj_plans, evaluate_graph_output_with_basicadj_plans_and_masks,
    evaluate_graph_output_with_basicadj_plans_and_masks_with_cancellation, evaluate_graph_window,
    evaluate_graph_with_basicadj_plans, evaluate_graph_with_basicadj_plans_and_masks,
    evaluate_graph_with_basicadj_plans_and_masks_with_cancellation,
};

pub use operation_stack::{
    CommandReceipt, InsertPosition, MigrationFinding, MigrationOutcome, MoveTarget,
    OpaqueOperation, OperationInstance, OperationStackError, OperationStackResult,
    OperationStackSnapshot, OperationStackTemplate, StackCommand, StackStage, StackStageFence,
};
