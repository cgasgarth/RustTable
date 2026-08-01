//! Source-to-Rust responsibility map for the integrated Sharpen CPU path.
//!
//! The source baseline is `src/iop/sharpen.c`, with the retained GPU path in
//! `data/kernels/sharpen.cl`; this map records the integrated CPU, descriptor,
//! typed-history, and Lab-stage responsibilities without claiming the deferred
//! shared-blend, GTK, runtime-preset, or GPU seams.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharpenSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: SharpenPortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenPortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const SHARPEN_SOURCE_MAP: &[SharpenSourceMapEntry] = &[
    SharpenSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_sharpen_params_t",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "SharpenParametersV1",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "commit_params",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "SharpenConfig::commit",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "init_gaussian_kernel",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "gaussian_kernel",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "process / USM convolution and border copies",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "SharpenPlan::execute_with_cancel",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "tiling_callback",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "tiling::SharpenTilingPlan",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "dt_iop_alloc_image_buffers / dt_iop_copy_image_roi",
        native_file: "src/common/imagebuf.c; src/iop/sharpen.c",
        rust_symbol: "checked Vec allocations / clone_pixels",
        status: SharpenPortStatus::ExistingDependency,
    },
    SharpenSourceMapEntry {
        native_symbol: "process_cl / sharpen_hblur / sharpen_vblur / sharpen_mix",
        native_file: "src/iop/sharpen.c; data/kernels/sharpen.cl",
        rust_symbol: "no Rust GPU capability",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "flags / default_colorspace",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "sharpen_descriptor / registry::operations::sharpen_definition",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "init_presets / built-in raw preset contract",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE / SHARPEN_V1_BUILTIN_PRESET_APPLICABILITY",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "init_presets / runtime preset materialization",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "shared preset and UI application hub",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "description / default_group UI projection",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "no Rust GTK/editor grouping capability",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "gui_init",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "no Rust GTK capability",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "outer mask blend / blend-if / alpha publication",
        native_file: "src/develop/blend.c; src/develop/blends/*",
        rust_symbol: "shared pixelpipe blend boundary",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "history dispatch / Sharpen v1 core parameter decoding",
        native_file: "src/iop/sharpen.c; src/develop/history.c",
        rust_symbol: "rusttable_compat::sharpen::decode_sharpen_history_step / rusttable_import::darktable::history",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "history dispatch / blend-mask and multi-instance materialization",
        native_file: "src/develop/history.c; src/develop/blend.c",
        rust_symbol: "preserved SharpenPendingBlend import boundary",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "name / descriptor identity",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "sharpen_descriptor / registry::operations::sharpen_definition",
        status: SharpenPortStatus::Ported,
    },
    SharpenSourceMapEntry {
        native_symbol: "init_pipe / cleanup_pipe",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "immutable SharpenPlan ownership boundary",
        status: SharpenPortStatus::ExistingDependency,
    },
    SharpenSourceMapEntry {
        native_symbol: "init_global / cleanup_global",
        native_file: "src/iop/sharpen.c",
        rust_symbol: "no Rust GPU resource lifecycle",
        status: SharpenPortStatus::ExplicitlyDeferred,
    },
    SharpenSourceMapEntry {
        native_symbol: "dt_iop_have_required_input_format / Lab stage boundary",
        native_file: "src/iop/sharpen.c; src/develop/imageop.c",
        rust_symbol: "rusttable_pixelpipe::cpu::{lab_input_channels, execute_sharpen_lab}",
        status: SharpenPortStatus::Ported,
    },
];
