//! Typed GTK controls for the darkroom mask-manager integration seam.

mod controller;
mod model;
mod view;

pub use controller::{MaskManagerController, MaskManagerControllerError};
pub use model::*;
pub use view::MaskManagerPanel;
