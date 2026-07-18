#![forbid(unsafe_code)]
#![doc = "Catalog state and use cases for `RustTable`."]
#![doc = "Normal dependencies flow from this crate to `rusttable-core`; UI, I/O, and processing stay outside it."]

/// Identifies the catalog crate's current dependency boundary.
#[must_use]
pub const fn dependency_direction() -> &'static str {
    "rusttable-catalog -> rusttable-core"
}
mod command;
mod error;
mod state;

pub use command::CatalogCommand;
pub use error::CatalogError;
pub use state::CatalogState;
