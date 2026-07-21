//! Bounded GPU-attempt recovery and deterministic output assembly.
//!
//! The model accepts planner-supplied candidates only, owns one fresh assembly
//! per attempt, permits at most two strictly smaller OOM retries, and exposes a
//! single exact CPU fallback/publication boundary.

#![allow(clippy::missing_errors_doc)]

mod assembly;
mod model;
mod session;

pub use assembly::{
    AssemblyPlan, AssemblyReceipt, AssemblyTile, CoverageError, CoverageReceipt, CoverageRect,
    OutputFragment,
};
pub use model::{
    AttemptFailure, AttemptFailureKind, AttemptId, AttemptOutcome, AttemptReceipt,
    AttemptResources, CleanupStatus, MAX_GPU_ATTEMPTS, MAX_OOM_RETRIES, PlanIdentity,
    PublicationBackend, PublicationReceipt, RecoveryAttemptPlan, RecoveryContext, RecoveryDecision,
    RecoveryError, RecoveryRequest, SnapshotIdentity, TileCandidate,
};
pub use session::RecoverySession;

#[cfg(test)]
mod tests;
