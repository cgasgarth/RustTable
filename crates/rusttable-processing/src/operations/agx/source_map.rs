//! Source-to-Rust responsibility map for the bounded AgX CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgxSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: AgxPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const AGX_SOURCE_MAP: &[AgxSourceMapEntry] = &[
    AgxSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_agx_params_t",
        native_file: "src/iop/agx.c",
        rust_symbol: "AgxParametersV7 / to_bytes / from_bytes",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "legacy_params",
        native_file: "src/iop/agx.c",
        rust_symbol: "AgxHistory::decode / current",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_set_default_curve_and_look_params / _set_*_primaries",
        native_file: "src/iop/agx.c",
        rust_symbol: "AgxParametersV7::*_defaults",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_scale / _sigmoid / _scaled_sigmoid / _apply_curve",
        native_file: "src/iop/agx.c",
        rust_symbol: "scale / sigmoid / scaled_sigmoid / apply_curve",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_calculate_tone_mapping_params / _adjust_pivot",
        native_file: "src/iop/agx.c",
        rust_symbol: "calculate_tone_mapping_parameters",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_agx_look / _apply_slope_lift / _lerp_hue",
        native_file: "src/iop/agx.c; src/common/math.h",
        rust_symbol: "agx_look / mul_add / lerp_hue",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "dt_RGB_2_HSV / dt_HSV_2_RGB",
        native_file: "src/common/colorspaces_inline_conversions.h",
        rust_symbol: "rgb_to_hsv / hsv_to_rgb",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_compress_into_gamut",
        native_file: "src/iop/agx.c",
        rust_symbol: "compress_into_gamut",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_create_matrices / _get_primaries_params",
        native_file: "src/iop/agx.c",
        rust_symbol: "create_matrices / get_primaries_params",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "dt_rotate_and_scale_primary",
        native_file: "src/common/custom_primaries.c",
        rust_symbol: "rotate_and_scale_primary / find_distance_to_edge",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "dt_make_transposed_matrices_from_primaries_and_whitepoint / mat3SSEinv / dt_colormatrix_mul",
        native_file: "src/common/colorspaces.c; src/common/matrices.c; src/common/dttypes.h",
        rust_symbol: "matrix_from_primaries / invert_matrix / multiply_matrices",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "process / input sanitisation / alpha copy",
        native_file: "src/iop/agx.c",
        rust_symbol: "AgxPlan::execute_with_cancel / sanitise_pixel",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "_agx_get_base_profile",
        native_file: "src/iop/agx.c; src/common/iop_profile.h",
        rust_symbol: "AgxPlan::new_with_profiles",
        status: AgxPortStatus::ExistingDependency,
    },
    AgxSourceMapEntry {
        native_symbol: "kernel_agx",
        native_file: "data/kernels/agx.cl",
        rust_symbol: "AgxCapabilities::require_gpu",
        status: AgxPortStatus::ExplicitlyDeferred,
    },
    AgxSourceMapEntry {
        native_symbol: "gui_init / gui_changed / gui_update / init_presets",
        native_file: "src/iop/agx.c",
        rust_symbol: "AgxCapabilities::require_gtk",
        status: AgxPortStatus::ExplicitlyDeferred,
    },
    AgxSourceMapEntry {
        native_symbol: "flags / default_colorspace / process integration",
        native_file: "src/iop/agx.c and processing integration hubs",
        rust_symbol: "agx_descriptor / agx_definition / ProcessingOperationKind::Agx",
        status: AgxPortStatus::Ported,
    },
    AgxSourceMapEntry {
        native_symbol: "configured profile lookup / masks / outer blending",
        native_file: "src/iop/agx.c and processing integration hubs",
        rust_symbol: "AgxCapabilities deferred capability fields",
        status: AgxPortStatus::ExplicitlyDeferred,
    },
];
