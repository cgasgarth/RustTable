//! Display-free navigation-thumbnail geometry derived from darkroom viewport state.

use crate::presentation::PreviewDimensions;
use crate::viewport_presentation::{DarkroomViewportState, DarkroomZoom};
use crate::widgets::canvas::ViewportSize;

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
            DarkroomZoom::Small | DarkroomZoom::Fit | DarkroomZoom::Fill => 1_000_u16,
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

    /// Projects Darktable's navigation bounds from the real viewport and processed-image sizes.
    ///
    /// Unlike the geometry-free compatibility projection, this preserves independent horizontal
    /// and vertical extents. Fill and fixed-percentage zooms generally expose a non-square crop.
    #[must_use]
    pub fn navigation_crop_for(
        self,
        viewport: ViewportSize,
        image: PreviewDimensions,
    ) -> NavigationCrop {
        let scale_milli = zoom_scale_milli(self.zoom(), viewport, image);
        let width = crop_extent(viewport.width(), image.width(), scale_milli);
        let height = crop_extent(viewport.height(), image.height(), scale_milli);
        NavigationCrop {
            x: pan_position(1_000_u16.saturating_sub(width), self.pan().x()),
            y: pan_position(1_000_u16.saturating_sub(height), self.pan().y()),
            width,
            height,
        }
    }
}

fn zoom_scale_milli(zoom: DarkroomZoom, viewport: ViewportSize, image: PreviewDimensions) -> u64 {
    let width = u64::from(viewport.width()) * 1_000 / u64::from(image.width());
    let height = u64::from(viewport.height()) * 1_000 / u64::from(image.height());
    match zoom {
        DarkroomZoom::Small => width.min(height).saturating_div(2).max(1),
        DarkroomZoom::Fit => width.min(height).max(1),
        DarkroomZoom::Fill => width.max(height).max(1),
        zoom => u64::from(zoom.percent().unwrap_or(100)) * 10,
    }
}

fn crop_extent(viewport: u32, image: u32, scale_milli: u64) -> u16 {
    let denominator = u64::from(image).saturating_mul(scale_milli);
    if denominator == 0 {
        return 1_000;
    }
    let extent = u64::from(viewport)
        .saturating_mul(1_000_000)
        .saturating_add(denominator / 2)
        / denominator;
    u16::try_from(extent.clamp(1, 1_000)).unwrap_or(1_000)
}

fn pan_position(available: u16, pan: i16) -> u16 {
    let shifted = i32::from(pan).saturating_add(1_000);
    let position = i32::from(available).saturating_mul(shifted) / 2_000;
    u16::try_from(position).unwrap_or(available).min(available)
}

#[cfg(test)]
mod tests {
    use rusttable_core::{PhotoId, Revision};

    use super::*;
    use crate::viewport_presentation::{
        DarkroomViewportAction, DarkroomViewportCommand, ViewportGeneration,
    };

    fn state_at(zoom: DarkroomZoom, pan_x: i32, pan_y: i32) -> DarkroomViewportState {
        let mut state = DarkroomViewportState::default();
        let generation = ViewportGeneration::new(1);
        state.select(PhotoId::new(1).expect("photo"), Revision::ZERO, generation);
        assert!(state.apply(DarkroomViewportCommand::new(
            generation,
            DarkroomViewportAction::SetZoom(zoom),
        )));
        if pan_x != 0 || pan_y != 0 {
            assert!(state.apply(DarkroomViewportCommand::new(
                generation,
                DarkroomViewportAction::Pan {
                    delta_x: pan_x,
                    delta_y: pan_y,
                },
            )));
        }
        state
    }

    #[test]
    fn fill_crop_uses_independent_image_axes() {
        let state = state_at(DarkroomZoom::Fill, 0, 0);
        let crop = state.navigation_crop_for(
            ViewportSize::new(800, 800).expect("viewport"),
            PreviewDimensions::new(400, 200).expect("image"),
        );

        assert_eq!(crop.width_milli(), 500);
        assert_eq!(crop.height_milli(), 1_000);
        assert_eq!(crop.x_milli(), 250);
        assert_eq!(crop.y_milli(), 0);
    }

    #[test]
    fn fixed_zoom_crop_accounts_for_fit_and_image_aspect() {
        let state = state_at(DarkroomZoom::TwoHundredPercent, 1_000, -1_000);
        let crop = state.navigation_crop_for(
            ViewportSize::new(800, 400).expect("viewport"),
            PreviewDimensions::new(800, 800).expect("image"),
        );

        assert_eq!(crop.width_milli(), 500);
        assert_eq!(crop.height_milli(), 250);
        assert_eq!(crop.x_milli(), 500);
        assert_eq!(crop.y_milli(), 0);
    }
}
