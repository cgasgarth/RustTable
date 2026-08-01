//! Source-to-Rust responsibility map for the bounded RGB Levels CPU leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbLevelsSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: RgbLevelsPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbLevelsPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const RGBLEVELS_SOURCE_MAP: &[RgbLevelsSourceMapEntry] = &[
    RgbLevelsSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION(1) / dt_iop_rgblevels_params_t",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsParametersV1 / RgbLevelsAbiLayout",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "init / generated enum defaults / levels[3][3] initialization",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsParametersV1::defaults",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "default_enabled initialization / add_iop DEFAULT_VISIBLE",
        native_file: "src/develop/imageop.c; src/iop/CMakeLists.txt",
        rust_symbol: "RGBLEVELS_DEFAULT_ENABLED / RGBLEVELS_DEFAULT_VISIBLE",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "history decode / no legacy_params migration",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsHistory::decode / current",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "commit_params / linked-channel level expansion",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsPlan::new / effective_levels",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "_compute_lut",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsPlan::new / LUT construction",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "process independent RGB branch / rgblevels_1c",
        native_file: "src/iop/rgblevels.c; data/kernels/rgblevels.cl",
        rust_symbol: "RgbLevelsPlan::map_channel / evaluate_pixel",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "dt_rgb_norm / dt_camera_rgb_luminance",
        native_file: "src/common/rgb_norms.h; src/common/colorspaces_inline_conversions.h",
        rust_symbol: "rgb_norm / RgbLevelsProfileEvidence::luminance",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "process linked RGB branch / ratio application",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsPlan::evaluate_pixel",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "semantic RGB writes / for_each_channel / copy_pixel_nontemporal",
        native_file: "src/iop/rgblevels.c; src/common/dttypes.h; src/develop/imageop.h; commit b9d34a318940",
        rust_symbol: "RgbLevelsPlan::evaluate_pixel",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "required four-channel input / copy-through trouble boundary",
        native_file: "src/iop/rgblevels.c; src/develop/imageop.c",
        rust_symbol: "RgbLevelsPlan::execute_required_format_with_cancel",
        status: RgbLevelsPortStatus::Ported,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "work-profile acquisition and ICC ownership",
        native_file: "src/iop/rgblevels.c; src/common/iop_profile.h",
        rust_symbol: "RgbLevelsProfileEvidence",
        status: RgbLevelsPortStatus::ExistingDependency,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "_auto_levels / _get_selected_area / histogram GUI state",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "shared preview, histogram, and GTK owners",
        status: RgbLevelsPortStatus::ExplicitlyDeferred,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "process_cl / rgblevels kernel",
        native_file: "src/iop/rgblevels.c; data/kernels/rgblevels.cl",
        rust_symbol: "RgbLevelsCapabilities::require_gpu",
        status: RgbLevelsPortStatus::ExplicitlyDeferred,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "IOP_FLAGS_SUPPORTS_BLENDING / outer masks",
        native_file: "src/iop/rgblevels.c; src/develop/blend.c",
        rust_symbol: "RgbLevelsCapabilities::require_masks / production routing",
        status: RgbLevelsPortStatus::ExplicitlyDeferred,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "gui_init / gui_changed / gui_update / presets",
        native_file: "src/iop/rgblevels.c",
        rust_symbol: "RgbLevelsCapabilities::require_gtk",
        status: RgbLevelsPortStatus::ExplicitlyDeferred,
    },
    RgbLevelsSourceMapEntry {
        native_symbol: "registry / typed history import / CPU pixelpipe dispatch",
        native_file: "processing integration hubs",
        rust_symbol: "rgblevels_definition / ProcessingOperationKind::RgbLevels",
        status: RgbLevelsPortStatus::Ported,
    },
];
