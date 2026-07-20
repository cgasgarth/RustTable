use std::path::Path;

use rusttable_core::Edit;
use rusttable_image::{DecodeLimits, ImageInput};
use rusttable_image_io::FileImageInput;
use rusttable_render::{
    PreviewBounds, RenderOutput, RenderPlan, RenderTarget, SourceColorPolicy, render_edit_with_plan,
};

/// Production CPU preview boundary used by the application composition.
#[derive(Debug, Clone, Copy)]
pub struct PreviewService {
    limits: DecodeLimits,
    bounds: PreviewBounds,
}

impl PreviewService {
    #[must_use]
    pub const fn new(limits: DecodeLimits, bounds: PreviewBounds) -> Self {
        Self { limits, bounds }
    }

    /// Decodes and renders `source` through the exact immutable edit.
    ///
    /// # Errors
    ///
    /// Returns [`PreviewError`] if decoding the source or evaluating the CPU
    /// render plan fails.
    pub fn render(&self, source: &Path, edit: &Edit) -> Result<RenderOutput, PreviewError> {
        let input = FileImageInput::new(self.limits)
            .decode_path(source)
            .map_err(PreviewError::Decode)?;
        let plan =
            RenderPlan::for_source(input.dimensions(), RenderTarget::PreviewFit(self.bounds));
        render_edit_with_plan(
            edit,
            &input,
            SourceColorPolicy::AssumeSrgbWhenUnspecified,
            plan,
        )
        .map_err(PreviewError::Render)
    }
}

#[derive(Debug)]
pub enum PreviewError {
    Decode(rusttable_image::ImageInputError),
    Render(rusttable_render::RenderError),
}

impl std::fmt::Display for PreviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(error) => write!(formatter, "preview decode failed: {error}"),
            Self::Render(error) => write!(formatter, "preview render failed: {error}"),
        }
    }
}

impl std::error::Error for PreviewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::Render(error) => Some(error),
        }
    }
}
