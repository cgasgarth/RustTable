//! Source-mapped generic GTK presentation for Darktable's `src/iop/highpass.c`.
//!
//! The native `gui_init` creates exactly two ordinary Bauhaus sliders. This
//! leaf keeps their source order and presentation metadata together while the
//! shared darkroom module widgets provide the GTK boundary and generic control
//! actions.

use crate::presentation::SourceMappedSliderSpec;

/// Stable Darktable operation name used by history, styles, and module order.
pub const HIGHPASS_MODULE_ID: &str = "highpass";
/// Native module title from `src/iop/highpass.c::name`.
pub const HIGHPASS_TITLE: &str = "highpass";
/// Native module description from `src/iop/highpass.c::description`.
pub const HIGHPASS_DESCRIPTION: &str = "isolate high frequencies in the image";
/// Native module groups in declaration order from `default_group`.
pub const HIGHPASS_GROUP_KEYS: [&str; 2] = ["group.effect", "group.effects"];

/// Exact module-level presentation retained from `src/iop/highpass.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighpassSourceMap {
    title: &'static str,
    description: &'static str,
    aliases: &'static [&'static str],
    group_keys: &'static [&'static str],
    default_enabled: bool,
    default_expanded: bool,
}

impl HighpassSourceMap {
    #[must_use]
    pub const fn title(self) -> &'static str {
        self.title
    }

    #[must_use]
    pub const fn description(self) -> &'static str {
        self.description
    }

    #[must_use]
    pub const fn aliases(self) -> &'static [&'static str] {
        self.aliases
    }

    #[must_use]
    pub const fn group_keys(self) -> &'static [&'static str] {
        self.group_keys
    }

    #[must_use]
    pub const fn default_enabled(self) -> bool {
        self.default_enabled
    }

    #[must_use]
    pub const fn default_expanded(self) -> bool {
        self.default_expanded
    }

    /// Returns only the controls created by native `gui_init`, in source order.
    #[must_use]
    pub fn slider(self, parameter_id: &str) -> Option<HighpassSliderSourceMap> {
        match parameter_id {
            "sharpness" => Some(HIGHPASS_SHARPNESS),
            "contrast" => Some(HIGHPASS_CONTRAST),
            _ => None,
        }
    }
}

/// Source module metadata. Highpass is not enabled in a new module instance.
pub const HIGHPASS_SOURCE_MAP: HighpassSourceMap = HighpassSourceMap {
    title: HIGHPASS_TITLE,
    description: HIGHPASS_DESCRIPTION,
    aliases: &[],
    group_keys: &HIGHPASS_GROUP_KEYS,
    default_enabled: false,
    default_expanded: false,
};

/// Exact source presentation for one native Highpass Bauhaus slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighpassSliderSourceMap {
    parameter_id: &'static str,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    default_value: f64,
    digits: i32,
    step: f64,
    automatic_step: bool,
    suffix: &'static str,
    tooltip: &'static str,
}

impl HighpassSliderSourceMap {
    #[must_use]
    pub const fn parameter_id(self) -> &'static str {
        self.parameter_id
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
    pub const fn step(self) -> f64 {
        self.step
    }

    #[must_use]
    pub const fn automatic_step(self) -> bool {
        self.automatic_step
    }

    #[must_use]
    pub const fn suffix(self) -> &'static str {
        self.suffix
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        self.tooltip
    }

    /// Stable logical control id used by generic darkroom actions.
    #[must_use]
    pub fn control_id(self) -> String {
        format!("{HIGHPASS_MODULE_ID}-{}", self.parameter_id())
    }

    /// Stable scale id under a module-instance widget prefix.
    #[must_use]
    pub fn widget_name(self, widget_id: &str) -> String {
        format!("{widget_id}-{}-widget", self.parameter_id())
    }

    pub(crate) const fn slider_presentation(self) -> SourceMappedSliderSpec {
        SourceMappedSliderSpec::new(self.suffix, self.digits, self.automatic_step, self.tooltip)
    }
}

/// Native `sharpness` slider, first in `gui_init`.
pub const HIGHPASS_SHARPNESS: HighpassSliderSourceMap = HighpassSliderSourceMap {
    parameter_id: "sharpness",
    label: "sharpness",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 50.0,
    digits: 2,
    step: 1.0,
    automatic_step: true,
    suffix: "%",
    tooltip: "the sharpness of highpass filter",
};

/// Native `contrast` slider, second in `gui_init`.
pub const HIGHPASS_CONTRAST: HighpassSliderSourceMap = HighpassSliderSourceMap {
    parameter_id: "contrast",
    label: "contrast boost",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 50.0,
    digits: 2,
    step: 1.0,
    automatic_step: true,
    suffix: "%",
    tooltip: "the contrast of highpass filter",
};

/// Source-required generic control order for the Highpass leaf.
pub const HIGHPASS_SLIDERS: [HighpassSliderSourceMap; 2] = [HIGHPASS_SHARPNESS, HIGHPASS_CONTRAST];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_map_preserves_native_module_metadata_and_disabled_default() {
        assert_eq!(HIGHPASS_MODULE_ID, "highpass");
        assert_eq!(HIGHPASS_SOURCE_MAP.title(), "highpass");
        assert_eq!(
            HIGHPASS_SOURCE_MAP.description(),
            "isolate high frequencies in the image"
        );
        assert!(HIGHPASS_SOURCE_MAP.aliases().is_empty());
        assert_eq!(
            HIGHPASS_SOURCE_MAP.group_keys(),
            ["group.effect", "group.effects"]
        );
        assert!(!HIGHPASS_SOURCE_MAP.default_enabled());
        assert!(!HIGHPASS_SOURCE_MAP.default_expanded());
    }

    #[test]
    fn source_map_preserves_only_the_two_native_gui_sliders_in_order() {
        assert_eq!(
            HIGHPASS_SLIDERS.map(HighpassSliderSourceMap::parameter_id),
            ["sharpness", "contrast"]
        );
        let expected = [
            (
                HIGHPASS_SHARPNESS,
                "sharpness",
                "sharpness",
                "the sharpness of highpass filter",
            ),
            (
                HIGHPASS_CONTRAST,
                "contrast",
                "contrast boost",
                "the contrast of highpass filter",
            ),
        ];
        for (slider, parameter_id, label, tooltip) in expected {
            assert_eq!(slider.parameter_id(), parameter_id);
            assert_eq!(slider.label(), label);
            assert_eq!(slider.range(), (0.0, 100.0));
            assert_eq!(slider.default_value().to_bits(), 50.0_f64.to_bits());
            assert_eq!(slider.digits(), 2);
            assert_eq!(slider.step().to_bits(), 1.0_f64.to_bits());
            assert!(slider.automatic_step());
            assert_eq!(slider.suffix(), "%");
            assert_eq!(slider.tooltip(), tooltip);
            assert_eq!(slider.control_id(), format!("highpass-{parameter_id}"));
            assert_eq!(
                slider.widget_name("highpass"),
                format!("highpass-{parameter_id}-widget")
            );
            assert_eq!(HIGHPASS_SOURCE_MAP.slider(parameter_id), Some(slider));
        }
        assert_eq!(HIGHPASS_SOURCE_MAP.slider("enabled"), None);
        assert_eq!(HIGHPASS_SOURCE_MAP.slider("sharpness[0]"), None);
    }
}
