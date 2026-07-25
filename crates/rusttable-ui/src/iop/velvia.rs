//! Source-mapped presentation contract for Darktable's Velvia operation.

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::gui::darktable_components::module_title;
use crate::presentation::SourceMappedSliderSpec;

/// Stable Darktable operation name used by history, styles, and module order.
pub const VELVIA_MODULE_ID: &str = "velvia";

const VELVIA_ICON_SVG: &[u8] =
    include_bytes!("../../../../data/pixmaps/plugins/darkroom/velvia.svg");

/// Exact module-level presentation retained from `src/iop/velvia.c` and its
/// module-group/icon callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VelviaSourceMap {
    title: &'static str,
    aliases: &'static [&'static str],
    group_keys: &'static [&'static str],
    default_enabled: bool,
    default_expanded: bool,
    icon_asset: &'static str,
}

impl VelviaSourceMap {
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
    pub const fn icon_asset(self) -> &'static str {
        self.icon_asset
    }

    #[must_use]
    pub fn slider(self, parameter_id: &str) -> Option<VelviaSliderSourceMap> {
        match parameter_id {
            "strength" => Some(VELVIA_STRENGTH),
            "bias" => Some(VELVIA_BIAS),
            _ => None,
        }
    }
}

/// Exact source presentation for one Velvia scalar parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelviaSliderSourceMap {
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

impl VelviaSliderSourceMap {
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
pub const VELVIA_SOURCE_MAP: VelviaSourceMap = VelviaSourceMap {
    title: "velvia",
    aliases: &["saturation"],
    group_keys: &["group.color", "group.grading"],
    default_enabled: false,
    default_expanded: false,
    icon_asset: "data/pixmaps/plugins/darkroom/velvia.svg",
};

/// Source `strength` slider metadata.
pub const VELVIA_STRENGTH: VelviaSliderSourceMap = VelviaSliderSourceMap {
    parameter_id: "strength",
    label: "strength",
    minimum: 0.0,
    maximum: 100.0,
    default_value: 25.0,
    digits: 2,
    step: 1.0,
    automatic_step: true,
    suffix: "%",
    tooltip: "the strength of saturation boost",
};

/// Source `bias` slider metadata.
pub const VELVIA_BIAS: VelviaSliderSourceMap = VelviaSliderSourceMap {
    parameter_id: "bias",
    label: "mid-tones bias",
    minimum: 0.0,
    maximum: 1.0,
    default_value: 1.0,
    digits: 2,
    step: 0.01,
    automatic_step: true,
    suffix: "",
    tooltip: "how much to spare highlights and shadows",
};

pub(crate) fn module_title_widget() -> gtk4::Box {
    let title = module_title(VELVIA_MODULE_ID, VELVIA_SOURCE_MAP.title());
    let image = gtk4::Image::new();
    image.set_widget_name("velvia-icon");
    image.set_pixel_size(14);
    image.set_size_request(14, 14);
    image.set_focusable(false);
    image.set_can_target(false);
    image.set_accessible_role(gtk4::AccessibleRole::Img);
    image.update_property(&[Property::Label("velvia processing module")]);
    let bytes = gtk4::glib::Bytes::from_static(VELVIA_ICON_SVG);
    if let Ok(texture) = gtk4::gdk::Texture::from_bytes(&bytes) {
        image.set_paintable(Some(&texture));
    }
    title.prepend(&image);
    title
}

#[cfg(test)]
mod tests {
    use super::{VELVIA_BIAS, VELVIA_SOURCE_MAP, VELVIA_STRENGTH};

    #[test]
    fn source_map_preserves_darktable_module_metadata() {
        assert_eq!(VELVIA_SOURCE_MAP.title(), "velvia");
        assert_eq!(VELVIA_SOURCE_MAP.aliases(), ["saturation"]);
        assert_eq!(
            VELVIA_SOURCE_MAP.group_keys(),
            ["group.color", "group.grading"]
        );
        assert!(!VELVIA_SOURCE_MAP.default_enabled());
        assert!(!VELVIA_SOURCE_MAP.default_expanded());
        assert_eq!(
            VELVIA_SOURCE_MAP.icon_asset(),
            "data/pixmaps/plugins/darkroom/velvia.svg"
        );
    }

    #[test]
    fn source_map_preserves_darktable_slider_contracts() {
        assert_eq!(VELVIA_STRENGTH.parameter_id(), "strength");
        assert_eq!(VELVIA_STRENGTH.label(), "strength");
        assert_eq!(VELVIA_STRENGTH.range(), (0.0, 100.0));
        assert_float_eq(VELVIA_STRENGTH.default_value(), 25.0);
        assert_eq!(VELVIA_STRENGTH.digits(), 2);
        assert_float_eq(VELVIA_STRENGTH.step(), 1.0);
        assert!(VELVIA_STRENGTH.automatic_step());
        assert_eq!(VELVIA_STRENGTH.suffix(), "%");
        assert_eq!(
            VELVIA_STRENGTH.tooltip(),
            "the strength of saturation boost"
        );

        assert_eq!(VELVIA_BIAS.parameter_id(), "bias");
        assert_eq!(VELVIA_BIAS.label(), "mid-tones bias");
        assert_eq!(VELVIA_BIAS.range(), (0.0, 1.0));
        assert_float_eq(VELVIA_BIAS.default_value(), 1.0);
        assert_eq!(VELVIA_BIAS.digits(), 2);
        assert_float_eq(VELVIA_BIAS.step(), 0.01);
        assert!(VELVIA_BIAS.automatic_step());
        assert_eq!(VELVIA_BIAS.suffix(), "");
        assert_eq!(
            VELVIA_BIAS.tooltip(),
            "how much to spare highlights and shadows"
        );
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert!((actual - expected).abs() <= f64::EPSILON);
    }
}
