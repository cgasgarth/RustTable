use std::collections::{BTreeMap, VecDeque};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rusttable_core::OperationId;
use rusttable_gpu::{
    BasicAdjPointParameters, BasicPointColorSpace, BasicPointError, BasicPointOperation,
    BasicPointRequest, BilateralGridError, BilateralGridRequest, BloomError as GpuBloomError,
    BloomRequest, CancellationToken as GpuCancellationToken,
    ColorReconstructionError as GpuColorReconstructionError,
    ColorReconstructionPrecedence as GpuColorReconstructionPrecedence, ColorReconstructionRequest,
    ColorReconstructionRoi, ColorZonesError as GpuColorZonesError,
    ColorZonesMode as GpuColorZonesMode, ColorZonesRequest, ColorZonesRequestIdentity,
    ColorZonesSelection as GpuColorZonesSelection, GpuRuntime, GrainPointError, GrainPointRequest,
    bloom_transient_memory_bytes, colorreconstruction_transient_memory_bytes,
    colorzones_transient_memory_bytes,
};
use rusttable_processing::operations::bloom::{BloomConfig, BloomPlan};
use rusttable_processing::operations::colorzones::{
    ColorZonesChannel, ColorZonesMode, ColorZonesPixel, ColorZonesPlan,
};
use rusttable_processing::operations::shadhi::{ShadhiAlgorithm, ShadhiConfig};
use rusttable_processing::{
    BasicAdjConfig, BasicAdjPlan, BasicAdjPlanSet, FiniteF32, GrainPlan, LinearRgb,
    RasterDimensions, ShadhiBilateralBoundaryError, ShadhiBilateralEvaluationError,
    WorkingFrameDescriptor, WorkingRgbImage, evaluate_bilateral_shadhi_with_cancellation,
    prepare_basicadj_plans_with_cancellation,
};
use sha2::{Digest, Sha256};

use crate::cpu::{
    color_transform, operation_is_semantically_active, output_descriptor, output_from_working,
    requires_full_frame_execution, to_linear_working, validate_input_encoding,
};
use crate::{
    Cache, CacheConfig, CacheError, CacheKey, CancellationError, CancellationReason,
    CancellationScope, CancellationStage, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, PipelineGeneration, RgbaF32ColorEncoding,
    RgbaF32Image, RgbaF32Pixel,
};

/// The backend that published one pixelpipe result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelpipeBackend {
    CpuCanonical,
    CpuTiledFallback,
    WgpuBasic,
    WgpuBloom,
    WgpuColorReconstruction,
    WgpuColorZones,
    WgpuTiled,
    WgpuBilateralHybrid,
}

/// Typed reason a qualified GPU path fell back to canonical CPU execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelpipeGpuFallback {
    Basic(BasicPointError),
    Bloom(GpuBloomError),
    ColorReconstruction(GpuColorReconstructionError),
    ColorZones(GpuColorZonesError),
    Grain(GrainPointError),
    Bilateral(BilateralGridError),
    ShadhiBoundary(ShadhiBilateralBoundaryError),
}

impl PixelpipeGpuFallback {
    const fn is_cancellation(&self) -> bool {
        matches!(
            self,
            Self::Bloom(GpuBloomError::Cancelled)
                | Self::ColorReconstruction(GpuColorReconstructionError::Cancelled)
                | Self::ColorZones(GpuColorZonesError::Cancelled)
                | Self::Bilateral(BilateralGridError::Cancelled)
                | Self::ShadhiBoundary(ShadhiBilateralBoundaryError::Operation(
                    rusttable_processing::operations::OperationExecutionError::Cancelled
                ))
        )
    }
}

impl From<BasicPointError> for PixelpipeGpuFallback {
    fn from(error: BasicPointError) -> Self {
        Self::Basic(error)
    }
}

impl From<GpuBloomError> for PixelpipeGpuFallback {
    fn from(error: GpuBloomError) -> Self {
        Self::Bloom(error)
    }
}

impl From<GpuColorReconstructionError> for PixelpipeGpuFallback {
    fn from(error: GpuColorReconstructionError) -> Self {
        Self::ColorReconstruction(error)
    }
}

impl From<GpuColorZonesError> for PixelpipeGpuFallback {
    fn from(error: GpuColorZonesError) -> Self {
        Self::ColorZones(error)
    }
}

impl From<GrainPointError> for PixelpipeGpuFallback {
    fn from(error: GrainPointError) -> Self {
        Self::Grain(error)
    }
}

impl From<BilateralGridError> for PixelpipeGpuFallback {
    fn from(error: BilateralGridError) -> Self {
        Self::Bilateral(error)
    }
}

impl std::fmt::Display for PixelpipeGpuFallback {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic(error) => std::fmt::Display::fmt(error, formatter),
            Self::Bloom(error) => std::fmt::Display::fmt(error, formatter),
            Self::ColorReconstruction(error) => std::fmt::Display::fmt(error, formatter),
            Self::ColorZones(error) => std::fmt::Display::fmt(error, formatter),
            Self::Grain(error) => std::fmt::Display::fmt(error, formatter),
            Self::Bilateral(error) => std::fmt::Display::fmt(error, formatter),
            Self::ShadhiBoundary(error) => std::fmt::Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for PixelpipeGpuFallback {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Basic(error) => Some(error),
            Self::Bloom(error) => Some(error),
            Self::ColorReconstruction(error) => Some(error),
            Self::ColorZones(error) => Some(error),
            Self::Grain(error) => Some(error),
            Self::Bilateral(error) => Some(error),
            Self::ShadhiBoundary(error) => Some(error),
        }
    }
}

/// Bounded provenance for one tiled execution and its recovery attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelpipeTilingReceipt {
    plan_identity: [u8; 32],
    tile_count: u64,
    attempts: u8,
}

impl PixelpipeTilingReceipt {
    #[must_use]
    pub const fn plan_identity(&self) -> [u8; 32] {
        self.plan_identity
    }

    #[must_use]
    pub const fn tile_count(&self) -> u64 {
        self.tile_count
    }

    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }
}

/// Bounded provenance for one service execution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelpipeExecutionReceipt {
    snapshot_identity: crate::CpuPixelpipeSnapshotIdentity,
    basicadj_plan_identity: [u8; 32],
    backend: PixelpipeBackend,
    gpu_fallback: Option<PixelpipeGpuFallback>,
    dispatches: u32,
    tiling: Option<PixelpipeTilingReceipt>,
}

impl PixelpipeExecutionReceipt {
    #[must_use]
    pub const fn snapshot_identity(&self) -> crate::CpuPixelpipeSnapshotIdentity {
        self.snapshot_identity
    }

    #[must_use]
    pub const fn basicadj_plan_identity(&self) -> [u8; 32] {
        self.basicadj_plan_identity
    }

    #[must_use]
    pub const fn backend(&self) -> PixelpipeBackend {
        self.backend
    }

    #[must_use]
    pub const fn gpu_fallback(&self) -> Option<&PixelpipeGpuFallback> {
        self.gpu_fallback.as_ref()
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }

    #[must_use]
    pub const fn tiling(&self) -> Option<&PixelpipeTilingReceipt> {
        self.tiling.as_ref()
    }
}

/// An image and the backend receipt that authorized its publication.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelpipeExecutionResult {
    image: Arc<RgbaF32Image>,
    receipt: PixelpipeExecutionReceipt,
}

impl PixelpipeExecutionResult {
    #[must_use]
    pub fn image(&self) -> &RgbaF32Image {
        self.image.as_ref()
    }

    #[must_use]
    pub const fn receipt(&self) -> &PixelpipeExecutionReceipt {
        &self.receipt
    }
}

/// Application-facing basic pixelpipe coordinator.
///
/// GPU eligibility is derived from the immutable snapshot. The coordinator
/// never skips an enabled unsupported node and always retains the canonical
/// CPU executor as the publication path when GPU preparation or execution
/// fails.
#[derive(Debug)]
pub struct PixelpipeExecutionService {
    cpu: CpuPixelpipeExecutor,
    gpu: Option<GpuRuntime>,
    cache: OnceLock<Cache>,
    execution_errors: OnceLock<Mutex<VecDeque<(CacheKey, CpuPixelpipeError)>>>,
    #[cfg(test)]
    uncached_executions: AtomicUsize,
}

impl PixelpipeExecutionService {
    #[must_use]
    pub const fn cpu_only() -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: None,
            cache: OnceLock::new(),
            execution_errors: OnceLock::new(),
            #[cfg(test)]
            uncached_executions: AtomicUsize::new(0),
        }
    }

    #[must_use]
    pub fn with_gpu(gpu: GpuRuntime) -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: Some(gpu),
            cache: OnceLock::new(),
            execution_errors: OnceLock::new(),
            #[cfg(test)]
            uncached_executions: AtomicUsize::new(0),
        }
    }

    /// Installs the initialized backend without replacing process-lifetime
    /// cache state or diagnostics.
    pub fn install_gpu(&mut self, gpu: GpuRuntime) {
        self.gpu = Some(gpu);
    }

    /// Executes the snapshot, selecting WGPU for a qualified point range or
    /// singleton bilateral Shadhi and otherwise publishing canonical CPU.
    ///
    /// # Errors
    ///
    /// Returns the canonical pixelpipe error when CPU publication fails.
    pub fn execute(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let scope = uncancelled_scope();
        self.execute_with_cancellation(snapshot, &scope)
    }

    /// Executes with a generation-owned cancellation scope. Cancellation is
    /// checked around GPU work and immediately before result publication.
    ///
    /// # Errors
    ///
    /// Returns terminal [`CpuPixelpipeError::Cancelled`] without CPU fallback
    /// when cancellation is observed.
    pub fn execute_with_cancellation(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        check_cancellation(scope, CancellationStage::Preparation)?;
        let key =
            CacheKey::for_cpu_execution(snapshot, self.backend_identity(), direct_mode_identity());
        let builder_key = key.clone();
        check_cancellation(scope, CancellationStage::CacheBuild)?;
        let cached = self.cache().get_or_build_until(
            &key,
            scope.token(),
            scope.deadline(),
            |shared_token| {
                let shared_scope = CancellationScope::from_shared_token(shared_token.clone());
                self.execute_uncached_with_cancellation(snapshot, &shared_scope)
                    .map_err(|error| match error {
                        CpuPixelpipeError::Cancelled(error) => CacheError::Cancellation(error),
                        error => {
                            self.record_execution_error(&builder_key, error.clone());
                            CacheError::BuildFailed(error.to_string())
                        }
                    })
            },
        );
        match cached {
            Ok(lease) => {
                self.clear_execution_error(lease.key());
                check_cancellation(scope, CancellationStage::Publication)?;
                Ok(lease.value().clone())
            }
            Err(error) => {
                check_cancellation(scope, CancellationStage::CacheBuild)?;
                if let CacheError::Cancellation(error) = error {
                    return Err(CpuPixelpipeError::Cancelled(error));
                }
                if let Some(error) = self.execution_error(&builder_key) {
                    Err(error)
                } else {
                    self.execute_uncached_with_cancellation(snapshot, scope)
                }
            }
        }
    }

    fn execute_uncached_with_cancellation(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        #[cfg(test)]
        self.uncached_executions.fetch_add(1, Ordering::AcqRel);
        check_cancellation(scope, CancellationStage::Preparation)?;
        let Some(qualified) = gpu_plan(snapshot, scope)? else {
            return self.cpu_result(snapshot, None, scope);
        };
        let plan = &qualified.plan;
        check_cancellation(scope, CancellationStage::Preparation)?;
        let Some(gpu) = self.gpu.as_ref() else {
            return self.cpu_result(snapshot, None, scope);
        };
        if !gpu.health_check() {
            return self.cpu_result(
                snapshot,
                Some(plan.availability_error(gpu.is_cpu_only())),
                scope,
            );
        }

        check_cancellation(scope, CancellationStage::Transfer)?;
        let gpu_result = execute_gpu(gpu, snapshot, plan, scope);
        check_cancellation(scope, CancellationStage::Transfer)?;
        match gpu_result {
            Ok((image, dispatches)) => {
                check_cancellation(scope, CancellationStage::Publication)?;
                Ok(PixelpipeExecutionResult {
                    image: Arc::new(image),
                    receipt: PixelpipeExecutionReceipt {
                        snapshot_identity: snapshot.identity(),
                        basicadj_plan_identity: qualified.basicadj_plan_identity,
                        backend: plan.backend(),
                        gpu_fallback: None,
                        dispatches,
                        tiling: None,
                    },
                })
            }
            Err(error) => {
                let fallback = gpu_fallback_or_cancellation(error, scope)?;
                self.cpu_result(snapshot, Some(fallback), scope)
            }
        }
    }

    fn cpu_result(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        fallback: Option<PixelpipeGpuFallback>,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let cpu_result = self.cpu.execute_with_cancellation(snapshot, scope)?;
        let (image, receipt) = cpu_result.into_parts();
        let basicadj_plan_identity = receipt.basicadj_plan_identity();
        check_cancellation(scope, CancellationStage::Publication)?;
        Ok(PixelpipeExecutionResult {
            image: Arc::new(image),
            receipt: PixelpipeExecutionReceipt {
                snapshot_identity: snapshot.identity(),
                basicadj_plan_identity,
                backend: PixelpipeBackend::CpuCanonical,
                gpu_fallback: fallback,
                dispatches: 0,
                tiling: None,
            },
        })
    }

    /// Executes eligible point operations in row-major tiles with bounded
    /// smaller-tile recovery before publishing the canonical CPU fallback.
    /// Bilateral Shadhi always executes or falls back as one full frame.
    ///
    /// Each GPU attempt uses a fresh tile assembly. A failed attempt cannot
    /// publish partial pixels, and at most three tile plans are tried.
    ///
    /// # Errors
    ///
    /// Returns the canonical CPU pixelpipe error if every bounded GPU attempt
    /// and its CPU fallback fail.
    pub fn execute_tiled(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        tile_plan: crate::CpuTilePlan,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let scope = uncancelled_scope();
        self.execute_tiled_with_cancellation(snapshot, tile_plan, &scope)
    }

    /// Executes a tiled request with cancellation checks around every GPU tile
    /// and immediately before publication.
    ///
    /// # Errors
    ///
    /// Returns terminal [`CpuPixelpipeError::Cancelled`] without recovery or
    /// CPU fallback when cancellation is observed.
    pub fn execute_tiled_with_cancellation(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        tile_plan: crate::CpuTilePlan,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        check_cancellation(scope, CancellationStage::Preparation)?;
        let key = CacheKey::for_cpu_execution(
            snapshot,
            self.backend_identity(),
            tiled_mode_identity(tile_plan),
        );
        let builder_key = key.clone();
        check_cancellation(scope, CancellationStage::CacheBuild)?;
        let cached = self.cache().get_or_build_until(
            &key,
            scope.token(),
            scope.deadline(),
            |shared_token| {
                let shared_scope = CancellationScope::from_shared_token(shared_token.clone());
                self.execute_tiled_uncached_with_cancellation(snapshot, tile_plan, &shared_scope)
                    .map_err(|error| match error {
                        CpuPixelpipeError::Cancelled(error) => CacheError::Cancellation(error),
                        error => {
                            self.record_execution_error(&builder_key, error.clone());
                            CacheError::BuildFailed(error.to_string())
                        }
                    })
            },
        );
        match cached {
            Ok(lease) => {
                self.clear_execution_error(lease.key());
                check_cancellation(scope, CancellationStage::Publication)?;
                Ok(lease.value().clone())
            }
            Err(error) => {
                check_cancellation(scope, CancellationStage::CacheBuild)?;
                if let CacheError::Cancellation(error) = error {
                    return Err(CpuPixelpipeError::Cancelled(error));
                }
                if let Some(error) = self.execution_error(&builder_key) {
                    Err(error)
                } else {
                    self.execute_tiled_uncached_with_cancellation(snapshot, tile_plan, scope)
                }
            }
        }
    }

    fn execute_tiled_uncached_with_cancellation(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        tile_plan: crate::CpuTilePlan,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        #[cfg(test)]
        self.uncached_executions.fetch_add(1, Ordering::AcqRel);
        check_cancellation(scope, CancellationStage::Preparation)?;
        let Some(qualified) = gpu_plan(snapshot, scope)? else {
            return self.cpu_tiled_result(snapshot, tile_plan, None, 0, scope);
        };
        let plan = &qualified.plan;
        check_cancellation(scope, CancellationStage::Preparation)?;
        if matches!(
            plan,
            GpuPlan::Bloom { .. }
                | GpuPlan::ColorReconstruction { .. }
                | GpuPlan::ShadhiBilateral { .. }
        ) {
            let Some(gpu) = self.gpu.as_ref() else {
                return self.cpu_result(snapshot, None, scope);
            };
            if !gpu.health_check() {
                return self.cpu_result(
                    snapshot,
                    Some(plan.availability_error(gpu.is_cpu_only())),
                    scope,
                );
            }
            check_cancellation(scope, CancellationStage::Transfer)?;
            let gpu_result = execute_gpu(gpu, snapshot, plan, scope);
            check_cancellation(scope, CancellationStage::Transfer)?;
            return match gpu_result {
                Ok((image, dispatches)) => {
                    let tiling = full_frame_tiling_receipt(snapshot);
                    check_cancellation(scope, CancellationStage::Publication)?;
                    Ok(PixelpipeExecutionResult {
                        image: Arc::new(image),
                        receipt: PixelpipeExecutionReceipt {
                            snapshot_identity: snapshot.identity(),
                            basicadj_plan_identity: qualified.basicadj_plan_identity,
                            backend: plan.backend(),
                            gpu_fallback: None,
                            dispatches,
                            tiling: Some(tiling),
                        },
                    })
                }
                Err(error) => {
                    let fallback = gpu_fallback_or_cancellation(error, scope)?;
                    self.cpu_result(snapshot, Some(fallback), scope)
                }
            };
        }
        let Some(gpu) = self.gpu.as_ref() else {
            return self.cpu_tiled_result(snapshot, tile_plan, None, 0, scope);
        };
        if !gpu.health_check() {
            return self.cpu_tiled_result(
                snapshot,
                tile_plan,
                Some(plan.availability_error(gpu.is_cpu_only())),
                0,
                scope,
            );
        }

        let plans = recovery_plans(tile_plan);
        let mut last_error = None;
        for (index, candidate) in plans.iter().copied().enumerate() {
            check_cancellation(scope, CancellationStage::Tile)?;
            match execute_gpu_tiled(gpu, snapshot, plan, candidate, scope) {
                Ok((image, dispatches, tile_count)) => {
                    let tiling = tiling_receipt(snapshot, candidate, tile_count, index + 1);
                    check_cancellation(scope, CancellationStage::Publication)?;
                    return Ok(PixelpipeExecutionResult {
                        image: Arc::new(image),
                        receipt: PixelpipeExecutionReceipt {
                            snapshot_identity: snapshot.identity(),
                            basicadj_plan_identity: qualified.basicadj_plan_identity,
                            backend: PixelpipeBackend::WgpuTiled,
                            gpu_fallback: None,
                            dispatches,
                            tiling: Some(tiling),
                        },
                    });
                }
                Err(GpuTiledExecutionError::Fallback(error)) => {
                    last_error = Some(gpu_fallback_or_cancellation(error, scope)?);
                }
                Err(GpuTiledExecutionError::Cancelled(error)) => {
                    return Err(CpuPixelpipeError::Cancelled(error));
                }
            }
        }
        self.cpu_tiled_result(
            snapshot,
            tile_plan,
            last_error,
            u8::try_from(plans.len()).unwrap_or(u8::MAX),
            scope,
        )
    }

    fn cpu_tiled_result(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        plan: crate::CpuTilePlan,
        fallback: Option<PixelpipeGpuFallback>,
        attempts: u8,
        scope: &CancellationScope,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        if requires_full_frame_execution(snapshot) {
            return self.cpu_result(snapshot, fallback, scope);
        }
        let result = self
            .cpu
            .execute_tiled_with_cancellation(snapshot, plan, scope)?;
        let (image, receipt) = result.into_parts();
        let basicadj_plan_identity = receipt.basicadj_plan_identity();
        let grid = plan
            .grid_for(snapshot.input().descriptor().dimensions())
            .map_err(|source| CpuPixelpipeError::TilePlan { source })?;
        let tiling = tiling_receipt(snapshot, plan, grid.tile_count(), usize::from(attempts));
        check_cancellation(scope, CancellationStage::Publication)?;
        Ok(PixelpipeExecutionResult {
            image: Arc::new(image),
            receipt: PixelpipeExecutionReceipt {
                snapshot_identity: snapshot.identity(),
                basicadj_plan_identity,
                backend: PixelpipeBackend::CpuTiledFallback,
                gpu_fallback: fallback,
                dispatches: 0,
                tiling: Some(tiling),
            },
        })
    }

    fn cache(&self) -> &Cache {
        self.cache
            .get_or_init(|| Cache::new(CacheConfig::default()))
    }

    fn backend_identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.pixelpipe.execution-backend.v1");
        match self.gpu.as_ref() {
            None => hasher.update([0]),
            Some(gpu) => {
                hasher.update([1]);
                hasher.update(
                    gpu.snapshot()
                        .canonical_hash()
                        .expect("bounded GPU capability snapshot is serializable"),
                );
                hasher.update(
                    postcard::to_allocvec(&gpu.fault_snapshot())
                        .expect("bounded GPU fault snapshot is serializable"),
                );
            }
        }
        hasher.finalize().into()
    }

    fn record_execution_error(&self, key: &CacheKey, error: CpuPixelpipeError) {
        let errors = self
            .execution_errors
            .get_or_init(|| Mutex::new(VecDeque::new()));
        if let Ok(mut errors) = errors.lock() {
            errors.retain(|(candidate, _)| candidate != key);
            errors.push_back((key.clone(), error));
            while errors.len() > 64 {
                errors.pop_front();
            }
        }
    }

    fn execution_error(&self, key: &CacheKey) -> Option<CpuPixelpipeError> {
        self.execution_errors
            .get()
            .and_then(|errors| errors.lock().ok())
            .and_then(|errors| {
                errors
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .map(|(_, error)| error.clone())
            })
    }

    fn clear_execution_error(&self, key: &CacheKey) {
        if let Some(errors) = self.execution_errors.get()
            && let Ok(mut errors) = errors.lock()
        {
            errors.retain(|(candidate, _)| candidate != key);
        }
    }

    #[cfg(test)]
    fn uncached_execution_count(&self) -> usize {
        self.uncached_executions.load(Ordering::Acquire)
    }
}

fn direct_mode_identity() -> [u8; 32] {
    Sha256::digest(b"rusttable.pixelpipe.execution-mode.direct.v1").into()
}

fn tiled_mode_identity(plan: crate::CpuTilePlan) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.pixelpipe.execution-mode.tiled.v1");
    hasher.update(plan.tile_width().to_le_bytes());
    hasher.update(plan.tile_height().to_le_bytes());
    hasher.update([3]);
    hasher.finalize().into()
}

fn uncancelled_scope() -> CancellationScope {
    CancellationScope::root(
        PipelineGeneration::new(1).expect("uncancelled service generation is nonzero"),
    )
}

fn check_cancellation(
    scope: &CancellationScope,
    stage: CancellationStage,
) -> Result<(), CpuPixelpipeError> {
    scope
        .child(stage)
        .check()
        .map_err(CpuPixelpipeError::Cancelled)
}

fn gpu_fallback_or_cancellation(
    error: PixelpipeGpuFallback,
    scope: &CancellationScope,
) -> Result<PixelpipeGpuFallback, CpuPixelpipeError> {
    if !error.is_cancellation() {
        return Ok(error);
    }
    check_cancellation(scope, CancellationStage::GpuRetirement)?;
    // A cancellation-aware backend must be linked to this scope. Preserve a
    // terminal failure even if a backend violates that invariant.
    scope.cancel(CancellationReason::ParentFailed);
    match check_cancellation(scope, CancellationStage::GpuRetirement) {
        Err(cancelled) => Err(cancelled),
        Ok(()) => unreachable!("cancelling a live scope is immediately observable"),
    }
}

fn recovery_plans(initial: crate::CpuTilePlan) -> Vec<crate::CpuTilePlan> {
    let mut plans = vec![initial];
    let mut width = initial.tile_width();
    let mut height = initial.tile_height();
    for _ in 0..2 {
        width = width.div_ceil(2);
        height = height.div_ceil(2);
        let Ok(plan) = crate::CpuTilePlan::new(width, height) else {
            break;
        };
        if plans.last().is_some_and(|previous| *previous == plan) {
            break;
        }
        plans.push(plan);
    }
    plans
}

fn tiling_receipt(
    snapshot: &CpuPixelpipeSnapshot,
    plan: crate::CpuTilePlan,
    tile_count: u64,
    attempts: usize,
) -> PixelpipeTilingReceipt {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.pixelpipe.tiling.v1");
    hasher.update(snapshot.identity().as_bytes());
    hasher.update(plan.tile_width().to_le_bytes());
    hasher.update(plan.tile_height().to_le_bytes());
    PixelpipeTilingReceipt {
        plan_identity: hasher.finalize().into(),
        tile_count,
        attempts: u8::try_from(attempts.min(usize::from(u8::MAX))).unwrap_or(u8::MAX),
    }
}

fn full_frame_tiling_receipt(snapshot: &CpuPixelpipeSnapshot) -> PixelpipeTilingReceipt {
    let dimensions = snapshot.input().descriptor().dimensions();
    let full_frame = crate::CpuTilePlan::new(dimensions.width(), dimensions.height())
        .expect("validated raster dimensions make one full-frame tile");
    tiling_receipt(snapshot, full_frame, 1, 1)
}

#[derive(Debug, Clone, PartialEq)]
enum GpuPlan {
    Basic(Vec<BasicPointOperation>),
    Bloom {
        config: BloomConfig,
        opacity: f32,
    },
    ColorReconstruction {
        config: rusttable_processing::operations::colorreconstruction::ColorReconstructionConfig,
        opacity: f32,
    },
    ColorZones {
        plan: ColorZonesPlan,
        opacity: f32,
    },
    Grain(rusttable_processing::operations::grain::GrainConfig),
    ShadhiBilateral {
        config: ShadhiConfig,
        opacity: f32,
    },
}

impl GpuPlan {
    const fn backend(&self) -> PixelpipeBackend {
        match self {
            Self::Bloom { .. } => PixelpipeBackend::WgpuBloom,
            Self::ColorReconstruction { .. } => PixelpipeBackend::WgpuColorReconstruction,
            Self::ColorZones { .. } => PixelpipeBackend::WgpuColorZones,
            Self::ShadhiBilateral { .. } => PixelpipeBackend::WgpuBilateralHybrid,
            Self::Basic(_) | Self::Grain(_) => PixelpipeBackend::WgpuBasic,
        }
    }

    fn availability_error(&self, cpu_only: bool) -> PixelpipeGpuFallback {
        match self {
            Self::Basic(_) => PixelpipeGpuFallback::Basic(if cpu_only {
                BasicPointError::CpuOnly
            } else {
                BasicPointError::Unhealthy
            }),
            Self::Bloom { .. } => PixelpipeGpuFallback::Bloom(if cpu_only {
                GpuBloomError::CpuOnly
            } else {
                GpuBloomError::Unhealthy
            }),
            Self::ColorReconstruction { .. } => {
                PixelpipeGpuFallback::ColorReconstruction(if cpu_only {
                    GpuColorReconstructionError::CpuOnly
                } else {
                    GpuColorReconstructionError::Unhealthy
                })
            }
            Self::ColorZones { .. } => PixelpipeGpuFallback::ColorZones(if cpu_only {
                GpuColorZonesError::CpuOnly
            } else {
                GpuColorZonesError::Unhealthy
            }),
            Self::Grain(_) => PixelpipeGpuFallback::Grain(if cpu_only {
                GrainPointError::CpuOnly
            } else {
                GrainPointError::Unhealthy
            }),
            Self::ShadhiBilateral { .. } => PixelpipeGpuFallback::Bilateral(if cpu_only {
                BilateralGridError::CpuOnly
            } else {
                BilateralGridError::Unhealthy
            }),
        }
    }
}

#[derive(Debug, Clone)]
struct QualifiedGpuPlan {
    plan: GpuPlan,
    basicadj_plan_identity: [u8; 32],
}

#[derive(Debug, Clone)]
enum GpuPlanCandidate {
    Basic(Vec<BasicPointCandidate>),
    Bloom {
        config: BloomConfig,
        opacity: f32,
    },
    ColorReconstruction {
        config: rusttable_processing::operations::colorreconstruction::ColorReconstructionConfig,
        opacity: f32,
    },
    ColorZones {
        plan: ColorZonesPlan,
        opacity: f32,
    },
    Grain(rusttable_processing::operations::grain::GrainConfig),
    ShadhiBilateral {
        config: ShadhiConfig,
        opacity: f32,
    },
}

impl GpuPlanCandidate {
    fn requires_basicadj_resolution(&self) -> bool {
        matches!(
            self,
            Self::Basic(operations)
                if operations.iter().any(BasicPointCandidate::requires_resolution)
        )
    }

    fn resolve(self, resolved: Option<&ResolvedBasicAdjPlans>) -> Option<GpuPlan> {
        match self {
            Self::Basic(operations) => operations
                .into_iter()
                .map(|operation| operation.resolve(resolved))
                .collect::<Option<Vec<_>>>()
                .map(GpuPlan::Basic),
            Self::Bloom { config, opacity } => Some(GpuPlan::Bloom { config, opacity }),
            Self::ColorReconstruction { config, opacity } => {
                Some(GpuPlan::ColorReconstruction { config, opacity })
            }
            Self::ColorZones { plan, opacity } => Some(GpuPlan::ColorZones { plan, opacity }),
            Self::Grain(config) => Some(GpuPlan::Grain(config)),
            Self::ShadhiBilateral { config, opacity } => {
                Some(GpuPlan::ShadhiBilateral { config, opacity })
            }
        }
    }
}

#[derive(Debug, Clone)]
enum BasicPointCandidate {
    BasicAdj {
        operation_id: OperationId,
        config: BasicAdjConfig,
    },
    Ready(BasicPointOperation),
}

impl BasicPointCandidate {
    fn requires_resolution(&self) -> bool {
        matches!(
            self,
            Self::BasicAdj { config, .. } if config.auto_controls().is_active()
        )
    }

    fn resolve(self, resolved: Option<&ResolvedBasicAdjPlans>) -> Option<BasicPointOperation> {
        match self {
            Self::BasicAdj {
                operation_id,
                config,
            } => {
                let plan = if config.auto_controls().is_active() {
                    resolved?.plan(operation_id)?.clone()
                } else {
                    BasicAdjPlan::new(config).ok()?
                };
                Some(basicadj_point_operation(&plan))
            }
            Self::Ready(operation) => Some(operation),
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedBasicAdjPlans {
    plans: BTreeMap<OperationId, BasicAdjPlan>,
    identity: [u8; 32],
}

impl ResolvedBasicAdjPlans {
    fn from_plan_set(snapshot: &CpuPixelpipeSnapshot, plan_set: &BasicAdjPlanSet) -> Self {
        let plans = snapshot
            .graph()
            .nodes()
            .filter_map(|node| {
                plan_set
                    .plan(node.operation().operation_id())
                    .cloned()
                    .map(|plan| (node.operation().operation_id(), plan))
            })
            .collect();
        Self {
            plans,
            identity: plan_set.identity(),
        }
    }

    fn plan(&self, operation_id: OperationId) -> Option<&BasicAdjPlan> {
        self.plans.get(&operation_id)
    }
}

fn gpu_plan(
    snapshot: &CpuPixelpipeSnapshot,
    scope: &CancellationScope,
) -> Result<Option<QualifiedGpuPlan>, CpuPixelpipeError> {
    let Some(candidate) = gpu_plan_candidate(snapshot) else {
        return Ok(None);
    };
    resolve_gpu_plan_candidate(candidate, || prepare_gpu_basicadj_plans(snapshot, scope))
}

fn resolve_gpu_plan_candidate<R>(
    candidate: GpuPlanCandidate,
    load_plans: R,
) -> Result<Option<QualifiedGpuPlan>, CpuPixelpipeError>
where
    R: FnOnce() -> Result<Option<ResolvedBasicAdjPlans>, CpuPixelpipeError>,
{
    let plan_evidence = if candidate.requires_basicadj_resolution() {
        let Some(plans) = load_plans()? else {
            return Ok(None);
        };
        Some(plans)
    } else {
        None
    };
    let basicadj_plan_identity = plan_evidence
        .as_ref()
        .map_or([0; 32], |plans| plans.identity);
    Ok(candidate
        .resolve(plan_evidence.as_ref())
        .map(|plan| QualifiedGpuPlan {
            plan,
            basicadj_plan_identity,
        }))
}

fn gpu_plan_candidate(snapshot: &CpuPixelpipeSnapshot) -> Option<GpuPlanCandidate> {
    if validate_input_encoding(snapshot.input()).is_err()
        || snapshot.mask_graph().is_some()
        || snapshot.mask_store().is_some()
    {
        return None;
    }
    let mut operations = Vec::new();
    let mut bloom = None;
    let mut colorreconstruct = None;
    let mut colorzones = None;
    let mut grain = None;
    let mut shadhi = None;
    let mut has_nodes = false;
    let mut has_active_nodes = false;
    for node in snapshot.graph().nodes() {
        has_nodes = true;
        let operation = node.operation();
        if !operation_is_semantically_active(operation) {
            continue;
        }
        has_active_nodes = true;
        let gpu_operation = match operation.kind() {
            rusttable_processing::ProcessingOperationKind::BasicAdj { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::BasicAdj {
                    operation_id: operation.operation_id(),
                    config: *config,
                }
            }
            rusttable_processing::ProcessingOperationKind::Exposure { stops, black } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(BasicPointOperation::Exposure {
                    stops: stops.get(),
                    black: black.get(),
                })
            }
            rusttable_processing::ProcessingOperationKind::LinearOffset { value } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(BasicPointOperation::LinearOffset { value: value.get() })
            }
            rusttable_processing::ProcessingOperationKind::RgbGain { red, green, blue } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(BasicPointOperation::RgbGain {
                    red: red.get(),
                    green: green.get(),
                    blue: blue.get(),
                })
            }
            rusttable_processing::ProcessingOperationKind::Velvia { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(velvia_point_operation(*config))
            }
            rusttable_processing::ProcessingOperationKind::ColorCorrection { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(colorcorrection_point_operation(*config))
            }
            rusttable_processing::ProcessingOperationKind::ColorContrast { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(colorcontrast_point_operation(*config))
            }
            rusttable_processing::ProcessingOperationKind::Vibrance { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
                    return None;
                }
                BasicPointCandidate::Ready(vibrance_point_operation(*config))
            }
            rusttable_processing::ProcessingOperationKind::Bloom { config } => {
                if bloom.is_some()
                    || colorreconstruct.is_some()
                    || colorzones.is_some()
                    || grain.is_some()
                    || shadhi.is_some()
                    || !operations.is_empty()
                {
                    return None;
                }
                bloom = Some((*config, operation.opacity().get()));
                continue;
            }
            rusttable_processing::ProcessingOperationKind::ColorReconstruction { config } => {
                if colorreconstruct.is_some()
                    || bloom.is_some()
                    || colorzones.is_some()
                    || grain.is_some()
                    || shadhi.is_some()
                    || !operations.is_empty()
                {
                    return None;
                }
                colorreconstruct = Some((*config, operation.opacity().get()));
                continue;
            }
            rusttable_processing::ProcessingOperationKind::ColorZones { plan } => {
                if bloom.is_some()
                    || colorreconstruct.is_some()
                    || colorzones.is_some()
                    || grain.is_some()
                    || shadhi.is_some()
                    || !operations.is_empty()
                {
                    return None;
                }
                colorzones = Some((plan.clone(), operation.opacity().get()));
                continue;
            }
            rusttable_processing::ProcessingOperationKind::Grain { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits()
                    || bloom.is_some()
                    || colorreconstruct.is_some()
                    || colorzones.is_some()
                    || grain.is_some()
                    || shadhi.is_some()
                    || !operations.is_empty()
                {
                    return None;
                }
                grain = Some(*config);
                continue;
            }
            rusttable_processing::ProcessingOperationKind::Shadhi { config } => {
                if operation.opacity().get() == 0.0
                    || config.shadhi_algo() != ShadhiAlgorithm::Bilateral
                    || bloom.is_some()
                    || colorreconstruct.is_some()
                    || colorzones.is_some()
                    || grain.is_some()
                    || shadhi.is_some()
                    || !operations.is_empty()
                {
                    return None;
                }
                shadhi = Some((*config, operation.opacity().get()));
                continue;
            }
            _ => return None,
        };
        if bloom.is_some()
            || colorreconstruct.is_some()
            || colorzones.is_some()
            || grain.is_some()
            || shadhi.is_some()
        {
            return None;
        }
        operations.push(gpu_operation);
    }
    if let Some((config, opacity)) = bloom {
        Some(GpuPlanCandidate::Bloom { config, opacity })
    } else if let Some((config, opacity)) = colorreconstruct {
        Some(GpuPlanCandidate::ColorReconstruction { config, opacity })
    } else if let Some((plan, opacity)) = colorzones {
        Some(GpuPlanCandidate::ColorZones { plan, opacity })
    } else if let Some(config) = grain {
        Some(GpuPlanCandidate::Grain(config))
    } else if let Some((config, opacity)) = shadhi {
        Some(GpuPlanCandidate::ShadhiBilateral { config, opacity })
    } else if has_nodes && !has_active_nodes {
        None
    } else {
        Some(GpuPlanCandidate::Basic(operations))
    }
}

fn prepare_gpu_basicadj_plans(
    snapshot: &CpuPixelpipeSnapshot,
    scope: &CancellationScope,
) -> Result<Option<ResolvedBasicAdjPlans>, CpuPixelpipeError> {
    let analysis_scope = scope.child(CancellationStage::Analysis);
    analysis_scope
        .check()
        .map_err(CpuPixelpipeError::Cancelled)?;
    let Ok(linear) = to_linear_working(snapshot.input()) else {
        return Ok(None);
    };
    analysis_scope
        .check()
        .map_err(CpuPixelpipeError::Cancelled)?;
    let plan_set = match prepare_basicadj_plans_with_cancellation(snapshot.graph(), &linear, || {
        analysis_scope.check().is_err()
    }) {
        Ok(plan_set) => plan_set,
        Err(source) if source.is_cancelled() => {
            if let Err(error) = analysis_scope.check() {
                return Err(CpuPixelpipeError::Cancelled(error));
            }
            scope.cancel(CancellationReason::ParentFailed);
            return Err(CpuPixelpipeError::Cancelled(
                analysis_scope
                    .check()
                    .expect_err("parent cancellation propagates to the analysis child"),
            ));
        }
        Err(_) => return Ok(None),
    };
    analysis_scope
        .check()
        .map_err(CpuPixelpipeError::Cancelled)?;
    Ok(Some(ResolvedBasicAdjPlans::from_plan_set(
        snapshot, &plan_set,
    )))
}

fn basicadj_point_operation(plan: &BasicAdjPlan) -> BasicPointOperation {
    let parameters = plan.gpu_parameters();
    BasicPointOperation::BasicAdj(BasicAdjPointParameters {
        black_point: parameters.black_point,
        scale: parameters.scale,
        gamma: parameters.gamma,
        middle_grey: parameters.middle_grey,
        contrast: parameters.contrast,
        hlcomp: parameters.hlcomp,
        hlrange: parameters.hlrange,
        preserve_colors: parameters.preserve_colors,
        saturation: parameters.saturation,
        vibrance: parameters.vibrance,
    })
}

fn velvia_point_operation(
    config: rusttable_processing::operations::velvia::VelviaConfig,
) -> BasicPointOperation {
    BasicPointOperation::Velvia {
        strength: config.normalized_strength(),
        bias: config.bias(),
    }
}

fn colorcorrection_point_operation(
    config: rusttable_processing::operations::colorcorrection::ColorCorrectionConfig,
) -> BasicPointOperation {
    let coefficients = config.committed_coefficients();
    BasicPointOperation::ColorCorrection {
        saturation: coefficients.saturation(),
        a_scale: coefficients.a_scale(),
        a_base: coefficients.a_base(),
        b_scale: coefficients.b_scale(),
        b_base: coefficients.b_base(),
    }
}

fn colorcontrast_point_operation(
    config: rusttable_processing::operations::colorcontrast::ColorContrastConfig,
) -> BasicPointOperation {
    BasicPointOperation::ColorContrast {
        a_steepness: config.a_steepness(),
        a_offset: config.a_offset(),
        b_steepness: config.b_steepness(),
        b_offset: config.b_offset(),
        unbound: config.is_unbound(),
    }
}

fn vibrance_point_operation(
    config: rusttable_processing::operations::vibrance::VibranceConfig,
) -> BasicPointOperation {
    BasicPointOperation::Vibrance {
        amount: config.normalized_amount(),
    }
}

fn execute_gpu(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    plan: &GpuPlan,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    match plan {
        GpuPlan::Basic(operations) => {
            execute_gpu_image(gpu, snapshot.input(), snapshot.output_mode(), operations)
        }
        GpuPlan::Bloom { config, opacity } => execute_gpu_bloom_image(
            gpu,
            snapshot.input(),
            snapshot.output_mode(),
            *config,
            *opacity,
            scope,
        ),
        GpuPlan::ColorReconstruction { config, opacity } => execute_gpu_colorreconstruct_image(
            gpu,
            snapshot.input(),
            snapshot.output_mode(),
            *config,
            *opacity,
            scope,
        ),
        GpuPlan::ColorZones { plan, opacity } => execute_gpu_colorzones_image(
            gpu,
            snapshot.input(),
            snapshot.output_mode(),
            plan,
            *opacity,
            snapshot.identity(),
            scope,
        ),
        GpuPlan::Grain(config) => execute_gpu_grain_image(
            gpu,
            snapshot.input(),
            snapshot.output_mode(),
            *config,
            snapshot.input().descriptor().dimensions(),
            (0, 0),
        ),
        GpuPlan::ShadhiBilateral { config, opacity } => {
            execute_gpu_shadhi_bilateral(gpu, snapshot, *config, *opacity, scope)
        }
    }
}

fn execute_gpu_shadhi_bilateral(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    config: ShadhiConfig,
    opacity: f32,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let working = to_linear_working(snapshot.input())
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let gpu_cancellation = GpuCancellationToken::new();
    let linked_cancellation = gpu_cancellation.clone();
    let _cleanup = scope.register_cleanup(move |_reason| linked_cancellation.cancel());
    let evaluation_scope = scope.child(CancellationStage::Node);
    let transfer_scope = scope.child(CancellationStage::Transfer);
    let mut dispatches = 0;
    let evaluated = evaluate_bilateral_shadhi_with_cancellation(
        &working,
        config,
        opacity,
        |request| {
            if transfer_scope.check().is_err() {
                return Err(BilateralGridError::Cancelled);
            }
            let geometry = request.geometry();
            let gpu_request = BilateralGridRequest::slice(
                request.guide(),
                geometry.width(),
                geometry.height(),
                geometry.grid_dimensions(),
                geometry.effective_sigma_s(),
                geometry.effective_sigma_r(),
                request.detail(),
                request.transient_memory_budget_bytes(),
            )
            .with_cancellation(&gpu_cancellation);
            let result = gpu.execute_bilateral_grid(gpu_request)?;
            if transfer_scope.check().is_err() {
                return Err(BilateralGridError::Cancelled);
            }
            dispatches = result.dispatches();
            Ok(result.into_pixels())
        },
        || evaluation_scope.check().is_err(),
    )
    .map_err(shadhi_gpu_fallback)?;
    let output = output_from_working(snapshot.output_mode(), snapshot.input(), &evaluated)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    Ok((output, dispatches))
}

fn shadhi_gpu_fallback(
    error: ShadhiBilateralEvaluationError<BilateralGridError>,
) -> PixelpipeGpuFallback {
    match error {
        ShadhiBilateralEvaluationError::Backend(error) => PixelpipeGpuFallback::Bilateral(error),
        ShadhiBilateralEvaluationError::Boundary(error) => {
            PixelpipeGpuFallback::ShadhiBoundary(error)
        }
    }
}

fn execute_gpu_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    operations: &[BasicPointOperation],
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let color_space =
        basic_point_chain_color_space(input.descriptor().color_encoding(), operations)?;
    if color_space == BasicPointColorSpace::LabD50 {
        let (packed, boundary) = packed_lab_d50(input, || false)?;
        let result = gpu.execute_basic_point(BasicPointRequest {
            pixels: &packed,
            operations,
            color_space,
        })?;
        return lab_image_from_packed(
            input,
            result.pixels(),
            result.dispatches(),
            boundary,
            output_mode,
            || false,
        );
    }
    let (frame, packed) = packed_linear_working(input)?;
    let result = gpu.execute_basic_point(BasicPointRequest {
        pixels: &packed,
        operations,
        color_space,
    })?;
    image_from_packed(
        input,
        output_mode,
        frame,
        result.pixels(),
        result.dispatches(),
    )
}

fn basic_point_chain_color_space(
    input_encoding: RgbaF32ColorEncoding,
    operations: &[BasicPointOperation],
) -> Result<BasicPointColorSpace, BasicPointError> {
    if operations.is_empty() {
        return Ok(if input_encoding == RgbaF32ColorEncoding::LabD50 {
            BasicPointColorSpace::LabD50
        } else {
            BasicPointColorSpace::LinearRgb
        });
    }
    let is_lab_point = |operation: &BasicPointOperation| {
        matches!(
            operation,
            BasicPointOperation::ColorContrast { .. }
                | BasicPointOperation::ColorCorrection { .. }
                | BasicPointOperation::Vibrance { .. }
        )
    };
    if !operations.iter().any(is_lab_point) {
        return Ok(BasicPointColorSpace::LinearRgb);
    }
    if operations.iter().all(is_lab_point) {
        return Ok(BasicPointColorSpace::LabD50);
    }
    Err(BasicPointError::ColorSpaceBoundaryUnavailable {
        required: BasicPointColorSpace::LabD50,
    })
}

struct LabGpuBoundary {
    from_lab: rusttable_color::TransformPlan,
    frame: WorkingFrameDescriptor,
}

fn packed_lab_d50<C: Fn() -> bool>(
    input: &RgbaF32Image,
    cancelled: C,
) -> Result<(Vec<f32>, Option<LabGpuBoundary>), PixelpipeGpuFallback> {
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        let mut packed = Vec::with_capacity(input.pixels().len() * 4);
        for pixel in input.pixels() {
            packed.extend([pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
        }
        return Ok((packed, None));
    }
    let working =
        to_linear_working(input).map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let to_lab = color_transform(
        working.frame().encoding(),
        rusttable_color::ColorEncoding::LabD50,
    )
    .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let from_lab = color_transform(
        rusttable_color::ColorEncoding::LabD50,
        working.frame().encoding(),
    )
    .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let mut packed = Vec::with_capacity(input.pixels().len() * 4);
    for (pixel_index, (pixel, source)) in working.pixels().zip(input.pixels()).enumerate() {
        let channels = to_lab
            .apply_rgb(
                [pixel.red().get(), pixel.green().get(), pixel.blue().get()],
                &cancelled,
            )
            .map_err(|error| {
                BasicPointError::Readback(format!(
                    "GPU Lab point ingress failed at pixel {pixel_index}: {error}"
                ))
            })?;
        packed.extend([channels[0], channels[1], channels[2], source.alpha()]);
    }
    Ok((
        packed,
        Some(LabGpuBoundary {
            from_lab,
            frame: working.frame(),
        }),
    ))
}

fn lab_image_from_packed<C: Fn() -> bool>(
    input: &RgbaF32Image,
    packed: &[f32],
    dispatches: u32,
    boundary: Option<LabGpuBoundary>,
    output_mode: CpuPixelpipeOutputMode,
    cancelled: C,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let (packed_pixels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }
    lab_image_from_pixels(
        input,
        packed_pixels,
        dispatches,
        boundary,
        output_mode,
        cancelled,
    )
}

fn lab_image_from_pixels<C: Fn() -> bool>(
    input: &RgbaF32Image,
    packed_pixels: &[[f32; 4]],
    dispatches: u32,
    boundary: Option<LabGpuBoundary>,
    output_mode: CpuPixelpipeOutputMode,
    cancelled: C,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    if packed_pixels.len() != input.pixels().len() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }
    if let Some(boundary) = boundary {
        let pixels = packed_pixels
            .iter()
            .enumerate()
            .map(|(pixel_index, pixel)| {
                let rgb = boundary
                    .from_lab
                    .apply_rgb([pixel[0], pixel[1], pixel[2]], &cancelled)
                    .map_err(|error| {
                        BasicPointError::Readback(format!(
                            "GPU Lab point egress failed at pixel {pixel_index}: {error}"
                        ))
                    })?;
                Ok(LinearRgb::new(
                    FiniteF32::new(rgb[0]).map_err(|_| {
                        BasicPointError::Readback(format!(
                            "GPU Lab point egress produced non-finite red at pixel {pixel_index}"
                        ))
                    })?,
                    FiniteF32::new(rgb[1]).map_err(|_| {
                        BasicPointError::Readback(format!(
                            "GPU Lab point egress produced non-finite green at pixel {pixel_index}"
                        ))
                    })?,
                    FiniteF32::new(rgb[2]).map_err(|_| {
                        BasicPointError::Readback(format!(
                            "GPU Lab point egress produced non-finite blue at pixel {pixel_index}"
                        ))
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, BasicPointError>>()?;
        let evaluated = WorkingRgbImage::new_with_frame(
            input.descriptor().dimensions(),
            pixels,
            boundary.frame,
        )
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
        let image = output_from_working(output_mode, input, &evaluated)
            .map_err(|error| BasicPointError::Readback(error.to_string()))?;
        return Ok((image, dispatches));
    }
    let pixels = packed_pixels
        .iter()
        .map(|pixel| RgbaF32Pixel::new(pixel[0], pixel[1], pixel[2], pixel[3]))
        .collect::<Vec<_>>();
    let image = RgbaF32Image::new(input.descriptor(), pixels)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    Ok((image, dispatches))
}

fn execute_gpu_bloom_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    config: BloomConfig,
    opacity: f32,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let (packed, boundary) = packed_lab_d50(input, || scope.check().is_err())?;
    let (source_channels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() || source_channels.len() != input.pixels().len() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }
    let dimensions = input.descriptor().dimensions();
    let width = usize::try_from(dimensions.width()).map_err(|_| GpuBloomError::SizeOverflow)?;
    let height = usize::try_from(dimensions.height()).map_err(|_| GpuBloomError::SizeOverflow)?;
    let radius = BloomPlan::new(config, dimensions)
        .map_err(|_| GpuBloomError::SizeOverflow)?
        .radius();
    let transient_memory_budget = bloom_transient_memory_bytes(width, height)?;

    let gpu_cancellation = GpuCancellationToken::new();
    let linked_cancellation = gpu_cancellation.clone();
    let _cleanup = scope.register_cleanup(move |_reason| linked_cancellation.cancel());
    let transfer_scope = scope.child(CancellationStage::Transfer);
    if transfer_scope.check().is_err() {
        return Err(GpuBloomError::Cancelled.into());
    }
    let request = BloomRequest::new(
        source_channels,
        width,
        height,
        radius,
        config.threshold(),
        config.strength(),
        transient_memory_budget,
    )
    .with_cancellation(&gpu_cancellation);
    let result = gpu.execute_bloom(request)?;
    if transfer_scope.check().is_err() {
        return Err(GpuBloomError::Cancelled.into());
    }
    let dispatches = result.dispatches();
    let candidate_channels = result.into_pixels();
    let output_channels = if opacity.to_bits() == 1.0_f32.to_bits() {
        candidate_channels
    } else {
        source_channels
            .iter()
            .zip(candidate_channels)
            .map(|(source, candidate)| {
                let mut blended = *source;
                blended[0] = source[0] + (candidate[0] - source[0]) * opacity;
                blended
            })
            .collect()
    };
    lab_image_from_pixels(
        input,
        &output_channels,
        dispatches,
        boundary,
        output_mode,
        || scope.check().is_err(),
    )
}

fn execute_gpu_colorreconstruct_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    config: rusttable_processing::operations::colorreconstruction::ColorReconstructionConfig,
    opacity: f32,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let transfer_scope = scope.child(CancellationStage::Transfer);
    transfer_scope
        .check()
        .map_err(|_| GpuColorReconstructionError::Cancelled)?;
    let (packed, boundary) = packed_lab_d50(input, || transfer_scope.check().is_err())
        .map_err(|error| colorreconstruct_boundary_fallback(error, &transfer_scope))?;
    let (source_channels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() || source_channels.len() != input.pixels().len() {
        return Err(GpuColorReconstructionError::Readback(
            BasicPointError::InvalidPixelPacking.to_string(),
        )
        .into());
    }
    let dimensions = input.descriptor().dimensions();
    let width = dimensions.width();
    let height = dimensions.height();
    let roi = ColorReconstructionRoi::new(0, 0, width as usize, height as usize, 1.0);
    let transient_memory_budget = colorreconstruction_transient_memory_bytes(
        roi,
        1.0,
        config.spatial().get(),
        config.range().get(),
    )?;
    let gpu_cancellation = GpuCancellationToken::new();
    let linked_cancellation = gpu_cancellation.clone();
    let _cleanup = scope.register_cleanup(move |_reason| linked_cancellation.cancel());
    let request = ColorReconstructionRequest::new(
        source_channels,
        roi,
        1.0,
        config.threshold().get(),
        config.spatial().get(),
        config.range().get(),
        config.hue().get(),
        gpu_colorreconstruct_precedence(config.precedence()),
        transient_memory_budget,
    )
    .with_cancellation(&gpu_cancellation);
    let result = gpu.execute_color_reconstruction(request)?;
    transfer_scope
        .check()
        .map_err(|_| GpuColorReconstructionError::Cancelled)?;
    let dispatches = result.dispatches();
    let candidate_channels = result.into_pixels();
    let output_channels = if opacity.to_bits() == 1.0_f32.to_bits() {
        candidate_channels
    } else {
        source_channels
            .iter()
            .zip(candidate_channels)
            .map(|(source, candidate)| {
                let mut blended = *source;
                for channel in 0..3 {
                    blended[channel] =
                        source[channel] * (1.0 - opacity) + candidate[channel] * opacity;
                }
                blended
            })
            .collect()
    };
    lab_image_from_pixels(
        input,
        &output_channels,
        dispatches,
        boundary,
        output_mode,
        || transfer_scope.check().is_err(),
    )
    .map_err(|error| colorreconstruct_boundary_fallback(error, &transfer_scope))
}

fn colorreconstruct_boundary_fallback(
    error: PixelpipeGpuFallback,
    scope: &CancellationScope,
) -> PixelpipeGpuFallback {
    if scope.check().is_err() {
        return GpuColorReconstructionError::Cancelled.into();
    }
    match error {
        PixelpipeGpuFallback::Basic(BasicPointError::Readback(error)) => {
            GpuColorReconstructionError::Readback(error).into()
        }
        PixelpipeGpuFallback::Basic(BasicPointError::Poll(error)) => {
            GpuColorReconstructionError::Poll(error).into()
        }
        PixelpipeGpuFallback::Basic(error) => {
            GpuColorReconstructionError::Readback(error.to_string()).into()
        }
        error => error,
    }
}

fn gpu_colorreconstruct_precedence(
    precedence: rusttable_processing::operations::colorreconstruction::ColorReconstructionPrecedence,
) -> GpuColorReconstructionPrecedence {
    match precedence {
        rusttable_processing::operations::colorreconstruction::ColorReconstructionPrecedence::None =>
            GpuColorReconstructionPrecedence::None,
        rusttable_processing::operations::colorreconstruction::ColorReconstructionPrecedence::Chroma =>
            GpuColorReconstructionPrecedence::Chroma,
        rusttable_processing::operations::colorreconstruction::ColorReconstructionPrecedence::Hue =>
            GpuColorReconstructionPrecedence::Hue,
    }
}

fn execute_gpu_colorzones_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    plan: &ColorZonesPlan,
    opacity: f32,
    snapshot_identity: crate::CpuPixelpipeSnapshotIdentity,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let (packed, boundary) = packed_lab_d50(input, || scope.check().is_err())?;
    let (source_channels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() || source_channels.len() != input.pixels().len() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }

    let gpu_cancellation = GpuCancellationToken::new();
    let linked_cancellation = gpu_cancellation.clone();
    let _cleanup = scope.register_cleanup(move |_reason| linked_cancellation.cancel());
    let transfer_scope = scope.child(CancellationStage::Transfer);
    if transfer_scope.check().is_err() {
        return Err(GpuColorZonesError::Cancelled.into());
    }

    let identity = ColorZonesRequestIdentity::new(snapshot_identity.as_bytes());
    let transient_memory_budget = colorzones_transient_memory_bytes(source_channels.len())
        .ok_or(GpuColorZonesError::SizeOverflow)?;
    let request = ColorZonesRequest::new(
        source_channels,
        plan.lut(ColorZonesChannel::Lightness),
        plan.lut(ColorZonesChannel::Chroma),
        plan.lut(ColorZonesChannel::Hue),
        gpu_colorzones_selection(plan.config().channel()),
        gpu_colorzones_mode(plan.config().mode()),
        identity,
        transient_memory_budget,
    )
    .with_cancellation(&gpu_cancellation);
    let result = gpu.execute_colorzones(request)?;
    if transfer_scope.check().is_err() {
        return Err(GpuColorZonesError::Cancelled.into());
    }
    if result.identity() != identity {
        return Err(GpuColorZonesError::Readback(
            "Color Zones result identity does not match the immutable snapshot".to_owned(),
        )
        .into());
    }
    let dispatches = result.dispatches();
    let candidate_channels = result.into_pixels();
    let output_channels = if opacity.to_bits() == 1.0_f32.to_bits() {
        candidate_channels
    } else {
        let sources = source_channels
            .iter()
            .copied()
            .map(ColorZonesPixel::from_channels)
            .collect::<Vec<_>>();
        let candidates = candidate_channels
            .into_iter()
            .map(ColorZonesPixel::from_channels)
            .collect::<Vec<_>>();
        ColorZonesPlan::blend_lab_candidates(&sources, &candidates, None, opacity)
            .into_iter()
            .map(ColorZonesPixel::channels)
            .collect()
    };
    lab_image_from_pixels(
        input,
        &output_channels,
        dispatches,
        boundary,
        output_mode,
        || transfer_scope.check().is_err(),
    )
}

const fn gpu_colorzones_selection(channel: ColorZonesChannel) -> GpuColorZonesSelection {
    match channel {
        ColorZonesChannel::Lightness => GpuColorZonesSelection::Lightness,
        ColorZonesChannel::Chroma => GpuColorZonesSelection::Chroma,
        ColorZonesChannel::Hue => GpuColorZonesSelection::Hue,
    }
}

const fn gpu_colorzones_mode(mode: ColorZonesMode) -> GpuColorZonesMode {
    match mode {
        ColorZonesMode::Smooth => GpuColorZonesMode::Smooth,
        ColorZonesMode::Strong => GpuColorZonesMode::Strong,
    }
}

fn execute_gpu_grain_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    config: rusttable_processing::operations::grain::GrainConfig,
    full_dimensions: RasterDimensions,
    origin: (u32, u32),
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let dimensions = input.descriptor().dimensions();
    let (frame, packed) = packed_linear_working(input)?;
    let plan = GrainPlan::new(config, full_dimensions)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let parameters = plan.gpu_parameters();
    let result = gpu.execute_grain_point(GrainPointRequest {
        pixels: &packed,
        width: dimensions.width(),
        height: dimensions.height(),
        full_width: full_dimensions.width(),
        full_height: full_dimensions.height(),
        origin_x: origin.0,
        origin_y: origin.1,
        channel: parameters.channel.id(),
        seed: parameters.seed,
        zoom: parameters.zoom,
        strength: parameters.strength,
        lut: plan.gpu_lut(),
    })?;
    image_from_packed(
        input,
        output_mode,
        frame,
        result.pixels(),
        result.dispatches(),
    )
}

fn packed_linear_working(
    input: &RgbaF32Image,
) -> Result<(WorkingFrameDescriptor, Vec<f32>), PixelpipeGpuFallback> {
    let linear =
        to_linear_working(input).map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let mut packed = Vec::with_capacity(input.pixels().len() * 4);
    for (working, source) in linear.pixels().zip(input.pixels()) {
        packed.extend([
            working.red().get(),
            working.green().get(),
            working.blue().get(),
            source.alpha(),
        ]);
    }
    Ok((linear.frame(), packed))
}

fn image_from_packed(
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    frame: WorkingFrameDescriptor,
    packed: &[f32],
    dispatches: u32,
) -> Result<(RgbaF32Image, u32), PixelpipeGpuFallback> {
    let dimensions = input.descriptor().dimensions();
    let (packed_pixels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() || packed_pixels.len() != input.pixels().len() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }
    let mut working_pixels = Vec::with_capacity(input.pixels().len());
    for (index, pixel) in packed_pixels.iter().enumerate() {
        working_pixels.push(LinearRgb::new(
            FiniteF32::new(pixel[0]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4,
            })?,
            FiniteF32::new(pixel[1]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 1,
            })?,
            FiniteF32::new(pixel[2]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 2,
            })?,
        ));
    }
    let working = WorkingRgbImage::new_with_frame(dimensions, working_pixels, frame)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let image = output_from_working(output_mode, input, &working)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    Ok((image, dispatches))
}

#[derive(Debug)]
enum GpuTiledExecutionError {
    Fallback(PixelpipeGpuFallback),
    Cancelled(CancellationError),
}

impl From<PixelpipeGpuFallback> for GpuTiledExecutionError {
    fn from(error: PixelpipeGpuFallback) -> Self {
        Self::Fallback(error)
    }
}

impl From<BasicPointError> for GpuTiledExecutionError {
    fn from(error: BasicPointError) -> Self {
        Self::Fallback(error.into())
    }
}

fn check_tiled_cancellation(
    scope: &CancellationScope,
    stage: CancellationStage,
) -> Result<(), GpuTiledExecutionError> {
    scope
        .child(stage)
        .check()
        .map_err(GpuTiledExecutionError::Cancelled)
}

fn execute_gpu_tiled(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    plan: &GpuPlan,
    tile_plan: crate::CpuTilePlan,
    scope: &CancellationScope,
) -> Result<(RgbaF32Image, u32, u64), GpuTiledExecutionError> {
    if matches!(
        plan,
        GpuPlan::Bloom { .. }
            | GpuPlan::ColorReconstruction { .. }
            | GpuPlan::ShadhiBilateral { .. }
    ) {
        check_tiled_cancellation(scope, CancellationStage::Transfer)?;
        let (image, dispatches) = execute_gpu(gpu, snapshot, plan, scope)?;
        check_tiled_cancellation(scope, CancellationStage::Transfer)?;
        return Ok((image, dispatches, 1));
    }
    let grid = tile_plan
        .grid_for(snapshot.input().descriptor().dimensions())
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let input = snapshot.input();
    let mut assembled = vec![None; input.pixels().len()];
    let mut dispatches = 0_u32;
    for tile_index in 0..grid.tile_count() {
        check_tiled_cancellation(scope, CancellationStage::Tile)?;
        let tile = grid
            .tile_at(tile_index)
            .map_err(|error| BasicPointError::Readback(error.to_string()))?
            .ok_or_else(|| BasicPointError::Readback("tile disappeared from grid".to_owned()))?;
        let tile_input = extract_tile(input, tile)?;
        check_tiled_cancellation(scope, CancellationStage::Transfer)?;
        let (tile_output, tile_dispatches) = match plan {
            GpuPlan::Basic(operations) => {
                execute_gpu_image(gpu, &tile_input, snapshot.output_mode(), operations)?
            }
            GpuPlan::ColorZones { plan, opacity } => execute_gpu_colorzones_image(
                gpu,
                &tile_input,
                snapshot.output_mode(),
                plan,
                *opacity,
                snapshot.identity(),
                scope,
            )?,
            GpuPlan::Grain(config) => execute_gpu_grain_image(
                gpu,
                &tile_input,
                snapshot.output_mode(),
                *config,
                input.descriptor().dimensions(),
                (tile.origin_x(), tile.origin_y()),
            )?,
            GpuPlan::Bloom { .. }
            | GpuPlan::ColorReconstruction { .. }
            | GpuPlan::ShadhiBilateral { .. } => {
                unreachable!("full-frame GPU plans are dispatched before tile iteration")
            }
        };
        check_tiled_cancellation(scope, CancellationStage::Transfer)?;
        dispatches = dispatches.saturating_add(tile_dispatches);
        place_tile(&mut assembled, input, tile, &tile_output)?;
    }
    let pixels = assembled
        .into_iter()
        .map(|pixel| pixel.ok_or_else(|| BasicPointError::Readback("tiled output gap".to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    let descriptor = output_descriptor(
        snapshot.output_mode(),
        input.descriptor(),
        input.descriptor().dimensions(),
    );
    let output = RgbaF32Image::new(descriptor, pixels)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    Ok((output, dispatches, grid.tile_count()))
}

fn extract_tile(
    input: &RgbaF32Image,
    tile: crate::CpuPixelpipeTile,
) -> Result<RgbaF32Image, BasicPointError> {
    let dimensions = tile.dimensions();
    let source_width = input.descriptor().dimensions().width();
    let pixel_count = usize::try_from(dimensions.pixel_count())
        .map_err(|_| BasicPointError::Readback("tile pixel count is too large".to_owned()))?;
    let mut pixels = Vec::with_capacity(pixel_count);
    for y in 0..dimensions.height() {
        let row = u64::from(tile.origin_y() + y)
            .checked_mul(u64::from(source_width))
            .and_then(|offset| offset.checked_add(u64::from(tile.origin_x())))
            .ok_or_else(|| BasicPointError::Readback("tile source index overflow".to_owned()))?;
        let start = usize::try_from(row)
            .map_err(|_| BasicPointError::Readback("tile source index is too large".to_owned()))?;
        let end = start
            .checked_add(dimensions.width() as usize)
            .ok_or_else(|| BasicPointError::Readback("tile row overflow".to_owned()))?;
        let row_pixels = input.pixels().get(start..end).ok_or_else(|| {
            BasicPointError::Readback("tile source row is out of bounds".to_owned())
        })?;
        pixels.extend_from_slice(row_pixels);
    }
    RgbaF32Image::new(
        input
            .descriptor()
            .with_dimensions_and_color_encoding(dimensions, input.descriptor().color_encoding()),
        pixels,
    )
    .map_err(|error| BasicPointError::Readback(error.to_string()))
}

fn place_tile(
    assembled: &mut [Option<RgbaF32Pixel>],
    input: &RgbaF32Image,
    tile: crate::CpuPixelpipeTile,
    output: &RgbaF32Image,
) -> Result<(), BasicPointError> {
    let source_width = input.descriptor().dimensions().width();
    let dimensions = tile.dimensions();
    for y in 0..dimensions.height() {
        let destination_row = u64::from(tile.origin_y() + y)
            .checked_mul(u64::from(source_width))
            .and_then(|offset| offset.checked_add(u64::from(tile.origin_x())))
            .ok_or_else(|| {
                BasicPointError::Readback("tile destination index overflow".to_owned())
            })?;
        let destination = usize::try_from(destination_row).map_err(|_| {
            BasicPointError::Readback("tile destination index is too large".to_owned())
        })?;
        let source = usize::try_from(u64::from(y) * u64::from(dimensions.width()))
            .map_err(|_| BasicPointError::Readback("tile output index is too large".to_owned()))?;
        for x in 0..dimensions.width() as usize {
            let destination_index = destination.checked_add(x).ok_or_else(|| {
                BasicPointError::Readback("tile destination row overflow".to_owned())
            })?;
            if assembled.get(destination_index).is_none()
                || output.pixels().get(source + x).is_none()
            {
                return Err(BasicPointError::Readback(
                    "tile output is out of bounds".to_owned(),
                ));
            }
            if assembled[destination_index].is_some() {
                return Err(BasicPointError::Readback("tiled output overlap".to_owned()));
            }
            assembled[destination_index] = Some(output.pixels()[source + x]);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Barrier;
    use std::time::Instant;

    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationKey, OperationOpacity, ParameterName,
        ParameterValue, PhotoId, Revision,
    };

    use super::*;
    use crate::{RgbaF32ColorEncoding, RgbaF32Descriptor};

    #[test]
    fn auto_basicadj_candidate_resolves_once_and_carries_the_same_identity() {
        let operation_id = OperationId::new(0xba51).expect("operation ID");
        let config = BasicAdjConfig::defaults()
            .with_auto_controls(rusttable_processing::BasicAdjAutoControls::all());
        let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
        let sample = LinearRgb::new(
            FiniteF32::new(0.15).expect("red"),
            FiniteF32::new(0.30).expect("green"),
            FiniteF32::new(0.45).expect("blue"),
        );
        let pixels = [sample; 6];
        let raster = rusttable_processing::BasicAdjAnalysisRaster::new(dimensions, &pixels, None)
            .expect("analysis raster");
        let plan = BasicAdjPlan::resolve(config, raster).expect("resolved automatic plan");
        let expected_operation = basicadj_point_operation(&plan);
        let identity = [0xa5; 32];
        let resolved = ResolvedBasicAdjPlans {
            plans: BTreeMap::from([(operation_id, plan)]),
            identity,
        };
        let calls = Cell::new(0_usize);
        let candidate = GpuPlanCandidate::Basic(vec![BasicPointCandidate::BasicAdj {
            operation_id,
            config,
        }]);

        let qualified = resolve_gpu_plan_candidate(candidate, || {
            calls.set(calls.get() + 1);
            Ok(Some(resolved))
        })
        .expect("qualification")
        .expect("qualified plan");

        assert_eq!(calls.get(), 1);
        assert_eq!(qualified.basicadj_plan_identity, identity);
        assert_eq!(
            qualified.plan,
            GpuPlan::Basic(vec![expected_operation]),
            "the operation and receipt identity must come from one resolution"
        );
    }

    #[test]
    fn auto_basicadj_analysis_cancellation_is_terminal() {
        let operation_id = OperationId::new(0xba52).expect("operation ID");
        let config = BasicAdjConfig::defaults()
            .with_auto_controls(rusttable_processing::BasicAdjAutoControls::all());
        let candidate = GpuPlanCandidate::Basic(vec![BasicPointCandidate::BasicAdj {
            operation_id,
            config,
        }]);
        let scope = CancellationScope::root(
            PipelineGeneration::new(18).expect("nonzero pipeline generation"),
        );
        scope.cancel(CancellationReason::EditChanged);
        let snapshot = empty_snapshot();
        let calls = Cell::new(0_usize);

        let result = resolve_gpu_plan_candidate(candidate, || {
            calls.set(calls.get() + 1);
            prepare_gpu_basicadj_plans(&snapshot, &scope)
        });

        let Err(CpuPixelpipeError::Cancelled(error)) = result else {
            panic!("automatic BasicAdj cancellation must not become CPU fallback");
        };
        assert_eq!(calls.get(), 1);
        assert_eq!(error.reason(), CancellationReason::EditChanged);
        assert_eq!(error.stage(), Some(CancellationStage::Analysis));
    }

    #[test]
    fn non_basicadj_qualification_skips_analysis_resolution() {
        let candidate = GpuPlanCandidate::Basic(vec![BasicPointCandidate::Ready(
            BasicPointOperation::Exposure {
                stops: 1.0,
                black: 0.0,
            },
        )]);

        let qualified = resolve_gpu_plan_candidate(candidate, || {
            panic!("non-BasicAdj qualification must not resolve BasicAdj evidence")
        })
        .expect("qualification")
        .expect("qualified plan");

        assert_eq!(qualified.basicadj_plan_identity, [0; 32]);
        assert_eq!(
            qualified.plan,
            GpuPlan::Basic(vec![BasicPointOperation::Exposure {
                stops: 1.0,
                black: 0.0,
            }])
        );
    }

    #[test]
    fn velvia_qualification_normalizes_native_percent_strength_once() {
        let config = rusttable_processing::operations::velvia::VelviaConfig::new(25.0, 0.75)
            .expect("Velvia config");
        assert_eq!(
            velvia_point_operation(config),
            BasicPointOperation::Velvia {
                strength: 0.25,
                bias: 0.75,
            }
        );
    }

    #[test]
    fn vibrance_qualification_normalizes_native_percent_amount_once() {
        let config = rusttable_processing::operations::vibrance::VibranceConfig::new(25.0)
            .expect("Vibrance config");
        assert_eq!(
            vibrance_point_operation(config),
            BasicPointOperation::Vibrance { amount: 0.25 }
        );
    }

    #[test]
    fn bloom_qualification_uses_a_full_frame_plan_and_preserves_opacity() {
        let opacity = OperationOpacity::new(0.375).expect("partial opacity");
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [scalar_edit_operation(
                0xb100,
                "rusttable.bloom",
                true,
                opacity,
                &[("size", 20.0), ("threshold", 90.0), ("strength", 25.0)],
            )],
        );

        let candidate = gpu_plan_candidate(&snapshot).expect("Bloom GPU candidate");
        let GpuPlanCandidate::Bloom {
            config,
            opacity: actual_opacity,
        } = candidate
        else {
            panic!("Bloom must use its dedicated full-frame path");
        };

        assert_eq!(f64::from(actual_opacity).to_bits(), opacity.get().to_bits());
        assert_eq!(config.size().to_bits(), 20.0_f32.to_bits());
        assert_eq!(config.threshold().to_bits(), 90.0_f32.to_bits());
        assert_eq!(config.strength().to_bits(), 25.0_f32.to_bits());
    }

    #[test]
    fn bloom_qualification_falls_back_for_masks_or_mixed_active_chains() {
        let bloom = || {
            scalar_edit_operation(
                0xb101,
                "rusttable.bloom",
                true,
                OperationOpacity::ONE,
                &[("size", 20.0), ("threshold", 90.0), ("strength", 25.0)],
            )
        };
        let masked = snapshot_with_operations(RgbaF32ColorEncoding::LabD50, [bloom()])
            .with_mask_graph(
                rusttable_masks::MaskGraphBuilder::new()
                    .build()
                    .expect("empty mask graph"),
            );
        let mixed = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [
                bloom(),
                scalar_edit_operation(
                    0xb102,
                    "rusttable.vibrance",
                    true,
                    OperationOpacity::ONE,
                    &[("amount", 25.0)],
                ),
            ],
        );

        assert!(gpu_plan_candidate(&masked).is_none());
        assert!(gpu_plan_candidate(&mixed).is_none());
    }

    #[test]
    fn colorzones_qualification_uses_a_dedicated_plan_and_preserves_opacity() {
        let opacity = OperationOpacity::new(0.375).expect("partial opacity");
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorzones_edit_operation(0xc700, opacity)],
        );

        let candidate = gpu_plan_candidate(&snapshot).expect("Color Zones GPU candidate");
        let GpuPlanCandidate::ColorZones {
            plan,
            opacity: actual_opacity,
        } = candidate
        else {
            panic!("Color Zones must not enter the generic point chain");
        };

        assert_eq!(f64::from(actual_opacity).to_bits(), opacity.get().to_bits());
        assert_eq!(plan.config().mode(), ColorZonesMode::Smooth);
        assert_eq!(plan.config().channel(), ColorZonesChannel::Hue);
        assert_eq!(
            gpu_colorzones_mode(plan.config().mode()),
            GpuColorZonesMode::Smooth
        );
        assert_eq!(
            gpu_colorzones_selection(plan.config().channel()),
            GpuColorZonesSelection::Hue
        );
    }

    #[test]
    fn colorzones_qualification_falls_back_for_masks_or_mixed_active_chains() {
        let masked = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorzones_edit_operation(0xc701, OperationOpacity::ONE)],
        )
        .with_mask_graph(
            rusttable_masks::MaskGraphBuilder::new()
                .build()
                .expect("empty mask graph"),
        );
        let mixed = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [
                colorzones_edit_operation(0xc702, OperationOpacity::ONE),
                scalar_edit_operation(
                    0xc703,
                    "rusttable.vibrance",
                    true,
                    OperationOpacity::ONE,
                    &[("amount", 25.0)],
                ),
            ],
        );

        assert!(gpu_plan_candidate(&masked).is_none());
        assert!(gpu_plan_candidate(&mixed).is_none());
    }

    #[test]
    fn colorcorrection_qualification_uses_native_committed_coefficients() {
        let config =
            rusttable_processing::ColorCorrectionConfig::new(20.0, -30.0, -10.0, 5.0, 1.25)
                .expect("Color Correction config");

        assert_eq!(
            colorcorrection_point_operation(config),
            BasicPointOperation::ColorCorrection {
                saturation: 1.25,
                a_scale: 0.3,
                a_base: -10.0,
                b_scale: -0.35,
                b_base: 5.0,
            }
        );
    }

    #[test]
    fn colorcorrection_gpu_plan_preserves_mixed_multiple_instances_in_authored_order() {
        use rusttable_processing::operations::colorcontrast::ColorContrastConfig;
        use rusttable_processing::operations::vibrance::VibranceConfig;

        let first_correction =
            rusttable_processing::ColorCorrectionConfig::new(20.0, -30.0, -10.0, 5.0, 1.25)
                .expect("first Color Correction config");
        let contrast =
            ColorContrastConfig::new(2.0, 3.0, 1.5, -4.0, 1).expect("Color Contrast config");
        let first_vibrance = VibranceConfig::new(25.0).expect("first Vibrance config");
        let second_correction =
            rusttable_processing::ColorCorrectionConfig::new(-8.0, 18.0, 2.0, -7.0, 0.8)
                .expect("second Color Correction config");
        let second_vibrance = VibranceConfig::new(-40.0).expect("second Vibrance config");
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [
                colorcorrection_edit_operation(
                    0xc001,
                    OperationOpacity::ONE,
                    [20.0, -30.0, -10.0, 5.0, 1.25],
                ),
                colorcontrast_edit_operation(
                    0xcc09,
                    OperationOpacity::ONE,
                    [2.0, 3.0, 1.5, -4.0],
                    1,
                ),
                scalar_edit_operation(
                    0x5101,
                    "rusttable.vibrance",
                    true,
                    OperationOpacity::ONE,
                    &[("amount", 25.0)],
                ),
                colorcorrection_edit_operation(
                    0xc002,
                    OperationOpacity::ONE,
                    [-8.0, 18.0, 2.0, -7.0, 0.8],
                ),
                scalar_edit_operation(
                    0x5102,
                    "rusttable.vibrance",
                    true,
                    OperationOpacity::ONE,
                    &[("amount", -40.0)],
                ),
            ],
        );
        let candidate = gpu_plan_candidate(&snapshot).expect("continuous Lab GPU candidate");
        let qualified = resolve_gpu_plan_candidate(candidate, || {
            panic!("Lab point operations do not require BasicAdj analysis")
        })
        .expect("qualification")
        .expect("qualified plan");
        let expected = vec![
            colorcorrection_point_operation(first_correction),
            colorcontrast_point_operation(contrast),
            vibrance_point_operation(first_vibrance),
            colorcorrection_point_operation(second_correction),
            vibrance_point_operation(second_vibrance),
        ];

        assert_eq!(qualified.plan, GpuPlan::Basic(expected.clone()));
        assert_eq!(
            basic_point_chain_color_space(
                snapshot.input().descriptor().color_encoding(),
                &expected,
            ),
            Ok(BasicPointColorSpace::LabD50)
        );
    }

    #[test]
    fn colorcontrast_gpu_plan_preserves_an_opaque_unmasked_lab_chain_in_authored_order() {
        use rusttable_processing::operations::colorcontrast::ColorContrastConfig;

        let first = ColorContrastConfig::new(2.0, 3.0, 1.5, -4.0, 1).expect("first config");
        let second = ColorContrastConfig::new(0.5, -2.0, 2.0, 5.0, 0).expect("second config");
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [
                colorcontrast_edit_operation(
                    0xcc01,
                    OperationOpacity::ONE,
                    [2.0, 3.0, 1.5, -4.0],
                    1,
                ),
                colorcontrast_edit_operation(
                    0xcc02,
                    OperationOpacity::ONE,
                    [0.5, -2.0, 2.0, 5.0],
                    0,
                ),
            ],
        );
        let candidate = gpu_plan_candidate(&snapshot).expect("opaque unmasked GPU candidate");
        let qualified = resolve_gpu_plan_candidate(candidate, || {
            panic!("Color Contrast does not require BasicAdj analysis")
        })
        .expect("qualification")
        .expect("qualified plan");
        let expected = vec![
            colorcontrast_point_operation(first),
            colorcontrast_point_operation(second),
        ];

        assert_eq!(qualified.plan, GpuPlan::Basic(expected.clone()));
        assert_eq!(
            basic_point_chain_color_space(
                snapshot.input().descriptor().color_encoding(),
                &expected,
            ),
            Ok(BasicPointColorSpace::LabD50)
        );
    }

    #[test]
    fn disabled_only_colorcontrast_graph_routes_directly_to_canonical_cpu() {
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcontrast_edit_operation_with_enabled(
                0xcc05,
                false,
                OperationOpacity::ONE,
                [2.0, 3.0, 1.5, -4.0],
                1,
            )],
        );

        assert!(gpu_plan_candidate(&snapshot).is_none());
    }

    #[test]
    fn zero_opacity_only_colorcontrast_graph_routes_directly_to_canonical_cpu() {
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcontrast_edit_operation(
                0xcc06,
                OperationOpacity::ZERO,
                [2.0, 3.0, 1.5, -4.0],
                1,
            )],
        );

        assert!(gpu_plan_candidate(&snapshot).is_none());
    }

    #[test]
    fn truly_empty_graph_retains_an_empty_gpu_plan() {
        let snapshot = snapshot_with_operations(RgbaF32ColorEncoding::LabD50, std::iter::empty());
        let candidate = gpu_plan_candidate(&snapshot).expect("empty graph GPU candidate");
        let qualified = resolve_gpu_plan_candidate(candidate, || {
            panic!("an empty point chain does not require BasicAdj analysis")
        })
        .expect("qualification")
        .expect("qualified plan");

        assert_eq!(qualified.plan, GpuPlan::Basic(Vec::new()));
    }

    #[test]
    fn zero_opacity_non_colorcontrast_preserves_an_active_lab_colorcontrast_chain() {
        use rusttable_processing::operations::colorcontrast::ColorContrastConfig;

        let first = ColorContrastConfig::new(1.25, 3.0, 0.8, -2.0, 1).expect("first config");
        let second = ColorContrastConfig::new(0.7, -4.0, 1.4, 5.0, 0).expect("second config");
        let snapshot = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [
                colorcontrast_edit_operation(
                    0xcc07,
                    OperationOpacity::ONE,
                    [1.25, 3.0, 0.8, -2.0],
                    1,
                ),
                scalar_edit_operation(
                    0xe001,
                    "rusttable.exposure",
                    true,
                    OperationOpacity::ZERO,
                    &[("stops", 3.0)],
                ),
                colorcontrast_edit_operation(
                    0xcc08,
                    OperationOpacity::ONE,
                    [0.7, -4.0, 1.4, 5.0],
                    0,
                ),
            ],
        );
        let candidate =
            gpu_plan_candidate(&snapshot).expect("continuous Color Contrast GPU candidate");
        let qualified = resolve_gpu_plan_candidate(candidate, || {
            panic!("Color Contrast does not require BasicAdj analysis")
        })
        .expect("qualification")
        .expect("qualified plan");
        let expected = vec![
            colorcontrast_point_operation(first),
            colorcontrast_point_operation(second),
        ];

        assert_eq!(qualified.plan, GpuPlan::Basic(expected.clone()));
        assert_eq!(
            basic_point_chain_color_space(
                snapshot.input().descriptor().color_encoding(),
                &expected,
            ),
            Ok(BasicPointColorSpace::LabD50)
        );
    }

    #[test]
    fn lab_gpu_routing_adds_an_rgb_boundary_but_rejects_mixed_domain_chains() {
        let colorcorrection = BasicPointOperation::ColorCorrection {
            saturation: 1.25,
            a_scale: 0.3,
            a_base: -10.0,
            b_scale: -0.35,
            b_base: 5.0,
        };
        let colorcontrast = BasicPointOperation::ColorContrast {
            a_steepness: 2.0,
            a_offset: 3.0,
            b_steepness: 1.5,
            b_offset: -4.0,
            unbound: true,
        };
        let exposure = BasicPointOperation::Exposure {
            stops: 1.0,
            black: 0.0,
        };
        let fallback = Err(BasicPointError::ColorSpaceBoundaryUnavailable {
            required: BasicPointColorSpace::LabD50,
        });

        assert_eq!(
            basic_point_chain_color_space(
                RgbaF32ColorEncoding::LinearSrgbD65,
                &[colorcorrection, colorcontrast],
            ),
            Ok(BasicPointColorSpace::LabD50)
        );
        for operations in [
            [colorcorrection, exposure],
            [exposure, colorcorrection],
            [colorcontrast, exposure],
            [exposure, colorcontrast],
        ] {
            assert_eq!(
                basic_point_chain_color_space(RgbaF32ColorEncoding::LabD50, &operations),
                fallback
            );
        }
    }

    #[test]
    fn colorcontrast_gpu_plan_rejects_partial_opacity_and_mask_state() {
        let partial = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcontrast_edit_operation(
                0xcc03,
                OperationOpacity::new(0.5).expect("partial opacity"),
                [1.0, 0.0, 1.0, 0.0],
                1,
            )],
        );
        let masked = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcontrast_edit_operation(
                0xcc04,
                OperationOpacity::ONE,
                [1.0, 0.0, 1.0, 0.0],
                1,
            )],
        )
        .with_mask_graph(
            rusttable_masks::MaskGraphBuilder::new()
                .build()
                .expect("empty mask graph"),
        );

        assert!(gpu_plan_candidate(&partial).is_none());
        assert!(gpu_plan_candidate(&masked).is_none());
    }

    #[test]
    fn colorcorrection_gpu_plan_rejects_partial_opacity_and_mask_state() {
        let partial = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcorrection_edit_operation(
                0xc003,
                OperationOpacity::new(0.5).expect("partial opacity"),
                [20.0, -30.0, -10.0, 5.0, 1.25],
            )],
        );
        let masked = snapshot_with_operations(
            RgbaF32ColorEncoding::LabD50,
            [colorcorrection_edit_operation(
                0xc004,
                OperationOpacity::ONE,
                [20.0, -30.0, -10.0, 5.0, 1.25],
            )],
        )
        .with_mask_graph(
            rusttable_masks::MaskGraphBuilder::new()
                .build()
                .expect("empty mask graph"),
        );

        assert!(gpu_plan_candidate(&partial).is_none());
        assert!(gpu_plan_candidate(&masked).is_none());
    }

    fn colorcorrection_edit_operation(
        id: u128,
        opacity: OperationOpacity,
        parameters: [f64; 5],
    ) -> Operation {
        let [hia, hib, loa, lob, saturation] = parameters;
        scalar_edit_operation(
            id,
            "rusttable.colorcorrection",
            true,
            opacity,
            &[
                ("hia", hia),
                ("hib", hib),
                ("loa", loa),
                ("lob", lob),
                ("saturation", saturation),
            ],
        )
    }

    fn colorcontrast_edit_operation(
        id: u128,
        opacity: OperationOpacity,
        parameters: [f64; 4],
        unbound: i64,
    ) -> Operation {
        colorcontrast_edit_operation_with_enabled(id, true, opacity, parameters, unbound)
    }

    fn colorcontrast_edit_operation_with_enabled(
        id: u128,
        enabled: bool,
        opacity: OperationOpacity,
        parameters: [f64; 4],
        unbound: i64,
    ) -> Operation {
        let [a_steepness, a_offset, b_steepness, b_offset] = parameters;
        let scalar = |value| {
            ParameterValue::Scalar(FiniteF64::new(value).expect("finite Color Contrast parameter"))
        };
        Operation::new_with_opacity(
            OperationId::new(id).expect("operation ID"),
            OperationKey::new("rusttable.colorcontrast").expect("operation key"),
            enabled,
            opacity,
            [
                ("a_steepness", scalar(a_steepness)),
                ("a_offset", scalar(a_offset)),
                ("b_steepness", scalar(b_steepness)),
                ("b_offset", scalar(b_offset)),
                ("unbound", ParameterValue::Integer(unbound)),
            ]
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
        )
        .expect("Color Contrast operation")
    }

    fn scalar_edit_operation(
        id: u128,
        key: &str,
        enabled: bool,
        opacity: OperationOpacity,
        parameters: &[(&str, f64)],
    ) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).expect("operation ID"),
            OperationKey::new(key).expect("operation key"),
            enabled,
            opacity,
            parameters.iter().map(|(name, value)| {
                (
                    ParameterName::new(*name).expect("parameter name"),
                    ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
                )
            }),
        )
        .expect("scalar operation")
    }

    fn colorzones_edit_operation(id: u128, opacity: OperationOpacity) -> Operation {
        let operation_id = OperationId::new(id).expect("Color Zones operation ID");
        let defaults = rusttable_processing::builtin_registry()
            .materialize_operation("rusttable.colorzones", operation_id)
            .expect("Color Zones defaults");
        Operation::new_with_opacity(
            defaults.id(),
            defaults.key().clone(),
            true,
            opacity,
            defaults
                .parameters()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .expect("Color Zones edit operation")
    }

    fn empty_snapshot() -> CpuPixelpipeSnapshot {
        snapshot_with_encoding(RgbaF32ColorEncoding::SrgbD65)
    }

    fn snapshot_with_encoding(encoding: RgbaF32ColorEncoding) -> CpuPixelpipeSnapshot {
        snapshot_with_operations(encoding, std::iter::empty())
    }

    fn snapshot_with_operations(
        encoding: RgbaF32ColorEncoding,
        operations: impl IntoIterator<Item = Operation>,
    ) -> CpuPixelpipeSnapshot {
        let edit = Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(1),
            operations,
        )
        .expect("edit");
        let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
        let input = RgbaF32Image::new(
            RgbaF32Descriptor::new(dimensions, encoding),
            vec![RgbaF32Pixel::new(0.25, 0.5, 0.75, 1.0)],
        )
        .expect("input");
        CpuPixelpipeSnapshot::new(
            input,
            rusttable_processing::CompiledOperationGraph::compile(&edit).expect("graph"),
            CpuPixelpipeOutputMode::FullExport,
        )
    }

    #[test]
    fn production_execution_reuses_the_complete_result_and_receipt() {
        let service = PixelpipeExecutionService::cpu_only();
        let snapshot = empty_snapshot();
        let first = service.execute(&snapshot).expect("first execution");
        let second_scope =
            CancellationScope::root(PipelineGeneration::new(99).expect("generation"));
        let second = service
            .execute_with_cancellation(&snapshot, &second_scope)
            .expect("cached execution");

        assert_eq!(first, second);
        assert!(
            Arc::ptr_eq(&first.image, &second.image),
            "a warm hit must share the immutable full-frame raster"
        );
        assert_eq!(service.uncached_execution_count(), 1);
    }

    #[test]
    fn typed_execution_failure_is_reused_without_duplicate_work() {
        let service = PixelpipeExecutionService::cpu_only();
        let snapshot = snapshot_with_encoding(RgbaF32ColorEncoding::Rec2020D65);
        let first = service
            .execute(&snapshot)
            .expect_err("unsupported encoding");
        let second = service.execute(&snapshot).expect_err("suppressed repeat");

        assert_eq!(first, second);
        assert!(matches!(
            second,
            CpuPixelpipeError::UnsupportedInputEncoding {
                actual: RgbaF32ColorEncoding::Rec2020D65
            }
        ));
        assert_eq!(service.uncached_execution_count(), 1);
    }

    #[test]
    fn concurrent_consumers_share_one_typed_execution_failure() {
        let service = Arc::new(PixelpipeExecutionService::cpu_only());
        let snapshot = Arc::new(snapshot_with_encoding(RgbaF32ColorEncoding::Rec2020D65));
        let start = Arc::new(Barrier::new(3));
        let workers = (0..2)
            .map(|_| {
                let service = service.clone();
                let snapshot = snapshot.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    service
                        .execute(&snapshot)
                        .expect_err("unsupported encoding")
                })
            })
            .collect::<Vec<_>>();
        start.wait();
        let errors = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker"))
            .collect::<Vec<_>>();

        assert_eq!(errors[0], errors[1]);
        assert!(matches!(
            errors[0],
            CpuPixelpipeError::UnsupportedInputEncoding {
                actual: RgbaF32ColorEncoding::Rec2020D65
            }
        ));
        assert_eq!(service.uncached_execution_count(), 1);
    }

    #[test]
    fn cancellation_wins_over_a_warm_production_cache_hit() {
        let service = PixelpipeExecutionService::cpu_only();
        let snapshot = empty_snapshot();
        service.execute(&snapshot).expect("warm cache");
        let scope = CancellationScope::root(PipelineGeneration::new(7).expect("generation"));
        scope.cancel(CancellationReason::SelectionChanged);

        let error = service
            .execute_with_cancellation(&snapshot, &scope)
            .expect_err("cancelled consumer");
        let CpuPixelpipeError::Cancelled(error) = error else {
            panic!("expected typed cancellation");
        };
        assert_eq!(error.reason(), CancellationReason::SelectionChanged);
        assert_eq!(service.uncached_execution_count(), 1);
    }

    #[test]
    fn expired_deadline_never_enters_a_cold_cache_build() {
        let service = PixelpipeExecutionService::cpu_only();
        let snapshot = empty_snapshot();
        let scope = CancellationScope::root(PipelineGeneration::new(8).expect("generation"))
            .with_deadline(crate::CancellationDeadline::at(Instant::now()));

        let error = service
            .execute_with_cancellation(&snapshot, &scope)
            .expect_err("expired deadline");
        let CpuPixelpipeError::Cancelled(error) = error else {
            panic!("expected typed cancellation");
        };
        assert_eq!(error.reason(), CancellationReason::DeadlineExceeded);
        assert_eq!(service.uncached_execution_count(), 0);
    }

    #[test]
    fn direct_and_tiled_production_receipts_never_alias() {
        let service = PixelpipeExecutionService::cpu_only();
        let snapshot = empty_snapshot();
        service.execute(&snapshot).expect("direct execution");
        let tiled = service
            .execute_tiled(&snapshot, crate::CpuTilePlan::new(1, 1).expect("tile plan"))
            .expect("tiled execution");

        assert!(tiled.receipt().tiling().is_some());
        assert_eq!(service.uncached_execution_count(), 2);
    }

    #[test]
    fn availability_failures_retain_the_selected_backend_type() {
        let basic = GpuPlan::Basic(Vec::new());
        let colorzones = GpuPlan::ColorZones {
            plan: ColorZonesPlan::new(
                rusttable_processing::operations::colorzones::ColorZonesConfig::defaults(),
            )
            .expect("Color Zones plan"),
            opacity: 0.5,
        };
        let grain =
            GpuPlan::Grain(rusttable_processing::operations::grain::GrainConfig::defaults());
        let bilateral = GpuPlan::ShadhiBilateral {
            config: ShadhiConfig::defaults(),
            opacity: 1.0,
        };

        assert_eq!(
            basic.availability_error(true),
            PixelpipeGpuFallback::Basic(BasicPointError::CpuOnly)
        );
        assert_eq!(
            basic.availability_error(false),
            PixelpipeGpuFallback::Basic(BasicPointError::Unhealthy)
        );
        assert_eq!(
            colorzones.availability_error(true),
            PixelpipeGpuFallback::ColorZones(GpuColorZonesError::CpuOnly)
        );
        assert_eq!(
            colorzones.availability_error(false),
            PixelpipeGpuFallback::ColorZones(GpuColorZonesError::Unhealthy)
        );
        assert_eq!(
            grain.availability_error(true),
            PixelpipeGpuFallback::Grain(GrainPointError::CpuOnly)
        );
        assert_eq!(
            grain.availability_error(false),
            PixelpipeGpuFallback::Grain(GrainPointError::Unhealthy)
        );
        assert_eq!(
            bilateral.availability_error(true),
            PixelpipeGpuFallback::Bilateral(BilateralGridError::CpuOnly)
        );
        assert_eq!(
            bilateral.availability_error(false),
            PixelpipeGpuFallback::Bilateral(BilateralGridError::Unhealthy)
        );
    }

    #[test]
    fn colorreconstruct_boundaries_retain_backend_and_cancellation_identity() {
        let scope = CancellationScope::root(PipelineGeneration::new(12).expect("generation"))
            .child(CancellationStage::Transfer);
        assert_eq!(
            colorreconstruct_boundary_fallback(
                PixelpipeGpuFallback::Basic(BasicPointError::Readback(
                    "RGB-to-Lab ingress failed".to_owned(),
                )),
                &scope,
            ),
            PixelpipeGpuFallback::ColorReconstruction(GpuColorReconstructionError::Readback(
                "RGB-to-Lab ingress failed".to_owned(),
            ),)
        );

        scope.cancel(CancellationReason::SelectionChanged);
        assert_eq!(
            colorreconstruct_boundary_fallback(
                PixelpipeGpuFallback::Basic(BasicPointError::Readback(
                    "cancelled transform".to_owned(),
                )),
                &scope,
            ),
            PixelpipeGpuFallback::ColorReconstruction(GpuColorReconstructionError::Cancelled,)
        );
    }

    #[test]
    fn shadhi_boundary_failures_do_not_flatten_into_basic_readback_errors() {
        let boundary = ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::Cancelled,
        );
        assert_eq!(
            shadhi_gpu_fallback(ShadhiBilateralEvaluationError::Boundary(boundary.clone())),
            PixelpipeGpuFallback::ShadhiBoundary(boundary)
        );
        assert_eq!(
            shadhi_gpu_fallback(ShadhiBilateralEvaluationError::Backend(
                BilateralGridError::Cancelled
            )),
            PixelpipeGpuFallback::Bilateral(BilateralGridError::Cancelled)
        );
    }

    #[test]
    fn gpu_cancellation_failures_are_terminal_instead_of_cpu_fallbacks() {
        for error in [
            PixelpipeGpuFallback::ColorZones(GpuColorZonesError::Cancelled),
            PixelpipeGpuFallback::Bilateral(BilateralGridError::Cancelled),
            PixelpipeGpuFallback::ShadhiBoundary(ShadhiBilateralBoundaryError::Operation(
                rusttable_processing::operations::OperationExecutionError::Cancelled,
            )),
        ] {
            let scope =
                CancellationScope::root(PipelineGeneration::new(17).expect("nonzero generation"));
            let result = gpu_fallback_or_cancellation(error, &scope);

            let Err(CpuPixelpipeError::Cancelled(cancelled)) = result else {
                panic!("GPU cancellation must be terminal: {result:?}");
            };
            assert_eq!(cancelled.reason(), CancellationReason::ParentFailed);
            assert_eq!(cancelled.stage(), Some(CancellationStage::GpuRetirement));
        }
    }
}
