use unicode_normalization::UnicodeNormalization;

/// Versioned search normalization contract; display collation intentionally remains separate.
pub const SEARCH_PIPELINE_VERSION: &str = "nfkc-default-lowercase-v1";

/// Normalizes user search text without changing the persisted source value.
#[must_use]
pub fn normalize_search(value: &str) -> String {
    value
        .nfkc()
        .map(|(character, _alignment)| character)
        .flat_map(char::to_lowercase)
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::{SEARCH_PIPELINE_VERSION, normalize_search};

    #[test]
    fn normalization_is_case_insensitive_and_composition_stable() {
        assert_eq!(normalize_search("ÉCOLE"), normalize_search("e\u{301}cole"));
        assert_eq!(SEARCH_PIPELINE_VERSION, "nfkc-default-lowercase-v1");
    }
}
