mod engine;
mod planning;
mod ports;
mod types;

pub use engine::RawLinearDenoiseWorkflow;
pub use planning::RawLinearPlanError;
pub use ports::{
    BlockingLinearRawDngPublisher, ImportGroupingOutcome, NoopRawLinearControl,
    NoopRawLinearObserver, PublishedLinearRaw, RawLinearCatalogError, RawLinearCatalogPort,
    RawLinearControl, RawLinearDenoiseModel, RawLinearDngPublisher, RawLinearModelDescriptor,
    RawLinearModelError, RawLinearObserver, RawLinearProgress, RawLinearPublishError,
    RawLinearStage, RawLinearTileInput, RawLinearWorkflowError,
};
pub use types::{
    CalibrationError, CollisionPolicy, LinearRawRgbU16, LinearRawRgbU16Error, PlanIdentityError,
    RAW_LINEAR_PLAN_VERSION, RAW_LINEAR_WORKFLOW_VERSION, RawEditError, RawLinearCalibration,
    RawLinearDenoiseRequest, RawLinearEditSnapshot, RawLinearOutputRequest, RawLinearPlan,
    RawLinearPreparationMode, RawLinearReceipt, RawLinearSource, RawLinearSourceKind,
    RawLinearTile, RawOperationError, RawOperationKind, RawOperationSpec, RequestError, Strength,
    StrengthError,
};

static NOOP_OBSERVER: NoopRawLinearObserver = NoopRawLinearObserver;
static NOOP_CONTROL: NoopRawLinearControl = NoopRawLinearControl;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use rusttable_color::{ColorEncoding, Matrix3};
    use rusttable_image::{
        BlackWhiteLevels, CfaPattern, CfaPhase, ImageDimensions, Orientation, RawMosaic,
    };

    use super::*;
    use crate::{ModelTask, Provider, ProviderPolicy, TensorLayout, TileCrop};

    const CPU: [Provider; 1] = [Provider::Cpu];
    const OPTIONAL_CPU: [Provider; 2] = [Provider::CoreMl, Provider::Cpu];

    fn calibration() -> RawLinearCalibration {
        RawLinearCalibration::new([9; 32], Matrix3::identity()).expect("calibration")
    }

    fn source() -> RawLinearSource {
        let pattern = CfaPattern::XTrans([
            [rusttable_image::CfaColor::Green; 6],
            [rusttable_image::CfaColor::Red; 6],
            [rusttable_image::CfaColor::Blue; 6],
            [rusttable_image::CfaColor::Green; 6],
            [rusttable_image::CfaColor::Red; 6],
            [rusttable_image::CfaColor::Blue; 6],
        ]);
        let raw = RawMosaic::new(
            ImageDimensions::new(6, 6).expect("dimensions"),
            6,
            vec![100; 36],
            pattern,
            CfaPhase::new(0, 0, pattern),
            BlackWhiteLevels::new(0, 100).expect("levels"),
            Orientation::Normal,
        )
        .expect("raw");
        RawLinearSource::XTrans {
            raw,
            active_area: None,
            calibration: calibration(),
            camera_identity: [7; 32],
        }
    }

    fn descriptor() -> RawLinearModelDescriptor {
        RawLinearModelDescriptor {
            identity: [8; 32],
            task: ModelTask::RawLinearDenoise,
            input_color: ColorEncoding::LinearRec2020D65,
            output_color: ColorEncoding::LinearRec2020D65,
            layout: TensorLayout::PlanarNchwRgb,
            full_range: true,
            finite_only: true,
            tile_width: 6,
            tile_height: 6,
            overlap: 1,
            valid_crop: TileCrop::all(1),
            minimum_width: 1,
            minimum_height: 1,
            providers: &CPU,
            qualified_providers: &CPU,
            estimated_session_bytes: 1,
        }
    }

    struct IdentityModel(RawLinearModelDescriptor);
    impl RawLinearDenoiseModel for IdentityModel {
        fn descriptor(&self) -> &RawLinearModelDescriptor {
            &self.0
        }
        fn infer(
            &self,
            _provider: Provider,
            input: &RawLinearTileInput<'_>,
            cancellation: &crate::CancellationToken,
        ) -> Result<Vec<f32>, RawLinearModelError> {
            if cancellation.is_cancelled() {
                return Err(RawLinearModelError::Cancelled);
            }
            Ok(input.nchw_rgb().to_vec())
        }
    }

    struct RetryModel {
        descriptor: RawLinearModelDescriptor,
        calls: Arc<Mutex<Vec<Provider>>>,
    }
    impl RawLinearDenoiseModel for RetryModel {
        fn descriptor(&self) -> &RawLinearModelDescriptor {
            &self.descriptor
        }
        fn infer(
            &self,
            provider: Provider,
            input: &RawLinearTileInput<'_>,
            _cancellation: &crate::CancellationToken,
        ) -> Result<Vec<f32>, RawLinearModelError> {
            self.calls.lock().expect("calls lock").push(provider);
            if provider != Provider::Cpu {
                return Err(RawLinearModelError::Execution);
            }
            Ok(input.nchw_rgb().to_vec())
        }
    }

    #[derive(Default)]
    struct Publisher {
        output: Mutex<Option<LinearRawRgbU16>>,
    }
    impl RawLinearDngPublisher for Publisher {
        fn publish(
            &mut self,
            request: &RawLinearDenoiseRequest,
            output: &LinearRawRgbU16,
            _cancellation: &crate::CancellationToken,
        ) -> Result<PublishedLinearRaw, RawLinearPublishError> {
            *self.output.lock().expect("output lock") = Some(output.clone());
            Ok(PublishedLinearRaw {
                destination: request.output().destination().display().to_string(),
                artifact_identity: [4; 32],
            })
        }
        fn probe(
            &self,
            _artifact: &PublishedLinearRaw,
        ) -> Result<LinearRawRgbU16, RawLinearPublishError> {
            self.output
                .lock()
                .expect("output lock")
                .clone()
                .ok_or(RawLinearPublishError::Probe)
        }
        fn discard(&mut self, _artifact: &PublishedLinearRaw) {}
    }

    #[derive(Default)]
    struct Catalog;
    impl RawLinearCatalogPort for Catalog {
        fn reconcile(
            &mut self,
            _request_identity: [u8; 32],
        ) -> Result<Option<RawLinearReceipt>, RawLinearCatalogError> {
            Ok(None)
        }
        fn import_and_group(
            &mut self,
            request: &RawLinearDenoiseRequest,
            _artifact: &PublishedLinearRaw,
        ) -> Result<ImportGroupingOutcome, RawLinearCatalogError> {
            Ok(ImportGroupingOutcome::Imported {
                grouped: request.output().group_with_source(),
            })
        }
    }

    struct ReconciledCatalog(RawLinearReceipt);
    impl RawLinearCatalogPort for ReconciledCatalog {
        fn reconcile(
            &mut self,
            request_identity: [u8; 32],
        ) -> Result<Option<RawLinearReceipt>, RawLinearCatalogError> {
            assert_eq!(self.0.request_identity, request_identity);
            Ok(Some(self.0.clone()))
        }

        fn import_and_group(
            &mut self,
            _request: &RawLinearDenoiseRequest,
            _artifact: &PublishedLinearRaw,
        ) -> Result<ImportGroupingOutcome, RawLinearCatalogError> {
            Err(RawLinearCatalogError::Pending)
        }
    }

    fn request() -> RawLinearDenoiseRequest {
        RawLinearDenoiseRequest::new(
            source(),
            RawLinearEditSnapshot::default(),
            [8; 32],
            ProviderPolicy::Cpu,
            Strength::new(1.0).expect("strength"),
            RawLinearOutputRequest::new(PathBuf::from("derived.dng"), [7; 32]).expect("output"),
        )
        .expect("request")
    }

    fn linear_request(strength: f32) -> RawLinearDenoiseRequest {
        let image = LinearRawRgbU16::new(
            ImageDimensions::new(1, 1).expect("dimensions"),
            vec![1, 2, 3],
            0,
            10,
            ColorEncoding::LinearRec2020D65,
            Orientation::Normal,
            [7; 32],
        )
        .expect("linear source");
        RawLinearDenoiseRequest::new(
            RawLinearSource::AlreadyLinearRgb {
                image,
                calibration: None,
            },
            RawLinearEditSnapshot::default(),
            [8; 32],
            ProviderPolicy::Cpu,
            Strength::new(strength).expect("strength"),
            RawLinearOutputRequest::new(PathBuf::from("derived.dng"), [7; 32])
                .expect("output")
                .with_catalog(false, false),
        )
        .expect("request")
    }

    #[test]
    fn xtrans_plan_is_minimal_and_deterministic() {
        let model = IdentityModel(descriptor());
        let first = planning::compile(&request(), model.descriptor())
            .expect("plan")
            .0;
        let second = planning::compile(&request(), model.descriptor())
            .expect("plan")
            .0;
        assert_eq!(first.identity(), second.identity());
        assert_eq!(
            first
                .included_operations()
                .iter()
                .map(|op| op.kind)
                .collect::<Vec<_>>(),
            vec![RawOperationKind::RawPrepare, RawOperationKind::Demosaic]
        );
    }

    #[test]
    fn recursive_denoise_is_disabled_and_recorded() {
        let edit = RawLinearEditSnapshot::new(
            3,
            vec![
                RawOperationSpec::new(4, 1, RawOperationKind::RawDenoise, vec![1], true)
                    .expect("op"),
            ],
        )
        .expect("edit");
        let request = RawLinearDenoiseRequest::new(
            source(),
            edit,
            [8; 32],
            ProviderPolicy::Cpu,
            Strength::new(1.0).expect("strength"),
            RawLinearOutputRequest::new(PathBuf::from("derived.dng"), [7; 32]).expect("output"),
        )
        .expect("request");
        let model = IdentityModel(descriptor());
        let plan = planning::compile(&request, model.descriptor())
            .expect("plan")
            .0;
        assert_eq!(
            plan.excluded_operations()[0].kind,
            RawOperationKind::RawDenoise
        );
    }

    #[test]
    fn workflow_round_trips_and_imports_without_reprocessing() {
        let model = IdentityModel(descriptor());
        let mut publisher = Publisher::default();
        let mut catalog = Catalog;
        let mut workflow =
            RawLinearDenoiseWorkflow::new(&model, &mut publisher).with_catalog(&mut catalog);
        let receipt = workflow
            .run(&request(), &crate::CancellationToken::default())
            .expect("workflow");
        assert_eq!(receipt.provider, Provider::Cpu);
        assert!(receipt.imported);
        assert!(receipt.grouped);
    }

    #[test]
    fn restart_reconciliation_returns_receipt_without_publishing_or_inference() {
        let request = request();
        let expected = RawLinearReceipt {
            workflow_version: RAW_LINEAR_WORKFLOW_VERSION,
            request_identity: request.identity(),
            source_identity: [1; 32],
            edit_identity: [2; 32],
            plan_identity: [3; 32],
            model_identity: [4; 32],
            provider: Provider::Cpu,
            tile_count: 4,
            strength_millis: 1000,
            output_identity: [5; 32],
            imported: true,
            grouped: true,
        };
        let model = IdentityModel(descriptor());
        let mut publisher = BlockingLinearRawDngPublisher;
        let mut catalog = ReconciledCatalog(expected.clone());
        let mut workflow =
            RawLinearDenoiseWorkflow::new(&model, &mut publisher).with_catalog(&mut catalog);
        assert_eq!(
            workflow
                .run(&request, &crate::CancellationToken::default())
                .expect("reconciled receipt"),
            expected
        );
    }

    #[test]
    fn explicit_linear_rgb_requires_linear_interpretation() {
        let error = LinearRawRgbU16::new(
            ImageDimensions::new(1, 1).expect("dimensions"),
            vec![1, 2, 3],
            0,
            10,
            ColorEncoding::SrgbD65,
            Orientation::Normal,
            [7; 32],
        )
        .expect_err("nonlinear source");
        assert_eq!(error, LinearRawRgbU16Error::InvalidInterpretation);
    }

    #[test]
    fn auto_provider_retries_identical_tile_on_cpu_and_receipts_actual_provider() {
        let mut descriptor = descriptor();
        descriptor.providers = &OPTIONAL_CPU;
        descriptor.qualified_providers = &OPTIONAL_CPU;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let model = RetryModel {
            descriptor,
            calls: Arc::clone(&calls),
        };
        let mut publisher = Publisher::default();
        let mut catalog = Catalog;
        let mut workflow =
            RawLinearDenoiseWorkflow::new(&model, &mut publisher).with_catalog(&mut catalog);
        let request = RawLinearDenoiseRequest::new(
            source(),
            RawLinearEditSnapshot::default(),
            [8; 32],
            ProviderPolicy::Auto,
            Strength::new(1.0).expect("strength"),
            RawLinearOutputRequest::new(PathBuf::from("derived.dng"), [7; 32]).expect("output"),
        )
        .expect("request");
        let receipt = workflow
            .run(&request, &crate::CancellationToken::default())
            .expect("workflow");
        assert_eq!(receipt.provider, Provider::Cpu);
        assert_eq!(
            calls.lock().expect("calls lock").as_slice(),
            &[
                Provider::CoreMl,
                Provider::Cpu,
                Provider::Cpu,
                Provider::Cpu,
                Provider::Cpu,
            ]
        );
    }

    #[test]
    fn strength_zero_skips_model_and_preserves_canonical_source() {
        let model = IdentityModel(descriptor());
        let mut publisher = Publisher::default();
        let mut workflow = RawLinearDenoiseWorkflow::new(&model, &mut publisher);
        let receipt = workflow
            .run(&linear_request(0.0), &crate::CancellationToken::default())
            .expect("workflow");
        assert_eq!(receipt.strength_millis, 0);
        assert_eq!(
            publisher
                .output
                .lock()
                .expect("output lock")
                .as_ref()
                .expect("output")
                .samples(),
            &[6554, 13107, 19661]
        );
    }

    #[test]
    fn cancellation_before_preparation_publishes_nothing() {
        let model = IdentityModel(descriptor());
        let mut publisher = Publisher::default();
        let mut workflow = RawLinearDenoiseWorkflow::new(&model, &mut publisher);
        let cancellation = crate::CancellationToken::default();
        cancellation.cancel();
        assert!(matches!(
            workflow.run(&request(), &cancellation),
            Err(RawLinearWorkflowError::Cancelled(RawLinearStage::Validate))
        ));
        assert!(publisher.output.lock().expect("output lock").is_none());
    }

    #[test]
    fn missing_dng_dependency_is_a_typed_blocking_error() {
        let model = IdentityModel(descriptor());
        let mut publisher = BlockingLinearRawDngPublisher;
        let mut workflow = RawLinearDenoiseWorkflow::new(&model, &mut publisher);
        let error = workflow
            .run(&request(), &crate::CancellationToken::default())
            .expect_err("missing DNG adapter");
        assert!(matches!(
            error,
            RawLinearWorkflowError::Publish(RawLinearPublishError::DngWriterUnavailable)
        ));
    }
}
