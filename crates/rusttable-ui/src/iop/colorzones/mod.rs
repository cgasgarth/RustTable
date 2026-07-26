//! Source-mapped Color Zones editor contract for Darktable `src/iop/colorzones.c`.
//!
//! The GTK registry deliberately does not mount this editor yet. This parent
//! keeps the pure parameter model, source-shaped interactions, and render data
//! preparation together so a future GTK4 adapter can remain presentation-only.

mod interaction;
mod model;
mod render;

pub use interaction::{
    COLORZONES_BAND_NAMES, COLORZONES_BANDS, COLORZONES_DEFAULT_STEP, COLORZONES_PICKER_FEATHER,
    ColorZonesArrowKey, ColorZonesBandEffect, ColorZonesBandOutcome, ColorZonesClick,
    ColorZonesInteraction, ColorZonesInteractionError, ColorZonesModifiers, ColorZonesPickerRange,
    ColorZonesPrimaryOutcome, ColorZonesScrollOutcome, ColorZonesSecondaryOutcome,
    ColorZonesSelection,
};
pub use model::{
    COLORZONES_MIN_X_DISTANCE, ColorZonesDeleteOutcome, ColorZonesEditorError,
    ColorZonesEditorState,
};
pub use render::{
    COLORZONES_ACTIVE_CURVE_OPACITY, COLORZONES_AREA_MARKER_WIDTH,
    COLORZONES_BACKGROUND_SATURATION_DEFAULT, COLORZONES_BACKGROUND_SATURATION_MAX,
    COLORZONES_BACKGROUND_SATURATION_MIN, COLORZONES_BAND_LANDMARKS, COLORZONES_CURVE_STROKE_WIDTH,
    COLORZONES_FIELD_COLUMNS, COLORZONES_FIELD_ROWS, COLORZONES_GRAPH_HEIGHT_DEFAULT,
    COLORZONES_GRAPH_HEIGHT_MAX, COLORZONES_GRAPH_HEIGHT_MIN, COLORZONES_GRAPH_INSET,
    COLORZONES_GRAPH_RESOLUTION, COLORZONES_INACTIVE_CURVE_OPACITY, COLORZONES_KNOT_RING_RADIUS,
    COLORZONES_RING_STROKE_WIDTH, COLORZONES_SELECTED_RING_RADIUS, ColorZonesBackgroundSaturation,
    ColorZonesBandLandmark, ColorZonesBottomCell, ColorZonesFieldCell, ColorZonesGraphCurve,
    ColorZonesGraphHeight, ColorZonesGraphPoint, ColorZonesKnot, ColorZonesLch, ColorZonesRect,
    ColorZonesRenderError, ColorZonesRenderModel, ColorZonesRenderOptions, ColorZonesViewTransform,
};

/// Stable Darktable operation name used by history, styles, and module order.
pub const COLORZONES_MODULE_ID: &str = "colorzones";
/// Native module title.
pub const COLORZONES_TITLE: &str = "color zones";
/// Native module description.
pub const COLORZONES_DESCRIPTION: &str = "selectively shift hues, chroma and lightness of pixels";
/// Native module groups in declaration order.
pub const COLORZONES_GROUP_KEYS: [&str; 2] = ["group.color", "group.grading"];

#[cfg(test)]
mod tests {
    use rusttable_processing::{
        COLORZONES_MAX_NODES, ColorZonesChannel, ColorZonesMode, ColorZonesSplinesVersion,
    };

    use super::{
        COLORZONES_BANDS, COLORZONES_DEFAULT_STEP, COLORZONES_DESCRIPTION,
        COLORZONES_GRAPH_RESOLUTION, COLORZONES_GROUP_KEYS, COLORZONES_MIN_X_DISTANCE,
        COLORZONES_MODULE_ID, COLORZONES_TITLE,
    };

    #[test]
    fn source_metadata_preserves_native_identity_and_group_order() {
        assert_eq!(COLORZONES_MODULE_ID, "colorzones");
        assert_eq!(COLORZONES_TITLE, "color zones");
        assert_eq!(
            COLORZONES_DESCRIPTION,
            "selectively shift hues, chroma and lightness of pixels"
        );
        assert_eq!(COLORZONES_GROUP_KEYS, ["group.color", "group.grading"]);
    }

    #[test]
    fn source_numeric_and_enum_constants_are_exact() {
        assert_eq!(COLORZONES_GRAPH_RESOLUTION, 256);
        assert_eq!(COLORZONES_BANDS, 8);
        assert_eq!(COLORZONES_MAX_NODES, 20);
        assert_eq!(COLORZONES_DEFAULT_STEP.to_bits(), 0.001_f32.to_bits());
        assert_eq!(COLORZONES_MIN_X_DISTANCE.to_bits(), 0.0025_f32.to_bits());
        assert_eq!(ColorZonesMode::Smooth.raw(), 0);
        assert_eq!(ColorZonesMode::Strong.raw(), 1);
        assert_eq!(ColorZonesSplinesVersion::V1.raw(), 0);
        assert_eq!(ColorZonesSplinesVersion::V2.raw(), 1);
        assert_eq!(ColorZonesChannel::Lightness.raw(), 0);
        assert_eq!(ColorZonesChannel::Chroma.raw(), 1);
        assert_eq!(ColorZonesChannel::Hue.raw(), 2);
    }
}
