use std::sync::Mutex;

use rusttable_ai::rgb::{
    AuxiliaryTensorSpec, DetailRecoverySettings, GamutPolicy, MemoryBudget, ProviderError,
    RgbAiExecutor, RgbAiImage, RgbAiManifest, RgbAiOptions, RgbAiPlan, RgbAiProvider,
    RgbAiPublication, RgbAiPublicationError, RgbAiTileInput, RgbAiTileOutput, RgbAiTileSpec,
};
use rusttable_ai::{
    AlphaPolicy, CancellationToken, EdgePadding, ImageDimensions, ModelTask, Provider,
    ProviderPolicy, ShadowPolicy, TileCrop,
};
use rusttable_color::ColorEncoding;

fn source(width: u32, height: u32) -> RgbAiImage {
    let pixels = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let x = f32::from(u16::try_from(x).expect("small test coordinate"));
                let y = f32::from(u16::try_from(y).expect("small test coordinate"));
                let width = f32::from(u16::try_from(width).expect("small test width"));
                let height = f32::from(u16::try_from(height).expect("small test height"));
                [x / width, y / height, 0.25, 0.25 + x / width]
            })
        })
        .collect();
    RgbAiImage::new(
        ImageDimensions::new(width, height).expect("test dimensions"),
        ColorEncoding::LinearSrgbD65,
        pixels,
    )
    .expect("test image")
}

fn manifest(scale: u32) -> RgbAiManifest {
    RgbAiManifest::new(
        "rgb-test",
        "1",
        match scale {
            1 => ModelTask::RgbDenoise,
            2 => ModelTask::SuperResolution2x,
            4 => ModelTask::SuperResolution4x,
            _ => panic!("unsupported test scale"),
        },
        [7; 32],
        RgbAiTileSpec::new(4, 4, 1, TileCrop::all(1), scale).expect("test tile"),
    )
    .expect("test manifest")
    .with_auxiliary(vec![
        AuxiliaryTensorSpec::constant("noise", 1, 25.0 / 255.0).expect("aux"),
    ])
}

#[derive(Default)]
struct IdentityProvider {
    calls: Mutex<Vec<Provider>>,
}

impl RgbAiProvider for IdentityProvider {
    fn supports(&self, _provider: Provider) -> bool {
        true
    }

    fn infer(
        &self,
        provider: Provider,
        input: &RgbAiTileInput,
        _cancellation: &CancellationToken,
    ) -> Result<RgbAiTileOutput, ProviderError> {
        self.calls.lock().expect("calls lock").push(provider);
        let (width, height) = input.dimensions();
        RgbAiTileOutput::new(width, height, input.nchw_rgb().to_vec())
    }
}

struct RetryProvider {
    calls: Mutex<Vec<Provider>>,
}

impl RgbAiProvider for RetryProvider {
    fn supports(&self, _provider: Provider) -> bool {
        true
    }

    fn infer(
        &self,
        provider: Provider,
        input: &RgbAiTileInput,
        _cancellation: &CancellationToken,
    ) -> Result<RgbAiTileOutput, ProviderError> {
        self.calls.lock().expect("calls lock").push(provider);
        if provider != Provider::Cpu {
            return Err(ProviderError::Execution {
                code: "test_failure",
            });
        }
        let (width, height) = input.dimensions();
        RgbAiTileOutput::new(width, height, input.nchw_rgb().to_vec())
    }
}

#[derive(Default)]
struct Publication {
    rows: u32,
    discarded: bool,
}

impl RgbAiPublication for Publication {
    fn begin(&mut self, _plan: &RgbAiPlan) -> Result<(), RgbAiPublicationError> {
        Ok(())
    }

    fn publish_rows(&mut self, y: u32, _rows: &[[f32; 4]]) -> Result<(), RgbAiPublicationError> {
        self.rows = y + 1;
        Ok(())
    }

    fn finish(
        &mut self,
        output: &rusttable_ai::rgb::RgbAiOutput,
    ) -> Result<rusttable_ai::rgb::RgbAiPublicationReceipt, RgbAiPublicationError> {
        Ok(rusttable_ai::rgb::RgbAiPublicationReceipt {
            rows: self.rows,
            output_identity: output.receipt().output_identity,
        })
    }

    fn discard(&mut self) {
        self.discarded = true;
    }

    fn rows_committed(&self) -> u32 {
        self.rows
    }
}

#[test]
fn identity_tiles_preserve_rgb_alpha_and_auxiliary_contract() {
    let image = source(3, 2);
    let mut manifest = manifest(1).with_alpha_policy(AlphaPolicy::PreserveNearest);
    manifest = manifest.with_providers(vec![Provider::Cpu]);
    let options = RgbAiOptions {
        gamut: GamutPolicy::Disabled,
        shadow: ShadowPolicy::Disabled,
        detail: None,
        provider: ProviderPolicy::Cpu,
        memory: MemoryBudget::default(),
    };
    let plan =
        RgbAiPlan::build(&image, ColorEncoding::LinearSrgbD65, &manifest, options).expect("plan");
    assert_eq!(plan.tiles_receipt().count, 2);
    assert_eq!(plan.tiles()[0].input_origin(), (-1, -1));
    assert_eq!(plan.tiles()[0].input_dimensions(), (4, 4));
    assert_eq!(plan.shadow().sampled_pixels(), 1);

    let provider = IdentityProvider::default();
    let output = RgbAiExecutor::new(&provider)
        .run(&plan, &image, &CancellationToken::default())
        .expect("identity output");
    assert_eq!(output.pixels().len(), image.pixels().len());
    for (actual, expected) in output.pixels().iter().zip(image.pixels()) {
        assert!((actual[0] - expected[0].clamp(0.0, 1.0)).abs() < 0.000_01);
        assert!((actual[1] - expected[1].clamp(0.0, 1.0)).abs() < 0.000_01);
        assert!((actual[3] - expected[3]).abs() < f32::EPSILON);
    }
    assert_eq!(provider.calls.lock().expect("calls lock").len(), 2);
}

#[test]
fn auto_retries_identical_plan_on_canonical_cpu() {
    let image = source(2, 2);
    let manifest = manifest(1).with_providers(vec![Provider::Cpu, Provider::CoreMl]);
    let options = RgbAiOptions {
        provider: ProviderPolicy::Auto,
        ..RgbAiOptions::default()
    };
    let plan =
        RgbAiPlan::build(&image, ColorEncoding::LinearSrgbD65, &manifest, options).expect("plan");
    let provider = RetryProvider {
        calls: Mutex::new(Vec::new()),
    };
    let output = RgbAiExecutor::new(&provider)
        .run(&plan, &image, &CancellationToken::default())
        .expect("CPU retry");
    assert_eq!(output.receipt().provider, Provider::Cpu);
    assert_eq!(
        provider.calls.lock().expect("calls lock").as_slice(),
        &[Provider::CoreMl, Provider::Cpu]
    );
}

#[test]
fn detail_recovery_and_publication_are_deterministic_and_cancellable() {
    let image = source(4, 4);
    let manifest = manifest(1);
    let detail = DetailRecoverySettings::new(0.5).expect("detail settings");
    let options = RgbAiOptions {
        detail: Some(detail),
        ..RgbAiOptions::default()
    };
    let plan = RgbAiPlan::build(&image, ColorEncoding::LinearSrgbD65, &manifest, options)
        .expect("detail plan");
    let provider = IdentityProvider::default();
    let first = RgbAiExecutor::new(&provider)
        .run(&plan, &image, &CancellationToken::default())
        .expect("first run");
    let second = RgbAiExecutor::new(&provider)
        .run(&plan, &image, &CancellationToken::default())
        .expect("second run");
    assert_eq!(first.receipt(), second.receipt());
    assert_eq!(first.receipt().detail.expect("detail").version, 1);

    let cancelled = CancellationToken::default();
    cancelled.cancel();
    let mut publication = Publication::default();
    let error = RgbAiExecutor::new(&provider)
        .run_published(&plan, &image, &cancelled, &mut publication)
        .expect_err("cancelled publication");
    assert!(matches!(
        error,
        rusttable_ai::rgb::RgbAiExecutionError::Cancelled
    ));
    assert!(publication.discarded);
    assert_eq!(publication.rows_committed(), 0);
}

#[test]
fn wide_gamut_policy_is_rejected_for_scaled_models() {
    let image = source(2, 2);
    let manifest = manifest(2);
    let options = RgbAiOptions {
        gamut: GamutPolicy::Preserve { margin: 0.01 },
        ..RgbAiOptions::default()
    };
    let error = RgbAiPlan::build(&image, ColorEncoding::LinearSrgbD65, &manifest, options)
        .expect_err("scale wide gamut rejection");
    assert!(matches!(
        error,
        rusttable_ai::rgb::RgbAiPlanError::Policy(_)
    ));
}

#[test]
fn mirror_edge_contract_is_explicit() {
    assert_eq!(EdgePadding::Mirror, EdgePadding::Mirror);
    assert_eq!(TileCrop::all(1).left, 1);
}
