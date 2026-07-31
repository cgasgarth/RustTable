//! Source-to-Rust responsibility map for the bounded Sigmoid CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigmoidSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: SigmoidPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigmoidPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const SIGMOID_SOURCE_MAP: &[SigmoidSourceMapEntry] = &[
    SigmoidSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_sigmoid_params_t",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidParametersV1 / V2 / V3",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "legacy_params",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidHistory::decode / current",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "_generalized_loglogistic_sigmoid / commit_params / dt_isnan",
        native_file: "src/iop/sigmoid.c; src/common/math.h",
        rust_symbol: "generalized_loglogistic_sigmoid / commit_parameters",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "_desaturate_negative_values / _pixel_channel_order",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "desaturate_negative_values / pixel_channel_order",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "_preserve_hue_and_energy",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "preserve_hue_and_energy",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "_calculate_adjusted_primaries / dt_rotate_and_scale_primary",
        native_file: "src/iop/sigmoid.c; src/common/custom_primaries.c",
        rust_symbol: "calculate_adjusted_primaries / rotate_and_scale_primary",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "dt_make_transposed_matrices_from_primaries_and_whitepoint / mat3SSEinv",
        native_file: "src/common/colorspaces.c; src/common/matrices.c",
        rust_symbol: "matrix_from_primaries / invert_matrix",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "process_loglogistic_rgb_ratio",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidPlan::process_rgb_ratio",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "process_loglogistic_per_channel",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidPlan::process_per_channel",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "process / alpha copy / finite publication boundary",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidPlan::execute_with_cancel",
        status: SigmoidPortStatus::Ported,
    },
    SigmoidSourceMapEntry {
        native_symbol: "sigmoid_loglogistic_per_channel / sigmoid_loglogistic_rgb_ratio",
        native_file: "data/kernels/sigmoid.cl",
        rust_symbol: "SigmoidCapabilities::require_gpu",
        status: SigmoidPortStatus::ExplicitlyDeferred,
    },
    SigmoidSourceMapEntry {
        native_symbol: "gui_init / gui_changed / gui_update / init_presets",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidCapabilities::require_gtk",
        status: SigmoidPortStatus::ExplicitlyDeferred,
    },
    SigmoidSourceMapEntry {
        native_symbol: "flags / default_colorspace / process integration / outer blending",
        native_file: "src/iop/sigmoid.c and processing integration hubs",
        rust_symbol: "SigmoidCapabilities::require_production_routing",
        status: SigmoidPortStatus::ExplicitlyDeferred,
    },
    SigmoidSourceMapEntry {
        native_symbol: "Lab colorspace support (not selected by default_colorspace)",
        native_file: "src/iop/sigmoid.c",
        rust_symbol: "SigmoidCapabilities::require_production_routing",
        status: SigmoidPortStatus::ExplicitlyDeferred,
    },
];
