pub const CONTENT_PADDING: f32 = 16.0;
pub const REGION_SPACING: f32 = 12.0;
pub const HEADER_HEIGHT: f32 = 48.0;
pub const SIDEBAR_WIDTH: f32 = 240.0;
pub const PHOTO_GRID_COLUMNS: usize = 3;
pub const PHOTO_CARD_WIDTH: f32 = 150.0;
pub const PHOTO_CARD_HEIGHT: f32 = 140.0;
pub const PHOTO_GRID_SPACING: f32 = 12.0;
pub const FOCUS_OUTLINE_WIDTH: f32 = 2.0;

#[must_use]
pub fn theme<T>(_: &T) -> iced::Theme {
    iced::Theme::Dark
}

#[must_use]
pub fn focus_outline() -> iced::Border {
    iced::border::color(iced::Color::from_rgb(0.3, 0.7, 1.0)).width(FOCUS_OUTLINE_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::{
        CONTENT_PADDING, HEADER_HEIGHT, PHOTO_CARD_HEIGHT, PHOTO_CARD_WIDTH, PHOTO_GRID_COLUMNS,
        PHOTO_GRID_SPACING, REGION_SPACING, SIDEBAR_WIDTH, theme,
    };
    #[test]
    fn shell_theme_and_layout_contract_are_fixed() {
        assert_eq!(theme(&()), iced::Theme::Dark);
        assert_eq!(CONTENT_PADDING.to_bits(), 16.0_f32.to_bits());
        assert_eq!(REGION_SPACING.to_bits(), 12.0_f32.to_bits());
        assert_eq!(HEADER_HEIGHT.to_bits(), 48.0_f32.to_bits());
        assert_eq!(SIDEBAR_WIDTH.to_bits(), 240.0_f32.to_bits());
        assert_eq!(PHOTO_GRID_COLUMNS, 3);
        assert_eq!(PHOTO_CARD_WIDTH.to_bits(), 150.0_f32.to_bits());
        assert_eq!(PHOTO_CARD_HEIGHT.to_bits(), 140.0_f32.to_bits());
        assert_eq!(PHOTO_GRID_SPACING.to_bits(), 12.0_f32.to_bits());
    }
}
