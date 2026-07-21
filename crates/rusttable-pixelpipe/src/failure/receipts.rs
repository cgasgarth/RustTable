#![allow(clippy::missing_errors_doc)]

use rusttable_image::ImageDimensions;

use super::{CacheAction, Failure, FailureBackend, PolicyAction, PublicationAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptReceipt {
    pub(crate) number: u8,
    pub(crate) backend: FailureBackend,
    pub(crate) action: PolicyAction,
    pub(crate) failure: Failure,
    pub(crate) cleaned: bool,
    pub(crate) cache_action: CacheAction,
    pub(crate) publication_action: PublicationAction,
}

impl AttemptReceipt {
    #[must_use]
    pub fn new(
        number: u8,
        backend: FailureBackend,
        action: PolicyAction,
        failure: Failure,
    ) -> Self {
        Self {
            number,
            backend,
            action,
            cache_action: failure.cache_action(),
            publication_action: failure.publication_action(),
            failure,
            cleaned: false,
        }
    }
    #[must_use]
    pub const fn number(&self) -> u8 {
        self.number
    }
    #[must_use]
    pub const fn backend(&self) -> FailureBackend {
        self.backend
    }
    #[must_use]
    pub const fn action(&self) -> &PolicyAction {
        &self.action
    }
    #[must_use]
    pub const fn failure(&self) -> &Failure {
        &self.failure
    }
    #[must_use]
    pub const fn cleaned(&self) -> bool {
        self.cleaned
    }
    #[must_use]
    pub const fn cache_action(&self) -> CacheAction {
        self.cache_action
    }
    #[must_use]
    pub const fn publication_action(&self) -> PublicationAction {
        self.publication_action
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputExpectation {
    dimensions: ImageDimensions,
    pixel_len: u64,
}

impl OutputExpectation {
    #[must_use]
    pub const fn new(dimensions: ImageDimensions, pixel_len: u64) -> Self {
        Self {
            dimensions,
            pixel_len,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCandidate {
    dimensions: ImageDimensions,
    pixel_len: u64,
    finite: bool,
    lease_valid: bool,
    cancelled: bool,
}

impl OutputCandidate {
    #[must_use]
    pub const fn new(
        dimensions: ImageDimensions,
        pixel_len: u64,
        finite: bool,
        lease_valid: bool,
        cancelled: bool,
    ) -> Self {
        Self {
            dimensions,
            pixel_len,
            finite,
            lease_valid,
            cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputValidationReceipt {
    dimensions: ImageDimensions,
    pixel_len: u64,
}

impl OutputValidationReceipt {
    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn pixel_len(self) -> u64 {
        self.pixel_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputValidationError {
    DimensionsMismatch,
    LengthMismatch,
    NonFinite,
    LeaseInvalid,
    Cancelled,
}

pub struct OutputValidator;

impl OutputValidator {
    pub fn validate(
        expected: OutputExpectation,
        actual: OutputCandidate,
    ) -> Result<OutputValidationReceipt, OutputValidationError> {
        if actual.cancelled {
            return Err(OutputValidationError::Cancelled);
        }
        if !actual.lease_valid {
            return Err(OutputValidationError::LeaseInvalid);
        }
        if actual.dimensions != expected.dimensions {
            return Err(OutputValidationError::DimensionsMismatch);
        }
        if actual.pixel_len != expected.pixel_len {
            return Err(OutputValidationError::LengthMismatch);
        }
        if !actual.finite {
            return Err(OutputValidationError::NonFinite);
        }
        Ok(OutputValidationReceipt {
            dimensions: actual.dimensions,
            pixel_len: actual.pixel_len,
        })
    }
}
