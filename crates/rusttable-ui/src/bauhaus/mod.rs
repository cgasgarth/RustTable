//! GTK4 ports of Darktable's Bauhaus controls.

pub mod numeric_input;
pub mod slider;
pub(crate) mod slider_input;
pub mod slider_popup;

/// Updates Darktable's Bauhaus `zoom_step` preference for every slider.
pub fn set_slider_zoom_step(enabled: bool) {
    slider_input::set_zoom_step(enabled);
}
