#![forbid(unsafe_code)]
#![doc = "Core domain foundation for the `RustTable` rewrite."]
#![doc = "The core crate has no normal dependencies; catalog code may depend on it, never the reverse."]

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
