//! Source-mapped Color Reconstruction editor for Darktable
//! `src/iop/colorreconstruction.c`.
//!
//! This leaf owns only parameter presentation and edit settlement. Color
//! reconstruction equations and pipeline qualification remain in processing.

mod gtk;
mod model;

pub use gtk::{
    ColorReconstructionGtkActionHandler, ColorReconstructionGtkHandlerOutcome,
    ColorReconstructionGtkLeaf, ColorReconstructionGtkState, ColorReconstructionSettledAction,
    build_colorreconstruction_gtk,
};
pub use model::{ColorReconstructionEditorState, ColorReconstructionParameter};

/// Stable Darktable operation name used by history, styles, and module order.
pub const COLORRECONSTRUCTION_MODULE_ID: &str = "colorreconstruct";
/// Native module title.
pub const COLORRECONSTRUCTION_TITLE: &str = "color reconstruction";
/// Native module description.
pub const COLORRECONSTRUCTION_DESCRIPTION: &str =
    "recover clipped highlights by propagating surrounding colors";
/// Native module groups in declaration order.
pub const COLORRECONSTRUCTION_GROUP_KEYS: [&str; 2] = ["group.basic", "group.technical"];

/// The text shown instead of controls for a monochrome image.
pub const COLORRECONSTRUCTION_MONOCHROME_LABEL: &str = "not applicable";
/// Native explanation attached to the monochrome replacement text.
pub const COLORRECONSTRUCTION_MONOCHROME_TOOLTIP: &str =
    "no highlights reconstruction for monochrome images";

/// One source-order entry in `gui_init`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorReconstructionControl {
    Threshold,
    Spatial,
    Range,
    Precedence,
    Hue,
}

/// Exact source control order.
pub const COLORRECONSTRUCTION_CONTROLS: [ColorReconstructionControl; 5] = [
    ColorReconstructionControl::Threshold,
    ColorReconstructionControl::Spatial,
    ColorReconstructionControl::Range,
    ColorReconstructionControl::Precedence,
    ColorReconstructionControl::Hue,
];

/// One source-order hue gradient stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionHueStop {
    position: f64,
    rgb: [f64; 3],
}

impl ColorReconstructionHueStop {
    #[must_use]
    pub const fn new(position: f64, rgb: [f64; 3]) -> Self {
        Self { position, rgb }
    }

    #[must_use]
    pub const fn position(self) -> f64 {
        self.position
    }

    #[must_use]
    pub const fn rgb(self) -> [f64; 3] {
        self.rgb
    }
}

/// Exact source presentation for one Color Reconstruction slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionSliderSpec {
    parameter: ColorReconstructionParameter,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    default_value: f64,
    digits: i32,
    gtk_step: f64,
    factor: f64,
    suffix: &'static str,
    fill_feedback: bool,
    tooltip: &'static str,
}

impl ColorReconstructionSliderSpec {
    #[must_use]
    pub const fn parameter(self) -> ColorReconstructionParameter {
        self.parameter
    }

    #[must_use]
    pub const fn parameter_id(self) -> &'static str {
        self.parameter.id()
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn range(self) -> (f64, f64) {
        (self.minimum, self.maximum)
    }

    #[must_use]
    pub const fn default_value(self) -> f64 {
        self.default_value
    }

    #[must_use]
    pub const fn digits(self) -> i32 {
        self.digits
    }

    #[must_use]
    pub const fn gtk_step(self) -> f64 {
        self.gtk_step
    }

    #[must_use]
    pub const fn factor(self) -> f64 {
        self.factor
    }

    #[must_use]
    pub const fn suffix(self) -> &'static str {
        self.suffix
    }

    #[must_use]
    pub const fn fill_feedback(self) -> bool {
        self.fill_feedback
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        self.tooltip
    }

    /// Returns the stable widget identity under a module-instance prefix.
    #[must_use]
    pub fn widget_name(self, widget_id: &str) -> String {
        format!("{widget_id}-{}", self.parameter_id())
    }
}

pub const COLORRECONSTRUCTION_THRESHOLD_SLIDER: ColorReconstructionSliderSpec =
    ColorReconstructionSliderSpec {
        parameter: ColorReconstructionParameter::Threshold,
        label: "threshold",
        minimum: 50.0,
        maximum: 150.0,
        default_value: 100.0,
        digits: 2,
        gtk_step: 1.0,
        factor: 1.0,
        suffix: "",
        fill_feedback: true,
        tooltip: "pixels with lightness values above this threshold are corrected",
    };

pub const COLORRECONSTRUCTION_SPATIAL_SLIDER: ColorReconstructionSliderSpec =
    ColorReconstructionSliderSpec {
        parameter: ColorReconstructionParameter::Spatial,
        label: "spatial",
        minimum: 0.0,
        maximum: 1000.0,
        default_value: 400.0,
        digits: 2,
        gtk_step: 1.0,
        factor: 1.0,
        suffix: "",
        fill_feedback: true,
        tooltip: "how far to look for replacement colors in spatial dimensions",
    };

pub const COLORRECONSTRUCTION_RANGE_SLIDER: ColorReconstructionSliderSpec =
    ColorReconstructionSliderSpec {
        parameter: ColorReconstructionParameter::Range,
        label: "range",
        minimum: 0.0,
        maximum: 50.0,
        default_value: 10.0,
        digits: 2,
        gtk_step: 1.0,
        factor: 1.0,
        suffix: "",
        fill_feedback: true,
        tooltip: "how far to look for replacement colors in the luminance dimension",
    };

pub const COLORRECONSTRUCTION_HUE_SLIDER: ColorReconstructionSliderSpec =
    ColorReconstructionSliderSpec {
        parameter: ColorReconstructionParameter::Hue,
        label: "hue",
        minimum: 0.0,
        maximum: 1.0,
        default_value: 0.66,
        digits: 2,
        gtk_step: 0.01,
        factor: 360.0,
        suffix: "°",
        fill_feedback: false,
        tooltip: "the hue tone which should be given precedence over other hue tones",
    };

/// Sliders in parameter order; `COLORRECONSTRUCTION_CONTROLS` retains the
/// combobox's position among them.
pub const COLORRECONSTRUCTION_SLIDERS: [ColorReconstructionSliderSpec; 4] = [
    COLORRECONSTRUCTION_THRESHOLD_SLIDER,
    COLORRECONSTRUCTION_SPATIAL_SLIDER,
    COLORRECONSTRUCTION_RANGE_SLIDER,
    COLORRECONSTRUCTION_HUE_SLIDER,
];

/// Exact source-order stops installed on the hue slider.
pub const COLORRECONSTRUCTION_HUE_STOPS: [ColorReconstructionHueStop; 7] = [
    ColorReconstructionHueStop::new(0.0, [1.0, 0.0, 0.0]),
    ColorReconstructionHueStop::new(0.166, [1.0, 1.0, 0.0]),
    ColorReconstructionHueStop::new(0.322, [0.0, 1.0, 0.0]),
    ColorReconstructionHueStop::new(0.498, [0.0, 1.0, 1.0]),
    ColorReconstructionHueStop::new(0.664, [0.0, 0.0, 1.0]),
    ColorReconstructionHueStop::new(0.830, [1.0, 0.0, 1.0]),
    ColorReconstructionHueStop::new(1.0, [1.0, 0.0, 0.0]),
];

pub const COLORRECONSTRUCTION_PRECEDENCE_LABEL: &str = "precedence";
pub const COLORRECONSTRUCTION_PRECEDENCE_OPTIONS: [&str; 3] = ["none", "saturated colors", "hue"];
pub const COLORRECONSTRUCTION_PRECEDENCE_TOOLTIP: &str =
    "if and how to give precedence to specific replacement colors";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_metadata_and_control_order_are_exact() {
        assert_eq!(COLORRECONSTRUCTION_MODULE_ID, "colorreconstruct");
        assert_eq!(COLORRECONSTRUCTION_TITLE, "color reconstruction");
        assert_eq!(
            COLORRECONSTRUCTION_DESCRIPTION,
            "recover clipped highlights by propagating surrounding colors"
        );
        assert_eq!(
            COLORRECONSTRUCTION_GROUP_KEYS,
            ["group.basic", "group.technical"]
        );
        assert_eq!(
            COLORRECONSTRUCTION_CONTROLS,
            [
                ColorReconstructionControl::Threshold,
                ColorReconstructionControl::Spatial,
                ColorReconstructionControl::Range,
                ColorReconstructionControl::Precedence,
                ColorReconstructionControl::Hue,
            ]
        );
    }

    #[test]
    fn source_slider_ranges_defaults_labels_formats_and_tooltips_are_exact() {
        let expected = [
            (
                COLORRECONSTRUCTION_THRESHOLD_SLIDER,
                "threshold",
                (50.0, 150.0),
                100.0_f64,
                "pixels with lightness values above this threshold are corrected",
            ),
            (
                COLORRECONSTRUCTION_SPATIAL_SLIDER,
                "spatial",
                (0.0, 1000.0),
                400.0_f64,
                "how far to look for replacement colors in spatial dimensions",
            ),
            (
                COLORRECONSTRUCTION_RANGE_SLIDER,
                "range",
                (0.0, 50.0),
                10.0_f64,
                "how far to look for replacement colors in the luminance dimension",
            ),
            (
                COLORRECONSTRUCTION_HUE_SLIDER,
                "hue",
                (0.0, 1.0),
                0.66,
                "the hue tone which should be given precedence over other hue tones",
            ),
        ];
        for (slider, label, range, default_value, tooltip) in expected {
            assert_eq!(slider.label(), label);
            assert_eq!(slider.range(), range);
            assert_eq!(slider.default_value().to_bits(), default_value.to_bits());
            assert_eq!(slider.digits(), 2);
            assert_eq!(slider.tooltip(), tooltip);
            assert_eq!(
                slider.widget_name("colorreconstruction"),
                format!("colorreconstruction-{}", slider.parameter_id())
            );
        }
        assert_eq!(
            COLORRECONSTRUCTION_HUE_SLIDER.factor().to_bits(),
            360.0_f64.to_bits()
        );
        assert_eq!(COLORRECONSTRUCTION_HUE_SLIDER.suffix(), "°");
        assert!(!COLORRECONSTRUCTION_HUE_SLIDER.fill_feedback());
        assert!(COLORRECONSTRUCTION_SLIDERS[..3].iter().all(|slider| {
            slider.fill_feedback() && slider.factor().to_bits() == 1.0_f64.to_bits()
        }));
    }

    #[test]
    fn precedence_hue_stops_and_monochrome_state_are_exact() {
        assert_eq!(COLORRECONSTRUCTION_PRECEDENCE_LABEL, "precedence");
        assert_eq!(
            COLORRECONSTRUCTION_PRECEDENCE_OPTIONS,
            ["none", "saturated colors", "hue"]
        );
        assert_eq!(
            COLORRECONSTRUCTION_HUE_STOPS.map(|stop| stop.position().to_bits()),
            [
                0.0_f64.to_bits(),
                0.166_f64.to_bits(),
                0.322_f64.to_bits(),
                0.498_f64.to_bits(),
                0.664_f64.to_bits(),
                0.830_f64.to_bits(),
                1.0_f64.to_bits()
            ]
        );
        assert_eq!(
            COLORRECONSTRUCTION_HUE_STOPS[0].rgb().map(f64::to_bits),
            [1.0_f64.to_bits(), 0.0_f64.to_bits(), 0.0_f64.to_bits()]
        );
        assert_eq!(
            COLORRECONSTRUCTION_HUE_STOPS[3].rgb().map(f64::to_bits),
            [0.0_f64.to_bits(), 1.0_f64.to_bits(), 1.0_f64.to_bits()]
        );
        assert_eq!(
            COLORRECONSTRUCTION_HUE_STOPS[6].rgb().map(f64::to_bits),
            [1.0_f64.to_bits(), 0.0_f64.to_bits(), 0.0_f64.to_bits()]
        );
        assert_eq!(COLORRECONSTRUCTION_MONOCHROME_LABEL, "not applicable");
        assert_eq!(
            COLORRECONSTRUCTION_MONOCHROME_TOOLTIP,
            "no highlights reconstruction for monochrome images"
        );
    }
}
