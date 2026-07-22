//! Display-free navigation-thumbnail geometry derived from darkroom viewport state.

use crate::viewport_presentation::{DarkroomViewportState, DarkroomZoom};

/// Normalized current-view rectangle painted over the navigation thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationCrop {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl NavigationCrop {
    #[must_use]
    pub const fn x_milli(self) -> u16 {
        self.x
    }

    #[must_use]
    pub const fn y_milli(self) -> u16 {
        self.y
    }

    #[must_use]
    pub const fn width_milli(self) -> u16 {
        self.width
    }

    #[must_use]
    pub const fn height_milli(self) -> u16 {
        self.height
    }
}

impl DarkroomViewportState {
    /// Projects zoom and pan into the bounded navigation-thumbnail coordinate space.
    #[must_use]
    pub fn navigation_crop(self) -> NavigationCrop {
        let extent = match self.zoom() {
            DarkroomZoom::Small | DarkroomZoom::Fit => 1_000_u16,
            DarkroomZoom::Fill => 850,
            zoom => zoom.percent().map_or(1_000, |percent| {
                u16::try_from((100_000_u32 / u32::from(percent)).clamp(80_u32, 1_000_u32))
                    .unwrap_or(1_000)
            }),
        };
        let available = 1_000_u16.saturating_sub(extent);
        NavigationCrop {
            x: pan_position(available, self.pan().x()),
            y: pan_position(available, self.pan().y()),
            width: extent,
            height: extent,
        }
    }
}

fn pan_position(available: u16, pan: i16) -> u16 {
    let shifted = i32::from(pan).saturating_add(1_000);
    let position = i32::from(available).saturating_mul(shifted) / 2_000;
    u16::try_from(position).unwrap_or(available).min(available)
}
