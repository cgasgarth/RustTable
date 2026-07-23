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

mod scale;
pub use scale::*;

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

/// Initial left/right rail allocations for one workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspacePanelWidths {
    pub left_px: u16,
    pub right_px: u16,
}

/// The captured Darktable layouts retain narrower collection rails and wider
/// editing rails while sharing the same resize bounds and panel components.
pub const LIGHTTABLE_PANEL_WIDTHS: WorkspacePanelWidths = WorkspacePanelWidths {
    left_px: 140,
    right_px: 164,
};
pub const DARKROOM_PANEL_WIDTHS: WorkspacePanelWidths = WorkspacePanelWidths {
    left_px: 180,
    right_px: 180,
};

#[must_use]
pub const fn workspace_panel_widths(mode: ViewMode) -> WorkspacePanelWidths {
    match mode {
        ViewMode::Lighttable => LIGHTTABLE_PANEL_WIDTHS,
        ViewMode::Darkroom => DARKROOM_PANEL_WIDTHS,
    }
}

impl SidePanelWidths {
    /// Returns whether a width is within the Darktable side-panel range.
    #[must_use]
    pub const fn accepts(self, width_px: u16) -> bool {
        width_px >= self.minimum_px && width_px <= self.maximum_px
    }
}

/// The stable lighttable toolbar contract.
///
/// Darktable presents the collection/filter controls as one center-top row;
/// the lighttable layout controls do not get a second row above it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LighttableToolbarSpec {
    /// Stable GTK widget name for the single lighttable top toolbar.
    pub widget_name: &'static str,
    /// Stable GTK widget name for the collection filter entry.
    pub filter_entry_name: &'static str,
    /// Number of top toolbar rows visible in lighttable.
    pub row_count: u8,
}

/// The single top toolbar retained by the `RustTable` lighttable.
pub const LIGHTTABLE_TOOLBAR: LighttableToolbarSpec = LighttableToolbarSpec {
    widget_name: "lighttable-collection-toolbar",
    filter_entry_name: "lighttable-filter-entry",
    row_count: 1,
};

/// One disclosure module in Darktable's lighttable right-center rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LighttableModuleSpec {
    /// Stable GTK widget name used by controller and smoke-test lookup.
    pub widget_name: &'static str,
    /// Visible module title from the Darktable lighttable.
    pub title: &'static str,
}

/// Direct lighttable rail order from Darktable's right-center container.
///
/// `modulegroups.c` is intentionally absent: Darktable exposes that selector
/// only in darkroom. Export is listed once because its GTK controller owns its
/// own disclosure widget.
pub const LIGHTTABLE_RIGHT_MODULES: [LighttableModuleSpec; 8] = [
    LighttableModuleSpec {
        widget_name: "selection",
        title: "selection",
    },
    LighttableModuleSpec {
        widget_name: "actions-on-selection",
        title: "actions on selection",
    },
    LighttableModuleSpec {
        widget_name: "history-stack",
        title: "history stack",
    },
    LighttableModuleSpec {
        widget_name: "styles",
        title: "styles",
    },
    LighttableModuleSpec {
        widget_name: "metadata-editor",
        title: "metadata editor",
    },
    LighttableModuleSpec {
        widget_name: "tagging",
        title: "tagging",
    },
    LighttableModuleSpec {
        widget_name: "geotagging",
        title: "geotagging",
    },
    LighttableModuleSpec {
        widget_name: "export",
        title: "export",
    },
];

/// Row and column counts that prevent duplicate lighttable chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LighttableCompositionSpec {
    /// Collection toolbar rows above the canvas.
    pub top_toolbar_rows: u8,
    /// Footer toolbar rows between canvas and filmstrip.
    pub footer_toolbar_rows: u8,
    /// Toolbars embedded inside the filmstrip itself.
    pub filmstrip_toolbar_rows: u8,
    /// Columns in Darktable's empty-collection guidance.
    pub empty_state_columns: u8,
    /// Fixed height of the shared collection chrome in both workspaces.
    pub top_toolbar_height_px: u8,
}

/// Darktable lighttable composition rendered by the GTK shell.
pub const LIGHTTABLE_COMPOSITION: LighttableCompositionSpec = LighttableCompositionSpec {
    top_toolbar_rows: 1,
    footer_toolbar_rows: 1,
    filmstrip_toolbar_rows: 0,
    empty_state_columns: 2,
    top_toolbar_height_px: 24,
};

/// Geometry of the Darktable darkroom center column and its adjacent rails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomGeometry {
    /// Fixed top proofing toolbar height.
    pub top_toolbar_height_px: u8,
    /// Fixed bottom viewport toolbar height.
    pub bottom_toolbar_height_px: u8,
    /// Minimum viewport height before the filmstrip is allowed to compress.
    pub viewport_minimum_height_px: u16,
    /// Initial scopes graph height in the right rail.
    pub histogram_height_px: u16,
    /// Minimum scopes graph height at the preferred narrow rail width.
    pub histogram_min_height_px: u16,
    /// Separator between the center column and the filmstrip.
    pub filmstrip_separator_px: u8,
    /// Height of the status and background-job row below the viewport toolbar.
    pub status_bar_height_px: u8,
    /// Darktable's configured inset around the processed image.
    pub image_border_px: u8,
}

/// The two deterministic side-rail modes used by the GTK shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomWindowLayout {
    /// The preferred editing rails with the full center minimum.
    Normal,
    /// The same two rails retained while the center viewport accepts a compact minimum.
    Narrow,
}

impl DarkroomWindowLayout {
    #[must_use]
    pub const fn center_minimum_width_px(self) -> u16 {
        match self {
            Self::Normal => 650,
            Self::Narrow => 320,
        }
    }
}

/// Returns the geometry mode without collapsing either darkroom rail.
#[must_use]
pub const fn darkroom_window_layout(window_width_px: u16) -> DarkroomWindowLayout {
    if LAYOUT_METRICS.preferred_center_width_px(window_width_px)
        >= DarkroomWindowLayout::Normal.center_minimum_width_px()
    {
        DarkroomWindowLayout::Normal
    } else {
        DarkroomWindowLayout::Narrow
    }
}

/// Stable scrolling boundaries for the two darkroom module rails.
pub const DARKROOM_RAIL_SCROLL_WIDGET_IDS: [&str; 2] = [
    "darkroom-left-module-scroll",
    "darkroom-right-module-scroll",
];

/// Operation controls follow module disclosure, status, and typed control order.
pub const DARKROOM_OPERATION_FOCUS_ORDER: [&str; 5] = [
    "module-disclosure",
    "module-enabled",
    "module-presets",
    "module-reset",
    "module-control",
];

pub const DARKROOM_GEOMETRY: DarkroomGeometry = DarkroomGeometry {
    top_toolbar_height_px: 18,
    bottom_toolbar_height_px: 18,
    viewport_minimum_height_px: 200,
    histogram_height_px: 180,
    histogram_min_height_px: 120,
    filmstrip_separator_px: 1,
    status_bar_height_px: 18,
    image_border_px: 10,
};

/// A display-free allocation receipt for the darkroom's named regions.
///
/// The receipt is intentionally integer-only so geometry tests do not depend on a
/// display server, font rasterization, or GTK's allocation timing.  GTK uses the
/// same preferred widths and minimums when it realizes the Paned hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomGeometryReceipt {
    pub window_width_px: u16,
    pub window_height_px: u16,
    pub left_panel_width_px: u16,
    pub center_width_px: u16,
    pub right_panel_width_px: u16,
    pub top_toolbar_height_px: u8,
    pub viewport_minimum_height_px: u16,
    pub bottom_toolbar_height_px: u8,
    pub status_bar_height_px: u8,
    pub filmstrip_height_px: u16,
    pub left_panel_visible: bool,
    pub right_panel_visible: bool,
    pub filmstrip_visible: bool,
}

impl DarkroomGeometryReceipt {
    /// Returns the deterministic darkroom allocation receipt for a desktop size.
    #[must_use]
    pub const fn for_window(
        window_width_px: u16,
        window_height_px: u16,
        left_panel_visible: bool,
        right_panel_visible: bool,
        filmstrip_visible: bool,
    ) -> Self {
        let left_panel_width = if left_panel_visible {
            DARKROOM_PANEL_WIDTHS.left_px
        } else {
            0
        };
        let right_panel_width = if right_panel_visible {
            DARKROOM_PANEL_WIDTHS.right_px
        } else {
            0
        };
        Self::for_window_with_panel_widths(
            window_width_px,
            window_height_px,
            left_panel_width,
            right_panel_width,
            filmstrip_visible,
        )
    }

    /// Returns the same receipt for a live Paned allocation. Keeping the
    /// calculation display-free makes rail-drag behavior deterministic and
    /// gives the GTK refresh callback one contract for viewport and histogram
    /// recomputation.
    #[must_use]
    pub const fn for_window_with_panel_widths(
        window_width_px: u16,
        window_height_px: u16,
        left_panel_width_px: u16,
        right_panel_width_px: u16,
        filmstrip_visible: bool,
    ) -> Self {
        let content_width = LAYOUT_METRICS.content_width_px(window_width_px);
        let filmstrip_height = if filmstrip_visible {
            LAYOUT_METRICS.filmstrip_heights.preferred_px
        } else {
            0
        };
        Self {
            window_width_px,
            window_height_px,
            left_panel_width_px,
            center_width_px: content_width
                .saturating_sub(left_panel_width_px)
                .saturating_sub(right_panel_width_px),
            right_panel_width_px,
            top_toolbar_height_px: DARKROOM_GEOMETRY.top_toolbar_height_px,
            viewport_minimum_height_px: DARKROOM_GEOMETRY.viewport_minimum_height_px,
            bottom_toolbar_height_px: DARKROOM_GEOMETRY.bottom_toolbar_height_px,
            status_bar_height_px: DARKROOM_GEOMETRY.status_bar_height_px,
            filmstrip_height_px: filmstrip_height,
            left_panel_visible: left_panel_width_px > 0,
            right_panel_visible: right_panel_width_px > 0,
            filmstrip_visible,
        }
    }

    /// Returns the center width after a side-panel visibility change.
    #[must_use]
    pub const fn center_width_px(self) -> u16 {
        self.center_width_px
    }
}

/// Pixel bounds for the initial Darktable grid and filmstrip thumbnail surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailMetrics {
    pub grid_width_px: u16,
    pub grid_height_px: u16,
    pub filmstrip_width_px: u16,
    pub filmstrip_height_px: u16,
}

pub const THUMBNAIL_METRICS: ThumbnailMetrics = ThumbnailMetrics {
    grid_width_px: 196,
    grid_height_px: 147,
    filmstrip_width_px: 104,
    filmstrip_height_px: 78,
};

/// A filmstrip is one horizontally scrolling thumbtable row, not a wrapped
/// second gallery. `GtkFlowBox` uses this bound to keep all items on that row.
pub const FILMSTRIP_MAX_CHILDREN_PER_LINE: u32 = u32::MAX;

/// The compact gap between adjacent Darktable filmstrip thumbnails.
pub const FILMSTRIP_ITEM_GAP_PX: u8 = 4;

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
    /// Height of the persistent Darktable header chrome.
    pub header_height_px: u8,
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

impl LayoutMetrics {
    /// Width available to the panel grid after Darktable's outer rails.
    #[must_use]
    pub const fn content_width_px(self, window_width_px: u16) -> u16 {
        window_width_px.saturating_sub(self.outer_border_px as u16 * 2)
    }

    /// Center-column width at the preferred side-panel sizes.
    #[must_use]
    pub const fn preferred_center_width_px(self, window_width_px: u16) -> u16 {
        self.content_width_px(window_width_px)
            .saturating_sub(self.side_panel_widths.preferred_px * 2)
    }

    /// Position of the right rail divider within the outer-border-adjusted grid.
    #[must_use]
    pub const fn preferred_right_panel_position_px(self, window_width_px: u16) -> u16 {
        self.content_width_px(window_width_px)
            .saturating_sub(self.side_panel_widths.preferred_px)
    }

    /// Position of the right rail divider inside an already border-adjusted grid.
    #[must_use]
    pub const fn preferred_right_panel_position_for_content_width(
        self,
        content_width_px: u16,
    ) -> u16 {
        content_width_px.saturating_sub(self.side_panel_widths.preferred_px)
    }
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
    /// One-pixel button border (`button_border`).
    pub button_border: ColorToken,
    /// General button hover overlay (`alpha(button_hover_bg, 0.5)`).
    pub button_hover_overlay: ColorToken,
    /// Disabled button border (`alpha(button_border, 0.35)`).
    pub disabled_button_border: ColorToken,
    /// Disabled control foreground (`disabled_fg_color`).
    pub disabled_foreground: ColorToken,
    /// Active field background (`field_active_bg`).
    pub active_field_background: ColorToken,
    /// Utility and processing module title (`plugin_label_color`).
    pub module_label: ColorToken,
    /// In-module section title (`section_label`).
    pub section_label: ColorToken,
    /// Inactive scrollbar thumb (`scroll_bar_inactive`).
    pub scrollbar_inactive: ColorToken,
    /// Hovered scrollbar thumb (`scroll_bar_active`).
    pub scrollbar_active: ColorToken,
    /// Scrollbar trough (`scroll_bar_bg`).
    pub scrollbar_background: ColorToken,
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
    window_width_px: 1_280,
    window_height_px: 768,
    outer_border_px: 7,
    header_height_px: 30,
    panel_module_spacing_px: 0,
    toolbar_padding_vertical: EmHundredths::new(14),
    toolbar_padding_horizontal: EmHundredths::new(28),
    toolbar_button_minimum: EmHundredths::new(170),
    center_minimum_width_px: 650,
    side_panel_widths: SidePanelWidths {
        minimum_px: 136,
        preferred_px: 180,
        maximum_px: 1_500,
    },
    filmstrip_heights: FilmstripHeights {
        minimum_px: 64,
        preferred_px: 82,
        maximum_px: 400,
    },
};

/// Darktable elegant-grey palette used by the matched reference captures.
pub const DARKTABLE_COLORS: DarktableColors = DarktableColors {
    background: ColorToken::new("bg_color", [0x6a, 0x6a, 0x6a, 0xff]),
    foreground: ColorToken::new("fg_color", [0xf1, 0xf1, 0xf1, 0xff]),
    border: ColorToken::new("border_color", [0x5e, 0x5e, 0x5e, 0xff]),
    module_background: ColorToken::new("plugin_bg_color", [0x71, 0x71, 0x71, 0xff]),
    button_background: ColorToken::new("button_bg", [0x7d, 0x7d, 0x7d, 0xff]),
    button_border: ColorToken::new("button_border", [0x82, 0x82, 0x82, 0xff]),
    button_hover_overlay: ColorToken::new("button_hover_bg", [0xab, 0xab, 0xab, 0x80]),
    disabled_button_border: ColorToken::new("button_border_disabled", [0x82, 0x82, 0x82, 0x59]),
    disabled_foreground: ColorToken::new("disabled_fg_color", [0x9e, 0x9e, 0x9e, 0xff]),
    active_field_background: ColorToken::new("field_active_bg", [0x77, 0x77, 0x77, 0xff]),
    module_label: ColorToken::new("plugin_label_color", [0xc6, 0xc6, 0xc6, 0xff]),
    section_label: ColorToken::new("section_label", [0xde, 0xde, 0xde, 0xff]),
    scrollbar_inactive: ColorToken::new("scroll_bar_inactive", [0x91, 0x91, 0x91, 0xff]),
    scrollbar_active: ColorToken::new("scroll_bar_active", [0xc6, 0xc6, 0xc6, 0xff]),
    scrollbar_background: ColorToken::new("scroll_bar_bg", [0x5e, 0x5e, 0x5e, 0xff]),
    lighttable_canvas: ColorToken::new("lighttable_bg_color", [0x91, 0x91, 0x91, 0xff]),
    darkroom_canvas: ColorToken::new("darkroom_bg_color", [0x77, 0x77, 0x77, 0xff]),
    thumbnail_background: ColorToken::new("thumbnail_bg_color", [0xab, 0xab, 0xab, 0xff]),
    filmstrip_background: ColorToken::new("filmstrip_bg_color", [0x91, 0x91, 0x91, 0xff]),
    selected_thumbnail: ColorToken::new("thumbnail_selected_bg_color", [0xc6, 0xc6, 0xc6, 0xff]),
    hovered_thumbnail: ColorToken::new("thumbnail_hover_bg_color", [0xf1, 0xf1, 0xf1, 0xff]),
    active_image_marker: ColorToken::new("active_image_marker", [0xf1, 0xf1, 0xf1, 0xff]),
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
        ColorToken, DARKROOM_GEOMETRY, DARKROOM_RAIL_SCROLL_WIDGET_IDS, DARKTABLE_COLORS,
        DARKTABLE_DESKTOP_SPEC, DESKTOP_REGIONS, DarkroomGeometryReceipt, DarkroomWindowLayout,
        DesktopRegion, FILMSTRIP_ITEM_GAP_PX, FILMSTRIP_MAX_CHILDREN_PER_LINE, LAYOUT_METRICS,
        LIGHTTABLE_COMPOSITION, LIGHTTABLE_RIGHT_MODULES, LIGHTTABLE_TOOLBAR, PANEL_SLOTS,
        PanelRole, PanelSlot, THUMBNAIL_METRICS, TOP_BAR_SECTIONS, ViewMode,
        darkroom_window_layout,
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
    fn darkroom_geometry_keeps_viewport_between_toolbars_and_filmstrip() {
        const {
            assert!(DARKROOM_GEOMETRY.top_toolbar_height_px > 0);
            assert!(DARKROOM_GEOMETRY.bottom_toolbar_height_px > 0);
            assert!(DARKROOM_GEOMETRY.viewport_minimum_height_px >= 200);
            assert!(DARKROOM_GEOMETRY.histogram_height_px >= 92);
        }
        assert_eq!(DARKROOM_GEOMETRY.filmstrip_separator_px, 1);
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
        assert_eq!(LAYOUT_METRICS.outer_border_px, 7);
        assert_eq!(LAYOUT_METRICS.header_height_px, 30);
        assert_eq!(LAYOUT_METRICS.panel_module_spacing_px, 0);
        assert_eq!(LAYOUT_METRICS.center_minimum_width_px, 650);
        assert_eq!(LAYOUT_METRICS.side_panel_widths.minimum_px, 136);
        assert_eq!(LAYOUT_METRICS.side_panel_widths.preferred_px, 180);
        assert!(LAYOUT_METRICS.side_panel_widths.accepts(136));
        assert!(LAYOUT_METRICS.side_panel_widths.accepts(1_500));
        assert!(!LAYOUT_METRICS.side_panel_widths.accepts(135));
        assert_eq!(LAYOUT_METRICS.filmstrip_heights.preferred_px, 82);
        assert!(LAYOUT_METRICS.filmstrip_heights.accepts(64));
        assert!(LAYOUT_METRICS.filmstrip_heights.accepts(400));
    }

    #[test]
    fn baseline_rail_geometry_leaves_a_stable_center_column() {
        assert_eq!(LAYOUT_METRICS.window_width_px, 1_280);
        assert_eq!(LAYOUT_METRICS.window_height_px, 768);
        assert_eq!(LAYOUT_METRICS.content_width_px(1_224), 1_210);
        assert_eq!(LAYOUT_METRICS.preferred_center_width_px(1_224), 850);
        assert_eq!(
            LAYOUT_METRICS.preferred_right_panel_position_px(1_224),
            1_030
        );
        assert_eq!(
            LAYOUT_METRICS.preferred_right_panel_position_for_content_width(1_210),
            1_030
        );
        assert!(
            LAYOUT_METRICS.preferred_center_width_px(1_224)
                >= LAYOUT_METRICS.center_minimum_width_px
        );
    }

    #[test]
    fn darkroom_narrow_mode_keeps_both_rails_and_compacts_only_the_center() {
        assert_eq!(darkroom_window_layout(1_224), DarkroomWindowLayout::Normal);
        assert_eq!(darkroom_window_layout(900), DarkroomWindowLayout::Narrow);
        assert_eq!(DarkroomWindowLayout::Normal.center_minimum_width_px(), 650);
        assert_eq!(DarkroomWindowLayout::Narrow.center_minimum_width_px(), 320);
        assert_eq!(DARKROOM_RAIL_SCROLL_WIDGET_IDS.len(), 2);
    }

    #[test]
    fn darkroom_geometry_receipt_is_deterministic_and_visibility_aware() {
        let full = DarkroomGeometryReceipt::for_window(1_224, 768, true, true, true);
        assert_eq!(full.left_panel_width_px, 180);
        assert_eq!(full.center_width_px(), 850);
        assert_eq!(full.right_panel_width_px, 180);
        assert_eq!(full.filmstrip_height_px, 82);
        assert_eq!(full.status_bar_height_px, 18);

        let compact = DarkroomGeometryReceipt::for_window(900, 500, false, true, false);
        assert_eq!(compact.left_panel_width_px, 0);
        assert_eq!(compact.center_width_px(), 706);
        assert_eq!(compact.right_panel_width_px, 180);
        assert_eq!(compact.filmstrip_height_px, 0);
        assert!(!compact.left_panel_visible);
        assert!(!compact.filmstrip_visible);
    }

    #[test]
    fn lighttable_keeps_one_filter_toolbar_row() {
        assert_eq!(
            LIGHTTABLE_TOOLBAR.widget_name,
            "lighttable-collection-toolbar"
        );
        assert_eq!(
            LIGHTTABLE_TOOLBAR.filter_entry_name,
            "lighttable-filter-entry"
        );
        assert_eq!(LIGHTTABLE_TOOLBAR.row_count, 1);
    }

    #[test]
    fn lighttable_right_rail_is_direct_ordered_and_has_one_export() {
        assert_eq!(
            LIGHTTABLE_RIGHT_MODULES.map(|module| module.widget_name),
            [
                "selection",
                "actions-on-selection",
                "history-stack",
                "styles",
                "metadata-editor",
                "tagging",
                "geotagging",
                "export",
            ]
        );
        assert_eq!(
            LIGHTTABLE_RIGHT_MODULES
                .iter()
                .filter(|module| module.widget_name == "export")
                .count(),
            1
        );
        assert!(
            LIGHTTABLE_RIGHT_MODULES
                .iter()
                .all(|module| module.widget_name != "module-groups")
        );
    }

    #[test]
    fn lighttable_rail_excludes_obsolete_neural_restore_surface() {
        assert!(
            LIGHTTABLE_RIGHT_MODULES
                .iter()
                .all(|module| module.widget_name != "neural-restore")
        );
        assert!(
            LIGHTTABLE_RIGHT_MODULES
                .iter()
                .all(|module| !module.title.eq_ignore_ascii_case("neural restore"))
        );
    }

    #[test]
    fn lighttable_has_one_top_one_footer_and_a_plain_filmstrip() {
        assert_eq!(LIGHTTABLE_COMPOSITION.top_toolbar_rows, 1);
        assert_eq!(LIGHTTABLE_COMPOSITION.footer_toolbar_rows, 1);
        assert_eq!(LIGHTTABLE_COMPOSITION.filmstrip_toolbar_rows, 0);
        assert_eq!(LIGHTTABLE_COMPOSITION.empty_state_columns, 2);
        assert_eq!(LIGHTTABLE_COMPOSITION.top_toolbar_height_px, 24);
    }

    #[test]
    fn thumbnail_bounds_keep_grid_and_filmstrip_visually_distinct() {
        assert_eq!(THUMBNAIL_METRICS.grid_width_px, 196);
        assert_eq!(THUMBNAIL_METRICS.grid_height_px, 147);
        assert_eq!(THUMBNAIL_METRICS.filmstrip_width_px, 104);
        assert_eq!(THUMBNAIL_METRICS.filmstrip_height_px, 78);
        assert_eq!(FILMSTRIP_ITEM_GAP_PX, 4);
        assert_eq!(FILMSTRIP_MAX_CHILDREN_PER_LINE, u32::MAX);
    }

    #[test]
    fn darktable_colors_preserve_the_elegant_grey_css_tokens() {
        assert_eq!(DARKTABLE_COLORS.background.css_name(), "bg_color");
        assert_eq!(DARKTABLE_COLORS.background.rgba(), [0x6a, 0x6a, 0x6a, 0xff]);
        assert_eq!(
            DARKTABLE_COLORS.lighttable_canvas.rgba(),
            [0x91, 0x91, 0x91, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.darkroom_canvas.rgba(),
            [0x77, 0x77, 0x77, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.thumbnail_background.rgba(),
            [0xab, 0xab, 0xab, 0xff]
        );
        assert_eq!(
            DARKTABLE_COLORS.active_image_marker.rgba(),
            [0xf1, 0xf1, 0xf1, 0xff]
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
