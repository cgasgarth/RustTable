//! Source-to-Rust responsibility map for the bounded Basecurve leaf.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasecurveSourceMapEntry {
    pub native_symbol: &'static str,
    pub native_file: &'static str,
    pub rust_symbol: &'static str,
    pub status: BasecurvePortStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasecurvePortStatus {
    Ported,
    ExplicitlyDeferred,
}

pub const BASECURVE_SOURCE_MAP: &[BasecurveSourceMapEntry] = &[
    BasecurveSourceMapEntry {
        native_symbol: "dt_iop_basecurve_params_t / legacy_params",
        native_file: "src/iop/basecurve.c",
        rust_symbol: "BasecurveParameters / BasecurveHistory::decode",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "init / set_presets / reload_defaults / _match / _check_camera",
        native_file: "src/iop/basecurve.c",
        rust_symbol: "BasecurveParameters::defaults / init_presets / reload_defaults / match_pattern / check_camera",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "commit_params",
        native_file: "src/iop/basecurve.c",
        rust_symbol: "BasecurvePlan::compile",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "apply_legacy_curve / process_lut",
        native_file: "src/iop/basecurve.c",
        rust_symbol: "BasecurvePlan::execute_rgba_with_profile",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "dt_iop_estimate_exp / dt_iop_eval_exp",
        native_file: "src/develop/imageop_math.h",
        rust_symbol: "estimate_exp / eval_exp / lookup_unbounded",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "apply_curve / dt_rgb_norm / dt_ioppr_get_rgb_matrix_luminance",
        native_file: "src/iop/basecurve.c; src/common/rgb_norms.h; src/common/iop_profile.h",
        rust_symbol: "apply_curve / rgb_norm / BasecurveProfileEvidence::working_luminance",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "tiling_callback",
        native_file: "src/iop/basecurve.c",
        rust_symbol: "BasecurveCapabilities::bounded_cpu_leaf",
        status: BasecurvePortStatus::Ported,
    },
    BasecurveSourceMapEntry {
        native_symbol: "process_fusion / process_cl_fusion",
        native_file: "src/iop/basecurve.c; data/kernels/basecurve.cl",
        rust_symbol: "BasecurveCompileError::UnsupportedExposureFusion",
        status: BasecurvePortStatus::ExplicitlyDeferred,
    },
    BasecurveSourceMapEntry {
        native_symbol: "process_cl / GUI callbacks / application registration",
        native_file: "src/iop/basecurve.c; data/kernels/basecurve.cl",
        rust_symbol: "BasecurveCapabilities (gpu=false, gtk=false, production_routing=deferred)",
        status: BasecurvePortStatus::ExplicitlyDeferred,
    },
];
