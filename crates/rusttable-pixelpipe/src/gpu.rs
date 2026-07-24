use std::collections::{BTreeMap, VecDeque};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use rusttable_core::OperationId;
use rusttable_gpu::{
    BasicAdjPointParameters, BasicPointError, BasicPointOperation, BasicPointRequest,
    BilateralGridError, BilateralGridRequest, CancellationToken as GpuCancellationToken,
    GpuRuntime, GrainPointError, GrainPointRequest,
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
    output_descriptor, output_from_working, requires_full_frame_execution, to_linear_working,
    validate_input_encoding,
};
use crate::{
    Cache, CacheConfig, CacheError, CacheKey, CancellationError, CancellationReason,
    CancellationScope, CancellationStage, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, PipelineGeneration, RgbaF32Image, RgbaF32Pixel,
};

/// The backend that published one pixelpipe result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelpipeBackend {
    CpuCanonical,
    CpuTiledFallback,
    WgpuBasic,
    WgpuTiled,
    WgpuBilateralHybrid,
}

/// Typed reason a qualified GPU path fell back to canonical CPU execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelpipeGpuFallback {
    Basic(BasicPointError),
    Grain(GrainPointError),
    Bilateral(BilateralGridError),
    ShadhiBoundary(ShadhiBilateralBoundaryError),
}

impl PixelpipeGpuFallback {
    const fn is_cancellation(&self) -> bool {
        matches!(
            self,
            Self::Bilateral(BilateralGridError::Cancelled)
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
        if matches!(plan, GpuPlan::ShadhiBilateral { .. }) {
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
                            backend: PixelpipeBackend::WgpuBilateralHybrid,
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
    Grain(rusttable_processing::operations::grain::GrainConfig),
    ShadhiBilateral { config: ShadhiConfig, opacity: f32 },
}

impl GpuPlan {
    const fn backend(&self) -> PixelpipeBackend {
        match self {
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
    Grain(rusttable_processing::operations::grain::GrainConfig),
    ShadhiBilateral { config: ShadhiConfig, opacity: f32 },
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
    let mut grain = None;
    let mut shadhi = None;
    for node in snapshot.graph().nodes() {
        let operation = node.operation();
        if !operation.is_enabled() {
            continue;
        }
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
            rusttable_processing::ProcessingOperationKind::Grain { config } => {
                if operation.opacity().get().to_bits() != 1.0_f32.to_bits()
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
        if grain.is_some() || shadhi.is_some() {
            return None;
        }
        operations.push(gpu_operation);
    }
    if let Some(config) = grain {
        Some(GpuPlanCandidate::Grain(config))
    } else if let Some((config, opacity)) = shadhi {
        Some(GpuPlanCandidate::ShadhiBilateral { config, opacity })
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
    let (frame, packed) = packed_linear_working(input)?;
    let result = gpu.execute_basic_point(BasicPointRequest {
        pixels: &packed,
        operations,
    })?;
    image_from_packed(
        input,
        output_mode,
        frame,
        result.pixels(),
        result.dispatches(),
    )
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
    if matches!(plan, GpuPlan::ShadhiBilateral { .. }) {
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
            GpuPlan::Grain(config) => execute_gpu_grain_image(
                gpu,
                &tile_input,
                snapshot.output_mode(),
                *config,
                input.descriptor().dimensions(),
                (tile.origin_x(), tile.origin_y()),
            )?,
            GpuPlan::ShadhiBilateral { .. } => {
                unreachable!("bilateral Shadhi is dispatched once before tile iteration")
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

    use rusttable_core::{Edit, EditId, PhotoId, Revision};

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

    fn empty_snapshot() -> CpuPixelpipeSnapshot {
        snapshot_with_encoding(RgbaF32ColorEncoding::SrgbD65)
    }

    fn snapshot_with_encoding(encoding: RgbaF32ColorEncoding) -> CpuPixelpipeSnapshot {
        let edit = Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(1),
            [],
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
