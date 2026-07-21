//! Darkroom RGB AI denoise controls and their application-service boundary.

mod controller;
mod model;
mod view;

pub use controller::{RgbDenoiseController, RgbDenoiseControllerError};
pub use model::*;
pub use view::RgbDenoisePanel;
