//! Source-mapped GTK4 presentation for Darktable's deprecated Vibrance operation.
//!
//! This maps `src/iop/vibrance.c::gui_init`, `gui_update`, module metadata, and
//! deprecation text without exposing controls absent from the native module.

use crate::presentation::SourceMappedSliderSpec;

/// Stable Darktable operation name used by history, styles, and module order.
pub const VIBRANCE_MODULE_ID: &str = "vibrance";

/// Exact module-level presentation retained from `src/iop/vibrance.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VibranceSourceMap {
    title: &'static str,
    aliases: &'static [&'static str],
    group_keys: &'static [&'static str],
    default_enabled: bool,
    default_expanded: bool,
    deprecated_message: &'static str,
}

impl VibranceSourceMap {
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

    #[must_use]
    pub const fn deprecated_message(self) -> &'static str {
        self.deprecated_message
    }

    #[must_use]
    pub fn slider(self, parameter_id: &str) -> Option<VibranceSliderSourceMap> {
        (parameter_id == "amount").then_some(VIBRANCE_AMOUNT)
    }
}

/// Exact source presentation for the native `amount` slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VibranceSliderSourceMap {
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

impl VibranceSliderSourceMap {
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

/// Source module metadata. Deprecated availability is supplied by the
/// processing descriptor; this retains the native user-facing explanation.
pub const VIBRANCE_SOURCE_MAP: VibranceSourceMap = VibranceSourceMap {
    title: "vibrance",
    aliases: &["saturation"],
    group_keys: &["group.color", "group.grading"],
    default_enabled: false,
    default_expanded: false,
    deprecated_message: "this module is deprecated. please use the vibrance slider in the color balance rgb module instead.",
};

/// Source `amount` slider metadata.
pub const VIBRANCE_AMOUNT: VibranceSliderSourceMap = VibranceSliderSourceMap {
    parameter_id: "amount",
    label: "vibrance",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 25.0,
    digits: 2,
    step: 1.0,
    automatic_step: true,
    suffix: "%",
    tooltip: "the amount of vibrance",
};

#[cfg(test)]
mod tests {
    use super::{VIBRANCE_AMOUNT, VIBRANCE_SOURCE_MAP};

    #[test]
    fn source_map_preserves_native_module_metadata_and_deprecation() {
        assert_eq!(VIBRANCE_SOURCE_MAP.title(), "vibrance");
        assert_eq!(VIBRANCE_SOURCE_MAP.aliases(), ["saturation"]);
        assert_eq!(
            VIBRANCE_SOURCE_MAP.group_keys(),
            ["group.color", "group.grading"]
        );
        assert!(!VIBRANCE_SOURCE_MAP.default_enabled());
        assert!(!VIBRANCE_SOURCE_MAP.default_expanded());
        assert_eq!(
            VIBRANCE_SOURCE_MAP.deprecated_message(),
            "this module is deprecated. please use the vibrance slider in the color balance rgb module instead."
        );
    }

    #[test]
    fn source_map_preserves_the_only_native_gui_slider() {
        assert_eq!(VIBRANCE_AMOUNT.parameter_id(), "amount");
        assert_eq!(VIBRANCE_AMOUNT.label(), "vibrance");
        assert_eq!(VIBRANCE_AMOUNT.range(), (0.0, 100.0));
        assert_float_eq(VIBRANCE_AMOUNT.default_value(), 25.0);
        assert_eq!(VIBRANCE_AMOUNT.digits(), 2);
        assert_float_eq(VIBRANCE_AMOUNT.step(), 1.0);
        assert!(VIBRANCE_AMOUNT.automatic_step());
        assert_eq!(VIBRANCE_AMOUNT.suffix(), "%");
        assert_eq!(VIBRANCE_AMOUNT.tooltip(), "the amount of vibrance");
        assert_eq!(VIBRANCE_SOURCE_MAP.slider("amount"), Some(VIBRANCE_AMOUNT));
        assert_eq!(VIBRANCE_SOURCE_MAP.slider("strength"), None);
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() <= f64::EPSILON);
    }
}
