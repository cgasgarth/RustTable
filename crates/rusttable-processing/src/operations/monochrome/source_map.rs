//! Source-to-Rust responsibility map for the bounded Monochrome CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonochromeSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: MonochromePortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonochromePortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const MONOCHROME_SOURCE_MAP: &[MonochromeSourceMapEntry] = &[
    MonochromeSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_monochrome_params_t",
        native_file: "src/iop/monochrome.c",
        rust_symbol: "MonochromeParametersV1 / MonochromeParametersV2",
        status: MonochromePortStatus::Ported,
    },
    MonochromeSourceMapEntry {
        native_symbol: "legacy_params",
        native_file: "src/iop/monochrome.c",
        rust_symbol: "MonochromeHistory::decode / current",
        status: MonochromePortStatus::Ported,
    },
    MonochromeSourceMapEntry {
        native_symbol: "_color_filter / _envelope / dt_fast_expf",
        native_file: "src/iop/monochrome.c; src/common/math.h",
        rust_symbol: "color_filter / envelope / fast_expf",
        status: MonochromePortStatus::Ported,
    },
    MonochromeSourceMapEntry {
        native_symbol: "process / commit_params",
        native_file: "src/iop/monochrome.c",
        rust_symbol: "MonochromeConfig / MonochromePlan::execute_with_cancel",
        status: MonochromePortStatus::Ported,
    },
    MonochromeSourceMapEntry {
        native_symbol: "dt_bilateral_init / splat / blur / slice",
        native_file: "src/common/bilateral.h; src/common/bilateral.c",
        rust_symbol: "BilateralGrid operation-local dependency",
        status: MonochromePortStatus::ExistingDependency,
    },
    MonochromeSourceMapEntry {
        native_symbol: "tiling_callback",
        native_file: "src/iop/monochrome.c",
        rust_symbol: "MonochromePlan::tiling",
        status: MonochromePortStatus::Ported,
    },
    MonochromeSourceMapEntry {
        native_symbol: "monochrome_filter / monochrome",
        native_file: "data/kernels/basic.cl",
        rust_symbol: "MonochromeCapabilities::require_gpu",
        status: MonochromePortStatus::ExplicitlyDeferred,
    },
    MonochromeSourceMapEntry {
        native_symbol: "requested standalone monochrome kernel file",
        native_file: "data/kernels/monochrome.cl",
        rust_symbol: "no Rust GPU claim",
        status: MonochromePortStatus::ExplicitlyDeferred,
    },
    MonochromeSourceMapEntry {
        native_symbol: "gui_init / _monochrome_draw / color_picker_apply",
        native_file: "src/iop/monochrome.c; src/common/colorspaces.h",
        rust_symbol: "MonochromeCapabilities::require_gtk",
        status: MonochromePortStatus::ExplicitlyDeferred,
    },
    MonochromeSourceMapEntry {
        native_symbol: "flags / application and history routing / outer blending",
        native_file: "src/iop/monochrome.c and integration hubs",
        rust_symbol: "MonochromeCapabilities::require_production_routing",
        status: MonochromePortStatus::ExplicitlyDeferred,
    },
];
