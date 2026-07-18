#![forbid(unsafe_code)]
#![doc = "Deterministic darktable feature-parity discovery and manifest validation."]

mod mapping;
mod model;
mod scan;
mod validate;

pub use model::{Capability, Manifest, Override, SummaryGroup};
pub use scan::{ScanError, scan_darktable, scan_darktable_with_overrides};
pub use validate::{parse_manifest, render_manifest, validate_manifest};
