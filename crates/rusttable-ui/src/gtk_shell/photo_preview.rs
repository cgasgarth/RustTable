//! Reusable GTK4 darkroom preview surface.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::PhotoId;

use crate::presentation::{self, PhotoDetailViewModel, SelectedPreviewState};

use super::{ThemeRole, apply_theme_role};

/// The central darkroom image surface and its typed metadata presentation.
///
/// The widget deliberately accepts a [`gdk4::Texture`] rather than image bytes.
/// Preview services own decoding and color conversion; this surface owns only
/// displaying the resulting paintable and the typed state that describes it.
#[derive(Clone)]
pub struct PhotoPreview {
    root: gtk4::Box,
    canvas: gtk4::Picture,
    placeholder: gtk4::Label,
    title: gtk4::Label,
    status: gtk4::Label,
    dimensions: gtk4::Label,
    facts: gtk4::Grid,
    texture: Rc<RefCell<Option<gtk4::gdk::Texture>>>,
    photo_id: Rc<RefCell<Option<PhotoId>>>,
}

/// Errors raised while adapting validated RGBA8 presentation data to a GTK texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoPreviewTextureError {
    WidthTooLarge,
    HeightTooLarge,
    StrideOverflow,
}

impl PhotoPreview {
    /// Creates a darktable-like darkroom canvas with a compact detail/status bar.
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_widget_name("darkroom-photo-preview");
        apply_theme_role(&root, ThemeRole::Darkroom);
        root.set_hexpand(true);
        root.set_vexpand(true);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        header.set_widget_name("darkroom-preview-header");
        header.set_margin_top(8);
        header.set_margin_bottom(8);
        header.set_margin_start(12);
        header.set_margin_end(12);

        let title = gtk4::Label::new(Some("darkroom"));
        title.set_widget_name("darkroom-preview-title");
        title.set_halign(gtk4::Align::Start);
        title.set_hexpand(true);
        title.add_css_class("title-3");

        let status = gtk4::Label::new(Some("preview unavailable"));
        status.set_widget_name("darkroom-preview-status");
        status.set_halign(gtk4::Align::End);
        status.add_css_class("dim-label");

        let dimensions = gtk4::Label::new(None);
        dimensions.set_widget_name("darkroom-preview-dimensions");
        dimensions.set_halign(gtk4::Align::End);
        dimensions.add_css_class("dim-label");

        header.append(&title);
        header.append(&status);
        header.append(&dimensions);

        let canvas = gtk4::Picture::new();
        canvas.set_widget_name("darkroom-image-canvas");
        apply_theme_role(&canvas, ThemeRole::Darkroom);
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_can_shrink(true);
        canvas.set_content_fit(gtk4::ContentFit::Contain);

        let placeholder = gtk4::Label::new(Some("preview unavailable"));
        placeholder.set_widget_name("darkroom-preview-placeholder");
        placeholder.add_css_class("dim-label");
        placeholder.set_halign(gtk4::Align::Center);
        placeholder.set_valign(gtk4::Align::Center);
        placeholder.set_hexpand(true);
        placeholder.set_vexpand(true);

        let overlay = gtk4::Overlay::new();
        overlay.set_widget_name("darkroom-preview-overlay");
        overlay.set_hexpand(true);
        overlay.set_vexpand(true);
        overlay.set_child(Some(&canvas));
        overlay.add_overlay(&placeholder);

        let canvas_frame = gtk4::Frame::new(None);
        canvas_frame.set_widget_name("darkroom-preview-canvas-frame");
        canvas_frame.set_hexpand(true);
        canvas_frame.set_vexpand(true);
        canvas_frame.set_child(Some(&overlay));

        let facts = gtk4::Grid::new();
        facts.set_widget_name("darkroom-preview-facts");
        facts.set_column_spacing(12);
        facts.set_row_spacing(4);
        facts.set_margin_top(8);
        facts.set_margin_bottom(8);
        facts.set_margin_start(12);
        facts.set_margin_end(12);

        root.append(&header);
        root.append(&canvas_frame);
        root.append(&facts);

        Self {
            root,
            canvas,
            placeholder,
            title,
            status,
            dimensions,
            facts,
            texture: Rc::new(RefCell::new(None)),
            photo_id: Rc::new(RefCell::new(None)),
        }
    }

    /// Returns the root GTK widget for insertion into a darkroom workspace.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Replaces the typed photo detail and clears any texture belonging to the previous photo.
    pub fn set_detail(&self, detail: &PhotoDetailViewModel) {
        if *self.photo_id.borrow() != Some(detail.id()) {
            self.clear_texture();
        }
        self.photo_id.replace(Some(detail.id()));
        self.title.set_text(detail.title().as_str());
        self.render_preview_state(detail.selected_preview());
        self.render_facts(detail);
    }

    /// Shows that the selected photo is being rendered without blocking the GTK main loop.
    pub fn set_loading(&self) {
        self.clear_texture();
        self.status.set_text("loading preview");
        self.dimensions.set_text("");
        self.placeholder.set_text("loading preview");
        self.placeholder.set_visible(true);
    }

    /// Shows the exact display-safe failure supplied by the application preview controller.
    pub fn set_failure(&self, message: &str) {
        self.clear_texture();
        self.status.set_text(message);
        self.dimensions.set_text("");
        self.placeholder.set_text(message);
        self.placeholder.set_visible(true);
    }

    /// Installs or replaces the rendered preview texture supplied by the application service.
    pub fn set_texture(&self, texture: &gtk4::gdk::Texture) {
        self.canvas.set_paintable(Some(texture));
        self.placeholder.set_visible(false);
        self.texture.replace(Some(texture.clone()));
    }

    /// Converts validated RGBA8 presentation pixels into a GTK memory texture and installs it.
    ///
    /// The application/rendering boundary owns decoding and color decisions. This method is the
    /// small typed hand-off into GTK: it accepts only the already-validated presentation contract,
    /// preserves its status and dimensions, and returns the installed paintable for callers that
    /// need to retain or forward it.
    ///
    /// # Errors
    ///
    /// Returns an error when the dimensions cannot be represented by the GTK texture API or the
    /// row stride overflows.
    pub fn set_rgba8(
        &self,
        metadata: &presentation::Rgba8PreviewMetadata,
    ) -> Result<gtk4::gdk::Texture, PhotoPreviewTextureError> {
        let (width, height, stride) = texture_parameters(metadata.dimensions())?;
        let bytes = gtk4::glib::Bytes::from_owned(metadata.pixels().to_owned());
        let memory_texture = gtk4::gdk::MemoryTexture::new(
            width,
            height,
            gtk4::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            stride,
        );
        let texture: gtk4::gdk::Texture = memory_texture.upcast();
        self.set_texture(&texture);
        self.status.set_text(metadata.status().as_str());
        self.dimensions.set_text(&format_dimensions(metadata));
        self.placeholder.set_text("rendering preview");
        Ok(texture)
    }

    /// Removes the rendered texture while retaining the typed status presentation.
    pub fn clear_texture(&self) {
        self.canvas.set_paintable(None::<&gtk4::gdk::Texture>);
        self.placeholder.set_visible(true);
        self.texture.replace(None);
    }

    /// Returns the currently installed texture, if the preview service supplied one.
    #[must_use]
    pub fn texture(&self) -> Option<gtk4::gdk::Texture> {
        self.texture.borrow().clone()
    }

    /// Returns the title label used by the darkroom surface.
    #[must_use]
    pub fn title_label(&self) -> &gtk4::Label {
        &self.title
    }

    /// Returns the status label used by the darkroom surface.
    #[must_use]
    pub fn status_label(&self) -> &gtk4::Label {
        &self.status
    }

    fn render_preview_state(&self, state: &SelectedPreviewState) {
        match state {
            SelectedPreviewState::Loading => {
                self.status.set_text("loading preview");
                self.dimensions.set_text("");
                self.placeholder.set_text("loading preview");
            }
            SelectedPreviewState::Ready(metadata) => {
                self.status.set_text(metadata.status().as_str());
                self.dimensions.set_text(&format_dimensions(metadata));
                self.placeholder.set_text("rendering preview");
            }
            SelectedPreviewState::Unavailable => {
                self.status.set_text("preview unavailable");
                self.dimensions.set_text("");
                self.placeholder.set_text("preview unavailable");
            }
            SelectedPreviewState::Failed(failure) => {
                self.status.set_text(failure.detail().as_str());
                self.dimensions.set_text("");
                self.placeholder.set_text("preview failed");
            }
        }
    }

    fn render_facts(&self, detail: &PhotoDetailViewModel) {
        clear_children(&self.facts);
        for (row, fact) in detail.facts().enumerate() {
            let Ok(row) = i32::try_from(row) else {
                break;
            };
            let label = gtk4::Label::new(Some(fact.label().as_str()));
            label.set_halign(gtk4::Align::Start);
            label.add_css_class("dim-label");
            let value = gtk4::Label::new(Some(fact.value().as_str()));
            value.set_halign(gtk4::Align::Start);
            value.set_hexpand(true);
            self.facts.attach(&label, 0, row, 1, 1);
            self.facts.attach(&value, 1, row, 1, 1);
        }
    }
}

impl Default for PhotoPreview {
    fn default() -> Self {
        Self::new()
    }
}

fn format_dimensions(metadata: &presentation::Rgba8PreviewMetadata) -> String {
    format!(
        "{} × {}",
        metadata.dimensions().width(),
        metadata.dimensions().height()
    )
}

fn texture_parameters(
    dimensions: presentation::PreviewDimensions,
) -> Result<(i32, i32, usize), PhotoPreviewTextureError> {
    let width =
        i32::try_from(dimensions.width()).map_err(|_| PhotoPreviewTextureError::WidthTooLarge)?;
    let height =
        i32::try_from(dimensions.height()).map_err(|_| PhotoPreviewTextureError::HeightTooLarge)?;
    let stride = usize::try_from(dimensions.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(PhotoPreviewTextureError::StrideOverflow)?;
    Ok((width, height, stride))
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::{PhotoPreviewTextureError, format_dimensions, texture_parameters};
    use crate::presentation::{
        PresentationText, PreviewDimensions, Rgba8PreviewMetadata, SelectedPreviewState,
    };

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("test text is valid")
    }

    #[test]
    fn ready_state_dimensions_are_displayed_in_readable_form() {
        let dimensions = PreviewDimensions::new(3, 2).expect("non-zero dimensions");
        let metadata = Rgba8PreviewMetadata::new(dimensions, text("rendered"), vec![0; 24])
            .expect("matching RGBA8 byte count");

        assert_eq!(format_dimensions(&metadata), "3 × 2");
        assert!(matches!(
            SelectedPreviewState::Ready(metadata),
            SelectedPreviewState::Ready(_)
        ));
    }

    #[test]
    fn texture_parameters_use_rgba_stride_without_display_initialization() {
        let dimensions = PreviewDimensions::new(3, 2).expect("non-zero dimensions");
        let metadata = Rgba8PreviewMetadata::new(dimensions, text("rendered"), vec![0; 24])
            .expect("matching RGBA8 byte count");

        assert_eq!(texture_parameters(metadata.dimensions()), Ok((3, 2, 12)));
    }

    #[test]
    fn texture_dimension_errors_are_typed_before_gtk_initialization() {
        let width = u32::try_from(i64::from(i32::MAX) + 1).expect("valid u32 test width");
        let dimensions = PreviewDimensions::new(width, 1).expect("non-zero dimensions");

        assert_eq!(
            texture_parameters(dimensions),
            Err(PhotoPreviewTextureError::WidthTooLarge)
        );
    }
}
