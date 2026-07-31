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
        status: "GPU unavailable; CPU/OpenCL threshold mismatch is not approximated",
    },
    Responsibility {
        native_symbol: "gui_init/gui_changed",
        source: "src/iop/tonecurve.c",
        status: "GTK/UI deferred and not projected through generic controls",
    },
    Responsibility {
        native_symbol: "registry/history/pixelpipe/blending/presets",
        source: "src/iop/tonecurve.c and integration hubs",
        status: "deferred outside operation-local ownership",
    },
];
