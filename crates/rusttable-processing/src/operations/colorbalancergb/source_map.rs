//! Source-to-Rust responsibility map for the bounded Color Balance RGB leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorBalanceRgbSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: ColorBalanceRgbPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const COLORBALANCERGB_SOURCE_MAP: &[ColorBalanceRgbSourceMapEntry] = &[
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_colorbalancergb_params_t",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "codec::{ColorBalanceRgbParametersV1..V5}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "legacy_params",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "ColorBalanceRgbHistory::current / migrate_v*_to_v5",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "commit_params",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "ColorBalanceRgbCoefficients::commit",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "opacity_masks",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "opacity_masks",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "process",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "ColorBalanceRgbPlan::execute_with_cancel",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "XYZ_D50_to_D65_CAT16 / XYZ_D65_to_D50_CAT16",
        native_file: "src/common/chromatic_adaptation.h",
        rust_symbol: "math::{input_matrix, output_matrix}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "LMS/Yrg/Ych/gradingRGB conversions and gamut_check_Yrg",
        native_file: "src/common/colorspaces_inline_conversions.h",
        rust_symbol: "math::{lms_to_yrg, yrg_to_ych, ...}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "dt_XYZ_2_JzAzBz / dt_JzAzBz_2_XYZ",
        native_file: "src/common/colorspaces_inline_conversions.h",
        rust_symbol: "math::{xyz_to_jzazbz, jzazbz_to_xyz}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "dt_UCS_* and lookup_gamut / soft_clip",
        native_file: "src/common/colorspaces_inline_conversions.h; src/common/darktable_ucs_22_helpers.h",
        rust_symbol: "math::{ucs_*, lookup_gamut, soft_clip}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "LUT_ELEM gamut preparation",
        native_file: "src/iop/colorbalancergb.c; src/common/darktable_ucs_22_helpers.h",
        rust_symbol: "math::{build_jz_gamut_lut, build_ucs_gamut_lut}",
        status: ColorBalanceRgbPortStatus::Ported,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "flags / default_colorspace / process routing",
        native_file: "src/iop/colorbalancergb.c; processing registry and pixelpipe",
        rust_symbol: "colorbalancergb_descriptor; shared hubs",
        status: ColorBalanceRgbPortStatus::ExplicitlyDeferred,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "outer mask blend / blend-if / alpha publication",
        native_file: "src/develop/blend.c; src/develop/blends/*",
        rust_symbol: "shared pixelpipe blend boundary",
        status: ColorBalanceRgbPortStatus::ExplicitlyDeferred,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "process_cl / colorbalancergb OpenCL kernel",
        native_file: "src/iop/colorbalancergb.c; data/kernels/extended.cl",
        rust_symbol: "no Rust GPU capability",
        status: ColorBalanceRgbPortStatus::ExplicitlyDeferred,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "gui_init / gui_update / init_presets / picker",
        native_file: "src/iop/colorbalancergb.c",
        rust_symbol: "no Rust GTK capability",
        status: ColorBalanceRgbPortStatus::ExplicitlyDeferred,
    },
    ColorBalanceRgbSourceMapEntry {
        native_symbol: "history dispatch / durable operation materialization",
        native_file: "src/develop/history.c",
        rust_symbol: "import and persistence seams",
        status: ColorBalanceRgbPortStatus::ExplicitlyDeferred,
    },
];
