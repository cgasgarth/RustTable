#![forbid(unsafe_code)]
#![doc = "Core domain foundation for the `RustTable` rewrite."]
#![doc = "The core crate has no normal dependencies; catalog code may depend on it, never the reverse."]

mod id;
mod model;
mod value;

pub use id::{AssetId, EditId, IdParseError, OperationId, PhotoId};
pub use model::{Asset, AssetRole, ByteLength, ContentHash, HashAlgorithm, Photo, PhotoBuildError};
pub use value::{FiniteF64, FiniteF64Error, Revision, RevisionOverflow};

/// IDs are intentionally nominally typed; a photo ID cannot stand in for an asset ID.
///
/// ```compile_fail
/// use rusttable_core::{AssetId, PhotoId};
///
/// fn takes_asset(_: AssetId) {}
///
/// let photo = PhotoId::new(1).expect("nonzero");
/// takes_asset(photo);
/// ```
/// Returns the stable product name used by the workspace smoke test.
#[must_use]
pub const fn product_name() -> &'static str {
    "RustTable"
}

#[cfg(test)]
mod tests {
    use super::product_name;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(product_name(), "RustTable");
    }
}
