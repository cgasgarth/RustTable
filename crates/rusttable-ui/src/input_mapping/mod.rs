//! GTK4 shortcut and device-mapping editor.
//!
//! The module follows Darktable's accelerator/preferences responsibility split
//! while keeping device access in the future #512 service boundary.

mod fixtures;
mod gtk;
mod profile;
mod state;
mod types;

pub use fixtures::default_snapshot;
pub use gtk::InputMappingEditor;
pub use profile::{LocalProfileIo, ProfileIo, ProfileIoError};
pub use state::*;
pub use types::*;
