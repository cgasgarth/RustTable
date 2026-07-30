//! Source-mapped presentation contract for Darktable's Color Contrast operation.

use crate::presentation::SourceMappedSliderSpec;

/// Stable Darktable operation name used by history, styles, and module order.
pub const COLORCONTRAST_MODULE_ID: &str = "colorcontrast";

/// Exact module-level presentation retained from `src/iop/colorcontrast.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorContrastSourceMap {
    title: &'static str,
    aliases: &'static [&'static str],
    group_keys: &'static [&'static str],
    default_enabled: bool,
    default_expanded: bool,
}

impl ColorContrastSourceMap {
    #[must_use]
    pub const fn title(self) -> &'static str {
        self.title
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

    /// Returns only controls created by native `gui_init`. Offsets and
    /// `unbound` remain persisted processing parameters without UI widgets.
    #[must_use]
    pub fn slider(self, parameter_id: &str) -> Option<ColorContrastSliderSourceMap> {
        match parameter_id {
            "a_steepness" => Some(COLORCONTRAST_A_STEEPNESS),
            "b_steepness" => Some(COLORCONTRAST_B_STEEPNESS),
            _ => None,
        }
    }
}

/// Exact source presentation for one Color Contrast scalar parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorContrastSliderSourceMap {
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

impl ColorContrastSliderSourceMap {
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

    pub(crate) const fn slider_presentation(self) -> SourceMappedSliderSpec {
        SourceMappedSliderSpec::new(self.suffix, self.digits, self.automatic_step, self.tooltip)
    }
}

/// Source module metadata. The color group precedes grading to match
/// Darktable's `IOP_GROUP_COLOR | IOP_GROUP_GRADING` declaration.
pub const COLORCONTRAST_SOURCE_MAP: ColorContrastSourceMap = ColorContrastSourceMap {
    title: "color contrast",
    aliases: &["saturation"],
    group_keys: &["group.color", "group.grading"],
    default_enabled: false,
    default_expanded: false,
};

/// Source `a_steepness` slider metadata.
pub const COLORCONTRAST_A_STEEPNESS: ColorContrastSliderSourceMap = ColorContrastSliderSourceMap {
    parameter_id: "a_steepness",
    label: "green-magenta contrast",
    minimum: 0.0,
    maximum: 5.0,
    default_value: 1.0,
    digits: 2,
    step: 0.05,
    automatic_step: true,
    suffix: "",
    tooltip: "steepness of the a* curve in Lab\nlower values desaturate greens and magenta while higher saturate them",
};

/// Source `b_steepness` slider metadata.
pub const COLORCONTRAST_B_STEEPNESS: ColorContrastSliderSourceMap = ColorContrastSliderSourceMap {
    parameter_id: "b_steepness",
    label: "blue-yellow contrast",
    minimum: 0.0,
    maximum: 5.0,
    default_value: 1.0,
    digits: 2,
    step: 0.05,
    automatic_step: true,
    suffix: "",
    tooltip: "steepness of the b* curve in Lab\nlower values desaturate blues and yellows while higher saturate them",
};

#[cfg(test)]
mod tests {
    use super::{COLORCONTRAST_A_STEEPNESS, COLORCONTRAST_B_STEEPNESS, COLORCONTRAST_SOURCE_MAP};

    #[test]
    fn source_map_preserves_darktable_module_metadata() {
        assert_eq!(COLORCONTRAST_SOURCE_MAP.title(), "color contrast");
        assert_eq!(COLORCONTRAST_SOURCE_MAP.aliases(), ["saturation"]);
        assert_eq!(
            COLORCONTRAST_SOURCE_MAP.group_keys(),
            ["group.color", "group.grading"]
        );
        assert!(!COLORCONTRAST_SOURCE_MAP.default_enabled());
        assert!(!COLORCONTRAST_SOURCE_MAP.default_expanded());
    }

    #[test]
    fn source_map_preserves_only_native_gui_sliders() {
        let expected = [
            (
                COLORCONTRAST_A_STEEPNESS,
                "a_steepness",
                "green-magenta contrast",
                "steepness of the a* curve in Lab\nlower values desaturate greens and magenta while higher saturate them",
            ),
            (
                COLORCONTRAST_B_STEEPNESS,
                "b_steepness",
                "blue-yellow contrast",
                "steepness of the b* curve in Lab\nlower values desaturate blues and yellows while higher saturate them",
            ),
        ];
        for (slider, parameter_id, label, tooltip) in expected {
            assert_eq!(slider.parameter_id(), parameter_id);
            assert_eq!(slider.label(), label);
            assert_eq!(slider.range(), (0.0, 5.0));
            assert_float_eq(slider.default_value(), 1.0);
            assert_eq!(slider.digits(), 2);
            assert_float_eq(slider.step(), 0.05);
            assert!(slider.automatic_step());
            assert_eq!(slider.suffix(), "");
            assert_eq!(slider.tooltip(), tooltip);
            assert_eq!(COLORCONTRAST_SOURCE_MAP.slider(parameter_id), Some(slider));
        }
        for hidden in ["a_offset", "b_offset", "unbound"] {
            assert_eq!(COLORCONTRAST_SOURCE_MAP.slider(hidden), None);
        }
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() <= f64::EPSILON);
    }
}
