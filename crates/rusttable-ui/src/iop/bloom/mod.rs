//! Source-mapped Bloom editor for Darktable `src/iop/bloom.c`.
//!
//! The leaf keeps parameter validation in a pure editor state and delegates all
//! slider interaction semantics to the shared Bauhaus GTK4 adapter.

mod gtk;
mod model;

pub use gtk::{
    BloomGtkActionHandler, BloomGtkHandlerOutcome, BloomGtkLeaf, BloomGtkState, BloomSettledAction,
    build_bloom_gtk,
};
pub use model::{BloomEditorState, BloomParameter};

/// Stable Darktable operation name used by history, styles, and module order.
pub const BLOOM_MODULE_ID: &str = "bloom";
/// Native module title.
pub const BLOOM_TITLE: &str = "bloom";
/// Native module description.
pub const BLOOM_DESCRIPTION: &str = "apply Orton effect for a dreamy ethereal look";
/// Native module groups in declaration order.
pub const BLOOM_GROUP_KEYS: [&str; 2] = ["group.effect", "group.effects"];

/// Exact source presentation for one ordinary Bloom Bauhaus slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomSliderSpec {
    parameter: BloomParameter,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    default_value: f64,
    digits: i32,
    gtk_step: f64,
    suffix: &'static str,
    tooltip: &'static str,
}

impl BloomSliderSpec {
    #[must_use]
    pub const fn parameter(self) -> BloomParameter {
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
    pub const fn suffix(self) -> &'static str {
        self.suffix
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

pub const BLOOM_THRESHOLD_SLIDER: BloomSliderSpec = BloomSliderSpec {
    parameter: BloomParameter::Threshold,
    label: "threshold",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 90.0,
    digits: 2,
    gtk_step: 1.0,
    suffix: "%",
    tooltip: "the threshold of light",
};

pub const BLOOM_SIZE_SLIDER: BloomSliderSpec = BloomSliderSpec {
    parameter: BloomParameter::Size,
    label: "size",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 20.0,
    digits: 2,
    gtk_step: 1.0,
    suffix: "%",
    tooltip: "the size of bloom",
};

pub const BLOOM_STRENGTH_SLIDER: BloomSliderSpec = BloomSliderSpec {
    parameter: BloomParameter::Strength,
    label: "strength",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 25.0,
    digits: 2,
    gtk_step: 1.0,
    suffix: "%",
    tooltip: "the strength of bloom",
};

/// Source-required vertical control order for the Bloom leaf.
pub const BLOOM_SLIDERS: [BloomSliderSpec; 3] = [
    BLOOM_SIZE_SLIDER,
    BLOOM_THRESHOLD_SLIDER,
    BLOOM_STRENGTH_SLIDER,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_metadata_and_slider_order_are_exact() {
        assert_eq!(BLOOM_MODULE_ID, "bloom");
        assert_eq!(BLOOM_TITLE, "bloom");
        assert_eq!(
            BLOOM_DESCRIPTION,
            "apply Orton effect for a dreamy ethereal look"
        );
        assert_eq!(BLOOM_GROUP_KEYS, ["group.effect", "group.effects"]);
        assert_eq!(
            BLOOM_SLIDERS.map(BloomSliderSpec::parameter),
            [
                BloomParameter::Size,
                BloomParameter::Threshold,
                BloomParameter::Strength,
            ]
        );
    }

    #[test]
    fn source_slider_labels_ranges_defaults_formats_and_tooltips_are_exact() {
        let expected = [
            (
                BLOOM_THRESHOLD_SLIDER,
                "threshold",
                90.0_f64,
                "the threshold of light",
            ),
            (BLOOM_SIZE_SLIDER, "size", 20.0_f64, "the size of bloom"),
            (
                BLOOM_STRENGTH_SLIDER,
                "strength",
                25.0_f64,
                "the strength of bloom",
            ),
        ];
        for (slider, label, default_value, tooltip) in expected {
            assert_eq!(slider.parameter_id(), label);
            assert_eq!(slider.label(), label);
            assert_eq!(slider.range(), (0.0, 100.0));
            assert_eq!(slider.default_value().to_bits(), default_value.to_bits());
            assert_eq!(slider.digits(), 2);
            assert_eq!(slider.gtk_step().to_bits(), 1.0_f64.to_bits());
            assert_eq!(slider.suffix(), "%");
            assert_eq!(slider.tooltip(), tooltip);
            assert_eq!(slider.widget_name("bloom"), format!("bloom-{label}"));
        }
    }
}
