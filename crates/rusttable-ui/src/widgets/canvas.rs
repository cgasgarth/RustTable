//! Bounded, generation-safe state for a Darktable-aligned image viewport.
//!
//! This module is deliberately GTK-free. It projects the existing validated preview-frame
//! contract into display metadata and deterministic integer geometry; a future GTK surface can
//! consume these types without taking ownership of rendering, color, or async-generation policy.

use rusttable_core::{PhotoId, Revision};

use crate::presentation::PreviewDimensions;
use crate::viewport_presentation::{
    DisplayPresentationFrame, PresentationGeneration, PresentationStatus, PresentationTicket,
    ViewportGeneration,
};

/// The largest accepted manual zoom percentage, matching Darktable's navigation choices.
pub const MAX_ZOOM_PERCENT: u16 = 1_600;
/// The smallest accepted manual zoom percentage.
pub const MIN_ZOOM_PERCENT: u16 = 1;
/// The largest normalized pan value in either direction.
pub const MAX_PAN: i16 = 1_000;

const MAX_VIEWPORT_DIMENSION: u32 = 32_768;
const MAX_PROJECTED_DIMENSION: u32 = 16_777_216;
const SCALE_DENOMINATOR: u32 = 1_000;

/// A validated manual zoom percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ZoomPercent(u16);

/// Errors returned when a manual zoom percentage is outside the safe viewport range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomPercentError {
    BelowMinimum,
    AboveMaximum,
}

impl ZoomPercent {
    /// Creates a zoom percentage in the inclusive `[1, 1600]` range.
    ///
    /// # Errors
    ///
    /// Returns [`ZoomPercentError::BelowMinimum`] for zero and
    /// [`ZoomPercentError::AboveMaximum`] for values above [`MAX_ZOOM_PERCENT`].
    pub const fn new(value: u16) -> Result<Self, ZoomPercentError> {
        if value < MIN_ZOOM_PERCENT {
            Err(ZoomPercentError::BelowMinimum)
        } else if value > MAX_ZOOM_PERCENT {
            Err(ZoomPercentError::AboveMaximum)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the validated percentage.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Darktable's compact navigation-box zoom choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ViewportZoom {
    /// A reduced view, never larger than the image's fit projection.
    Small,
    /// Preserve the complete image inside the viewport.
    #[default]
    Fit,
    /// Fill the viewport while preserving the image aspect ratio.
    Fill,
    /// Use an explicit bounded percentage.
    Percent(ZoomPercent),
}

impl ViewportZoom {
    /// Returns a stable Darktable-style control label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Fit => "fit",
            Self::Fill => "fill",
            Self::Percent(_) => "manual",
        }
    }

    fn scale_milli(self, viewport: ViewportSize, image: PreviewDimensions) -> u32 {
        let fit = ratio_milli(
            viewport.width,
            image.width(),
            viewport.height,
            image.height(),
            false,
        );
        let fill = ratio_milli(
            viewport.width,
            image.width(),
            viewport.height,
            image.height(),
            true,
        );
        match self {
            Self::Small => (fit / 2).max(1),
            Self::Fit => fit,
            Self::Fill => fill,
            Self::Percent(percent) => u32::from(percent.get()) * 10,
        }
        .max(1)
    }
}

/// A bounded normalized pan offset. `-1000` is start/top and `1000` is end/bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PanOffset {
    x: i16,
    y: i16,
}

impl PanOffset {
    /// Creates an offset, clamping both axes to the safe normalized range.
    #[must_use]
    pub fn new(x: i32, y: i32) -> Self {
        Self {
            x: clamp_pan(x),
            y: clamp_pan(y),
        }
    }

    /// Returns the horizontal normalized offset.
    #[must_use]
    pub const fn x(self) -> i16 {
        self.x
    }

    /// Returns the vertical normalized offset.
    #[must_use]
    pub const fn y(self) -> i16 {
        self.y
    }

    /// Adds a bounded delta without allowing integer overflow or unbounded drift.
    #[must_use]
    pub fn adjust(self, delta_x: i32, delta_y: i32) -> Self {
        Self::new(
            i32::from(self.x).saturating_add(delta_x),
            i32::from(self.y).saturating_add(delta_y),
        )
    }
}

fn clamp_pan(value: i32) -> i16 {
    i16::try_from(value.clamp(-i32::from(MAX_PAN), i32::from(MAX_PAN)))
        .expect("pan value is clamped to i16 range")
}

/// A validated viewport size used by deterministic geometry projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportSize {
    width: u32,
    height: u32,
}

/// Errors returned when a viewport size cannot be safely projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportSizeError {
    ZeroWidth,
    ZeroHeight,
    TooWide,
    TooHigh,
}

impl ViewportSize {
    /// Creates a non-empty viewport within the bounded surface range.
    ///
    /// # Errors
    ///
    /// Returns a zero-axis error for an empty viewport or a size error when either axis exceeds
    /// the bounded surface limit.
    pub const fn new(width: u32, height: u32) -> Result<Self, ViewportSizeError> {
        if width == 0 {
            Err(ViewportSizeError::ZeroWidth)
        } else if height == 0 {
            Err(ViewportSizeError::ZeroHeight)
        } else if width > MAX_VIEWPORT_DIMENSION {
            Err(ViewportSizeError::TooWide)
        } else if height > MAX_VIEWPORT_DIMENSION {
            Err(ViewportSizeError::TooHigh)
        } else {
            Ok(Self { width, height })
        }
    }

    /// Returns the viewport width in logical pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the viewport height in logical pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the paintable viewport after reserving Darktable's fixed border on every side.
    #[must_use]
    pub const fn inset(self, border: u32) -> Option<Self> {
        let Some(inset) = border.checked_mul(2) else {
            return None;
        };
        let Some(width) = self.width.checked_sub(inset) else {
            return None;
        };
        let Some(height) = self.height.checked_sub(inset) else {
            return None;
        };
        if width == 0 || height == 0 {
            None
        } else {
            Some(Self { width, height })
        }
    }
}

/// The identity of a preview request that is allowed to update the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PreviewFrameKey {
    photo_id: PhotoId,
    edit_revision: Revision,
    presentation_generation: PresentationGeneration,
}

impl PreviewFrameKey {
    /// Extracts immutable identity from a presentation ticket.
    #[must_use]
    pub const fn from_ticket(ticket: PresentationTicket) -> Self {
        let request = ticket.request();
        Self {
            photo_id: request.photo_id(),
            edit_revision: request.edit_revision(),
            presentation_generation: request.profile_generation(),
        }
    }

    /// Returns the selected photo identity.
    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    /// Returns the edit revision used to render the frame.
    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }

    /// Returns the display-profile generation used to render the frame.
    #[must_use]
    pub const fn presentation_generation(self) -> PresentationGeneration {
        self.presentation_generation
    }
}

/// Display-only projection metadata for one accepted preview frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewFrameProjection {
    key: PreviewFrameKey,
    dimensions: PreviewDimensions,
    status: PresentationStatus,
}

impl PreviewFrameProjection {
    fn from_frame(frame: &DisplayPresentationFrame) -> Self {
        Self {
            key: PreviewFrameKey::from_ticket(frame.ticket()),
            dimensions: frame.metadata().dimensions(),
            status: frame.status(),
        }
    }

    /// Returns the accepted frame identity.
    #[must_use]
    pub const fn key(self) -> PreviewFrameKey {
        self.key
    }

    /// Returns the source image dimensions.
    #[must_use]
    pub const fn dimensions(self) -> PreviewDimensions {
        self.dimensions
    }

    /// Returns the display status attached to the frame.
    #[must_use]
    pub const fn status(self) -> PresentationStatus {
        self.status
    }
}

/// Monotonic redraw identity used to reject work scheduled for an older projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RedrawToken {
    generation: ViewportGeneration,
    serial: u64,
}

impl RedrawToken {
    /// Returns the selected-photo generation represented by this redraw.
    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }

    /// Returns the monotonic redraw serial.
    #[must_use]
    pub const fn serial(self) -> u64 {
        self.serial
    }
}

/// Result of attempting to install a presentation frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameProjectionResult {
    /// The frame replaced the current projection and produced a new redraw token.
    Applied(RedrawToken),
    /// The frame belongs to an older generation or request and was ignored.
    Stale,
}

/// GTK-free state for the image viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportCanvasState {
    generation: ViewportGeneration,
    expected_frame: Option<PreviewFrameKey>,
    frame: Option<PreviewFrameProjection>,
    zoom: ViewportZoom,
    pan: PanOffset,
    redraw_serial: u64,
}

impl Default for ViewportCanvasState {
    fn default() -> Self {
        Self {
            generation: ViewportGeneration::default(),
            expected_frame: None,
            frame: None,
            zoom: ViewportZoom::Fit,
            pan: PanOffset::default(),
            redraw_serial: 0,
        }
    }
}

impl ViewportCanvasState {
    /// Returns the selected-photo generation currently owned by the viewport.
    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }

    /// Returns the current zoom choice.
    #[must_use]
    pub const fn zoom(self) -> ViewportZoom {
        self.zoom
    }

    /// Returns the current bounded pan offset.
    #[must_use]
    pub const fn pan(self) -> PanOffset {
        self.pan
    }

    /// Returns the currently accepted frame projection, if one exists.
    #[must_use]
    pub const fn frame(self) -> Option<PreviewFrameProjection> {
        self.frame
    }

    /// Returns the current token used by a redraw callback.
    #[must_use]
    pub const fn redraw_token(self) -> RedrawToken {
        RedrawToken {
            generation: self.generation,
            serial: self.redraw_serial,
        }
    }

    /// Returns whether a queued redraw still describes the current projection.
    #[must_use]
    pub const fn accepts_redraw(self, token: RedrawToken) -> bool {
        self.redraw_token().generation.get() == token.generation.get()
            && self.redraw_token().serial == token.serial
    }

    /// Starts a new selected-photo projection and invalidates prior frame/redraw work.
    pub fn begin_generation(
        &mut self,
        generation: ViewportGeneration,
        ticket: PresentationTicket,
    ) -> bool {
        if generation.get() < self.generation.get() {
            return false;
        }
        self.generation = generation;
        self.expected_frame = Some(PreviewFrameKey::from_ticket(ticket));
        self.frame = None;
        self.zoom = ViewportZoom::Fit;
        self.pan = PanOffset::default();
        self.bump_redraw();
        true
    }

    /// Clears the frame and invalidates its redraw token without changing generation ordering.
    pub fn clear(&mut self) {
        self.expected_frame = None;
        self.frame = None;
        self.zoom = ViewportZoom::Fit;
        self.pan = PanOffset::default();
        self.bump_redraw();
    }

    /// Accepts a frame only when both its viewport generation and request identity match.
    pub fn accept_frame(
        &mut self,
        generation: ViewportGeneration,
        frame: &DisplayPresentationFrame,
    ) -> FrameProjectionResult {
        let key = PreviewFrameKey::from_ticket(frame.ticket());
        if generation != self.generation || self.expected_frame != Some(key) {
            return FrameProjectionResult::Stale;
        }
        self.frame = Some(PreviewFrameProjection::from_frame(frame));
        FrameProjectionResult::Applied(self.bump_redraw())
    }

    /// Changes zoom and resets pan when entering a fit-like mode.
    pub fn set_zoom(&mut self, zoom: ViewportZoom) -> bool {
        if self.zoom == zoom {
            if matches!(zoom, ViewportZoom::Small | ViewportZoom::Fit)
                && self.pan != PanOffset::default()
            {
                self.pan = PanOffset::default();
                self.bump_redraw();
                return true;
            }
            return false;
        }
        self.zoom = zoom;
        if matches!(zoom, ViewportZoom::Small | ViewportZoom::Fit) {
            self.pan = PanOffset::default();
        }
        self.bump_redraw();
        true
    }

    /// Applies a bounded pan delta.
    pub fn pan_by(&mut self, delta_x: i32, delta_y: i32) -> bool {
        if matches!(self.zoom, ViewportZoom::Small | ViewportZoom::Fit) {
            return false;
        }
        let pan = self.pan.adjust(delta_x, delta_y);
        if pan == self.pan {
            return false;
        }
        self.pan = pan;
        self.bump_redraw();
        true
    }

    /// Switches to the Darktable fit projection and centers the image.
    pub fn fit(&mut self) -> bool {
        self.set_zoom(ViewportZoom::Fit)
    }

    /// Projects the accepted frame into integer paint geometry.
    #[must_use]
    pub fn geometry(self, viewport: ViewportSize) -> Option<ProjectedImage> {
        let frame = self.frame?;
        let scale_milli = self.zoom.scale_milli(viewport, frame.dimensions());
        let width = scaled_dimension(frame.dimensions().width(), scale_milli);
        let height = scaled_dimension(frame.dimensions().height(), scale_milli);
        Some(ProjectedImage {
            x: centered_offset(viewport.width(), width, self.pan.x()),
            y: centered_offset(viewport.height(), height, self.pan.y()),
            width,
            height,
            scale_milli,
        })
    }

    /// Projects into a full widget allocation while reserving an equal image border on all sides.
    ///
    /// Darktable computes zoom from `orig_width - 2 * border_size` and
    /// `orig_height - 2 * border_size`, then offsets the painted image by that border.
    #[must_use]
    pub fn geometry_in_allocation(
        self,
        allocation: ViewportSize,
        border: u32,
    ) -> Option<ProjectedImage> {
        let viewport = allocation.inset(border)?;
        let mut projected = self.geometry(viewport)?;
        let border = i32::try_from(border).ok()?;
        projected.x = projected.x.saturating_add(border);
        projected.y = projected.y.saturating_add(border);
        Some(projected)
    }

    fn bump_redraw(&mut self) -> RedrawToken {
        self.redraw_serial = self.redraw_serial.saturating_add(1);
        self.redraw_token()
    }
}

/// Integer geometry for painting one preview texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectedImage {
    /// Horizontal paint origin relative to the viewport.
    pub x: i32,
    /// Vertical paint origin relative to the viewport.
    pub y: i32,
    /// Projected image width.
    pub width: u32,
    /// Projected image height.
    pub height: u32,
    /// Scale in thousandths.
    pub scale_milli: u32,
}

fn ratio_milli(
    viewport_width: u32,
    image_width: u32,
    viewport_height: u32,
    image_height: u32,
    fill: bool,
) -> u32 {
    let width = (u64::from(viewport_width) * u64::from(SCALE_DENOMINATOR)) / u64::from(image_width);
    let height =
        (u64::from(viewport_height) * u64::from(SCALE_DENOMINATOR)) / u64::from(image_height);
    let ratio = if fill {
        width.max(height)
    } else {
        width.min(height)
    };
    u32::try_from(ratio).unwrap_or(u32::MAX).max(1)
}

fn scaled_dimension(value: u32, scale_milli: u32) -> u32 {
    let scaled = (u64::from(value) * u64::from(scale_milli) + u64::from(SCALE_DENOMINATOR / 2))
        / u64::from(SCALE_DENOMINATOR);
    u32::try_from(scaled)
        .unwrap_or(MAX_PROJECTED_DIMENSION)
        .clamp(1, MAX_PROJECTED_DIMENSION)
}

fn centered_offset(viewport: u32, content: u32, pan: i16) -> i32 {
    let centered = (i64::from(viewport) - i64::from(content)) / 2;
    let overflow = i64::from(content.saturating_sub(viewport));
    let pan_offset = overflow * i64::from(pan) / (2 * i64::from(MAX_PAN));
    i32::try_from(centered - pan_offset).unwrap_or_else(|_| {
        if centered.is_negative() {
            i32::MIN
        } else {
            i32::MAX
        }
    })
}

#[cfg(test)]
mod tests {
    use rusttable_display_profile::ProfileSelection;

    use super::*;
    use crate::viewport_presentation::{DisplayPresentationRequest, PresentationMode};

    fn ticket(photo: u128, profile_generation: u64) -> PresentationTicket {
        PresentationTicket::new(DisplayPresentationRequest::new(
            PhotoId::new(photo).expect("test photo"),
            Revision::from_u64(7),
            None,
            PresentationGeneration::new(profile_generation),
            PresentationMode::Sdr,
        ))
    }

    fn frame(ticket: PresentationTicket) -> DisplayPresentationFrame {
        DisplayPresentationFrame::new(
            ticket,
            PreviewDimensions::new(400, 200).expect("test dimensions"),
            vec![0; 400 * 200 * 4],
            PresentationStatus::Ready {
                mode: PresentationMode::Sdr,
                profile: ProfileSelection::OperatingSystem,
            },
        )
        .expect("test frame")
    }

    #[test]
    fn zoom_percent_is_bounded_before_projection() {
        assert_eq!(ZoomPercent::new(0), Err(ZoomPercentError::BelowMinimum));
        assert_eq!(ZoomPercent::new(1_601), Err(ZoomPercentError::AboveMaximum));
        assert_eq!(
            ZoomPercent::new(1_600).expect("maximum"),
            ZoomPercent(1_600)
        );
    }

    #[test]
    fn pan_is_clamped_and_saturating() {
        let pan = PanOffset::new(i32::MAX, i32::MIN).adjust(i32::MAX, i32::MIN);
        assert_eq!(pan, PanOffset::new(i32::from(MAX_PAN), -i32::from(MAX_PAN)));
    }

    #[test]
    fn fit_projection_is_deterministic_and_preserves_aspect_ratio() {
        let mut state = ViewportCanvasState::default();
        let current = ticket(9, 3);
        assert!(state.begin_generation(ViewportGeneration::new(4), current));
        assert_eq!(
            state.accept_frame(ViewportGeneration::new(4), &frame(current)),
            FrameProjectionResult::Applied(RedrawToken {
                generation: ViewportGeneration::new(4),
                serial: 2,
            })
        );
        let geometry = state
            .geometry(ViewportSize::new(800, 800).expect("viewport"))
            .expect("geometry");
        assert_eq!((geometry.width, geometry.height), (800, 400));
        assert_eq!((geometry.x, geometry.y), (0, 200));
    }

    #[test]
    fn fit_is_not_capped_by_the_manual_zoom_menu() {
        let viewport = ViewportSize::new(800, 600).expect("viewport");
        let image = PreviewDimensions::new(20, 10).expect("image");

        assert_eq!(ViewportZoom::Fit.scale_milli(viewport, image), 40_000);
    }

    #[test]
    fn projection_rounds_instead_of_losing_the_limiting_fit_pixel() {
        let viewport = ViewportSize::new(10, 10).expect("viewport");
        let image = PreviewDimensions::new(3, 2).expect("image");
        let scale = ViewportZoom::Fit.scale_milli(viewport, image);

        assert_eq!(scaled_dimension(image.width(), scale), 10);
    }

    #[test]
    fn full_allocation_reserves_darktable_border_symmetrically() {
        let mut state = ViewportCanvasState::default();
        let current = ticket(12, 3);
        assert!(state.begin_generation(ViewportGeneration::new(4), current));
        assert!(matches!(
            state.accept_frame(ViewportGeneration::new(4), &frame(current)),
            FrameProjectionResult::Applied(_)
        ));

        let geometry = state
            .geometry_in_allocation(ViewportSize::new(820, 620).expect("allocation"), 10)
            .expect("geometry");
        assert_eq!((geometry.width, geometry.height), (800, 400));
        assert_eq!((geometry.x, geometry.y), (10, 110));
    }

    #[test]
    fn positive_pan_moves_the_view_toward_the_image_end() {
        let mut state = ViewportCanvasState::default();
        let current = ticket(13, 3);
        assert!(state.begin_generation(ViewportGeneration::new(4), current));
        assert!(matches!(
            state.accept_frame(ViewportGeneration::new(4), &frame(current)),
            FrameProjectionResult::Applied(_)
        ));
        assert!(state.set_zoom(ViewportZoom::Percent(ZoomPercent::new(400).expect("zoom"))));
        assert!(state.pan_by(i32::from(MAX_PAN), 0));

        let geometry = state
            .geometry(ViewportSize::new(800, 400).expect("viewport"))
            .expect("geometry");
        assert_eq!(geometry.x, -800);
        assert_eq!(geometry.y, -200);
    }

    #[test]
    fn fit_and_small_cannot_retain_pan() {
        let mut state = ViewportCanvasState::default();
        assert!(!state.pan_by(100, 100));
        assert_eq!(state.pan(), PanOffset::default());

        assert!(state.set_zoom(ViewportZoom::Percent(ZoomPercent::new(200).expect("zoom"))));
        assert!(state.pan_by(100, 100));
        assert!(state.set_zoom(ViewportZoom::Fill));
        assert_eq!(state.pan(), PanOffset::new(100, 100));
        assert!(state.fit());
        assert_eq!(state.pan(), PanOffset::default());
    }

    #[test]
    fn stale_frame_cannot_replace_current_projection_or_redraw_token() {
        let mut state = ViewportCanvasState::default();
        let old = ticket(1, 1);
        let current = ticket(1, 2);
        assert!(state.begin_generation(ViewportGeneration::new(1), old));
        assert!(state.begin_generation(ViewportGeneration::new(2), current));
        let before = state.redraw_token();
        assert_eq!(
            state.accept_frame(ViewportGeneration::new(1), &frame(old)),
            FrameProjectionResult::Stale
        );
        assert_eq!(state.frame(), None);
        assert_eq!(state.redraw_token(), before);
        assert!(state.accepts_redraw(before));
    }

    #[test]
    fn changed_view_invalidates_queued_redraw_token() {
        let mut state = ViewportCanvasState::default();
        let token = state.redraw_token();
        assert!(state.set_zoom(ViewportZoom::Percent(ZoomPercent::new(100).expect("zoom"))));
        assert!(!state.accepts_redraw(token));
        assert!(state.redraw_token().serial() > token.serial());
    }

    #[test]
    fn frame_projection_exposes_status_without_pixel_payload() {
        let current = ticket(11, 5);
        let preview = PreviewFrameProjection::from_frame(&frame(current));
        assert_eq!(preview.key(), PreviewFrameKey::from_ticket(current));
        assert_eq!(
            preview.dimensions(),
            PreviewDimensions::new(400, 200).expect("dimensions")
        );
        assert!(matches!(preview.status(), PresentationStatus::Ready { .. }));
    }
}
