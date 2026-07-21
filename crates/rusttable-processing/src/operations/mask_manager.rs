use std::fmt;

/// Historical mask-manager state is retained separately from graph identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaskManagerParameters {
    selected_mask: u32,
}

impl MaskManagerParameters {
    #[must_use]
    pub const fn new(selected_mask: u32) -> Self {
        Self { selected_mask }
    }
    #[must_use]
    pub const fn selected_mask(self) -> u32 {
        self.selected_mask
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskManagerError {
    UnexpectedParameter,
}

impl fmt::Display for MaskManagerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("mask_manager accepts only its opaque historical state")
    }
}
impl std::error::Error for MaskManagerError {}
