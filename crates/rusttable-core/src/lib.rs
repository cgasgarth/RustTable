#![forbid(unsafe_code)]
#![doc = "Core foundation for the `RustTable` rewrite."]

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
