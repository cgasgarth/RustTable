//! Source responsibility map for the bounded RGB Curve leaf.
//!
//! Every entry names the retained native symbol and its Rust destination. It
//! is intentionally operation-local so integration owners can see deferred
//! seams without implying that the production registry or GTK has been ported.

#![forbid(unsafe_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceResponsibility {
    pub native_file: &'static str,
    pub native_symbol: &'static str,
    pub rust_file: &'static str,
    pub status: &'static str,
}

pub const RESPONSIBILITIES: &[SourceResponsibility] = &[
    entry(
        "src/iop/rgbcurve.c",
        "name/default_group/flags/default_colorspace/description",
        "execution.rs",
        "operation metadata contract; shared registry deferred",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "init_presets",
        "presets.rs",
        "ported leaf with generic/default/RGB-display metadata",
    ),
    entry("src/iop/rgbcurve.c", "init", "parameters.rs", "ported leaf"),
    entry(
        "src/iop/rgbcurve.c",
        "init_pipe",
        "execution.rs",
        "ported leaf",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "cleanup_pipe",
        "execution.rs",
        "operation-local ownership contract; shared lifecycle deferred",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "commit_params",
        "execution.rs",
        "ported leaf",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "_generate_curve_lut",
        "curve.rs",
        "ported leaf",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "process",
        "execution.rs",
        "ported CPU leaf",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "init_global/cleanup_global",
        "execution.rs",
        "GPU lifecycle documented; executable GPU deferred",
    ),
    entry(
        "src/iop/rgbcurve.c + data/kernels/rgbcurve.cl",
        "process_cl/kernel argument ABI",
        "execution.rs",
        "GPU unavailable; CPU fallback required",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "process_cl",
        "execution.rs",
        "GPU unavailable; fail closed",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "gui_changed",
        "editor.rs",
        "pure state only; profile transform modeled",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "_add_node",
        "editor.rs",
        "pure state only",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "_add_node_from_picker",
        "editor.rs",
        "pure state only; caller supplies normalized/profile-scaled values",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "_sanity_check",
        "editor.rs",
        "pure state only",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "gui_reset",
        "editor.rs",
        "pure state only",
    ),
    entry(
        "src/iop/rgbcurve.c",
        "change_image",
        "editor.rs",
        "pure state only",
    ),
    entry(
        "src/common/curve_tools.c",
        "CurveDataSample",
        "curve.rs",
        "shared V1 sampler",
    ),
    entry(
        "src/common/curve_tools.c",
        "interpolate_set",
        "common/curve_tools.rs",
        "read-only Rust oracle",
    ),
    entry(
        "src/common/curve_tools.c",
        "interpolate_val",
        "common/curve_tools.rs",
        "read-only Rust oracle",
    ),
    entry(
        "src/develop/imageop_math.h",
        "dt_iop_estimate_exp",
        "curve.rs",
        "ported leaf",
    ),
    entry(
        "src/develop/imageop_math.h",
        "dt_iop_eval_exp",
        "curve.rs",
        "ported leaf",
    ),
    entry(
        "src/common/rgb_norms.h",
        "dt_rgb_norm",
        "execution.rs",
        "ported leaf",
    ),
    entry(
        "src/common/iop_profile.h",
        "matrix_in/matrix_out/nonlinearlut evidence",
        "curve.rs",
        "row-major orientation and independent TRC state",
    ),
    entry(
        "src/common/colorspaces_inline_conversions.h",
        "middle-grey conversions",
        "curve.rs",
        "operation-local evidence",
    ),
    entry(
        "data/kernels/rgbcurve.cl",
        "rgbcurve",
        "execution.rs",
        "GPU unavailable; fail closed",
    ),
    entry(
        "src/gui/draw.h",
        "dt_draw_curve_calc_value",
        "editor.rs",
        "pure state only",
    ),
    entry(
        "src/dtgtk/drawingarea.c",
        "dtgtk_drawing_area_new_with_height",
        "editor.rs",
        "GTK deferred",
    ),
];

const fn entry(
    native_file: &'static str,
    native_symbol: &'static str,
    rust_file: &'static str,
    status: &'static str,
) -> SourceResponsibility {
    SourceResponsibility {
        native_file,
        native_symbol,
        rust_file,
        status,
    }
}
