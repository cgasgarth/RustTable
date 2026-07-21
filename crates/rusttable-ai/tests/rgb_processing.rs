use std::sync::{Arc, Mutex};

use rusttable_ai::rgb::{
    AuxiliaryTensorSpec, DetailRecoveryPlan, GamutPolicy, MemoryBudget, ProviderError,
    RgbAiExecutor, RgbAiImage, RgbAiManifest, RgbAiOptions, RgbAiPlan, RgbAiProvider,
    RgbAiTileOutput, RgbAiTileSpec, extended_srgb_decode, extended_srgb_encode,
};
use rusttable_ai::{
    CancellationToken, ImageDimensions, ModelTask, Provider, ProviderPolicy, ShadowPolicy, TileCrop,
};
use rusttable_color::ColorEncoding;

fn manifest(task: ModelTask) -> RgbAiManifest {
    let scale = task.scale_factor();
    let tile = RgbAiTileSpec::new(4, 4, 1, TileCrop::all(1), scale).unwrap();
    RgbAiManifest::new("identity", "1", task, [7; 32], tile)
        .unwrap()
        .with_providers(vec![Provider::CoreMl, Provider::Cpu])
}

fn image(profile: ColorEncoding) -> RgbAiImage {
    RgbAiImage::new(
        ImageDimensions::new(2, 2).unwrap(),
        profile,
        vec![
            [0.10, 0.12, 0.14, 0.1],
            [0.20, 0.22, 0.24, 0.2],
            [0.30, 0.32, 0.34, 0.3],
            [0.40, 0.42, 0.44, 0.4],
        ],
    )
    .unwrap()
}

#[derive(Default)]
struct IdentityProvider {
    calls: Mutex<Vec<Provider>>,
    fail_coreml_once: Mutex<bool>,
    auxiliary_shapes: Mutex<Vec<[u32; 4]>>,
}

impl IdentityProvider {
    fn failing_coreml_once() -> Self {
        Self {
            fail_coreml_once: Mutex::new(true),
            ..Self::default()
        }
    }
}

impl RgbAiProvider for IdentityProvider {
    fn supports(&self, provider: Provider) -> bool {
        matches!(provider, Provider::CoreMl | Provider::Cpu)
    }

    fn infer(
        &self,
        provider: Provider,
        input: &rusttable_ai::rgb::RgbAiTileInput,
        _cancellation: &CancellationToken,
    ) -> Result<RgbAiTileOutput, ProviderError> {
        self.calls.lock().unwrap().push(provider);
        if provider == Provider::CoreMl && *self.fail_coreml_once.lock().unwrap() {
            *self.fail_coreml_once.lock().unwrap() = false;
            return Err(ProviderError::Execution { code: "test" });
        }
        self.auxiliary_shapes
            .lock()
            .unwrap()
            .extend(input.auxiliary().iter().map(|tensor| {
                let (width, height) = tensor.dimensions();
                [1, tensor.channels(), height, width]
            }));
        let (width, height) = input.dimensions();
        RgbAiTileOutput::new(width, height, input.nchw_rgb().to_vec())
    }
}

#[test]
fn identity_model_is_deterministic_and_mirror_padded() {
    let source = image(ColorEncoding::LinearSrgbD65);
    let options = RgbAiOptions {
        provider: ProviderPolicy::Cpu,
        shadow: ShadowPolicy::Disabled,
        ..RgbAiOptions::default()
    };
    let plan = RgbAiPlan::build(
        &source,
        ColorEncoding::LinearSrgbD65,
        &manifest(ModelTask::RgbDenoise),
        options,
    )
    .unwrap();
    assert_eq!(plan.tiles().len(), 1);
    let provider = IdentityProvider::default();
    let executor = RgbAiExecutor::new(&provider);
    let first = executor
        .run(&plan, &source, &CancellationToken::default())
        .unwrap();
    let second = executor
        .run(&plan, &source, &CancellationToken::default())
        .unwrap();
    assert_eq!(first.pixels(), second.pixels());
    assert_eq!(
        first.receipt().output_identity,
        second.receipt().output_identity
    );
    assert!((first.pixels()[0][3] - 0.1).abs() < f32::EPSILON);
}

#[test]
fn auto_retries_identical_plan_on_cpu_before_publication() {
    let source = image(ColorEncoding::LinearSrgbD65);
    let plan = RgbAiPlan::build(
        &source,
        ColorEncoding::LinearSrgbD65,
        &manifest(ModelTask::RgbDenoise),
        RgbAiOptions::default(),
    )
    .unwrap();
    let provider = IdentityProvider::failing_coreml_once();
    let output = RgbAiExecutor::new(&provider)
        .run(&plan, &source, &CancellationToken::default())
        .unwrap();
    assert_eq!(output.receipt().provider, Provider::Cpu);
    assert_eq!(
        *provider.calls.lock().unwrap(),
        vec![Provider::CoreMl, Provider::Cpu]
    );
}

#[test]
fn auxiliary_tensor_is_declared_and_filled_per_tile() {
    let source = image(ColorEncoding::LinearSrgbD65);
    let manifest = manifest(ModelTask::RgbDenoise).with_auxiliary(vec![
        AuxiliaryTensorSpec::constant("noise_level", 1, 25.0 / 255.0).unwrap(),
    ]);
    let provider = IdentityProvider::default();
    let plan = RgbAiPlan::build(
        &source,
        ColorEncoding::LinearSrgbD65,
        &manifest,
        RgbAiOptions::default(),
    )
    .unwrap();
    RgbAiExecutor::new(&provider)
        .run(&plan, &source, &CancellationToken::default())
        .unwrap();
    assert_eq!(
        *provider.auxiliary_shapes.lock().unwrap(),
        vec![[1, 1, 4, 4]]
    );
}

#[test]
fn wide_gamut_is_rejected_for_upscale_and_memory_is_bounded() {
    let source = image(ColorEncoding::LinearSrgbD65);
    let options = RgbAiOptions {
        gamut: GamutPolicy::Preserve { margin: 0.01 },
        ..RgbAiOptions::default()
    };
    assert!(
        RgbAiPlan::build(
            &source,
            ColorEncoding::LinearSrgbD65,
            &manifest(ModelTask::SuperResolution2x),
            options,
        )
        .is_err()
    );
    let options = RgbAiOptions {
        memory: MemoryBudget {
            max_total_bytes: 1,
            max_tile_bytes: 1,
            max_concurrency: 1,
        },
        ..RgbAiOptions::default()
    };
    assert!(
        RgbAiPlan::build(
            &source,
            ColorEncoding::LinearSrgbD65,
            &manifest(ModelTask::RgbDenoise),
            options,
        )
        .is_err()
    );
}

#[test]
fn cancellation_publishes_no_output() {
    let source = image(ColorEncoding::LinearSrgbD65);
    let plan = RgbAiPlan::build(
        &source,
        ColorEncoding::LinearSrgbD65,
        &manifest(ModelTask::RgbDenoise),
        RgbAiOptions::default(),
    )
    .unwrap();
    let cancellation = CancellationToken::default();
    cancellation.cancel();
    let provider = Arc::new(IdentityProvider::default());
    let result = RgbAiExecutor::new(provider.as_ref()).run(&plan, &source, &cancellation);
    assert!(matches!(
        result,
        Err(rusttable_ai::rgb::RgbAiExecutionError::Cancelled)
    ));
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[test]
fn extended_srgb_is_sign_preserving_and_roundtrips_hdr_values() {
    for value in [-2.0_f32, -0.01, 0.0, 0.003, 0.5, 2.0] {
        let encoded = extended_srgb_encode(value).unwrap();
        let decoded = extended_srgb_decode(encoded).unwrap();
        assert_eq!(encoded.is_sign_negative(), value.is_sign_negative());
        assert!((decoded - value).abs() < 0.000_01 * value.abs().max(1.0));
    }
}

#[test]
fn five_band_detail_recovery_is_deterministic_and_hashable() {
    let original = vec![[0.0, 0.0, 0.0, 1.0]; 16];
    let mut denoised = original.clone();
    denoised[5] = [1.0, 1.0, 1.0, 1.0];
    let plan = DetailRecoveryPlan::new(0.5).unwrap();
    let mut first = denoised.clone();
    let mut second = denoised;
    let cancel = CancellationToken::default();
    let (first_residual, first_receipt) =
        plan.recover(&original, &mut first, 4, 4, &cancel).unwrap();
    let (second_residual, second_receipt) =
        plan.recover(&original, &mut second, 4, 4, &cancel).unwrap();
    assert_eq!(first, second);
    assert_eq!(first_residual, second_residual);
    assert_eq!(first_receipt, second_receipt);
    assert_eq!(first_receipt.residual_hash, second_receipt.residual_hash);
    assert!(first_residual.iter().any(|value| *value != 0.0));
}
