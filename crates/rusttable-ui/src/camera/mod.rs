//! GTK4 camera/tethered-capture workflow composed around the #469 service port.

mod controller;
mod model;
mod view;

pub use controller::{CameraAction, CameraController, CameraControllerError};
pub use model::CameraViewModel;
pub use view::{CAMERA_FOCUS_ORDER, CameraPanel};
