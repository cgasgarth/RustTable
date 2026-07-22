//! Native GTK thumbnail surfaces shared by lighttable and filmstrip.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::presentation::Rgba8PreviewMetadata;

use super::preview::{PhotoPreviewTextureError, texture_parameters};

const MAX_THUMBNAIL_RGBA8_BYTES: usize = 2 * 1024 * 1024;

/// A bounded GTK paintable with deterministic loading, ready, and failed states.
#[derive(Clone)]
pub struct ThumbnailSurface {
    root: gtk4::Overlay,
    picture: gtk4::Picture,
    placeholder: gtk4::Label,
    texture: Rc<RefCell<Option<gtk4::gdk::Texture>>>,
    state: Rc<RefCell<ThumbnailState>>,
    target_width: u32,
    target_height: u32,
}

/// Display state retained when the lighttable rebuilds its GTK children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ThumbnailState {
    Loading,
    Ready(Rgba8PreviewMetadata),
    Unavailable,
    Failed,
}

#[allow(dead_code)] // status helpers are consumed by the accessibility adapter
impl ThumbnailState {
    #[must_use]
    pub(crate) const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    #[must_use]
    pub(crate) const fn is_terminal(&self) -> bool {
        !matches!(self, Self::Loading)
    }

    #[must_use]
    pub(crate) const fn status_text(&self) -> &'static str {
        match self {
            Self::Loading => "loading…",
            Self::Ready(_) => "thumbnail ready",
            Self::Unavailable => "preview unavailable",
            Self::Failed => "preview failed",
        }
    }
}

impl ThumbnailSurface {
    #[must_use]
    pub fn new(id: &str, accessible_name: &str, width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let picture = gtk4::Picture::new();
        picture.set_widget_name(&format!("{id}-image"));
        picture.set_content_fit(gtk4::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_size_request(width, height);
        picture.set_accessible_role(gtk4::AccessibleRole::Img);
        picture.update_property(&[Property::Label(accessible_name)]);

        let placeholder = gtk4::Label::new(Some("loading…"));
        placeholder.set_widget_name(&format!("{id}-placeholder"));
        placeholder.add_css_class("dt_thumbnail_placeholder");
        placeholder.set_halign(gtk4::Align::Center);
        placeholder.set_valign(gtk4::Align::Center);

        let root = gtk4::Overlay::new();
        root.set_widget_name(id);
        root.add_css_class("dt_thumbnail_surface");
        root.set_size_request(width, height);
        root.set_overflow(gtk4::Overflow::Hidden);
        root.set_child(Some(&picture));
        root.add_overlay(&placeholder);
        Self {
            root,
            picture,
            placeholder,
            texture: Rc::new(RefCell::new(None)),
            state: Rc::new(RefCell::new(ThumbnailState::Loading)),
            target_width: u32::try_from(width).unwrap_or(1),
            target_height: u32::try_from(height).unwrap_or(1),
        }
    }

    #[must_use]
    pub fn widget(&self) -> &gtk4::Overlay {
        &self.root
    }

    /// Installs already validated RGBA8 pixels without decoding on GTK's thread.
    pub fn set_rgba8(
        &self,
        metadata: &Rgba8PreviewMetadata,
    ) -> Result<(), PhotoPreviewTextureError> {
        let bounded = fit_metadata(metadata, self.target_width, self.target_height);
        if bounded.pixels().len() > MAX_THUMBNAIL_RGBA8_BYTES {
            return Err(PhotoPreviewTextureError::PixelPayloadTooLarge);
        }
        let (width, height, stride) = texture_parameters(bounded.dimensions())?;
        let bytes = gtk4::glib::Bytes::from_owned(bounded.pixels().to_owned());
        let texture: gtk4::gdk::Texture = gtk4::gdk::MemoryTexture::new(
            width,
            height,
            gtk4::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            stride,
        )
        .upcast();
        self.picture.set_paintable(Some(&texture));
        self.placeholder.set_visible(false);
        self.texture.replace(Some(texture));
        self.state.replace(ThumbnailState::Ready(bounded));
        Ok(())
    }

    pub(crate) fn set_state(&self, state: &ThumbnailState) -> Result<(), PhotoPreviewTextureError> {
        match state {
            ThumbnailState::Loading => {
                self.set_loading();
                Ok(())
            }
            ThumbnailState::Ready(metadata) => self.set_rgba8(metadata),
            ThumbnailState::Unavailable => {
                self.set_unavailable();
                Ok(())
            }
            ThumbnailState::Failed => {
                self.set_failed();
                Ok(())
            }
        }
    }

    pub fn set_loading(&self) {
        self.picture.set_paintable(None::<&gtk4::gdk::Texture>);
        self.placeholder.set_text("loading…");
        self.placeholder.set_visible(true);
        self.texture.replace(None);
        self.state.replace(ThumbnailState::Loading);
    }

    pub fn set_unavailable(&self) {
        self.picture.set_paintable(None::<&gtk4::gdk::Texture>);
        self.placeholder.set_text("preview unavailable");
        self.placeholder.set_visible(true);
        self.texture.replace(None);
        self.state.replace(ThumbnailState::Unavailable);
    }

    pub fn set_failed(&self) {
        self.picture.set_paintable(None::<&gtk4::gdk::Texture>);
        self.placeholder.set_text("preview failed");
        self.placeholder.set_visible(true);
        self.texture.replace(None);
        self.state.replace(ThumbnailState::Failed);
    }

    #[must_use]
    pub(crate) fn state(&self) -> ThumbnailState {
        self.state.borrow().clone()
    }
}

fn fit_metadata(
    metadata: &Rgba8PreviewMetadata,
    target_width: u32,
    target_height: u32,
) -> Rgba8PreviewMetadata {
    let source = metadata.dimensions();
    if source.width() <= target_width && source.height() <= target_height {
        return metadata.clone();
    }
    let (width, height) = if u64::from(source.width()) * u64::from(target_height)
        > u64::from(source.height()) * u64::from(target_width)
    {
        (
            target_width,
            u32::try_from(
                (u64::from(source.height()) * u64::from(target_width) / u64::from(source.width()))
                    .max(1),
            )
            .expect("fitted thumbnail height is bounded by the target"),
        )
    } else {
        (
            u32::try_from(
                (u64::from(source.width()) * u64::from(target_height) / u64::from(source.height()))
                    .max(1),
            )
            .expect("fitted thumbnail width is bounded by the target"),
            target_height,
        )
    };
    let pixel_bytes = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .expect("fitted thumbnail payload fits usize");
    let mut pixels = vec![0_u8; pixel_bytes];
    for y in 0..height {
        let source_y = y * source.height() / height;
        for x in 0..width {
            let source_x = x * source.width() / width;
            let source_offset = usize::try_from(
                (u64::from(source_y) * u64::from(source.width()) + u64::from(source_x)) * 4,
            )
            .expect("validated preview offset fits usize");
            let target_offset =
                usize::try_from((u64::from(y) * u64::from(width) + u64::from(x)) * 4)
                    .expect("fitted thumbnail offset fits usize");
            pixels[target_offset..target_offset + 4]
                .copy_from_slice(&metadata.pixels()[source_offset..source_offset + 4]);
        }
    }
    Rgba8PreviewMetadata::new(
        crate::presentation::PreviewDimensions::new(width, height)
            .expect("fitted thumbnail dimensions stay non-zero"),
        metadata.status().clone(),
        pixels,
    )
    .expect("fitted thumbnail remains a valid bounded RGBA8 surface")
}

/// The two synchronized native thumbnail surfaces for one photo.
#[derive(Clone)]
pub struct ThumbnailPair {
    lighttable: ThumbnailSurface,
    filmstrip: ThumbnailSurface,
}

impl ThumbnailPair {
    #[must_use]
    pub const fn new(lighttable: ThumbnailSurface, filmstrip: ThumbnailSurface) -> Self {
        Self {
            lighttable,
            filmstrip,
        }
    }

    pub fn set_rgba8(
        &self,
        metadata: &Rgba8PreviewMetadata,
    ) -> Result<(), PhotoPreviewTextureError> {
        self.lighttable.set_rgba8(metadata)?;
        self.filmstrip.set_rgba8(metadata)
    }

    pub(crate) fn set_state(&self, state: &ThumbnailState) -> Result<(), PhotoPreviewTextureError> {
        self.lighttable.set_state(state)?;
        self.filmstrip.set_state(state)
    }

    pub(crate) fn set_loading(&self) {
        self.lighttable.set_loading();
        self.filmstrip.set_loading();
    }

    pub(crate) fn set_unavailable(&self) {
        self.lighttable.set_unavailable();
        self.filmstrip.set_unavailable();
    }

    #[must_use]
    pub(crate) fn state(&self) -> ThumbnailState {
        self.lighttable.state()
    }

    pub(crate) fn filmstrip(&self) -> ThumbnailSurface {
        self.filmstrip.clone()
    }

    pub fn set_failed(&self) {
        self.lighttable.set_failed();
        self.filmstrip.set_failed();
    }
}

#[cfg(test)]
mod tests {
    use super::{ThumbnailState, fit_metadata};
    use crate::presentation::{PresentationText, PreviewDimensions, Rgba8PreviewMetadata};

    #[test]
    fn thumbnail_states_have_truthful_status_contracts() {
        assert_eq!(ThumbnailState::Loading.status_text(), "loading…");
        assert_eq!(
            ThumbnailState::Unavailable.status_text(),
            "preview unavailable"
        );
        assert_eq!(ThumbnailState::Failed.status_text(), "preview failed");
        assert!(!ThumbnailState::Loading.is_ready());
    }

    #[test]
    fn thumbnail_pixels_are_downscaled_to_the_requested_surface() {
        let dimensions = PreviewDimensions::new(180, 120).expect("valid source dimensions");
        let status = PresentationText::new("rendered").expect("valid status");
        let metadata = Rgba8PreviewMetadata::new(dimensions, status, vec![127; 180 * 120 * 4])
            .expect("valid source payload");

        let fitted = fit_metadata(&metadata, 78, 78);

        assert_eq!(fitted.dimensions().width(), 78);
        assert_eq!(fitted.dimensions().height(), 52);
        assert_eq!(fitted.pixels().len(), 78 * 52 * 4);
        assert!(fitted.pixels().iter().all(|channel| *channel == 127));
    }
}
