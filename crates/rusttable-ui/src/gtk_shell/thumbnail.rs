//! Native GTK thumbnail surfaces shared by lighttable and filmstrip.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use crate::presentation::Rgba8PreviewMetadata;

use super::photo_preview::{PhotoPreviewTextureError, texture_parameters};

/// A bounded GTK paintable with deterministic loading, ready, and failed states.
#[derive(Clone)]
pub struct ThumbnailSurface {
    root: gtk4::Overlay,
    picture: gtk4::Picture,
    placeholder: gtk4::Label,
    texture: Rc<RefCell<Option<gtk4::gdk::Texture>>>,
}

impl ThumbnailSurface {
    #[must_use]
    pub fn new(id: &str, accessible_name: &str, width: i32, height: i32) -> Self {
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
        root.set_child(Some(&picture));
        root.add_overlay(&placeholder);
        Self {
            root,
            picture,
            placeholder,
            texture: Rc::new(RefCell::new(None)),
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
        let (width, height, stride) = texture_parameters(metadata.dimensions())?;
        let bytes = gtk4::glib::Bytes::from_owned(metadata.pixels().to_owned());
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
        Ok(())
    }

    pub fn set_failed(&self) {
        self.picture.set_paintable(None::<&gtk4::gdk::Texture>);
        self.placeholder.set_text("preview failed");
        self.placeholder.set_visible(true);
        self.texture.replace(None);
    }
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

    pub fn set_failed(&self) {
        self.lighttable.set_failed();
        self.filmstrip.set_failed();
    }
}
