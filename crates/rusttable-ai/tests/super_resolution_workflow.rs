use std::fmt::Write as _;
use std::path::Path;

use rusttable_ai::{
    AlphaPolicy, AuxiliaryTensor, CancellationToken, CatalogPort, CatalogReceipt, ColorProfile,
    ImageDimensions, LinearRgbaImage, MemoryLimits, ModelInferenceError, ModelManifest,
    ModelOutput, ModelTask, ModelTileContract, ProfileKind, ProviderPolicy, PublicationReceipt,
    RgbMatrix, ShadowPolicy, SuperResolutionModel, SuperResolutionPublisher,
    SuperResolutionRequest, SuperResolutionScale, SuperResolutionSettings, TiffBitDepth, TiffProbe,
    TiffSampleFormat, TiffSettings, execute_super_resolution, plan_super_resolution,
};

fn profile(name: &str, icc: &[u8]) -> ColorProfile {
    ColorProfile::new(
        name,
        ProfileKind::Matrix,
        icc.to_vec(),
        RgbMatrix::identity(),
    )
    .expect("profile")
}

fn source(width: u32, height: u32, value: f32) -> LinearRgbaImage {
    let dimensions = ImageDimensions::new(width, height).expect("dimensions");
    LinearRgbaImage::new(
        dimensions,
        vec![[value, value, value, 0.75]; dimensions.pixels().expect("pixels")],
    )
    .expect("image")
}

#[derive(Debug)]
struct FixtureModel {
    manifest: ModelManifest,
    calls: usize,
    last_input: Option<rusttable_ai::ModelInput>,
}

impl FixtureModel {
    fn new(task: ModelTask, tile: ModelTileContract) -> Self {
        Self {
            manifest: ModelManifest::new(
                "fixture-upscale",
                "1",
                task,
                tile,
                b"fixture-model".to_vec(),
            ),
            calls: 0,
            last_input: None,
        }
    }
}

impl SuperResolutionModel for FixtureModel {
    fn manifest(&self) -> &ModelManifest {
        &self.manifest
    }

    fn infer(
        &mut self,
        input: &rusttable_ai::ModelInput,
        _: &CancellationToken,
    ) -> Result<ModelOutput, ModelInferenceError> {
        self.calls += 1;
        self.last_input = Some(input.clone());
        let pixels =
            usize::try_from(u64::from(input.width()) * u64::from(input.height())).expect("tile");
        let mut output =
            vec![0.0; pixels * 3 * usize::try_from(input.scale() * input.scale()).expect("scale")];
        let output_width = input.width() * input.scale();
        let output_height = input.height() * input.scale();
        let output_pixels =
            usize::try_from(u64::from(output_width) * u64::from(output_height)).expect("output");
        output.resize(output_pixels * 3, 0.5);
        Ok(ModelOutput::new(output_width, output_height, output))
    }
}

#[derive(Debug, Default)]
struct MemoryPublisher {
    artifact: Option<Vec<u8>>,
    probe: Option<TiffProbe>,
}

impl SuperResolutionPublisher for MemoryPublisher {
    fn publish(
        &mut self,
        _: Option<&Path>,
        artifact: &[u8],
        probe: &TiffProbe,
    ) -> Result<PublicationReceipt, rusttable_ai::PublicationError> {
        self.artifact = Some(artifact.to_vec());
        self.probe = Some(probe.clone());
        Ok(PublicationReceipt::from_verified_artifact(
            probe.dimensions(),
            artifact,
        ))
    }
}

#[derive(Debug, Default)]
struct MemoryCatalog {
    calls: usize,
}

impl CatalogPort for MemoryCatalog {
    fn register_derived(
        &mut self,
        _: u64,
        _: u64,
        _: &PublicationReceipt,
    ) -> Result<CatalogReceipt, rusttable_ai::CatalogError> {
        self.calls += 1;
        Ok(CatalogReceipt {
            derived_key: "derived-fixture".to_owned(),
        })
    }
}

#[test]
fn super_resolution_workflow_publishes_exact_2x_dimensions_and_profile() {
    let mut model = FixtureModel::new(
        ModelTask::SuperResolution2x,
        ModelTileContract::new(4, 4, 1, 2),
    );
    let output_profile = profile("rec2020-fixture", b"icc-rec2020-fixture");
    let request = SuperResolutionRequest::new(
        source(3, 2, 0.4),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    )
    .with_settings(
        SuperResolutionSettings::new(output_profile.clone()).with_source_identity(9, 12),
    );
    let mut publisher = MemoryPublisher::default();
    let mut catalog = MemoryCatalog::default();
    let receipt = execute_super_resolution(
        &request,
        &mut model,
        &mut publisher,
        Some(&mut catalog),
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
        &CancellationToken::default(),
        |_| {},
    )
    .expect("workflow");
    assert_eq!(
        receipt.publication.dimensions(),
        ImageDimensions::new(6, 4).expect("dimensions")
    );
    assert_eq!(
        publisher.probe.as_ref().expect("probe").icc_sha256(),
        sha256(output_profile.icc())
    );
    assert_eq!(publisher.probe.as_ref().expect("probe").channels(), 4);
    assert_eq!(model.calls, receipt.plan.tiles().len());
    assert_eq!(catalog.calls, 1);
}

#[test]
fn super_resolution_workflow_supports_4x_rgb_8_bit_output() {
    let mut model = FixtureModel::new(
        ModelTask::SuperResolution4x,
        ModelTileContract::new(3, 3, 1, 4),
    );
    let output = profile("display-p3-fixture", b"icc-p3-fixture");
    let settings = SuperResolutionSettings::new(output)
        .with_tiff(TiffSettings::new(TiffBitDepth::Eight, false, 10_000).expect("settings"));
    let request = SuperResolutionRequest::new(
        source(3, 1, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X4,
    )
    .with_settings(settings);
    let mut publisher = MemoryPublisher::default();
    let receipt = execute_super_resolution(
        &request,
        &mut model,
        &mut publisher,
        None::<&mut MemoryCatalog>,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
        &CancellationToken::default(),
        |_| {},
    )
    .expect("workflow");
    let probe = publisher.probe.expect("probe");
    assert_eq!(
        receipt.publication.dimensions(),
        ImageDimensions::new(12, 4).expect("dimensions")
    );
    assert_eq!(probe.bit_depth(), TiffBitDepth::Eight);
    assert_eq!(probe.channels(), 3);
    assert_eq!(model.calls, receipt.plan.tiles().len());
}

#[test]
fn super_resolution_workflow_supports_32_bit_float_tiff_output() {
    let mut model = FixtureModel::new(
        ModelTask::SuperResolution2x,
        ModelTileContract::new(4, 4, 1, 2),
    );
    let settings = SuperResolutionSettings::new(profile("out", b"icc-out"))
        .with_tiff(TiffSettings::new(TiffBitDepth::ThirtyTwo, true, 10_000).expect("settings"));
    let request = SuperResolutionRequest::new(
        source(1, 1, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    )
    .with_settings(settings);
    let mut publisher = MemoryPublisher::default();
    execute_super_resolution(
        &request,
        &mut model,
        &mut publisher,
        None::<&mut MemoryCatalog>,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
        &CancellationToken::default(),
        |_| {},
    )
    .expect("workflow");
    let probe = publisher.probe.expect("probe");
    assert_eq!(probe.sample_format(), TiffSampleFormat::IeeeFloat);
    assert_eq!(probe.bit_depth(), TiffBitDepth::ThirtyTwo);
}

#[test]
fn scale_mismatch_is_rejected_before_inference() {
    let model = FixtureModel::new(
        ModelTask::SuperResolution4x,
        ModelTileContract::new(4, 4, 1, 4),
    );
    let request = SuperResolutionRequest::new(
        source(2, 2, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    );
    let error = plan_super_resolution(
        &request,
        &model,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
    )
    .expect_err("mismatch");
    assert!(matches!(
        error,
        rusttable_ai::SuperResolutionError::Model(
            rusttable_ai::ModelValidationError::TaskScaleMismatch
        )
    ));
}

#[test]
fn shadow_gate_is_frozen_before_tiling_and_auxiliary_is_bounded() {
    let tile = ModelTileContract::new(4, 4, 1, 2);
    let mut model = FixtureModel::new(ModelTask::SuperResolution2x, tile);
    model.manifest = model
        .manifest
        .clone()
        .with_shadow_boost(true)
        .with_auxiliary(vec![AuxiliaryTensor::Constant {
            name: "noise",
            channels: 1,
            value: 25.0 / 255.0,
        }])
        .with_alpha_policy(AlphaPolicy::PreserveNearest);
    let request = SuperResolutionRequest::new(
        source(2, 2, 0.001),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    )
    .with_settings(
        SuperResolutionSettings::new(profile("out", b"icc-out"))
            .with_shadow_policy(ShadowPolicy::Auto),
    );
    let plan = plan_super_resolution(
        &request,
        &model,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
    )
    .expect("plan");
    assert!(plan.shadow_boost());
    let mut publisher = MemoryPublisher::default();
    execute_super_resolution(
        &request,
        &mut model,
        &mut publisher,
        None::<&mut MemoryCatalog>,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
        &CancellationToken::default(),
        |_| {},
    )
    .expect("workflow");
    let input = model.last_input.expect("model input");
    assert_eq!(input.auxiliary().len(), 1);
    assert_eq!(input.auxiliary()[0].len(), 16);
    assert!(input.planar_rgb()[0] > 0.0);
}

#[test]
fn cancellation_never_publishes() {
    let mut model = FixtureModel::new(
        ModelTask::SuperResolution2x,
        ModelTileContract::new(4, 4, 1, 2),
    );
    let request = SuperResolutionRequest::new(
        source(2, 2, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    );
    let mut publisher = MemoryPublisher::default();
    let cancel = CancellationToken::default();
    cancel.cancel();
    let error = execute_super_resolution(
        &request,
        &mut model,
        &mut publisher,
        None::<&mut MemoryCatalog>,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
        &cancel,
        |_| {},
    )
    .expect_err("cancel");
    assert!(matches!(
        error,
        rusttable_ai::SuperResolutionError::Cancelled(_)
    ));
    assert!(publisher.artifact.is_none());
}

#[test]
fn wide_gamut_preservation_is_unavailable_for_upscale() {
    let model = FixtureModel::new(
        ModelTask::SuperResolution2x,
        ModelTileContract::new(4, 4, 1, 2),
    );
    let settings =
        SuperResolutionSettings::new(profile("out", b"icc-out")).with_wide_gamut_preservation(true);
    let request = SuperResolutionRequest::new(
        source(1, 1, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    )
    .with_settings(settings);
    let error = plan_super_resolution(
        &request,
        &model,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
    )
    .expect_err("wide gamut");
    assert!(matches!(
        error,
        rusttable_ai::SuperResolutionError::UnsupportedWideGamut
    ));
}

#[test]
fn tiff_limit_is_rejected_before_model_inference() {
    let model = FixtureModel::new(
        ModelTask::SuperResolution2x,
        ModelTileContract::new(4, 4, 1, 2),
    );
    let settings = SuperResolutionSettings::new(profile("out", b"icc-out"))
        .with_tiff(TiffSettings::new(TiffBitDepth::Sixteen, true, 1).expect("settings"));
    let request = SuperResolutionRequest::new(
        source(2, 2, 0.3),
        profile("working", b"icc-working"),
        SuperResolutionScale::X2,
    )
    .with_settings(settings);
    let error = plan_super_resolution(
        &request,
        &model,
        ProviderPolicy::Cpu,
        MemoryLimits::default(),
    )
    .expect_err("TIFF limit");
    assert!(matches!(error, rusttable_ai::SuperResolutionError::Tiff(_)));
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut output = String::with_capacity(64);
    for byte in sha2::Sha256::digest(bytes) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
