//! Source responsibility map for the bounded Tone Equalizer leaf.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Responsibility {
    pub native_symbol: &'static str,
    pub source: &'static str,
    pub status: &'static str,
}

pub const RESPONSIBILITIES: &[Responsibility] = &[
    Responsibility {
        native_symbol: "dt_iop_toneequalizer_params_t",
        source: "src/iop/toneequal.c",
        status: "ported v2 ABI and v1 migration",
    },
    Responsibility {
        native_symbol: "compute_correction_lut",
        source: "src/iop/toneequal.c",
        status: "ported eight-basis radial tone curve and 80001-entry LUT",
    },
    Responsibility {
        native_symbol: "luminance_mask",
        source: "src/common/luminance_mask.h",
        status: "ported all seven scene-linear RGB estimators",
    },
    Responsibility {
        native_symbol: "fast_surface_blur",
        source: "src/common/fast_guided_filter.h and src/common/box_filters.cc",
        status: "ported source-specific downsampled guided filter; generic blur not substituted",
    },
    Responsibility {
        native_symbol: "fast_eigf_surface_blur",
        source: "src/common/eigf.h and src/common/gaussian.c",
        status: "ported exposure-independent guided filter and recursive Gaussian statistics",
    },
    Responsibility {
        native_symbol: "process",
        source: "src/iop/toneequal.c",
        status: "ported bounded CPU RGBA execution: corrected output scales all four lanes; mask-display output preserves input alpha via the native copy, with cancellation and publication",
    },
    Responsibility {
        native_symbol: "modify_roi_in",
        source: "src/iop/toneequal.c",
        status: "ported radius equation; independent tiling deferred because native has no tile callback and EIGF is image-stateful",
    },
    Responsibility {
        native_symbol: "process_cl/gui_init/init_presets",
        source: "src/iop/toneequal.c and data/kernels",
        status: "GPU, GTK, presets, registry, history routing, blending, masks, and pixelpipe integration deferred",
    },
];
