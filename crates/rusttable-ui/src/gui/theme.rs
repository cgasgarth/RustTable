//! GTK4 style installation and semantic classes for the Darktable visual contract.
//!
//! The CSS template is deliberately small and local to the shell. Its colors
//! are substituted from [`super::DARKTABLE_COLORS`] at runtime, keeping the
//! palette in the display-free specification while allowing GTK to consume
//! native CSS. The API is usable by an application before constructing or
//! presenting a [`super::GtkShell`].

use gtk4::prelude::*;

use super::{
    ColorToken, DARKROOM_GEOMETRY, DARKTABLE_COLORS, DARKTABLE_DESKTOP_SPEC, DARKTABLE_UI_TOKENS,
    LIGHTTABLE_COMPOSITION,
};

const DARKTABLE_THEME_TEMPLATE: &str = include_str!("theme.css");
const BUTTON_BORDER: ColorToken = ColorToken::new("button_border", [0x82, 0x82, 0x82, 0xff]);
const BUTTON_HOVER_OVERLAY: ColorToken =
    ColorToken::new("button_hover_bg", [0xab, 0xab, 0xab, 0xff]);
const DISABLED_BUTTON_BORDER: ColorToken =
    ColorToken::new("button_border_disabled", [0x82, 0x82, 0x82, 0x59]);
const DISABLED_FOREGROUND: ColorToken =
    ColorToken::new("disabled_fg_color", [0x9e, 0x9e, 0x9e, 0xff]);
const SECTION_LABEL: ColorToken = ColorToken::new("section_label", [0xde, 0xde, 0xde, 0xff]);
const SCROLLBAR_INACTIVE: ColorToken =
    ColorToken::new("scroll_bar_inactive", [0x91, 0x91, 0x91, 0xff]);
const SCROLLBAR_ACTIVE: ColorToken = ColorToken::new("scroll_bar_active", [0xc6, 0xc6, 0xc6, 0xff]);
const SCROLLBAR_BACKGROUND: ColorToken = ColorToken::new("scroll_bar_bg", [0x5e, 0x5e, 0x5e, 0xff]);

/// Semantic GTK classes corresponding to the visual roles in Darktable's CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeRole {
    /// The complete `RustTable` shell.
    Shell,
    /// The application header and global controls.
    Header,
    /// A side panel containing collection or processing modules.
    Panel,
    /// A central lighttable or darkroom workspace.
    Workspace,
    /// The lighttable thumbnail surface.
    Lighttable,
    /// The darkroom image surface.
    Darkroom,
    /// The bottom image filmstrip.
    Filmstrip,
    /// A toolbar row.
    Toolbar,
    /// A module or module group.
    Module,
    /// A photo card or thumbnail.
    PhotoCard,
    /// The selected photo card or thumbnail.
    SelectedPhoto,
    /// The two-column empty collection/help message.
    EmptyState,
    /// A compact view switcher control.
    ViewSwitcher,
    /// A collapsible navigation or processing group.
    ModuleGroup,
    /// The image portion of a thumbnail tile.
    ThumbnailImage,
}

impl ThemeRole {
    /// Returns the `dt_*` CSS class used for this visual role.
    #[must_use]
    pub const fn class_name(self) -> &'static str {
        match self {
            Self::Shell => "dt_shell",
            Self::Header => "dt_header",
            Self::Panel => "dt_panel",
            Self::Workspace => "dt_workspace",
            Self::Lighttable => "dt_lighttable",
            Self::Darkroom => "dt_darkroom_canvas",
            Self::Filmstrip => "dt_filmstrip",
            Self::Toolbar => "dt_toolbar",
            Self::Module => "dt_module",
            Self::PhotoCard => "dt_photo_card",
            Self::SelectedPhoto => "dt_selected",
            Self::EmptyState => "dt_empty_state",
            Self::ViewSwitcher => "dt_view_switcher",
            Self::ModuleGroup => "dt_module_group",
            Self::ThumbnailImage => "dt_thumbnail_image",
        }
    }
}

/// Stateless handle for installing the `RustTable` GTK theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DarktableTheme;

impl DarktableTheme {
    /// Builds a GTK CSS provider from the Rust-owned Darktable palette.
    #[must_use]
    pub fn provider() -> gtk4::CssProvider {
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(&darktable_theme_css());
        provider
    }

    /// Installs the theme for a GTK display at application priority.
    pub fn install(display: &gtk4::gdk::Display) {
        let provider = Self::provider();
        gtk4::style_context_add_provider_for_display(
            display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Returns the GTK4 CSS generated from the Darktable palette tokens.
#[must_use]
pub fn darktable_theme_css() -> String {
    let colors = DARKTABLE_COLORS;
    let css = apply_color_tokens(DARKTABLE_THEME_TEMPLATE, &colors);
    let tokens = DARKTABLE_UI_TOKENS;
    let dimensions = [
        ("{{base_font_pt}}", i32::from(tokens.typography.base_pt)),
        (
            "{{compact_font_pt}}",
            i32::from(tokens.typography.compact_pt),
        ),
        ("{{micro_font_pt}}", i32::from(tokens.typography.micro_pt)),
        (
            "{{heading_font_pt}}",
            i32::from(tokens.typography.heading_pt),
        ),
        (
            "{{rail_min_width}}",
            i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.minimum_px),
        ),
        (
            "{{header_height}}",
            i32::from(DARKTABLE_DESKTOP_SPEC.layout.header_height_px),
        ),
        (
            "{{header_content_height}}",
            i32::from(DARKTABLE_DESKTOP_SPEC.layout.header_height_px) - 5,
        ),
        (
            "{{lighttable_toolbar_height}}",
            i32::from(LIGHTTABLE_COMPOSITION.top_toolbar_height_px),
        ),
        (
            "{{outer_border_width}}",
            i32::from(DARKTABLE_DESKTOP_SPEC.layout.outer_border_px),
        ),
        ("{{control_height}}", tokens.controls.control_height),
        ("{{module_row_height}}", tokens.controls.module_row_height),
        (
            "{{module_title_height}}",
            tokens.controls.module_title_height,
        ),
        (
            "{{module_header_button_size}}",
            tokens.controls.module_header_button_size,
        ),
        (
            "{{module_header_icon_size}}",
            tokens.controls.module_header_icon_size,
        ),
        (
            "{{toolbar_button_size}}",
            tokens.controls.toolbar_button_size,
        ),
        ("{{toolbar_height}}", tokens.controls.toolbar_height),
        ("{{status_height}}", tokens.controls.status_height),
        ("{{control_gap}}", tokens.controls.control_gap),
        ("{{module_gap}}", tokens.controls.module_gap),
        ("{{module_padding}}", tokens.controls.module_padding),
        (
            "{{histogram_height}}",
            i32::from(DARKROOM_GEOMETRY.histogram_height_px),
        ),
        (
            "{{histogram_min_height}}",
            i32::from(DARKROOM_GEOMETRY.histogram_min_height_px),
        ),
        (
            "{{card_min_width}}",
            i32::from(tokens.cards.minimum_width_px),
        ),
        (
            "{{card_preferred_width}}",
            i32::from(tokens.cards.preferred_width_px),
        ),
        (
            "{{card_metadata_height}}",
            i32::from(tokens.cards.metadata_height_px),
        ),
    ];
    dimensions
        .into_iter()
        .fold(css, |css, (placeholder, value)| {
            css.replace(placeholder, &value.to_string())
        })
}

fn apply_color_tokens(template: &str, colors: &super::DarktableColors) -> String {
    let replacements = [
        ("{{background}}", colors.background),
        ("{{foreground}}", colors.foreground),
        ("{{border}}", colors.border),
        ("{{module_background}}", colors.module_background),
        ("{{button_background}}", colors.button_background),
        ("{{button_border}}", BUTTON_BORDER),
        ("{{button_hover_overlay}}", BUTTON_HOVER_OVERLAY),
        ("{{disabled_button_border}}", DISABLED_BUTTON_BORDER),
        ("{{disabled_foreground}}", DISABLED_FOREGROUND),
        (
            "{{active_field_background}}",
            colors.active_field_background,
        ),
        ("{{module_label}}", colors.module_label),
        ("{{section_label}}", SECTION_LABEL),
        ("{{scrollbar_inactive}}", SCROLLBAR_INACTIVE),
        ("{{scrollbar_active}}", SCROLLBAR_ACTIVE),
        ("{{scrollbar_background}}", SCROLLBAR_BACKGROUND),
        ("{{lighttable_canvas}}", colors.lighttable_canvas),
        ("{{darkroom_canvas}}", colors.darkroom_canvas),
        ("{{thumbnail_background}}", colors.thumbnail_background),
        ("{{filmstrip_background}}", colors.filmstrip_background),
        ("{{selected_thumbnail}}", colors.selected_thumbnail),
        ("{{hovered_thumbnail}}", colors.hovered_thumbnail),
        ("{{active_image_marker}}", colors.active_image_marker),
    ];
    replacements
        .into_iter()
        .fold(template.to_owned(), |css, (placeholder, color)| {
            css.replace(placeholder, &css_color(color))
        })
}

/// Installs the theme for a GTK display.
pub fn install_darktable_theme(display: &gtk4::gdk::Display) {
    DarktableTheme::install(display);
}

/// Applies a semantic Darktable class to a GTK widget.
pub fn apply_theme_role<W: IsA<gtk4::Widget>>(widget: &W, role: ThemeRole) {
    widget.add_css_class(role.class_name());
}

fn css_color(token: ColorToken) -> String {
    let [red, green, blue, alpha] = token.rgba();
    format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}")
}

#[cfg(test)]
mod tests {
    use super::{ThemeRole, darktable_theme_css};

    #[test]
    fn css_uses_the_spec_palette_and_has_no_unexpanded_tokens() {
        let css = darktable_theme_css();

        assert!(!css.contains("{{"));
        assert!(css.contains("#6a6a6aff"));
        assert!(css.contains("#777777ff"));
        assert!(css.contains("#f1f1f1ff"));
        assert!(css.contains("#abababff"));
        assert!(css.contains("#c6c6c6ff"));
        assert!(css.contains(".dt_photo_card"));
        assert!(css.contains(".dt_empty_state"));
        assert!(css.contains("font-size: 12pt"));
        assert!(!css.contains("font-size: 0.85em"));
        assert!(css.contains("\"Roboto Light\", \"Roboto\""));
        assert!(css.contains("\"SF Pro Display Light\", \"SF Pro Display\""));
        assert!(css.contains(".dt_view_switcher"));
        assert!(css.contains("button:disabled"));
        assert!(css.contains("#export-rail-content"));
        assert!(css.contains("#right-panel #export"));
        assert!(!css.contains("max-width:"));
    }

    #[test]
    fn css_keeps_darktable_control_states_on_their_semantic_colors() {
        let css = darktable_theme_css();

        assert!(css.contains("border: 1px solid #828282ff"));
        assert!(css.contains("background-color: #abababff"));
        assert!(css.contains("border-color: #82828259"));
        assert!(css.contains("color: #9e9e9eff"));
        assert!(css.contains("background-color: #919191ff"));
        assert!(css.contains("background-color: #c6c6c6ff"));
    }

    #[test]
    fn css_ports_darktable_reset_and_configured_widget_metrics() {
        let css = darktable_theme_css();

        assert!(css.contains("min-width: 0;\n  min-height: 0;"));
        assert!(css.contains("min-width: 14px;\n  min-height: 14px;"));
        assert!(css.contains("min-width: 21px;\n  min-height: 21px;"));
        assert!(css.contains("-gtk-icon-size: 11px"));
        assert!(css.contains("padding: 0.14em 0.28em"));
        assert!(css.contains("min-height: 20px"));
        assert!(css.contains("min-height: 25px"));
        assert!(css.contains("font-size: 18pt"));
    }

    #[test]
    fn semantic_roles_follow_darktable_class_naming() {
        assert_eq!(ThemeRole::Shell.class_name(), "dt_shell");
        assert_eq!(ThemeRole::Darkroom.class_name(), "dt_darkroom_canvas");
        assert_eq!(ThemeRole::SelectedPhoto.class_name(), "dt_selected");
    }

    #[test]
    fn css_exposes_the_shell_roles_used_by_screenshot_smoke_checks() {
        let css = darktable_theme_css();

        for selector in [
            "#header",
            "#left-panel",
            "#center-workspace",
            "#right-panel",
            "#bottom-filmstrip",
            ".dt_empty_state",
            ".dt_module_group",
            ".dt_thumbnail_image",
            ".dt_view_switcher",
        ] {
            assert!(css.contains(selector), "missing selector {selector}");
        }
    }

    #[test]
    fn semantic_roles_are_unique_and_stable() {
        let roles = [
            ThemeRole::Shell,
            ThemeRole::Header,
            ThemeRole::Panel,
            ThemeRole::Workspace,
            ThemeRole::Lighttable,
            ThemeRole::Darkroom,
            ThemeRole::Filmstrip,
            ThemeRole::Toolbar,
            ThemeRole::Module,
            ThemeRole::PhotoCard,
            ThemeRole::SelectedPhoto,
            ThemeRole::EmptyState,
            ThemeRole::ViewSwitcher,
            ThemeRole::ModuleGroup,
            ThemeRole::ThumbnailImage,
        ];
        let names = roles
            .into_iter()
            .map(ThemeRole::class_name)
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(names.len(), roles.len());
    }
}
