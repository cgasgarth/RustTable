//! GTK4 AI model-management surface and its service boundary.

mod controller;
mod model;
mod view;

pub use controller::{AiModelsController, AiModelsControllerError};
pub use model::*;
pub use view::AiModelsPanel;
