//! Source responsibility map for the bounded Tone Curve leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Responsibility {
    pub native_symbol: &'static str,
    pub source: &'static str,
    pub status: &'static str,
}

pub const RESPONSIBILITIES: &[Responsibility] = &[
    Responsibility {
        native_symbol: "dt_iop_tonecurve_params_t",
        source: "src/iop/tonecurve.c",
        status: "ported v5 ABI and defaults",
    },
    Responsibility {
        native_symbol: "legacy_params",
        source: "src/iop/tonecurve.c",
        status: "ported v1/v3/v4 to v5; v2 and unsupported versions fail closed",
    },
    Responsibility {
        native_symbol: "commit_params",
        source: "src/iop/tonecurve.c",
        status: "ported curve build, sampling, scaling, linked derivation, and fits",
    },
    Responsibility {
        native_symbol: "process",
        source: "src/iop/tonecurve.c",
        status: "ported bounded CPU RGBA leaf with exact alpha and cancellation boundary",
    },
    Responsibility {
        native_symbol: "dt_ioppr_get_rgb_matrix_luminance",
        source: "src/common/iop_profile.h and src/common/colorspaces_inline_conversions.h",
        status: "ported with explicit ProPhoto profile evidence; missing evidence fails",
    },
    Responsibility {
        native_symbol: "process_cl",
        source: "data/kernels/basic.cl",
        status: "deferred: GPU unavailable; CPU/OpenCL threshold mismatch is not approximated",
    },
    Responsibility {
        native_symbol: "gui_init/gui_changed",
        source: "src/iop/tonecurve.c",
        status: "deferred: GTK/UI is not projected through generic controls",
    },
    Responsibility {
        native_symbol: "production registry / operation descriptor",
        source: "src/iop/tonecurve.c and processing integration hubs",
        status: "implemented",
    },
    Responsibility {
        native_symbol: "typed history import",
        source: "src/iop/tonecurve.c and production history route",
        status: "implemented",
    },
    Responsibility {
        native_symbol: "CPU pixelpipe dispatch",
        source: "processing evaluator and pixelpipe integration hubs",
        status: "implemented",
    },
    Responsibility {
        native_symbol: "snapshot identity",
        source: "processing operation configuration and pixelpipe snapshot",
        status: "implemented",
    },
    Responsibility {
        native_symbol: "presets",
        source: "src/iop/tonecurve.c and shared preset owners",
        status: "deferred",
    },
    Responsibility {
        native_symbol: "runtime mask coverage / operation opacity",
        source: "shared pixelpipe owners",
        status: "implemented by the dedicated Lab D50 CPU route",
    },
    Responsibility {
        native_symbol: "imported native blend/mask payloads",
        source: "src/develop/blend.c and production import owners",
        status: "deferred: opaque native blend/mask blobs are not materialized or interpreted",
    },
];
