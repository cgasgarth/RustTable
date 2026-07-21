use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusttable_core::{EditId, PhotoId};
use rusttable_export::CollisionPolicy;
use rusttable_render::{PreviewBounds, RenderTarget};

pub const MAX_OUTPUT_EDGE: u32 = 16_384;
pub const MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;

/// The bounded output-size choices exposed by the GTK save surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSizeSelection {
    Original,
    Fit2048,
    Fit4096,
    CustomMaximum(u32),
}

impl ExportSizeSelection {
    /// Creates a custom maximum in the inclusive 1..=16,384 range.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero or oversized maximum.
    pub const fn custom_maximum(maximum_edge: u32) -> Result<Self, ExportSizeError> {
        if maximum_edge == 0 {
            return Err(ExportSizeError::TooSmall { maximum_edge });
        }
        if maximum_edge > MAX_OUTPUT_EDGE {
            return Err(ExportSizeError::TooLarge { maximum_edge });
        }
        Ok(Self::CustomMaximum(maximum_edge))
    }

    #[must_use]
    pub const fn into_size(self) -> ExportSize {
        match self {
            Self::Original => ExportSize::Original,
            Self::Fit2048 => ExportSize::FitMaximum(2_048),
            Self::Fit4096 => ExportSize::FitMaximum(4_096),
            Self::CustomMaximum(maximum_edge) => ExportSize::FitMaximum(maximum_edge),
        }
    }
}

/// Validation failures for custom GTK output-size input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSizeError {
    TooSmall { maximum_edge: u32 },
    TooLarge { maximum_edge: u32 },
}

impl std::fmt::Display for ExportSizeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { maximum_edge } => {
                write!(formatter, "custom PNG maximum {maximum_edge} is below 1")
            }
            Self::TooLarge { maximum_edge } => write!(
                formatter,
                "custom PNG maximum {maximum_edge} exceeds {MAX_OUTPUT_EDGE}"
            ),
        }
    }
}

impl std::error::Error for ExportSizeError {}

/// The immutable render-size request used by the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSize {
    Original,
    FitMaximum(u32),
}

impl ExportSize {
    #[must_use]
    pub const fn max_edge(self) -> u32 {
        match self {
            Self::Original => MAX_OUTPUT_EDGE,
            Self::FitMaximum(maximum_edge) => maximum_edge,
        }
    }

    ///
    /// # Panics
    ///
    /// Panics only if an invalid zero custom size is constructed without using
    /// [`ExportSizeSelection::custom_maximum`].
    #[must_use]
    pub fn render_target(self) -> RenderTarget {
        match self {
            Self::Original => RenderTarget::FullResolution,
            Self::FitMaximum(maximum_edge) => RenderTarget::PreviewFit(
                PreviewBounds::new(maximum_edge, maximum_edge)
                    .expect("validated export size is nonzero"),
            ),
        }
    }
}

/// The collision action selected before the immutable worker starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCollisionSelection {
    CreateNew,
    ReplaceExisting,
}

impl ExportCollisionSelection {
    #[must_use]
    pub const fn policy(self) -> CollisionPolicy {
        match self {
            Self::CreateNew => CollisionPolicy::CreateNew,
            Self::ReplaceExisting => CollisionPolicy::ReplaceExisting,
        }
    }
}

/// Immutable controls captured for one selected-photo save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSettings {
    size: ExportSize,
    collision: ExportCollisionSelection,
}

impl ExportSettings {
    #[must_use]
    pub const fn from_selection(
        size: ExportSizeSelection,
        collision: ExportCollisionSelection,
    ) -> Self {
        Self {
            size: size.into_size(),
            collision,
        }
    }

    #[must_use]
    pub const fn original() -> Self {
        Self::from_selection(
            ExportSizeSelection::Original,
            ExportCollisionSelection::CreateNew,
        )
    }

    #[must_use]
    pub const fn size(self) -> ExportSize {
        self.size
    }

    #[must_use]
    pub const fn collision(self) -> ExportCollisionSelection {
        self.collision
    }

    #[must_use]
    pub const fn with_collision(self, collision: ExportCollisionSelection) -> Self {
        Self { collision, ..self }
    }
}

/// A complete snapshot of the selected persisted photo and edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    catalog_path: PathBuf,
    source_root: PathBuf,
    photo_id: PhotoId,
    edit_id: EditId,
    destination: PathBuf,
    settings: ExportSettings,
}

impl ExportRequest {
    #[must_use]
    pub fn new(
        catalog_path: PathBuf,
        source_root: PathBuf,
        photo_id: PhotoId,
        edit_id: EditId,
        destination: PathBuf,
        settings: ExportSettings,
    ) -> Self {
        Self {
            catalog_path,
            source_root,
            photo_id,
            edit_id,
            destination,
            settings,
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn edit_id(&self) -> EditId {
        self.edit_id
    }

    #[must_use]
    pub fn catalog_path(&self) -> &Path {
        &self.catalog_path
    }

    #[must_use]
    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    pub const fn settings(&self) -> ExportSettings {
        self.settings
    }

    #[must_use]
    pub fn with_collision(&self, collision: ExportCollisionSelection) -> Self {
        Self {
            settings: self.settings.with_collision(collision),
            ..self.clone()
        }
    }
}

/// Cooperative cancellation shared by GTK and the export worker.
#[derive(Debug, Clone, Default)]
pub struct ExportCancellation(Arc<AtomicBool>);

impl ExportCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl PartialEq for ExportCancellation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ExportCancellation {}

#[cfg(test)]
mod tests {
    use super::{
        ExportCancellation, ExportCollisionSelection, ExportSettings, ExportSizeSelection,
    };

    #[test]
    fn custom_size_is_bounded_and_maps_to_a_fit_request() {
        assert!(ExportSizeSelection::custom_maximum(0).is_err());
        assert!(ExportSizeSelection::custom_maximum(16_385).is_err());
        assert_eq!(
            ExportSizeSelection::custom_maximum(1_024)
                .expect("custom size")
                .into_size()
                .max_edge(),
            1_024
        );
    }

    #[test]
    fn cancellation_is_shared_by_clones() {
        let cancellation = ExportCancellation::default();
        let clone = cancellation.clone();
        cancellation.cancel();
        assert!(clone.is_cancelled());
        assert_eq!(
            ExportSettings::from_selection(
                ExportSizeSelection::Original,
                ExportCollisionSelection::ReplaceExisting,
            )
            .collision(),
            ExportCollisionSelection::ReplaceExisting
        );
    }
}
