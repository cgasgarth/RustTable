//! Darktable-aligned desktop scale and responsive allocation tokens.
//!
//! The values map the installed Darktable 5.6 desktop contract in
//! `data/darktablerc`, `data/themes/darktable.css`, and `src/gui/gtk.c` into
//! display-free Rust values. GTK widgets and CSS consume this one scale instead
//! of carrying view-local pixel guesses.

#![forbid(unsafe_code)]

use super::LAYOUT_METRICS;

/// Typography sizes expressed in points, matching Darktable's configured font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypographyTokens {
    pub base_pt: u8,
    pub compact_pt: u8,
    pub micro_pt: u8,
    pub heading_pt: u8,
}

/// Shared row, control, and spacing dimensions for both primary workspaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlScaleTokens {
    pub control_height: i32,
    pub module_row_height: i32,
    pub module_title_height: i32,
    pub toolbar_height: i32,
    pub status_height: i32,
    pub control_gap: i32,
    pub module_gap: i32,
    pub module_padding: i32,
    pub module_control_min_width: i32,
    pub rail_scrollbar_reserve: i32,
}

/// Bounded lighttable card geometry recomputed from the center viewport width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LighttableCardTokens {
    pub minimum_width_px: u16,
    pub preferred_width_px: u16,
    pub maximum_width_px: u16,
    pub horizontal_chrome_px: u16,
    pub metadata_height_px: u16,
    pub item_gap_px: u16,
    pub image_aspect_width: u16,
    pub image_aspect_height: u16,
}

impl LighttableCardTokens {
    /// Returns one bounded outer card width for a viewport and discrete density.
    #[must_use]
    pub fn width_for_viewport(self, viewport_width_px: u16, columns: usize) -> u16 {
        let columns = if columns == 0 { 1 } else { columns };
        let columns_u16 = u16::try_from(columns).unwrap_or(u16::MAX);
        let gaps = columns_u16
            .saturating_sub(1)
            .saturating_mul(self.item_gap_px);
        let available = viewport_width_px.saturating_sub(gaps) / columns_u16;
        if available < self.minimum_width_px {
            self.minimum_width_px
        } else if available > self.maximum_width_px {
            self.maximum_width_px
        } else {
            available
        }
    }

    /// Returns the image surface width after card padding and borders.
    #[must_use]
    pub const fn image_width_px(self, card_width_px: u16) -> u16 {
        card_width_px.saturating_sub(self.horizontal_chrome_px)
    }

    /// Returns a stable image height using the Darktable card aspect token.
    #[must_use]
    pub const fn image_height_px(self, image_width_px: u16) -> u16 {
        image_width_px.saturating_mul(self.image_aspect_height) / self.image_aspect_width
    }
}

/// Complete Rust-owned scale consumed by shared GTK components and CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarktableUiTokens {
    pub typography: TypographyTokens,
    pub controls: ControlScaleTokens,
    pub cards: LighttableCardTokens,
}

/// Compact typography and chrome from the matched Darktable desktop capture.
pub const DARKTABLE_UI_TOKENS: DarktableUiTokens = DarktableUiTokens {
    typography: TypographyTokens {
        base_pt: 9,
        compact_pt: 8,
        micro_pt: 7,
        heading_pt: 13,
    },
    controls: ControlScaleTokens {
        control_height: 18,
        module_row_height: 20,
        module_title_height: 16,
        toolbar_height: 18,
        status_height: 18,
        control_gap: 3,
        module_gap: 1,
        module_padding: 3,
        module_control_min_width: 42,
        rail_scrollbar_reserve: 10,
    },
    cards: LighttableCardTokens {
        minimum_width_px: 148,
        preferred_width_px: 196,
        maximum_width_px: 260,
        horizontal_chrome_px: 12,
        metadata_height_px: 44,
        item_gap_px: 6,
        image_aspect_width: 4,
        image_aspect_height: 3,
    },
};

/// Horizontal allocation used by every module row inside a scrolling rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleControlAllocationReceipt {
    pub rail_width_px: u16,
    pub content_width_px: u16,
    pub label_width_px: u16,
    pub control_width_px: u16,
    pub scrollbar_width_px: u16,
}

impl ModuleControlAllocationReceipt {
    #[must_use]
    pub fn for_rail(rail_width_px: u16) -> Self {
        let controls = DARKTABLE_UI_TOKENS.controls;
        let scrollbar = u16::try_from(controls.rail_scrollbar_reserve).unwrap_or_default();
        let padding = u16::try_from(controls.module_padding.saturating_mul(2)).unwrap_or_default();
        let gap = u16::try_from(controls.control_gap).unwrap_or_default();
        let control = u16::try_from(controls.module_control_min_width).unwrap_or_default();
        let content = rail_width_px
            .saturating_sub(scrollbar)
            .saturating_sub(padding);
        let label = content.saturating_sub(gap).saturating_sub(control);
        Self {
            rail_width_px,
            content_width_px: content,
            label_width_px: label,
            control_width_px: control,
            scrollbar_width_px: scrollbar,
        }
    }

    #[must_use]
    pub const fn fits(self) -> bool {
        self.content_width_px
            .saturating_add(self.scrollbar_width_px)
            <= self.rail_width_px
            && self.label_width_px > 0
    }
}

/// Allocation shared by viewport, histogram, and lighttable resize paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponsiveGeometryReceipt {
    pub window_width_px: u16,
    pub window_height_px: u16,
    pub left_rail_width_px: u16,
    pub center_width_px: u16,
    pub right_rail_width_px: u16,
    pub viewport_height_px: u16,
    pub histogram_width_px: u16,
    pub histogram_height_px: u16,
}

impl ResponsiveGeometryReceipt {
    /// Resolves the supported desktop geometry without allowing either rail to
    /// fall below the readable Darktable-aligned preferred width.
    #[must_use]
    pub fn for_window(window_width_px: u16, window_height_px: u16) -> Self {
        let rail = LAYOUT_METRICS.side_panel_widths.preferred_px;
        let center_width = LAYOUT_METRICS
            .content_width_px(window_width_px)
            .saturating_sub(rail.saturating_mul(2));
        let vertical_chrome = u16::from(LAYOUT_METRICS.header_height_px)
            .saturating_add(u16::from(LAYOUT_METRICS.outer_border_px))
            .saturating_add(u16::from(
                super::LIGHTTABLE_COMPOSITION.top_toolbar_height_px,
            ))
            .saturating_add(
                u16::try_from(DARKTABLE_UI_TOKENS.controls.status_height).unwrap_or(u16::MAX),
            )
            .saturating_add(LAYOUT_METRICS.filmstrip_heights.preferred_px);
        Self {
            window_width_px,
            window_height_px,
            left_rail_width_px: rail,
            center_width_px: center_width,
            right_rail_width_px: rail,
            viewport_height_px: window_height_px.saturating_sub(vertical_chrome),
            histogram_width_px: rail,
            histogram_height_px: super::DARKROOM_GEOMETRY.histogram_height_px,
        }
    }
}
