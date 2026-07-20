#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    CancellationError, CancellationStage, CancellationToken, PipelineGeneration,
    PipelineSnapshotIdentity, PublicationGeneration, SourceIdentity, TargetIdentity,
};

/// A stable identity for one submitted request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(NonZeroU64);

impl RequestId {
    /// Creates a nonzero request identity.
    pub fn new(value: u64) -> Result<Self, PublicationError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(PublicationError::InvalidRequestId)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// The only destinations that may cross the stale-result publication boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicationTarget {
    Pixelpipe(TargetIdentity),
    Cache(TargetIdentity),
    Ui(TargetIdentity),
    File(TargetIdentity),
}

impl PublicationTarget {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Pixelpipe(_) => "pixelpipe",
            Self::Cache(_) => "cache",
            Self::Ui(_) => "ui",
            Self::File(_) => "file",
        }
    }
}

/// All immutable evidence needed to authorize one result publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicationIdentity {
    request: RequestId,
    generation: PipelineGeneration,
    target: PublicationTarget,
    source: SourceIdentity,
    snapshot: PipelineSnapshotIdentity,
    destination: PublicationGeneration,
}

impl PublicationIdentity {
    #[must_use]
    pub const fn new(
        request: RequestId,
        generation: PipelineGeneration,
        target: PublicationTarget,
        source: SourceIdentity,
        snapshot: PipelineSnapshotIdentity,
        destination: PublicationGeneration,
    ) -> Self {
        Self {
            request,
            generation,
            target,
            source,
            snapshot,
            destination,
        }
    }

    #[must_use]
    pub const fn request(self) -> RequestId {
        self.request
    }
    #[must_use]
    pub const fn generation(self) -> PipelineGeneration {
        self.generation
    }
    #[must_use]
    pub const fn target(self) -> PublicationTarget {
        self.target
    }
    #[must_use]
    pub const fn source(self) -> SourceIdentity {
        self.source
    }
    #[must_use]
    pub const fn snapshot(self) -> PipelineSnapshotIdentity {
        self.snapshot
    }
    #[must_use]
    pub const fn destination(self) -> PublicationGeneration {
        self.destination
    }
}

/// A current destination snapshot supplied at the last possible check.
pub type PublicationContext = PublicationIdentity;

/// Prevents a stale, mismatched, cancelled, or repeated publication.
#[derive(Debug)]
pub struct PublicationGate {
    identity: PublicationIdentity,
    token: CancellationToken,
    issued: AtomicBool,
}

impl PublicationGate {
    #[must_use]
    pub fn new(identity: PublicationIdentity, token: CancellationToken) -> Self {
        Self {
            identity,
            token,
            issued: AtomicBool::new(false),
        }
    }

    #[must_use]
    pub const fn identity(&self) -> PublicationIdentity {
        self.identity
    }

    /// Takes the one cache/UI/file/pixelpipe permit from this gate.
    pub fn authorize(
        &self,
        current: PublicationContext,
    ) -> Result<PublicationPermit, PublicationError> {
        self.verify(current, CancellationStage::Publication)?;
        if self.issued.swap(true, Ordering::AcqRel) {
            return Err(PublicationError::AlreadyIssued);
        }
        Ok(PublicationPermit {
            identity: self.identity,
            token: self.token.clone(),
            used: false,
            cleanup: None,
        })
    }

    fn verify(
        &self,
        current: PublicationContext,
        stage: CancellationStage,
    ) -> Result<(), PublicationError> {
        self.token
            .check(stage)
            .map_err(PublicationError::Cancelled)?;
        if current != self.identity {
            return Err(PublicationError::IdentityMismatch {
                expected: self.identity,
                actual: current,
            });
        }
        Ok(())
    }
}

/// A linear permit. It is consumed by `commit`, so a permit cannot be reused.
pub struct PublicationPermit {
    identity: PublicationIdentity,
    token: CancellationToken,
    used: bool,
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl fmt::Debug for PublicationPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PublicationPermit")
            .field("target", &self.identity.target().tag())
            .field("used", &self.used)
            .finish()
    }
}

impl PublicationPermit {
    #[must_use]
    pub const fn identity(&self) -> PublicationIdentity {
        self.identity
    }

    /// Installs cleanup for allocations owned by the pending publication.
    pub fn on_drop<F>(&mut self, cleanup: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.cleanup = Some(Box::new(cleanup));
    }

    /// Performs the final cancellation/identity check and then publishes once.
    pub fn commit<T, F>(
        mut self,
        current: PublicationContext,
        publish: F,
    ) -> Result<T, PublicationError>
    where
        F: FnOnce() -> T,
    {
        self.token
            .check(CancellationStage::Publication)
            .map_err(PublicationError::Cancelled)?;
        if current != self.identity {
            return Err(PublicationError::IdentityMismatch {
                expected: self.identity,
                actual: current,
            });
        }
        self.used = true;
        self.cleanup = None;
        Ok(publish())
    }
}

impl Drop for PublicationPermit {
    fn drop(&mut self) {
        if !self.used
            && let Some(cleanup) = self.cleanup.take()
        {
            cleanup();
        }
    }
}

/// Separate type aliases make cache and product publication authorization
/// visibly distinct even though both use the same verified identity model.
pub type CachePublicationPermit = PublicationPermit;
pub type ProductPublicationPermit = PublicationPermit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationError {
    InvalidRequestId,
    Cancelled(CancellationError),
    AlreadyIssued,
    PermitAlreadyUsed,
    IdentityMismatch {
        expected: PublicationIdentity,
        actual: PublicationIdentity,
    },
}

impl fmt::Display for PublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequestId => formatter.write_str("publication request ID is zero"),
            Self::Cancelled(error) => error.fmt(formatter),
            Self::AlreadyIssued => formatter.write_str("publication permit was already issued"),
            Self::PermitAlreadyUsed => formatter.write_str("publication permit was already used"),
            Self::IdentityMismatch { expected, actual } => write!(
                formatter,
                "publication identity mismatch: expected {:?}, got {:?}",
                expected, actual
            ),
        }
    }
}

impl std::error::Error for PublicationError {}

/// Ensures that a detached resource is retired before a stale result escapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRetirementReceipt {
    pub released: bool,
}

/// A small helper for GPU work that cannot be physically cancelled.
#[derive(Debug, Clone)]
pub struct GpuRetirement {
    token: CancellationToken,
}

impl GpuRetirement {
    #[must_use]
    pub fn new(token: CancellationToken) -> Self {
        Self { token }
    }

    pub fn retire(self) -> Result<ResourceRetirementReceipt, CancellationError> {
        let _ = self.token.check(CancellationStage::GpuRetirement);
        Ok(ResourceRetirementReceipt { released: true })
    }
}
