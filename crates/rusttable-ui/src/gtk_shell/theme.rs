//! GTK4 style installation and semantic classes for the Darktable visual contract.
//!
//! The CSS template is deliberately small and local to the shell. Its colors
//! are substituted from [`super::DARKTABLE_COLORS`] at runtime, keeping the
//! palette in the display-free specification while allowing GTK to consume
//! native CSS. The API is usable by an application before constructing or
//! presenting a [`super::GtkShell`].

use gtk4::prelude::*;

use super::{ColorToken, DARKTABLE_COLORS};

const DARKTABLE_THEME_TEMPLATE: &str = include_str!("theme.css");

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
    let replacements = [
        ("{{background}}", colors.background),
        ("{{foreground}}", colors.foreground),
        ("{{border}}", colors.border),
        ("{{module_background}}", colors.module_background),
        ("{{button_background}}", colors.button_background),
        (
            "{{active_field_background}}",
            colors.active_field_background,
        ),
        ("{{lighttable_canvas}}", colors.lighttable_canvas),
        ("{{darkroom_canvas}}", colors.darkroom_canvas),
        ("{{filmstrip_background}}", colors.filmstrip_background),
        ("{{selected_thumbnail}}", colors.selected_thumbnail),
        ("{{hovered_thumbnail}}", colors.hovered_thumbnail),
        ("{{active_image_marker}}", colors.active_image_marker),
    ];
    replacements.into_iter().fold(
        DARKTABLE_THEME_TEMPLATE.to_owned(),
        |css, (placeholder, color)| css.replace(placeholder, &css_color(color)),
    )
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
        assert!(css.contains("#262626ff"));
        assert!(css.contains("#777777ff"));
        assert!(css.contains("#ffbb00ff"));
        assert!(css.contains(".dt_photo_card"));
    }

    #[test]
    fn semantic_roles_follow_darktable_class_naming() {
        assert_eq!(ThemeRole::Shell.class_name(), "dt_shell");
        assert_eq!(ThemeRole::Darkroom.class_name(), "dt_darkroom_canvas");
        assert_eq!(ThemeRole::SelectedPhoto.class_name(), "dt_selected");
    }
}
