#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateLimits {
    pub(crate) expressions: usize,
    pub(crate) literal_bytes: usize,
    pub(crate) output_bytes: usize,
    pub(crate) components: usize,
    pub(crate) expansions: usize,
    pub(crate) tag_values: usize,
    pub(crate) format_width: usize,
    pub(crate) nesting: usize,
}

impl Default for TemplateLimits {
    fn default() -> Self {
        Self {
            expressions: 256,
            literal_bytes: 4_096,
            output_bytes: 4_096,
            components: 64,
            expansions: 256,
            tag_values: 256,
            format_width: 64,
            nesting: 8,
        }
    }
}

impl TemplateLimits {
    #[must_use]
    pub const fn with_max_output_bytes(mut self, value: usize) -> Self {
        self.output_bytes = value;
        self
    }

    #[must_use]
    pub const fn with_max_component_count(mut self, value: usize) -> Self {
        self.components = value;
        self
    }

    #[must_use]
    pub const fn with_max_nesting(mut self, value: usize) -> Self {
        self.nesting = value;
        self
    }
}
