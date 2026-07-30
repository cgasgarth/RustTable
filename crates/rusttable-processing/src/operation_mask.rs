use std::fmt;

use rusttable_core::OperationId;
use rusttable_masks::{MaskExecutionError, MaskRaster, MaskRoi};

/// Immutable operation-to-raster bindings for one pixelpipe evaluation.
///
/// The mask graph is evaluated before operation execution and the resulting
/// planes are carried by this detached set. Keeping the binding keyed by the
/// operation identity prevents a mask from accidentally being reused after a
/// graph reorder or edit-generation change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OperationMaskSet {
    entries: Vec<(OperationId, MaskRaster)>,
}

impl OperationMaskSet {
    /// Creates a set and rejects duplicate operation bindings.
    ///
    /// # Errors
    ///
    /// Returns an error when an operation is bound more than once.
    pub fn from_entries(
        entries: impl IntoIterator<Item = (OperationId, MaskRaster)>,
    ) -> Result<Self, OperationMaskSetError> {
        let mut output = Self::default();
        for (operation_id, raster) in entries {
            if output
                .entries
                .iter()
                .any(|(candidate, _)| *candidate == operation_id)
            {
                return Err(OperationMaskSetError::DuplicateOperation { operation_id });
            }
            output.entries.push((operation_id, raster));
        }
        Ok(output)
    }

    #[must_use]
    pub fn mask_for(&self, operation_id: OperationId) -> Option<&MaskRaster> {
        self.entries
            .iter()
            .find(|(candidate, _)| *candidate == operation_id)
            .map(|(_, raster)| raster)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Crops every bound plane to the same tile ROI.
    ///
    /// # Errors
    ///
    /// Returns an error when the ROI is outside any bound raster.
    pub fn crop(&self, roi: MaskRoi) -> Result<Self, OperationMaskSetError> {
        Self::from_entries(
            self.entries
                .iter()
                .map(|(operation_id, raster)| {
                    raster.crop(roi).map(|cropped| (*operation_id, cropped))
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationMaskSetError {
    DuplicateOperation { operation_id: OperationId },
    Raster(MaskExecutionError),
}

impl fmt::Display for OperationMaskSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateOperation { operation_id } => {
                write!(
                    formatter,
                    "operation {operation_id} has duplicate mask bindings"
                )
            }
            Self::Raster(error) => write!(formatter, "operation mask raster failed: {error}"),
        }
    }
}

impl std::error::Error for OperationMaskSetError {}

impl From<MaskExecutionError> for OperationMaskSetError {
    fn from(error: MaskExecutionError) -> Self {
        Self::Raster(error)
    }
}
