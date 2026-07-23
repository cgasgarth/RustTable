//! Backend-neutral image-analysis contracts and bounded CPU reference generators.
//!
//! The analysis port consumes borrowed pixels and publishes owned integer buffers. It never
//! mutates or retains pipeline raster storage. Rendering, interaction, masks, and GPU execution
//! remain outside this module.

mod contracts;
mod generator;
mod raster;
mod result;

pub use contracts::{
    ANALYSIS_NUMERICAL_CONTRACT, ANALYSIS_SCHEMA_VERSION, AnalysisAlphaPolicy, AnalysisBoundary,
    AnalysisCacheIdentity, AnalysisChannel, AnalysisGraticule, AnalysisIntensity, AnalysisKind,
    AnalysisNormalization, AnalysisOutputDimensions, AnalysisRequest, AnalysisRequestError,
    AnalysisRequestIdentity, AnalysisSampling, AnalysisSamplingError, AnalysisSourceColorSpace,
    AnalysisTile, MAX_ANALYSIS_BYTES, MAX_ANALYSIS_DIMENSION, MAX_ANALYSIS_TILES,
    WaveformOrientation,
};
pub use generator::{AnalysisAggregator, AnalysisError, AnalysisPartial, CpuAnalysisGenerator};
pub use raster::{AnalysisMask, AnalysisRaster, AnalysisRasterError};
pub use result::{AnalysisPlane, AnalysisProvenance, AnalysisResult, AnalysisStatistics};
