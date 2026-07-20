//! Pure GTK4 visual specifications for the Darktable desktop composition.
//!
//! This module is a product-facing visual contract, not a generic theme
//! abstraction.  Its structure follows Darktable's desktop responsibilities:
//! the header, view toolbars, side-module panels, central image workspace, and
//! bottom filmstrip.  GTK widgets consume these values in the runtime layer;
//! this file intentionally has no GTK dependency so the contract stays easy to
//! test without a display server.
//!
//! The pinned Darktable checkout remains the behavioral and visual reference;
//! all implementation and palette values in this module are Rust-owned.

#![forbid(unsafe_code)]

/// The two primary modes exposed by Darktable's view switcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Collection browser and thumbnail workspace.
    #[default]
    Lighttable,
    /// Single-image processing workspace.
    Darkroom,
}

impl ViewMode {
    /// The lowercase label used by Darktable's view switcher and stack names.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lighttable => "lighttable",
            Self::Darkroom => "darkroom",
        }
    }

    /// The GTK stack child name for this workspace.
    #[must_use]
    pub const fn stack_name(self) -> &'static str {
        self.label()
    }
}

/// Persistent top-level regions in the Darktable desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopRegion {
    /// Application header and global view controls.
    Header,
    /// Collection and navigation modules.
    LeftPanel,
    /// The active lighttable or darkroom canvas.
    CenterWorkspace,
    /// Processing, metadata, and module controls.
    RightPanel,
    /// Thumbnail navigation for the active collection or edited image.
    BottomFilmstrip,
}

impl DesktopRegion {
    /// Stable GTK widget name for this region.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::LeftPanel => "left-panel",
            Self::CenterWorkspace => "center-workspace",
            Self::RightPanel => "right-panel",
            Self::BottomFilmstrip => "bottom-filmstrip",
        }
    }
}

/// The three alignment slots in Darktable's application header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopBarSection {
    /// Darktable branding and global actions.
    HeaderLeft,
    /// Expandable collection/status content.
    HeaderCenter,
    /// View switcher and view-specific actions.
    HeaderRight,
}

impl TopBarSection {
    /// Stable GTK widget name for this header slot.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::HeaderLeft => "header-left",
            Self::HeaderCenter => "header-center",
            Self::HeaderRight => "header-right",
        }
    }
}

/// A Darktable panel role, including the persistent bottom filmstrip panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRole {
    /// Collection and filtering modules.
    Left,
    /// Image operations and metadata modules.
    Right,
    /// Thumbnail strip and its navigation controls.
    BottomFilmstrip,
}

impl PanelRole {
    /// Stable GTK widget name for this panel role.
    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Left => "left-panel",
            Self::Right => "right-panel",
            Self::BottomFilmstrip => "bottom-filmstrip",
        }
    }

    /// Whether this role is one of the resizable side panels.
    #[must_use]
    pub const fn is_side_panel(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// The six panel slots in Darktable's GTK shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelSlot {
    /// Global header panel.
    Top,
    /// Toolbar immediately above the central workspace.
    CenterTop,
    /// Toolbar immediately below the central workspace.
    CenterBottom,
    /// Left collection/module panel.
    Left,
    /// Right processing/module panel.
    Right,
    /// Bottom filmstrip panel.
    Bottom,
}

impl PanelSlot {
    /// Stable Darktable panel configuration name.
    #[must_use]
    pub const fn configuration_name(self) -> &'static str {
        match self {
            Self::Top => "header",
            Self::CenterTop => "toolbar_top",
            Self::CenterBottom => "toolbar_bottom",
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
        }
    }

    /// The visual region occupied by this panel slot.
    #[must_use]
    pub const fn region(self) -> DesktopRegion {
        match self {
            Self::Top => DesktopRegion::Header,
            Self::CenterTop | Self::CenterBottom => DesktopRegion::CenterWorkspace,
            Self::Left => DesktopRegion::LeftPanel,
            Self::Right => DesktopRegion::RightPanel,
            Self::Bottom => DesktopRegion::BottomFilmstrip,
        }
    }
}

/// An integer number of hundredths of an `em`, retaining the source CSS unit
/// without introducing floating-point layout drift into the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EmHundredths(u16);

impl EmHundredths {
    /// Creates an `em` value represented in hundredths.
    #[must_use]
    pub const fn new(hundredths: u16) -> Self {
        Self(hundredths)
    }

    /// Returns the source CSS value in hundredths of an `em`.
    #[must_use]
    pub const fn hundredths(self) -> u16 {
        self.0
    }
}

/// Side-panel width constraints from Darktable's configuration contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidePanelWidths {
    /// Minimum width at which a side module remains usable.
    pub minimum_px: u16,
    /// Preferred width for the initial GTK layout.
    pub preferred_px: u16,
    /// Maximum width accepted while dragging a side-panel handle.
    pub maximum_px: u16,
}

impl SidePanelWidths {
    /// Returns whether a width is within the Darktable side-panel range.
    #[must_use]
    pub const fn accepts(self, width_px: u16) -> bool {
        width_px >= self.minimum_px && width_px <= self.maximum_px
    }
}

/// Bottom filmstrip height constraints from Darktable's GTK configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilmstripHeights {
    /// Minimum height while resizing the filmstrip.
    pub minimum_px: u16,
    /// Initial height used when no saved size exists.
    pub preferred_px: u16,
    /// Maximum height while resizing the filmstrip.
    pub maximum_px: u16,
}

impl FilmstripHeights {
    /// Returns whether a height is within the Darktable filmstrip range.
    #[must_use]
    pub const fn accepts(self, height_px: u16) -> bool {
        height_px >= self.minimum_px && height_px <= self.maximum_px
    }
}

/// Fixed desktop spacing and sizing values that are visible in the GTK shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutMetrics {
    /// Default main-window width from Darktable's GTK initialization.
    pub window_width_px: u16,
    /// Default main-window height from Darktable's GTK initialization.
    pub window_height_px: u16,
    /// Width of the outer collapse-border controls.
    pub outer_border_px: u8,
    /// Gap between adjacent modules in a side panel.
    pub panel_module_spacing_px: u8,
    /// Header and footer toolbar vertical padding in hundredths of an `em`.
    pub toolbar_padding_vertical: EmHundredths,
    /// Header and footer toolbar horizontal padding in hundredths of an `em`.
    pub toolbar_padding_horizontal: EmHundredths,
    /// Minimum size of a header/footer module button in hundredths of an `em`.
    pub toolbar_button_minimum: EmHundredths,
    /// Minimum center-column width from Darktable's layout constraints.
    pub center_minimum_width_px: u16,
    /// Resizable side-panel widths.
    pub side_panel_widths: SidePanelWidths,
    /// Resizable filmstrip heights.
    pub filmstrip_heights: FilmstripHeights,
}

/// An opaque sRGB color token from Darktable's default theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorToken {
    css_name: &'static str,
    rgba: [u8; 4],
}

impl ColorToken {
    /// Creates a named opaque or translucent sRGB token.
    #[must_use]
    pub const fn new(css_name: &'static str, rgba: [u8; 4]) -> Self {
        Self { css_name, rgba }
    }

    /// The source Darktable CSS token name.
    #[must_use]
    pub const fn css_name(self) -> &'static str {
        self.css_name
    }

    /// The token as red, green, blue, alpha bytes.
    #[must_use]
    pub const fn rgba(self) -> [u8; 4] {
        self.rgba
    }
}

/// Darktable's default-theme colors needed by the desktop shell and views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarktableColors {
    /// General panel background (`bg_color`).
    pub background: ColorToken,
    /// General text (`fg_color`).
    pub foreground: ColorToken,
    /// Panel border (`border_color`).
    pub border: ColorToken,
    /// Module background (`plugin_bg_color`).
    pub module_background: ColorToken,
    /// Button background (`button_bg`).
    pub button_background: ColorToken,
    /// Active field background (`field_active_bg`).
    pub active_field_background: ColorToken,
    /// Lighttable canvas (`lighttable_bg_color`).
    pub lighttable_canvas: ColorToken,
    /// Darkroom canvas (`darkroom_bg_color`).
    pub darkroom_canvas: ColorToken,
    /// Thumbnail tile background (`thumbnail_bg_color`).
    pub thumbnail_background: ColorToken,
    /// Filmstrip background (`filmstrip_bg_color`).
    pub filmstrip_background: ColorToken,
    /// Selected thumbnail background (`thumbnail_selected_bg_color`).
    pub selected_thumbnail: ColorToken,
    /// Hovered thumbnail background (`thumbnail_hover_bg_color`).
    pub hovered_thumbnail: ColorToken,
    /// Active-image marker used by thumbnail borders.
    pub active_image_marker: ColorToken,
}

/// Ordered regions of the Darktable desktop grid.
pub const DESKTOP_REGIONS: [DesktopRegion; 5] = [
    DesktopRegion::Header,
    DesktopRegion::LeftPanel,
    DesktopRegion::CenterWorkspace,
    DesktopRegion::RightPanel,
    DesktopRegion::BottomFilmstrip,
];

/// Ordered header slots from the Darktable GTK UI container contract.
pub const TOP_BAR_SECTIONS: [TopBarSection; 3] = [
    TopBarSection::HeaderLeft,
    TopBarSection::HeaderCenter,
    TopBarSection::HeaderRight,
];

/// Ordered panel slots from `dt_ui_panel_t`.
pub const PANEL_SLOTS: [PanelSlot; 6] = [
    PanelSlot::Top,
    PanelSlot::CenterTop,
    PanelSlot::CenterBottom,
    PanelSlot::Left,
    PanelSlot::Right,
    PanelSlot::Bottom,
];

/// Darktable's fixed desktop metrics and resize constraints.
pub const LAYOUT_METRICS: LayoutMetrics = LayoutMetrics {
    window_width_px: 1224,
    window_height_px: 768,
    outer_border_px: 10,
    panel_module_spacing_px: 0,
    toolbar_padding_vertical: EmHundredths::new(14),
    toolbar_padding_horizontal: EmHundredths::new(28),
    toolbar_button_minimum: EmHundredths::new(170),
    center_minimum_width_px: 650,
    side_panel_widths: SidePanelWidths {
        minimum_px: 150,
        preferred_px: 154,
        maximum_px: 1_500,
    },
    filmstrip_heights: FilmstripHeights {
        minimum_px: 64,
        preferred_px: 104,
        maximum_px: 400,
    },
};

/// Default Darktable palette values used by the GTK shell.
pub const DARKTABLE_COLORS: DarktableColors = DarktableColors {
    background: ColorToken::new("bg_color", [0x26, 0x26, 0x26, 0xff]),
    foreground: ColorToken::new("fg_color", [0xb9, 0xb9, 0xb9, 0xff]),
    border: ColorToken::new("border_color", [0x1b, 0x1b, 0x1b, 0xff]),
    module_background: ColorToken::new("plugin_bg_color", [0x30, 0x30, 0x30, 0xff]),
    button_background: ColorToken::new("button_bg", [0x3b, 0x3b, 0x3b, 0xff]),
    active_field_background: ColorToken::new("field_active_bg", [0x3b, 0x3b, 0x3b, 0xff]),
    lighttable_canvas: ColorToken::new("lighttable_bg_color", [0x5e, 0x5e, 0x5e, 0xff]),
    darkroom_canvas: ColorToken::new("darkroom_bg_color", [0x77, 0x77, 0x77, 0xff]),
    thumbnail_background: ColorToken::new("thumbnail_bg_color", [0x77, 0x77, 0x77, 0xff]),
    filmstrip_background: ColorToken::new("filmstrip_bg_color", [0x5e, 0x5e, 0x5e, 0xff]),
    selected_thumbnail: ColorToken::new("thumbnail_selected_bg_color", [0xab, 0xab, 0xab, 0xff]),
    hovered_thumbnail: ColorToken::new("thumbnail_hover_bg_color", [0xd4, 0xd4, 0xd4, 0xff]),
    active_image_marker: ColorToken::new("active_image_marker", [0xff, 0xbb, 0x00, 0xff]),
};

/// The complete display-free contract consumed by the GTK4 runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarktableDesktopSpec {
    /// Ordered desktop regions.
    pub regions: &'static [DesktopRegion; 5],
    /// Ordered header sections.
    pub top_bar_sections: &'static [TopBarSection; 3],
    /// Ordered Darktable panel slots.
    pub panel_slots: &'static [PanelSlot; 6],
    /// Fixed dimensions and resize constraints.
    pub layout: LayoutMetrics,
    /// Default Darktable color tokens.
    pub colors: DarktableColors,
}

/// The single visual specification for the `RustTable` GTK4 desktop.
pub const DARKTABLE_DESKTOP_SPEC: DarktableDesktopSpec = DarktableDesktopSpec {
    regions: &DESKTOP_REGIONS,
    top_bar_sections: &TOP_BAR_SECTIONS,
    panel_slots: &PANEL_SLOTS,
    layout: LAYOUT_METRICS,
    colors: DARKTABLE_COLORS,
};

#[cfg(test)]
mod tests {
    use super::{
        ColorToken, DARKTABLE_COLORS, DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, DesktopRegion,
        LAYOUT_METRICS, PANEL_SLOTS, PanelRole, PanelSlot, TOP_BAR_SECTIONS, ViewMode,
    };

    #[test]
    fn modes_match_darktable_view_names_and_order() {
        assert_eq!(ViewMode::Lighttable.label(), "lighttable");
        assert_eq!(ViewMode::Darkroom.label(), "darkroom");
        assert_eq!(ViewMode::Lighttable.stack_name(), "lighttable");
        assert_eq!(ViewMode::Darkroom.stack_name(), "darkroom");
    }

    #[test]
    fn desktop_regions_keep_the_center_between_side_panels() {
        assert_eq!(
            DESKTOP_REGIONS,
            [
                DesktopRegion::Header,
                DesktopRegion::LeftPanel,
                DesktopRegion::CenterWorkspace,
                DesktopRegion::RightPanel,
                DesktopRegion::BottomFilmstrip,
            ]
        );
        assert_eq!(
            DesktopRegion::CenterWorkspace.identifier(),
            "center-workspace"
        );
    }

    #[test]
    fn header_and_panel_slots_match_the_gtk_container_contract() {
        assert_eq!(TOP_BAR_SECTIONS[0].identifier(), "header-left");
        assert_eq!(TOP_BAR_SECTIONS[1].identifier(), "header-center");
        assert_eq!(TOP_BAR_SECTIONS[2].identifier(), "header-right");
        assert_eq!(PANEL_SLOTS[0].configuration_name(), "header");
        assert_eq!(PANEL_SLOTS[1].configuration_name(), "toolbar_top");
        assert_eq!(PANEL_SLOTS[2].configuration_name(), "toolbar_bottom");
        assert_eq!(PANEL_SLOTS[5].configuration_name(), "bottom");
        assert_eq!(
            PanelSlot::CenterTop.region(),
            DesktopRegion::CenterWorkspace
        );
    }

    #[test]
    fn panel_metrics_preserve_darktable_resize_bounds() {
        assert_eq!(LAYOUT_METRICS.outer_border_px, 10);
        assert_eq!(LAYOUT_METRICS.panel_module_spacing_px, 0);
        assert_eq!(LAYOUT_METRICS.center_minimum_width_px, 650);
        assert_eq!(LAYOUT_METRICS.side_panel_widths.preferred_px, 154);
        assert!(LAYOUT_METRICS.side_panel_widths.accepts(150));
        assert!(LAYOUT_METRICS.side_panel_widths.accepts(1_500));
        assert!(!LAYOUT_METRICS.side_panel_widths.accepts(149));
        assert_eq!(LAYOUT_METRICS.filmstrip_heights.preferred_px, 104);
        assert!(LAYOUT_METRICS.filmstrip_heights.accepts(64));
        assert!(LAYOUT_METRICS.filmstrip_heights.accepts(400));
    }

    #[test]
    fn darktable_colors_preserve_the_default_css_tokens() {
        assert_eq!(DARKTABLE_COLORS.background.css_name(), "bg_color");
        assert_eq!(DARKTABLE_COLORS.background.rgba(), [0x26, 0x26, 0x26, 0xff]);
        assert_eq!(
            DARKTABLE_COLORS.lighttable_canvas.rgba(),
            [0x5e, 0x5e, 0x5e, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.darkroom_canvas.rgba(),
            [0x77, 0x77, 0x77, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.thumbnail_background.rgba(),
            [0x77, 0x77, 0x77, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.active_image_marker.rgba(),
            [0xff, 0xbb, 0x00, 0xff]
        );
    }

    #[test]
    fn top_level_spec_is_self_consistent() {
        assert_eq!(DARKTABLE_DESKTOP_SPEC.regions, &DESKTOP_REGIONS);
        assert_eq!(DARKTABLE_DESKTOP_SPEC.top_bar_sections, &TOP_BAR_SECTIONS);
        assert_eq!(DARKTABLE_DESKTOP_SPEC.panel_slots, &PANEL_SLOTS);
        assert_eq!(DARKTABLE_DESKTOP_SPEC.layout, LAYOUT_METRICS);
        assert_eq!(DARKTABLE_DESKTOP_SPEC.colors, DARKTABLE_COLORS);
    }

    #[test]
    fn panel_roles_distinguish_side_panels_from_filmstrip() {
        assert!(PanelRole::Left.is_side_panel());
        assert!(PanelRole::Right.is_side_panel());
        assert!(!PanelRole::BottomFilmstrip.is_side_panel());
        assert_eq!(PanelRole::BottomFilmstrip.identifier(), "bottom-filmstrip");
    }

    #[test]
    fn color_tokens_are_copyable_value_objects() {
        const TOKEN: ColorToken = DARKTABLE_COLORS.foreground;
        assert_eq!(TOKEN.css_name(), "fg_color");
        assert_eq!(TOKEN.rgba()[3], 0xff);
    }
}
