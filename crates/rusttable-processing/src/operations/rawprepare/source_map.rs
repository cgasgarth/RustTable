//! Source-to-Rust responsibility map for the bounded rawprepare leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawPrepareSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: RawPreparePortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPreparePortStatus {
    Ported,
    ExplicitlyDeferred,
    ExistingDependency,
}

pub const RAWPREPARE_SOURCE_MAP: &[RawPrepareSourceMapEntry] = &[
    RawPrepareSourceMapEntry {
        native_symbol: "DT_MODULE_INTROSPECTION / dt_iop_rawprepare_params_t",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "codec::{RawPrepareParametersV1, RawPrepareParametersV2}",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "legacy_params",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPrepareHistory::migrate_to_v2 / migrate_v1_to_v2",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "_image_set_rawcrops / _image_is_normalized",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "cropped_dimensions / classify_input",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "commit_params",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPreparePlan::new_with_budget",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "modify_roi_in / modify_roi_out / _compute_proper_crop",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPrepareTile::new / RawPreparePlan::full_frame_tile",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "_BL / dt_rawspeed_crop_dcraw_filters",
        native_file: "src/iop/rawprepare.c; src/imageio/imageio_rawspeed.cc",
        rust_symbol: "black_level_index / RawPrepareCfa::after_crop",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "_adjust_xtrans_filters",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPrepareCfa::xtrans_table_after_crop",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "process raw mosaic branches",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPreparePlan::execute_u16 / execute_f32",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "process pre-downsampled buffer branch",
        native_file: "src/iop/rawprepare.c; data/kernels/basic.cl",
        rust_symbol: "RawPreparePlan::execute_four_channel",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "_check_gain_maps / embedded GainMap interpolation",
        native_file: "src/iop/rawprepare.c; src/common/dng_opcode.h",
        rust_symbol: "RawPrepareGainMapSet / gain_map_gain",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "flags / process_tiling",
        native_file: "src/iop/rawprepare.c; src/develop/tiling.h",
        rust_symbol: "RawPrepareTiling / RawPrepareTile",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "OpenMP/OpenCL scheduling and publication",
        native_file: "src/iop/rawprepare.c; data/kernels/basic.cl",
        rust_symbol: "execute_* cancellation-safe Vec publication",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "dt_dev_write_scharr_mask",
        native_file: "src/iop/rawprepare.c; src/develop/pixelpipe_hb.c",
        rust_symbol: "pixelpipe detail-mask seam",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "dt_opencl_* / rawprepare_* kernels",
        native_file: "src/iop/rawprepare.c; data/kernels/basic.cl",
        rust_symbol: "no Rust GPU capability",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "imageio_rawspeed metadata and sample decoding",
        native_file: "src/imageio/imageio_rawspeed.cc; src/common/image.h",
        rust_symbol: "RawPrepareImageMetadata input boundary",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "gui_init / gui_update / init_presets",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "no Rust GTK capability",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "shared operation registry / history / pixelpipe routing",
        native_file: "src/iop/rawprepare.c; processing shared hubs",
        rust_symbol: "no shared-hub changes in this leaf",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
];
