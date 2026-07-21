//! Shared selected-photo DTO retained for the RGB denoise darkroom service seam.

use rusttable_core::PhotoId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotoSourceKind {
    BayerRaw,
    XTransRaw,
    LinearRaw,
    Raster,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoSelection {
    photo: Option<PhotoId>,
    source_kind: Option<PhotoSourceKind>,
    renderable: bool,
    revision: u64,
    multiple: bool,
}

impl PhotoSelection {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            photo: None,
            source_kind: None,
            renderable: false,
            revision: 0,
            multiple: false,
        }
    }

    #[must_use]
    pub const fn single(
        photo: PhotoId,
        source_kind: PhotoSourceKind,
        renderable: bool,
        revision: u64,
    ) -> Self {
        Self {
            photo: Some(photo),
            source_kind: Some(source_kind),
            renderable,
            revision,
            multiple: false,
        }
    }

    #[must_use]
    pub const fn multiple() -> Self {
        Self {
            photo: None,
            source_kind: None,
            renderable: false,
            revision: 0,
            multiple: true,
        }
    }

    #[must_use]
    pub const fn photo(&self) -> Option<PhotoId> {
        self.photo
    }
    #[must_use]
    pub const fn source_kind(&self) -> Option<PhotoSourceKind> {
        self.source_kind
    }
    #[must_use]
    pub const fn renderable(&self) -> bool {
        self.renderable
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn is_multiple(&self) -> bool {
        self.multiple
    }
}

pub use model_aliases::{
    NeuralRestoreAction, NeuralRestoreController, NeuralRestoreControllerError,
};

mod model_aliases {
    pub use crate::rgb_denoise::{
        RgbDenoiseAction as NeuralRestoreAction, RgbDenoiseController as NeuralRestoreController,
        RgbDenoiseControllerError as NeuralRestoreControllerError,
    };
}
