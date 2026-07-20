mod detail;
mod engine;
mod ports;
mod tiff;
mod types;

pub use ports::{
    DerivedPhotoImporter, ImportError, ImportOutcome, ModelTile, NoopControl, NoopObserver,
    PublishError, PublishedArtifact, RgbDenoiseControl, RgbDenoiseModel, RgbDenoiseObserver,
    RgbDenoisePublisher,
};
pub use tiff::FileTiffPublisher;
pub use types::{
    AlphaOutput, CollisionPolicy, DETAIL_BAND_MULTIPLIERS, Matrix3, ModelDescriptor, ModelError,
    ModelTask, OutputBitDepth, ProfileError, ProviderSelection, ProviderUsed, RequestError,
    RgbDenoiseBatchReceipt, RgbDenoisePlan, RgbDenoiseProgress, RgbDenoiseReceipt,
    RgbDenoiseRequest, RgbDenoiseStage, RgbProfile, Strength, StrengthError, TiffCompression,
    TiffRecipe, TiffRecipeError, WORKFLOW_VERSION,
};

const MAX_BATCH_ITEMS: usize = 256;

static NOOP_OBSERVER: NoopObserver = NoopObserver;
static NOOP_CONTROL: NoopControl = NoopControl;

pub struct RgbDenoiseWorkflow<'a> {
    model: &'a dyn RgbDenoiseModel,
    publisher: &'a dyn RgbDenoisePublisher,
    importer: Option<&'a mut dyn DerivedPhotoImporter>,
    observer: &'a dyn RgbDenoiseObserver,
    control: &'a dyn RgbDenoiseControl,
}

impl<'a> RgbDenoiseWorkflow<'a> {
    #[must_use]
    pub fn new(model: &'a dyn RgbDenoiseModel, publisher: &'a dyn RgbDenoisePublisher) -> Self {
        Self {
            model,
            publisher,
            importer: None,
            observer: &NOOP_OBSERVER,
            control: &NOOP_CONTROL,
        }
    }

    #[must_use]
    pub fn with_importer(mut self, importer: &'a mut dyn DerivedPhotoImporter) -> Self {
        self.importer = Some(importer);
        self
    }

    #[must_use]
    pub fn with_observer(mut self, observer: &'a dyn RgbDenoiseObserver) -> Self {
        self.observer = observer;
        self
    }

    #[must_use]
    pub fn with_control(mut self, control: &'a dyn RgbDenoiseControl) -> Self {
        self.control = control;
        self
    }

    /// Runs one immutable snapshot through inference, recovery, color, TIFF,
    /// probe, publication, and optional catalog import/group.
    ///
    /// # Errors
    ///
    /// Returns a typed stage failure. Import failures occur only after the
    /// TIFF is durable and therefore leave a reconcilable published artifact.
    pub fn run(&mut self, request: &RgbDenoiseRequest) -> Result<RgbDenoiseReceipt, WorkflowError> {
        let processed = engine::process(request, self.model, self.observer, self.control)
            .map_err(WorkflowError::Process)?;
        if self.control.is_cancelled(RgbDenoiseStage::Encode) {
            return Err(WorkflowError::Cancelled(RgbDenoiseStage::Encode));
        }
        self.observer.progress(RgbDenoiseProgress {
            stage: RgbDenoiseStage::Encode,
            completed: 1,
            total: 1,
        });
        let artifact = self
            .publisher
            .publish(
                request.destination(),
                request.tiff(),
                request.collision(),
                request.output_profile(),
                &processed.pixels,
                processed.plan.dimensions,
                processed.artifact_key,
            )
            .map_err(WorkflowError::Publish)?;
        self.observer.progress(RgbDenoiseProgress {
            stage: RgbDenoiseStage::Publication,
            completed: 1,
            total: 1,
        });
        self.observer.progress(RgbDenoiseProgress {
            stage: RgbDenoiseStage::Probe,
            completed: 1,
            total: 1,
        });
        let mut imported = false;
        let mut grouped = false;
        if request.add_to_catalog() {
            if self.control.is_cancelled(RgbDenoiseStage::Import) {
                return Err(WorkflowError::Cancelled(RgbDenoiseStage::Import));
            }
            let import_port = self.importer.as_deref_mut().ok_or(WorkflowError::Import {
                source: ImportError::Failed("catalog import port is not configured".to_owned()),
            })?;
            match import_port
                .import_and_group(
                    request.input().source_identity().as_bytes(),
                    &artifact.destination,
                    request.group_with_source(),
                )
                .map_err(|source| WorkflowError::Import { source })?
            {
                ImportOutcome::Imported | ImportOutcome::AlreadyPresent => imported = true,
            }
            grouped = request.group_with_source();
            self.observer.progress(RgbDenoiseProgress {
                stage: RgbDenoiseStage::Import,
                completed: 1,
                total: 1,
            });
            if grouped {
                self.observer.progress(RgbDenoiseProgress {
                    stage: RgbDenoiseStage::Group,
                    completed: 1,
                    total: 1,
                });
            }
        }
        Ok(RgbDenoiseReceipt {
            workflow_version: WORKFLOW_VERSION,
            artifact_key: processed.artifact_key,
            destination: artifact.destination,
            dimensions: processed.plan.dimensions,
            provider: processed.provider,
            detail_recovery_strength: processed.plan.detail_recovery_strength,
            preserve_wide_gamut: processed.plan.preserve_wide_gamut,
            shadow_boost: processed.shadow_boost,
            tile_count: processed.plan.tile_count,
            imported,
            grouped,
            source_identity: request.input().source_identity(),
        })
    }

    /// Executes a bounded batch and retains one independent outcome per item.
    /// # Errors
    ///
    /// Returns a batch-size error when more than the bounded item count is supplied.
    pub fn run_batch(
        &mut self,
        requests: &[RgbDenoiseRequest],
    ) -> Result<RgbDenoiseBatchReceipt, WorkflowError> {
        if requests.len() > MAX_BATCH_ITEMS {
            return Err(WorkflowError::BatchTooLarge {
                limit: MAX_BATCH_ITEMS,
                actual: requests.len(),
            });
        }
        let outcomes = requests
            .iter()
            .map(|request| self.run(request).map_err(|error| error.to_string()))
            .collect();
        Ok(RgbDenoiseBatchReceipt { outcomes })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    Process(engine::ProcessError),
    Publish(PublishError),
    Import { source: ImportError },
    Cancelled(RgbDenoiseStage),
    BatchTooLarge { limit: usize, actual: usize },
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RGB denoise workflow failed: {self:?}")
    }
}

impl std::error::Error for WorkflowError {}
