//! Source-to-Rust responsibility map for the bounded Color Mapping CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorMappingSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: ColorMappingPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const COLOR_MAPPING_SOURCE_MAP: &[ColorMappingSourceMapEntry] = &[
    ColorMappingSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_colormapping_params_t",
        native_file: "src/iop/colormapping.c",
        rust_symbol: "ColorMappingParametersV1",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "capture_histogram",
        native_file: "src/iop/colormapping.c",
        rust_symbol: "capture_histogram",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "invert_histogram",
        native_file: "src/iop/colormapping.c",
        rust_symbol: "invert_histogram",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "get_cluster_mapping / get_clusters",
        native_file: "src/iop/colormapping.c; data/kernels/extended.cl",
        rust_symbol: "get_cluster_mapping / get_clusters",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "kmeans / dt_points_get_for state transition",
        native_file: "src/iop/colormapping.c; src/common/points.h",
        rust_symbol: "kmeans_with_cancel / caller-injected PointsState",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "process / commit_params / alpha pass-through",
        native_file: "src/iop/colormapping.c",
        rust_symbol: "ColorMappingPlan::execute_with_cancel",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "dt_bilateral_grid_size / dt_bilateral_init / splat / blur / slice",
        native_file: "src/common/bilateral.c; src/common/bilateral.h",
        rust_symbol: "rusttable_processing::common::bilateral::BilateralGrid",
        status: ColorMappingPortStatus::ExistingDependency,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "tiling_callback",
        native_file: "src/iop/colormapping.c; src/common/bilateral.c",
        rust_symbol: "ColorMappingPlan::tiling(cpu_threads)",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "dt_points_init / dt_points_get / dt_points_cleanup process-global ownership",
        native_file: "src/common/points.h; process darktable.points owner",
        rust_symbol: "deferred shared per-worker points owner and production injection",
        status: ColorMappingPortStatus::ExplicitlyDeferred,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "gui_init / process_clusters / cluster_preview_draw",
        native_file: "src/iop/colormapping.c; src/common/colorspaces.h",
        rust_symbol: "ColorMappingCapabilities::require_gtk",
        status: ColorMappingPortStatus::ExplicitlyDeferred,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "dt_colorspaces_get_profile / cmsCreateTransform / cmsDoTransform",
        native_file: "src/iop/colormapping.c; src/common/colorspaces.c; src/common/colorspaces.h",
        rust_symbol: "deferred GTK profile-preview colorspace boundary",
        status: ColorMappingPortStatus::ExplicitlyDeferred,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "process_cl / colormapping_histogram / colormapping_mapping",
        native_file: "src/iop/colormapping.c; data/kernels/extended.cl",
        rust_symbol: "ColorMappingCapabilities::require_gpu",
        status: ColorMappingPortStatus::ExplicitlyDeferred,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "flags / default_colorspace / registry and pixelpipe routing",
        native_file: "src/iop/colormapping.c and processing integration hubs",
        rust_symbol: "colormapping_descriptor / compile_colormapping / execute_lab_point_chain",
        status: ColorMappingPortStatus::Ported,
    },
    ColorMappingSourceMapEntry {
        native_symbol: "outer blending / masks",
        native_file: "src/iop/colormapping.c; src/develop/blend.c",
        rust_symbol: "ColorMappingCapabilities::require_outer_blending",
        status: ColorMappingPortStatus::ExplicitlyDeferred,
    },
];
