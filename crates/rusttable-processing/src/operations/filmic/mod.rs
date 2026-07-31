//! Isolated legacy Darktable `filmic` processing leaf.
//!
//! Source lineage: `src/iop/filmic.c`,
//! `src/common/colorspaces_inline_conversions.h`, `src/common/dttypes.h`,
//! `src/common/math.h`, `src/develop/imageop.h`, `src/gui/draw.h`,
//! `src/common/curve_tools.c`, and `src/common/curve_tools.h`.
//!
//! The leaf is intentionally not included from the shared operation module:
//! registration, imported-history materialization, pixelpipe routing, GPU
//! binding, and GUI remain unavailable until a later integration milestone.

#![forbid(unsafe_code)]
#![allow(
    unused_imports,
    dead_code,
    reason = "the operation-local exports are consumed by the deferred integration seam"
)]

pub mod codec;
mod curve;
pub mod descriptor;
mod execution;

pub use codec::{
    CodecError as FilmicCodecError, History as FilmicHistory, ParametersV1 as FilmicParametersV1,
    ParametersV2 as FilmicParametersV2, ParametersV3 as FilmicParametersV3,
    SCHEMA_VERSION as FILMIC_SCHEMA_VERSION, V1_PARAMETER_BYTES as FILMIC_V1_PARAMETER_BYTES,
    V2_PARAMETER_BYTES as FILMIC_V2_PARAMETER_BYTES,
    V3_PARAMETER_BYTES as FILMIC_V3_PARAMETER_BYTES, migrate_v1_to_v3 as migrate_filmic_v1_to_v3,
    migrate_v2_to_v3 as migrate_filmic_v2_to_v3,
};
pub use curve::{
    CurveBuildError as FilmicCurveBuildError, CurveLuts as FilmicCurveLuts,
    LUT_SIZE as FILMIC_LUT_SIZE, NodeLoss as FilmicNodeLoss, Nodes as FilmicNodes,
    build_luts as build_filmic_luts, derive_filmic_nodes,
};
pub use descriptor::{FilmicDescriptor, descriptor as filmic_descriptor};
pub use execution::{
    EPS as FILMIC_EPS, FilmicPixel, FilmicPlan, FilmicPlanError, fastlog2, lab_d50_to_xyz,
    lut_index, prophoto_rgb_to_lab, vector_exp2, vector_log2, xyz_d50_to_prophoto_rgb,
};

/// Compatibility identifier used by native history records.
pub const FILMIC_COMPATIBILITY_ID: &str = "filmic";
/// Rust identity reserved for the later shared registry integration.
pub const FILMIC_RUST_ID: &str = "rusttable.filmic";

/// Validated finite v3 configuration.  Native slider ranges are intentionally
/// not applied: persisted finite values remain unchanged at this boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmicConfig {
    parameters: FilmicParametersV3,
}

impl FilmicConfig {
    pub fn new(parameters: FilmicParametersV3) -> Result<Self, FilmicCodecError> {
        parameters
            .validate_finite()
            .map(|parameters| Self { parameters })
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            parameters: FilmicParametersV3::defaults(),
        }
    }

    #[must_use]
    pub const fn parameters(self) -> FilmicParametersV3 {
        self.parameters
    }

    pub fn plan(self) -> Result<FilmicPlan, FilmicPlanError> {
        FilmicPlan::from_parameters(self.parameters)
    }
}

/// Operation-local alias for callers that distinguish parameter validation from
/// history byte decoding.
pub type FilmicParameterError = FilmicCodecError;
