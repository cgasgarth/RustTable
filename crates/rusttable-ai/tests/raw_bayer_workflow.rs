use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusttable_ai::workflows::raw_bayer::*;
use rusttable_ai::{CancellationToken, ModelTask, Provider, ProviderPolicy, TileCrop};
use rusttable_color::Matrix3;
use rusttable_image::{
    BlackWhiteLevels, CfaPattern, CfaPhase, ImageDimensions, Orientation, RawMosaic, Roi,
};

const CPU: [Provider; 1] = [Provider::Cpu];
const OPTIONAL: [Provider; 2] = [Provider::CoreMl, Provider::Cpu];

fn frame(phase: (u32, u32), active: Option<Roi>, masked: Vec<Roi>) -> RawFrame {
    let pattern = CfaPattern::bayer_rggb();
    let dimensions = ImageDimensions::new(5, 5).expect("dimensions");
    let samples = (0..25)
        .map(|index| 100 + u16::try_from(index % 4).expect("small fixture index") * 100)
        .collect();
    let raw = RawMosaic::new(
        dimensions,
        5,
        samples,
        pattern,
        CfaPhase::new(phase.0, phase.1, pattern),
        BlackWhiteLevels::new(0, 1000).expect("levels"),
        Orientation::Normal,
    )
    .expect("raw");
    let calibration = BayerCalibration::new(
        [7; 32],
        [0; 4],
        [1000; 4],
        [2.0, 1.0, 1.0, 1.0],
        Matrix3::identity(),
    )
    .expect("calibration");
    RawFrame::new(raw, active, None, masked, calibration).expect("frame")
}

fn descriptor() -> RawBayerModelDescriptor {
    RawBayerModelDescriptor {
        identity: [8; 32],
        task: ModelTask::RawBayerDenoise,
        tile_width: 2,
        tile_height: 2,
        overlap: 0,
        valid_crop: TileCrop::all(0),
        minimum_width: 1,
        minimum_height: 1,
        scale: 1.0,
        offset: 0.0,
        domain_min: 0.0,
        domain_max: 4.0,
        white_balanced_input: true,
        providers: &CPU,
        qualified_providers: &CPU,
        estimated_session_bytes: 1,
    }
}

fn request(source: RawFrame, strength: f32, provider: ProviderPolicy) -> RawBayerDenoiseRequest {
    RawBayerDenoiseRequest::new(
        source,
        RawBayerEditSnapshot::default(),
        [8; 32],
        provider,
        Strength::new(strength).expect("strength"),
        RawBayerOutputRequest::new(PathBuf::from("derived.dng"), [7; 32])
            .expect("output")
            .with_catalog(false, false),
    )
    .expect("request")
}

fn catalog_request(source: RawFrame) -> RawBayerDenoiseRequest {
    RawBayerDenoiseRequest::new(
        source,
        RawBayerEditSnapshot::default(),
        [8; 32],
        ProviderPolicy::Cpu,
        Strength::new(1.0).expect("strength"),
        RawBayerOutputRequest::new(PathBuf::from("derived.dng"), [7; 32])
            .expect("output")
            .with_catalog(true, true),
    )
    .expect("request")
}

struct IdentityModel {
    descriptor: RawBayerModelDescriptor,
    calls: Arc<Mutex<Vec<Provider>>>,
    fail_non_cpu: bool,
}
impl RawBayerDenoiseModel for IdentityModel {
    fn descriptor(&self) -> &RawBayerModelDescriptor {
        &self.descriptor
    }
    fn infer(
        &self,
        provider: Provider,
        input: &RawBayerTileInput<'_>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<f32>, RawBayerModelError> {
        self.calls.lock().expect("calls").push(provider);
        if cancellation.is_cancelled() {
            return Err(RawBayerModelError::Cancelled);
        }
        if self.fail_non_cpu && provider != Provider::Cpu {
            return Err(RawBayerModelError::Execution);
        }
        Ok(input.tensor().to_vec())
    }
}

struct AddModel {
    descriptor: RawBayerModelDescriptor,
}
impl RawBayerDenoiseModel for AddModel {
    fn descriptor(&self) -> &RawBayerModelDescriptor {
        &self.descriptor
    }
    fn infer(
        &self,
        _provider: Provider,
        input: &RawBayerTileInput<'_>,
        _cancellation: &CancellationToken,
    ) -> Result<Vec<f32>, RawBayerModelError> {
        Ok(input.tensor().iter().map(|value| value + 0.25).collect())
    }
}

#[derive(Default)]
struct Publisher {
    output: Option<CfaBayerU16>,
    publishes: usize,
}
impl RawBayerDngPublisher for Publisher {
    fn publish(
        &mut self,
        request: &RawBayerDenoiseRequest,
        output: &CfaBayerU16,
        _cancellation: &CancellationToken,
    ) -> Result<PublishedCfaBayer, RawBayerPublishError> {
        self.publishes += 1;
        self.output = Some(output.clone());
        Ok(PublishedCfaBayer {
            destination: request.output().destination().display().to_string(),
            artifact_identity: output.output_identity(),
        })
    }
    fn probe(&self, _artifact: &PublishedCfaBayer) -> Result<CfaBayerU16, RawBayerPublishError> {
        self.output.clone().ok_or(RawBayerPublishError::Probe)
    }
    fn discard(&mut self, _artifact: &PublishedCfaBayer) {
        self.output = None;
    }
}

struct Catalog {
    reconciled: Option<RawBayerReceipt>,
    imports: usize,
}
impl RawBayerCatalogPort for Catalog {
    fn reconcile(
        &mut self,
        _request_identity: [u8; 32],
    ) -> Result<Option<RawBayerReceipt>, RawBayerCatalogError> {
        Ok(self.reconciled.clone())
    }
    fn import_and_group(
        &mut self,
        request: &RawBayerDenoiseRequest,
        _artifact: &PublishedCfaBayer,
    ) -> Result<ImportGroupingOutcome, RawBayerCatalogError> {
        self.imports += 1;
        Ok(ImportGroupingOutcome::Imported {
            grouped: request.output().group_with_source(),
        })
    }
}

#[test]
fn all_four_bayer_phases_round_trip_semantic_four_plane_identity() {
    for y in 0..2 {
        for x in 0..2 {
            let model = IdentityModel {
                descriptor: descriptor(),
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_non_cpu: false,
            };
            let source = frame((x, y), None, Vec::new());
            let expected = source.raw().samples().to_vec();
            let mut publisher = Publisher::default();
            let mut workflow = RawBayerDenoiseWorkflow::new(&model, &mut publisher);
            workflow
                .run(
                    &request(source, 1.0, ProviderPolicy::Cpu),
                    &CancellationToken::default(),
                )
                .expect("workflow");
            assert_eq!(
                publisher.output.expect("output").samples(),
                expected.as_slice()
            );
        }
    }
}

#[test]
fn odd_active_area_and_masked_sensor_area_are_preserved() {
    let active = Roi::new(1, 1, 3, 3).expect("active");
    let masked = Roi::new(2, 2, 1, 1).expect("mask");
    let source = frame((1, 0), Some(active), vec![masked]);
    let original = source.raw().samples().to_vec();
    let model = IdentityModel {
        descriptor: descriptor(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_non_cpu: false,
    };
    let mut publisher = Publisher::default();
    RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .run(
            &request(source, 1.0, ProviderPolicy::Cpu),
            &CancellationToken::default(),
        )
        .expect("workflow");
    let output = publisher.output.expect("output");
    assert_eq!(output.samples()[12], original[12]);
    assert_eq!(output.active_area(), Some(active));
}

#[test]
fn strength_blends_model_at_mosaic_sample_level_and_uses_nearest_even_quantization() {
    let model = AddModel {
        descriptor: descriptor(),
    };
    let source = frame((0, 0), None, Vec::new());
    let original = source.raw().samples().to_vec();
    let mut publisher = Publisher::default();
    RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .run(
            &request(source, 0.5, ProviderPolicy::Cpu),
            &CancellationToken::default(),
        )
        .expect("workflow");
    let output = publisher.output.expect("output");
    assert!(
        output
            .samples()
            .iter()
            .zip(original)
            .any(|(actual, source)| actual > &source)
    );
}

#[test]
fn auto_retries_identical_plan_on_cpu_and_receipts_actual_provider() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let model = IdentityModel {
        descriptor: RawBayerModelDescriptor {
            providers: &OPTIONAL,
            qualified_providers: &OPTIONAL,
            ..descriptor()
        },
        calls: Arc::clone(&calls),
        fail_non_cpu: true,
    };
    let mut publisher = Publisher::default();
    let receipt = RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .run(
            &request(frame((0, 0), None, Vec::new()), 1.0, ProviderPolicy::Auto),
            &CancellationToken::default(),
        )
        .expect("workflow");
    assert_eq!(receipt.provider, Provider::Cpu);
    assert_eq!(
        calls.lock().expect("calls").as_slice(),
        &[
            Provider::CoreMl,
            Provider::Cpu,
            Provider::Cpu,
            Provider::Cpu,
            Provider::Cpu
        ]
    );
}

#[test]
fn cancellation_and_missing_dng_boundary_are_typed_and_publish_nothing() {
    let model = IdentityModel {
        descriptor: descriptor(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_non_cpu: false,
    };
    let mut publisher = BlockingCfaBayerDngPublisher;
    let token = CancellationToken::default();
    token.cancel();
    let mut workflow = RawBayerDenoiseWorkflow::new(&model, &mut publisher);
    assert!(matches!(
        workflow.run(
            &request(frame((0, 0), None, Vec::new()), 1.0, ProviderPolicy::Cpu),
            &token
        ),
        Err(RawBayerWorkflowError::Cancelled(RawBayerStage::Validate))
    ));
    let mut publisher = BlockingCfaBayerDngPublisher;
    let mut workflow = RawBayerDenoiseWorkflow::new(&model, &mut publisher);
    let error = workflow
        .run(
            &request(frame((0, 0), None, Vec::new()), 0.0, ProviderPolicy::Cpu),
            &CancellationToken::default(),
        )
        .expect_err("blocking DNG");
    assert!(matches!(
        error,
        RawBayerWorkflowError::Publish(RawBayerPublishError::DngWriterUnavailable)
    ));
}

#[test]
fn production_publisher_writes_and_probes_real_dng_before_workflow_receipt() {
    let model = IdentityModel {
        descriptor: descriptor(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_non_cpu: false,
    };
    let destination =
        std::env::temp_dir().join(format!("rusttable-bayer-{}.dng", std::process::id()));
    let output_request = RawBayerOutputRequest::new(destination.clone(), [7; 32])
        .expect("output")
        .with_catalog(false, false)
        .with_collision(CollisionPolicy::Fail);
    let request = RawBayerDenoiseRequest::new(
        frame((1, 0), None, Vec::new()),
        RawBayerEditSnapshot::default(),
        [8; 32],
        ProviderPolicy::Cpu,
        Strength::new(0.0).expect("strength"),
        output_request,
    )
    .expect("request");
    let mut publisher = FileCfaBayerDngPublisher::default();
    let receipt = RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .run(&request, &CancellationToken::default())
        .expect("real DNG workflow");
    assert_ne!(receipt.output_identity, [0; 32]);
    assert!(destination.is_file());
    fs::remove_file(destination).expect("cleanup");
}

#[test]
fn restart_reconciliation_returns_receipt_without_inference_or_publication() {
    let model = IdentityModel {
        descriptor: descriptor(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_non_cpu: false,
    };
    let request = request(frame((0, 0), None, Vec::new()), 0.0, ProviderPolicy::Cpu);
    let expected = RawBayerReceipt {
        workflow_version: RAW_BAYER_WORKFLOW_VERSION,
        request_identity: request.identity(),
        source_identity: [1; 32],
        edit_identity: [2; 32],
        plan_identity: [3; 32],
        model_identity: [4; 32],
        provider: Provider::Cpu,
        tile_count: 1,
        strength_millis: 0,
        output_identity: [5; 32],
        imported: true,
        grouped: true,
    };
    let mut catalog = Catalog {
        reconciled: Some(expected.clone()),
        imports: 0,
    };
    let mut publisher = Publisher::default();
    let receipt = RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .with_catalog(&mut catalog)
        .run(&request, &CancellationToken::default())
        .expect("reconcile");
    assert_eq!(receipt, expected);
    assert_eq!(publisher.publishes, 0);
    assert!(model.calls.lock().expect("calls").is_empty());
}

#[test]
fn import_and_group_commit_happens_after_probe() {
    let model = IdentityModel {
        descriptor: descriptor(),
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_non_cpu: false,
    };
    let mut publisher = Publisher::default();
    let mut catalog = Catalog {
        reconciled: None,
        imports: 0,
    };
    let receipt = RawBayerDenoiseWorkflow::new(&model, &mut publisher)
        .with_catalog(&mut catalog)
        .run(
            &catalog_request(frame((0, 0), None, Vec::new())),
            &CancellationToken::default(),
        )
        .expect("workflow");
    assert!(receipt.imported);
    assert!(receipt.grouped);
    assert_eq!(catalog.imports, 1);
    assert!(publisher.output.is_some());
}

#[test]
fn unsupported_layout_and_missing_calibration_evidence_block_with_typed_errors() {
    let xtrans = CfaPattern::XTrans([[rusttable_image::CfaColor::Green; 6]; 6]);
    let raw = RawMosaic::new(
        ImageDimensions::new(2, 2).expect("dimensions"),
        2,
        vec![1; 4],
        xtrans,
        CfaPhase::new(0, 0, xtrans),
        BlackWhiteLevels::new(0, 10).expect("levels"),
        Orientation::Normal,
    )
    .expect("raw");
    let calibration =
        BayerCalibration::new([7; 32], [0; 4], [10; 4], [1.0; 4], Matrix3::identity())
            .expect("calibration");
    assert!(matches!(
        RawFrame::new(raw, None, None, Vec::new(), calibration),
        Err(RawFrameError::UnsupportedCfa)
    ));
    assert_eq!(Strength::new(f32::NAN), Err(StrengthError::OutOfRange));
}
