use iced::{Color, Theme};

use crate::shell::ThemeSelection;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignTokens {
    pub spacing_unit: f32,
    pub focus_ring_width: f32,
    pub motion_scale: f32,
    pub density_scale: f32,
    pub body_text_size: f32,
    pub heading_text_size: f32,
    pub focus_ring: Color,
}

impl DesignTokens {
    #[must_use]
    pub const fn for_theme(theme: ThemeSelection, reduced_motion: bool) -> Self {
        let focus_ring = match theme {
            ThemeSelection::Light => Color::from_rgb(0.0, 0.25, 0.7),
            ThemeSelection::System | ThemeSelection::Dark => Color::from_rgb(0.3, 0.7, 1.0),
        };
        Self {
            spacing_unit: 4.0,
            focus_ring_width: 2.0,
            motion_scale: if reduced_motion { 0.0 } else { 1.0 },
            density_scale: 1.0,
            body_text_size: 14.0,
            heading_text_size: 20.0,
            focus_ring,
        }
    }

    #[must_use]
    pub const fn spacing(self, units: f32) -> f32 {
        self.spacing_unit * units * self.density_scale
    }
}

#[must_use]
pub fn theme(selection: ThemeSelection) -> Theme {
    match selection {
        ThemeSelection::Light => Theme::Light,
        ThemeSelection::Dark | ThemeSelection::System => Theme::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::{DesignTokens, theme};
    use crate::shell::ThemeSelection;

    #[test]
    fn tokens_cover_density_focus_motion_and_theme() {
        let tokens = DesignTokens::for_theme(ThemeSelection::Light, true);
        assert_eq!(tokens.spacing(4.0).to_bits(), 16.0_f32.to_bits());
        assert_eq!(tokens.motion_scale.to_bits(), 0.0_f32.to_bits());
        assert_eq!(theme(ThemeSelection::Dark), iced::Theme::Dark);
        assert_eq!(theme(ThemeSelection::Light), iced::Theme::Light);
    }
}
