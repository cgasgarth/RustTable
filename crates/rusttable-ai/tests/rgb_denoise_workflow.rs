use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rusttable_ai::workflows::rgb_denoise::{
    AlphaOutput, CollisionPolicy, DetailRecoveryPolicy, FileTiffPublisher, GamutPolicy, Matrix3,
    ModelDescriptor, ModelError, ModelTask, OutputBitDepth, ProviderUsed, RgbDenoiseModel,
    RgbDenoisePlan, RgbDenoisePublisher, RgbDenoiseRequest, RgbDenoiseWorkflow, RgbProfile,
    Strength, TiffCompression, TiffRecipe,
};
use rusttable_ai::workflows::rgb_denoise::{ModelTile, PublishError, PublishedArtifact};
use rusttable_image::ImageDimensions;
use rusttable_pixelpipe::{RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel};
use rusttable_processing::RasterDimensions;
use tiff::ColorType;
use tiff::decoder::Decoder;
use tiff::tags::Tag;

const IDENTITY: Matrix3 = Matrix3::from_rows([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn profile() -> RgbProfile {
    RgbProfile::new(
        "test-srgb",
        IDENTITY,
        IDENTITY,
        IDENTITY,
        b"test-icc-profile".to_vec(),
    )
    .expect("profile")
}

fn image(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("dimensions");
    let pixels = pixels
        .into_iter()
        .map(|pixel| RgbaF32Pixel::new(pixel[0], pixel[1], pixel[2], pixel[3]))
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("image")
}

fn model_descriptor() -> ModelDescriptor {
    ModelDescriptor::new(
        "fixture-rgb-denoise",
        ModelTask::RgbDenoise,
        1,
        4,
        1,
        true,
        false,
    )
    .expect("model")
}

#[derive(Debug)]
struct IdentityModel {
    descriptor: ModelDescriptor,
    calls: Arc<Mutex<Vec<ProviderUsed>>>,
    fail_gpu: bool,
}

impl IdentityModel {
    fn new(fail_gpu: bool) -> Self {
        Self {
            descriptor: model_descriptor(),
            calls: Arc::new(Mutex::new(Vec::new())),
            fail_gpu,
        }
    }
}

impl RgbDenoiseModel for IdentityModel {
    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn infer(&self, provider: ProviderUsed, tile: ModelTile<'_>) -> Result<Vec<f32>, ModelError> {
        self.calls.lock().expect("calls lock").push(provider);
        if self.fail_gpu && provider == ProviderUsed::Gpu {
            return Err(ModelError::ProviderFailure);
        }
        Ok(tile.planar_rgb.to_vec())
    }
}

fn request(width: u32, height: u32, pixels: Vec<[f32; 4]>) -> RgbDenoiseRequest {
    RgbDenoiseRequest::new(
        image(width, height, pixels),
        [7; 32],
        profile(),
        profile(),
        PathBuf::from("/tmp/rusttable-ai-test/output.tiff"),
    )
    .expect("request")
    .with_catalog_import(false, false)
}

#[test]
fn immutable_plan_contains_bounded_memory_and_identity_inputs() {
    let model = model_descriptor();
    let plan = RgbDenoisePlan::build(
        640,
        480,
        &model,
        &profile(),
        &profile(),
        Strength::new(70).expect("strength"),
        GamutPolicy::PreserveWideGamut,
        rusttable_ai::workflows::rgb_denoise::ShadowPolicy::Disabled,
        DetailRecoveryPolicy::Recover { strength: 30 },
    )
    .expect("plan");
    assert_eq!(plan.tile.width, 4);
    assert!(plan.memory.bytes > 0);
    assert_ne!(plan.identity, [0; 32]);
    assert_eq!(plan.gamut_policy, GamutPolicy::PreserveWideGamut);
}

#[test]
fn model_contract_rejects_unqualified_and_non_scale_one_models() {
    let unqualified =
        ModelDescriptor::new("fixture", ModelTask::RgbDenoise, 1, 4, 1, false, false).unwrap();
    let model = IdentityModel {
        descriptor: unqualified,
        calls: Arc::new(Mutex::new(Vec::new())),
        fail_gpu: false,
    };
    let publisher = RecordingPublisher::default();
    let mut workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    let error = workflow.run(&request(1, 1, vec![[0.2, 0.2, 0.2, 1.0]]));
    assert!(matches!(
        error,
        Err(rusttable_ai::workflows::rgb_denoise::WorkflowError::Process(_))
    ));
    assert_eq!(
        Strength::new(101),
        Err(rusttable_ai::workflows::rgb_denoise::StrengthError::OutOfRange { value: 101 })
    );
}

#[test]
fn tiling_is_row_major_and_auto_retries_identical_tile_on_cpu() {
    let model = IdentityModel::new(true);
    let calls = Arc::clone(&model.calls);
    let publisher = RecordingPublisher::default();
    let mut workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    let receipt = workflow
        .run(&request(5, 3, vec![[0.2, 0.3, 0.4, 1.0]; 15]))
        .expect("workflow");
    assert_eq!(receipt.provider, ProviderUsed::Cpu);
    assert_eq!(receipt.tile_count, 6);
    assert_eq!(
        calls.lock().expect("calls lock").as_slice(),
        &[
            ProviderUsed::Gpu,
            ProviderUsed::Cpu,
            ProviderUsed::Cpu,
            ProviderUsed::Cpu,
            ProviderUsed::Cpu,
            ProviderUsed::Cpu,
            ProviderUsed::Cpu
        ]
    );
}

#[test]
fn strength_zero_reintroduces_filtered_texture_but_full_strength_does_not() {
    let pixels = (0_u32..16)
        .map(|index| {
            let value = if index.is_multiple_of(2) { 1.0 } else { 0.0 };
            [value, value, value, 1.0]
        })
        .collect::<Vec<_>>();
    let full = request(4, 4, pixels.clone());
    let zero = request(4, 4, pixels).with_strength(Strength::new(0).expect("strength"));
    let model = IdentityModel::new(false);
    let publisher = RecordingPublisher::default();
    let mut full_workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    let full_receipt = full_workflow.run(&full).expect("full strength");
    let mut zero_workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    let zero_receipt = zero_workflow.run(&zero).expect("zero strength");
    assert_eq!(full_receipt.detail_recovery_strength, 0);
    assert_eq!(zero_receipt.detail_recovery_strength, 100);
    assert!(publisher.max_pixel() > 0.0);
}

#[test]
fn wide_gamut_preservation_keeps_out_of_model_gamut_chroma() {
    let working_to_model = Matrix3::from_rows([[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let wide_profile = RgbProfile::new(
        "wide",
        working_to_model,
        IDENTITY,
        IDENTITY,
        b"wide-icc".to_vec(),
    )
    .expect("profile");
    let input = image(2, 1, vec![[0.8, 0.1, 0.1, 1.0], [0.2, 0.2, 0.2, 1.0]]);
    let request = RgbDenoiseRequest::new(
        input,
        [9; 32],
        wide_profile,
        profile(),
        PathBuf::from("/tmp/rusttable-ai-test/wide.tiff"),
    )
    .expect("request")
    .with_catalog_import(false, false);
    let model = IdentityModel::new(false);
    let publisher = RecordingPublisher::default();
    let mut workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    workflow.run(&request).expect("workflow");
    let output = publisher.last_pixels().expect("pixels");
    assert!(output[0][0] > output[0][1] * 3.0);
}

#[test]
fn file_publisher_writes_profiled_single_page_tiff_at_all_supported_depths() {
    let root = temp_directory();
    let model = IdentityModel::new(false);
    for (index, bit_depth) in [
        OutputBitDepth::Eight,
        OutputBitDepth::Sixteen,
        OutputBitDepth::ThirtyTwoFloat,
    ]
    .into_iter()
    .enumerate()
    {
        let destination = root.join(format!("depth-{index}.tiff"));
        let recipe = TiffRecipe::new(
            bit_depth,
            AlphaOutput::PreserveStraight,
            TiffCompression::Uncompressed,
            1_000_000,
        )
        .expect("recipe");
        let request = RgbDenoiseRequest::new(
            image(2, 1, vec![[0.25, 0.5, 0.75, 0.5], [0.1, 0.2, 0.3, 1.0]]),
            [u8::try_from(index).expect("three test depths fit in u8"); 32],
            profile(),
            profile(),
            destination.clone(),
        )
        .expect("request")
        .with_tiff(recipe)
        .with_catalog_import(false, false);
        let publisher = FileTiffPublisher;
        let mut workflow = RgbDenoiseWorkflow::new(&model, &publisher);
        workflow.run(&request).expect("file workflow");
        let file = std::fs::File::open(&destination).expect("TIFF");
        let mut decoder = Decoder::new(file).expect("decoder");
        assert_eq!(decoder.dimensions().expect("dimensions"), (2, 1));
        assert_eq!(
            decoder.get_tag_u8_vec(Tag::IccProfile).expect("ICC"),
            b"test-icc-profile"
        );
        assert!(!decoder.more_images());
        let expected = match bit_depth {
            OutputBitDepth::Eight => ColorType::RGBA(8),
            OutputBitDepth::Sixteen => ColorType::RGBA(16),
            OutputBitDepth::ThirtyTwoFloat => ColorType::RGBA(32),
        };
        assert_eq!(decoder.colortype().expect("color type"), expected);
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn repeated_request_reconciles_to_one_artifact() {
    let root = temp_directory();
    let destination = root.join("same.tiff");
    let model = IdentityModel::new(false);
    let publisher = FileTiffPublisher;
    let request = RgbDenoiseRequest::new(
        image(1, 1, vec![[0.2, 0.3, 0.4, 1.0]]),
        [3; 32],
        profile(),
        profile(),
        destination.clone(),
    )
    .expect("request")
    .with_catalog_import(false, false)
    .with_collision(CollisionPolicy::UniqueSuffix);
    let mut first = RgbDenoiseWorkflow::new(&model, &publisher);
    let first_receipt = first.run(&request).expect("first");
    let mut second = RgbDenoiseWorkflow::new(&model, &publisher);
    let second_receipt = second.run(&request).expect("second");
    assert_eq!(first_receipt.destination, second_receipt.destination);
    assert!(!root.join("same-1.tiff").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn durable_output_requires_an_explicit_import_port_when_requested() {
    let root = temp_directory();
    let destination = root.join("import.tiff");
    let model = IdentityModel::new(false);
    let publisher = FileTiffPublisher;
    let request = RgbDenoiseRequest::new(
        image(1, 1, vec![[0.2, 0.3, 0.4, 1.0]]),
        [4; 32],
        profile(),
        profile(),
        destination.clone(),
    )
    .expect("request");
    let mut workflow = RgbDenoiseWorkflow::new(&model, &publisher);
    assert!(workflow.run(&request).is_err());
    assert!(destination.exists());
    let _ = std::fs::remove_dir_all(root);
}

fn temp_directory() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-ai-rgb-denoise-{}-{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp directory");
    path
}

struct RecordingPublisher {
    pixels: Mutex<Vec<[f32; 4]>>,
}

impl RecordingPublisher {
    fn max_pixel(&self) -> f32 {
        self.pixels
            .lock()
            .expect("pixels lock")
            .iter()
            .flat_map(|pixel| pixel[..3].iter().copied())
            .fold(0.0, f32::max)
    }

    fn last_pixels(&self) -> Option<Vec<[f32; 4]>> {
        let pixels = self.pixels.lock().expect("pixels lock");
        (!pixels.is_empty()).then(|| pixels.clone())
    }
}

impl Default for RecordingPublisher {
    fn default() -> Self {
        Self {
            pixels: Mutex::new(Vec::new()),
        }
    }
}

impl RgbDenoisePublisher for RecordingPublisher {
    fn publish(
        &self,
        _destination: &Path,
        _recipe: &TiffRecipe,
        _collision: CollisionPolicy,
        _profile: &RgbProfile,
        pixels: &[[f32; 4]],
        _dimensions: ImageDimensions,
        _artifact_key: [u8; 32],
    ) -> Result<PublishedArtifact, PublishError> {
        *self.pixels.lock().expect("pixels lock") = pixels.to_vec();
        Ok(PublishedArtifact {
            destination: PathBuf::from("/tmp/recording.tiff"),
            encoded_bytes: 1,
        })
    }
}
