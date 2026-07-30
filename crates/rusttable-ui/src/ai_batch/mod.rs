//! GTK4 review, preflight, progress, and recovery surface for AI batches.

mod controller;
mod model;
mod view;

pub use controller::{AiBatchController, AiBatchControllerError};
pub use model::*;
pub use view::AiBatchPanel;
