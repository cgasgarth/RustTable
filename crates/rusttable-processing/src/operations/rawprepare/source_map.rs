//! Source-to-Rust responsibility map and typed source-stage routing.

#![forbid(unsafe_code)]

use super::{RawPrepareSourceOperation, execution::RawPrepareImageMetadata};

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
        rust_symbol: "cropped_dimensions / rawprepare_route",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "commit_params / output_format",
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
        native_file: "src/iop/rawprepare.c; src/imageio/imageio_rawspeed.h",
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
        native_symbol: "process raw mosaic U16 branch",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPreparePlan::execute_u16",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "process pre-downsampled/SRAW/float branches",
        native_file: "src/iop/rawprepare.c",
        rust_symbol: "RawPrepareRouteRejection",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "_check_gain_maps / dt_dng_gain_map_t",
        native_file: "src/iop/rawprepare.c; src/common/dng_opcode.h",
        rust_symbol: "typed source metadata boundary",
        status: RawPreparePortStatus::ExplicitlyDeferred,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "IOP_FLAGS_ALLOW_TILING / modify_roi_*",
        native_file: "src/iop/rawprepare.c; src/develop/tiling.h",
        rust_symbol: "RawPrepareTiling / RawPrepareTile",
        status: RawPreparePortStatus::Ported,
    },
    RawPrepareSourceMapEntry {
        native_symbol: "OpenMP/OpenCL/detail-mask publication",
        native_file: "src/iop/rawprepare.c; data/kernels/basic.cl",
        rust_symbol: "no GPU or detail-mask capability",
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
];

/// The only registered source-stage operation in this lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawPrepareSourceRegistration {
    pub operation: RawPrepareSourceOperation,
    pub stage: &'static str,
    pub input: &'static str,
    pub output: &'static str,
    pub gpu: bool,
    pub generic_rgb_registry: bool,
}

pub const RAWPREPARE_SOURCE_REGISTRATION: RawPrepareSourceRegistration =
    RawPrepareSourceRegistration {
        operation: RawPrepareSourceOperation::RawPrepare,
        stage: "raw-sensor-linear",
        input: "raw-u16-mosaic",
        output: "raw-f32-mosaic",
        gpu: false,
        generic_rgb_registry: false,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPrepareRoute {
    RawPrepare(RawPrepareSourceOperation),
    Rejected(RawPrepareRouteRejection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawPrepareRouteRejection {
    NonRaw,
    Sraw,
    FloatRaw,
    UnsupportedLayout,
}

/// Maps decoder metadata to the typed source operation without constructing a
/// generic linear-RGB operation. Unsupported native branches are explicit.
#[must_use]
pub fn rawprepare_route(metadata: &RawPrepareImageMetadata) -> RawPrepareRoute {
    if metadata.flags() & super::execution::DT_IMAGE_S_RAW != 0 {
        return RawPrepareRoute::Rejected(RawPrepareRouteRejection::Sraw);
    }
    if metadata.flags() & super::execution::DT_IMAGE_RAW == 0 {
        return RawPrepareRoute::Rejected(RawPrepareRouteRejection::NonRaw);
    }
    if metadata.sample_format() != super::execution::RawPrepareSampleFormat::U16 {
        return RawPrepareRoute::Rejected(RawPrepareRouteRejection::FloatRaw);
    }
    if metadata.channels() != 1 || !metadata.cfa().is_mosaic() {
        return RawPrepareRoute::Rejected(RawPrepareRouteRejection::UnsupportedLayout);
    }
    if let super::execution::RawPrepareCfa::Bayer { filters, .. } = metadata.cfa()
        && (filters == 0 || filters == 9)
    {
        return RawPrepareRoute::Rejected(RawPrepareRouteRejection::UnsupportedLayout);
    }
    RawPrepareRoute::RawPrepare(RawPrepareSourceOperation::RawPrepare)
}
