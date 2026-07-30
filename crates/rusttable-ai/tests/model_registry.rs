use rusttable_ai::native::{
    Dimension, GraphMetadata, RuntimeIdentity, TensorDataType, TensorDescriptor,
};
use rusttable_ai::{
    AlphaPolicy, BatchPolicy, BlendWindow, ColorContract, ColorPrimaries, ColorRange, DataAsset,
    DimensionSpec, InferenceLimits, InferencePlan, ModelPackage, ModelRegistry, ModelTask,
    OnnxContract, PackageError, PackageLimits, Provider, ProviderPolicy,
    ProviderQualificationReceipt, QualificationFixture, RegistryManifest, ResourceContract,
    SessionCache, TensorContract, TensorDtype, TensorSpec, TileContract, TileCropContract,
    TransferFunction,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn digest(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(value, "{byte:02x}").expect("string write");
    }
    value
}

fn manifest(model: &[u8], assets: &[(&str, &[u8])]) -> RegistryManifest {
    RegistryManifest {
        schema: 1,
        id: "fixture-model".to_owned(),
        version: "1.0.0".to_owned(),
        task: ModelTask::RgbDenoise,
        onnx: OnnxContract {
            ir_version: 8,
            opset: 17,
            required_operators: BTreeSet::from(["Conv".to_owned()]),
        },
        tensors: TensorContract {
            input: tensor("input"),
            output: tensor("output"),
        },
        color: ColorContract {
            source_primaries: ColorPrimaries::Srgb,
            target_primaries: ColorPrimaries::Srgb,
            input_transfer: TransferFunction::ExtendedSrgb,
            output_transfer: TransferFunction::ExtendedSrgb,
            range: ColorRange::Full,
            alpha: AlphaPolicy::Opaque,
            normalization_scale: 1.0,
            normalization_offset: 0.0,
            finite_only: true,
        },
        tile: TileContract {
            width: 4,
            height: 4,
            overlap: 1,
            alignment: 1,
            valid_crop: TileCropContract {
                left: 1,
                top: 1,
                right: 1,
                bottom: 1,
            },
            scale: 1,
            edge_padding: rusttable_ai::EdgePadding::Mirror,
            blend_window: BlendWindow::Linear,
            minimum_width: 1,
            minimum_height: 1,
        },
        providers: BTreeSet::from([Provider::Cpu]),
        resources: ResourceContract {
            estimated_model_bytes: model.len() as u64,
            estimated_session_bytes: 4096,
            estimated_tile_bytes: 1024,
            max_concurrency: 1,
        },
        data_assets: assets
            .iter()
            .map(|(name, bytes)| DataAsset {
                name: (*name).to_owned(),
                sha256: digest(bytes),
                max_bytes: 1024,
            })
            .collect(),
        hashes: rusttable_ai::ManifestHashes {
            model_sha256: digest(model),
            data_sha256: String::new(),
        },
    }
}

fn tensor(name: &str) -> TensorSpec {
    TensorSpec {
        name: name.to_owned(),
        dtype: TensorDtype::F32,
        dimensions: vec![
            DimensionSpec::static_size(1).expect("dimension"),
            DimensionSpec::static_size(3).expect("dimension"),
            DimensionSpec::dynamic("height"),
            DimensionSpec::dynamic("width"),
        ],
        layout: rusttable_ai::TensorLayout::PlanarNchwRgb,
        channels: 3,
        channel_meaning: "rgb".to_owned(),
        batch: BatchPolicy::FixedOne,
    }
}

fn package_bytes(model: &[u8], assets: &[(&str, &[u8])]) -> Vec<u8> {
    let manifest = toml::to_string(&manifest(model, assets)).expect("manifest serialization");
    package_bytes_with_manifest(&manifest, model, assets)
}

fn package_bytes_with_manifest(manifest: &str, model: &[u8], assets: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in [("model.onnx", model), ("model.toml", manifest.as_bytes())]
        .into_iter()
        .chain(assets.iter().copied())
    {
        writer.start_file(name, options).expect("zip file");
        writer.write_all(bytes).expect("zip bytes");
    }
    writer.finish().expect("zip finish").into_inner()
}

#[test]
fn package_rejects_traversal_extra_entries_and_hash_drift() {
    let bytes = package_bytes(b"onnx-fixture", &[]);
    let package = ModelPackage::from_rtmodel(&bytes, PackageLimits::default()).expect("package");
    assert_eq!(package.manifest().id, "fixture-model");
    let original_manifest = toml::to_string(&manifest(b"onnx-fixture", &[])).expect("manifest");
    assert!(matches!(
        ModelPackage::from_rtmodel(
            &package_bytes_with_manifest(&original_manifest, b"different", &[]),
            PackageLimits::default()
        ),
        Err(PackageError::ModelHashMismatch)
    ));

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    writer.start_file("../escape", options).expect("zip file");
    writer.write_all(b"bad").expect("zip bytes");
    let traversal = writer.finish().expect("zip finish").into_inner();
    assert!(matches!(
        ModelPackage::from_rtmodel(&traversal, PackageLimits::default()),
        Err(PackageError::UnsafePath)
    ));

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default();
    let extra_manifest = toml::to_string(&manifest(b"onnx-fixture", &[])).expect("manifest");
    for (name, bytes) in [
        ("model.onnx", b"onnx-fixture".as_slice()),
        ("model.toml", extra_manifest.as_bytes()),
        ("undeclared.bin", b"extra".as_slice()),
    ] {
        writer.start_file(name, options).expect("zip file");
        writer.write_all(bytes).expect("zip bytes");
    }
    let extra = writer.finish().expect("zip finish").into_inner();
    assert!(matches!(
        ModelPackage::from_rtmodel(&extra, PackageLimits::default()),
        Err(PackageError::UndeclaredEntry)
    ));
}

#[test]
fn raw_linear_task_is_a_scale_one_registry_contract() {
    let mut raw_manifest = manifest(b"onnx-fixture", &[]);
    raw_manifest.task = ModelTask::RawLinearDenoise;
    raw_manifest
        .validate()
        .expect("valid RawLinearDenoise manifest");
    assert_eq!(raw_manifest.task.scale_factor(), 1);
    assert_eq!(raw_manifest.task.super_resolution_scale(), None);
    assert!(raw_manifest.providers.contains(&Provider::Cpu));
}

#[test]
fn graph_contract_rejects_external_data_and_accepts_exact_typed_graph() {
    let package = ModelPackage::from_rtmodel(
        &package_bytes(b"onnx-fixture", &[]),
        PackageLimits::default(),
    )
    .expect("package");
    let graph = GraphMetadata::new(
        8,
        17,
        vec!["Conv".to_owned()],
        vec![graph_tensor("input")],
        vec![graph_tensor("output")],
        false,
    );
    package.validate_graph(&graph).expect("graph contract");
    let external = GraphMetadata::new(8, 17, vec!["Conv".to_owned()], vec![], vec![], true);
    assert!(matches!(
        package.validate_graph(&external),
        Err(rusttable_ai::ContractError::ExternalModelData)
    ));
}

fn graph_tensor(name: &str) -> TensorDescriptor {
    TensorDescriptor::new(
        name,
        TensorDataType::F32,
        vec![
            Dimension::Static(1),
            Dimension::Static(3),
            Dimension::Dynamic("height".to_owned()),
            Dimension::Dynamic("width".to_owned()),
        ],
    )
}

#[test]
fn identity_is_content_addressed_and_registry_reconciles_lifecycle() {
    let root = std::env::temp_dir().join(format!("rusttable-ai-478-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut registry = ModelRegistry::open(&root, PackageLimits::default()).expect("registry");
    let installed = registry
        .install_bytes(&package_bytes(b"onnx-fixture", &[]))
        .expect("install");
    let identity = installed.identity();
    assert!(installed.enabled());
    registry.set_enabled(identity, false).expect("disable");
    assert!(registry.acquire(identity).is_err());
    registry.set_enabled(identity, true).expect("enable");
    registry.acquire(identity).expect("acquire");
    assert!(matches!(
        registry.remove(identity),
        Err(rusttable_ai::RegistryError::InUse)
    ));
    registry.release(identity);
    registry.remove(identity).expect("remove");
    assert!(registry.snapshot().models().is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn planner_is_row_major_bounded_and_cache_is_lru() {
    let package = ModelPackage::from_rtmodel(
        &package_bytes(b"onnx-fixture", &[]),
        PackageLimits::default(),
    )
    .expect("package");
    let plan = InferencePlan::build(
        package.identity(),
        rusttable_ai::ImageDimensions::new(7, 6).expect("dimensions"),
        &package.manifest().tile,
        ProviderPolicy::Cpu,
        InferenceLimits::default(),
        3,
        4096,
    )
    .expect("plan");
    assert_eq!(plan.tiles().first().expect("first").output_origin(), (0, 0));
    assert!(
        plan.tiles()
            .windows(2)
            .all(|tiles| tiles[0].output_origin().1 <= tiles[1].output_origin().1)
    );
    let tiny = InferencePlan::build(
        package.identity(),
        rusttable_ai::ImageDimensions::new(2, 2).expect("dimensions"),
        &package.manifest().tile,
        ProviderPolicy::Cpu,
        InferenceLimits::default(),
        3,
        4096,
    )
    .expect("tiny plan");
    assert!(tiny.tiles().iter().all(|tile| tile.padded()));

    let mut cache = SessionCache::new(8);
    assert!(cache.insert(1_u8, "a", 4));
    assert!(cache.insert(2_u8, "b", 4));
    assert_eq!(cache.get_mut(&1).copied(), Some("a"));
    assert!(cache.insert(3_u8, "c", 4));
    assert!(cache.get_mut(&2).is_none());
    assert_eq!(cache.len(), 2);
}

#[test]
fn qualification_is_typed_and_tolerance_bound() {
    let package = ModelPackage::from_rtmodel(
        &package_bytes(b"onnx-fixture", &[]),
        PackageLimits::default(),
    )
    .expect("package");
    let receipt = ProviderQualificationReceipt::qualify(
        package.identity(),
        Provider::Cpu,
        &RuntimeIdentity::new("1.0", "adapter", "test-target"),
        [3; 32],
        QualificationFixture {
            identity: "fixture-input-v1",
            expected: &[1.0, 2.0],
            actual: &[1.0, 2.001],
            absolute_tolerance: 0.01,
            relative_tolerance: 0.0,
        },
    )
    .expect("receipt");
    assert_eq!(receipt.target(), "test-target");
    assert!(
        ProviderQualificationReceipt::qualify(
            package.identity(),
            Provider::Cpu,
            &RuntimeIdentity::new("1.0", "adapter", "test-target"),
            [3; 32],
            QualificationFixture {
                identity: "fixture",
                expected: &[1.0],
                actual: &[1.2],
                absolute_tolerance: 0.01,
                relative_tolerance: 0.0
            }
        )
        .is_err()
    );
}
