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
        self.render_preview_decoded(&input, edit)
    }

    /// Decodes immutable snapshot bytes and renders the exact edit.
    ///
    /// # Errors
    ///
    /// Returns a typed decode or CPU-render failure.
    pub fn render_bytes(&self, source: &[u8], edit: &Edit) -> Result<RenderOutput, PreviewError> {
        let input = FileImageInput::new(self.limits)
            .decode_bytes(source)
            .map_err(PreviewError::Decode)?;
        self.render_preview_decoded(&input, edit)
    }

    /// Decodes immutable snapshot bytes and renders the exact edit at source resolution.
    ///
    /// This is deliberately separate from [`Self::render_bytes`]: preview callers
    /// retain their bounded display target while export callers share the same
    /// production decode, edit, and color path without a second renderer.
    ///
    /// # Errors
    ///
    /// Returns a typed decode or CPU-render failure.
    pub fn render_full_resolution_bytes(
        &self,
        source: &[u8],
        edit: &Edit,
    ) -> Result<RenderOutput, PreviewError> {
        let input = FileImageInput::new(self.limits)
            .decode_bytes(source)
            .map_err(PreviewError::Decode)?;
        render_full_resolution(&input, edit)
    }

    fn render_preview_decoded(
        &self,
        input: &rusttable_image::DecodedImage,
        edit: &Edit,
    ) -> Result<RenderOutput, PreviewError> {
        let plan =
            RenderPlan::for_source(input.dimensions(), RenderTarget::PreviewFit(self.bounds));
        render_edit_with_plan(
            edit,
            input,
            SourceColorPolicy::AssumeSrgbWhenUnspecified,
            plan,
        )
        .map_err(PreviewError::Render)
    }
}

fn render_full_resolution(
    input: &rusttable_image::DecodedImage,
    edit: &Edit,
) -> Result<RenderOutput, PreviewError> {
    let plan = RenderPlan::for_source(input.dimensions(), RenderTarget::FullResolution);
    render_edit_with_plan(
        edit,
        input,
        SourceColorPolicy::AssumeSrgbWhenUnspecified,
        plan,
    )
    .map_err(PreviewError::Render)
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
