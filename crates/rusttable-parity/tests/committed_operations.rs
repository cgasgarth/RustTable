use std::fs;
use std::path::Path;

use rusttable_parity::{
    OperationOverride, canonical_layout_hash, parse_operation_manifest, validate_operation_manifest,
};

const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(serde::Deserialize)]
struct OperationOverridesFile {
    operation: Vec<OperationOverride>,
    bloom_completion: BloomCompletion,
    colorreconstruct_completion: ColorReconstructCompletion,
}

#[derive(serde::Deserialize)]
struct BloomCompletion {
    schema: String,
    authoritative_rusttable_baseline: String,
    source_content_commit: String,
    native_source: String,
    status: String,
    payload_fixture_id: String,
    canonical_predecessor: String,
    canonical_order: usize,
    canonical_successor: String,
    retained_registrations: Vec<String>,
    claim: Vec<CompletionClaim>,
    deferral: Vec<BloomDeferral>,
}

#[derive(serde::Deserialize)]
struct CompletionClaim {
    name: String,
    status: String,
    prerequisites: Vec<String>,
}

#[derive(serde::Deserialize)]
struct BloomDeferral {
    name: String,
    source_path: String,
    reason: String,
}

#[derive(serde::Deserialize)]
struct ColorReconstructCompletion {
    schema: String,
    authoritative_rusttable_baseline: String,
    source_content_commit: String,
    canonical_identity: String,
    rust_id: String,
    native_source: String,
    native_kernel_source: String,
    status: String,
    parameter_fixture_ids: Vec<String>,
    migration_fixture_ids: Vec<String>,
    canonical_predecessor: String,
    canonical_order: usize,
    canonical_successor: String,
    retained_registrations: Vec<String>,
    retained_dependencies: Vec<String>,
    claim: Vec<CompletionClaim>,
    deferral: Vec<BloomDeferral>,
}

#[test]
fn committed_operation_manifest_is_valid_and_pinned() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/darktable-operations.toml");
    let source = fs::read_to_string(&path).expect("read committed operation manifest");
    let manifest = parse_operation_manifest(&source).expect("parse committed operation manifest");
    validate_operation_manifest(&manifest).expect("validate committed operation manifest");
    assert_eq!(manifest.reference.source_commit, PINNED_COMMIT);
    assert!(!manifest.operations.is_empty());
}

#[test]
fn larger_tonal_abi_overrides_are_typed_and_match_generated_contracts() {
    let overrides_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let overrides_source = fs::read_to_string(overrides_path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&overrides_source).expect("parse typed operation overrides");
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/darktable-operations.toml");
    let manifest_source = fs::read_to_string(manifest_path).expect("read operation manifest");
    let manifest = parse_operation_manifest(&manifest_source).expect("parse operation manifest");

    for (name, byte_size, decoder, fields) in [
        (
            "colormapping",
            16_600,
            "rusttable.colormapping.decode.v1",
            12,
        ),
        (
            "colortransfer",
            8_280,
            "rusttable.colortransfer.decode.v1",
            5,
        ),
    ] {
        let operation = overrides
            .operation
            .iter()
            .find(|operation| operation.name == name)
            .expect("tonal ABI override");
        assert_eq!(operation.parameter_size, Some(byte_size));
        assert_eq!(operation.parameter_decoder.as_deref(), Some(decoder));
        let version = operation
            .parameter_versions
            .as_ref()
            .and_then(|versions| versions.last())
            .expect("current parameter version");
        assert_eq!(version.byte_size, byte_size);
        assert_eq!(version.abi_layouts.len(), 3);
        assert!(version.abi_layouts.iter().all(|layout| {
            layout.total_size == byte_size
                && layout.alignment == 4
                && layout.fields.len() == fields
                && layout.fields.iter().all(|field| field.name != "raw")
                && canonical_layout_hash(layout) == layout.layout_hash
        }));
        let codec = version.codec.as_ref().expect("typed native codec");
        assert_eq!(codec.decoder, decoder);
        assert_eq!(codec.byte_size, byte_size);
        assert_eq!(codec.fields.len(), fields);

        let generated = manifest
            .operations
            .iter()
            .find(|operation| operation.name == name)
            .expect("generated tonal operation");
        assert_eq!(generated.parameter_size, byte_size);
        let generated_version = generated
            .parameter_versions
            .last()
            .expect("generated version");
        assert_eq!(generated_version.decoder, decoder);
        assert_eq!(generated_version.abi_layouts, version.abi_layouts);
        assert_eq!(generated_version.codec.as_ref(), Some(codec));
    }
}

#[test]
fn bloom_codegen_override_matches_the_source_payload_and_contract() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let source = fs::read_to_string(&path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&source).expect("parse typed operation overrides");
    let bloom = overrides
        .operation
        .iter()
        .find(|operation| operation.name == "bloom")
        .expect("Bloom override");

    assert_eq!(bloom.module_version, Some(1));
    assert_eq!(bloom.parameter_size, Some(12));
    assert_eq!(bloom.default_order, Some(61));
    assert_eq!(bloom.input_color_space.as_deref(), Some("LabD50"));
    assert_eq!(bloom.output_color_space.as_deref(), Some("LabD50"));
    assert_eq!(bloom.roi_behavior.as_deref(), Some("identity"));
    assert_eq!(bloom.tiling_requirement.as_deref(), Some("tiled"));
    assert_eq!(bloom.multi_instance, Some(true));
    assert_eq!(bloom.supports_blend_masks, Some(true));

    let evidence = bloom.evidence.as_ref().expect("Bloom source evidence");
    let registration = evidence
        .iter()
        .find(|item| item.field == "registration")
        .expect("Bloom registration evidence");
    assert_eq!(
        registration.evidence.source_path.as_deref(),
        Some("src/iop/CMakeLists.txt")
    );
    assert_eq!(registration.evidence.line_start, Some(68));
    assert_eq!(registration.evidence.line_end, Some(68));
    let ordering = evidence
        .iter()
        .find(|item| item.field == "ordering")
        .expect("Bloom ordering evidence");
    assert_eq!(
        ordering.evidence.source_path.as_deref(),
        Some("src/common/iop_order.c")
    );
    assert_eq!(
        (ordering.evidence.line_start, ordering.evidence.line_end),
        (Some(638), Some(640))
    );

    let versions = bloom
        .parameter_versions
        .as_ref()
        .expect("Bloom parameter versions");
    assert_eq!(versions.len(), 1);
    let version = &versions[0];
    assert_eq!((version.version, version.byte_size), (1, 12));
    assert_eq!(version.decoder, "rusttable.bloom.decode.v1");
    assert!(!version.opaque_blocking);
    assert_eq!(version.fixture_id, "operation.bloom.params.v1");
    assert_eq!(version.abi_layouts.len(), 3);
    assert!(version.abi_layouts.iter().all(|layout| {
        layout.total_size == 12
            && layout.alignment == 4
            && layout.layout_hash == canonical_layout_hash(layout)
            && layout
                .fields
                .iter()
                .map(|field| {
                    (
                        field.name.as_str(),
                        field.offset,
                        field.size,
                        field.alignment,
                    )
                })
                .eq([
                    ("size", 0, 4, 4),
                    ("threshold", 4, 4, 4),
                    ("strength", 8, 4, 4),
                ])
    }));
    let codec = version.codec.as_ref().expect("Bloom v1 codec override");
    assert_eq!(codec.byte_size, 12);
    assert_eq!(codec.decoder, "rusttable.bloom.decode.v1");
    assert_eq!(codec.encoder, "rusttable.bloom.encode.v1");
    assert_eq!(codec.format, "darktable.iop.bloom.v1");
    assert_eq!(
        codec
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.kind.as_str(), field.offset))
            .collect::<Vec<_>>(),
        [
            ("size", "f32", 0),
            ("threshold", "f32", 4),
            ("strength", "f32", 8),
        ]
    );
    let color = bloom.color_contract.as_ref().expect("Bloom color contract");
    assert_eq!(color.input.value, "LabD50");
    assert_eq!(color.output.value, "LabD50");
    let capabilities = bloom
        .capability_contract
        .as_ref()
        .expect("Bloom capability contract");
    assert!(capabilities.supports_shared_blending);
    assert!(capabilities.supports_drawn_masks);
    let roi = bloom.roi_contract.as_ref().expect("Bloom ROI contract");
    assert_eq!(roi.behavior, "identity");
    assert_eq!(roi.scale, "roi_in.scale / piece.iscale");
    assert_eq!(
        roi.overlap,
        "5 * min(256, ceil(trunc(256 * min(100, size + 1) / 100) * roi_in.scale / piece.iscale))"
    );
    let tiling = bloom
        .tiling_contract
        .as_ref()
        .expect("Bloom tiling contract");
    assert_eq!(tiling.class, "scaled-overlap");
    assert_eq!(tiling.overlap, 1280);

    let canonical = ["colorzones", "bloom", "colorize"].map(|name| {
        overrides
            .operation
            .iter()
            .find(|operation| operation.name == name)
            .and_then(|operation| operation.default_order)
            .expect("canonical operation order")
    });
    assert_eq!(canonical, [60, 61, 62]);
}

#[test]
fn bloom_completion_claims_are_gated_and_deferrals_are_explicit() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let source = fs::read_to_string(&path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&source).expect("parse typed operation overrides");
    let completion = overrides.bloom_completion;

    assert_eq!(completion.schema, "rusttable.bloom-completion.v1");
    assert_eq!(
        completion.authoritative_rusttable_baseline,
        "d8628e8103989bc4ef06dbfb9fd01f3809f884bf"
    );
    assert_eq!(
        completion.source_content_commit,
        "cfe57f3bbf5269bfacf31e832267279caa6938ad"
    );
    assert_eq!(completion.native_source, "src/iop/bloom.c");
    assert_eq!(completion.status, "non-deletion-milestone");
    assert_eq!(completion.payload_fixture_id, "operation.bloom.params.v1");
    assert_eq!(
        (
            completion.canonical_predecessor.as_str(),
            completion.canonical_order,
            completion.canonical_successor.as_str(),
        ),
        ("colorzones", 61, "colorize")
    );
    assert_eq!(
        completion.retained_registrations,
        ["src/iop/CMakeLists.txt:68", "data/kernels/programs.conf:15"]
    );

    assert_eq!(
        completion
            .claim
            .iter()
            .map(|claim| claim.name.as_str())
            .collect::<Vec<_>>(),
        ["cpu", "gpu", "ui", "import"]
    );
    assert!(
        completion.claim.iter().all(|claim| {
            claim.status == "prerequisite-gated" && !claim.prerequisites.is_empty()
        })
    );
    let cpu = completion
        .claim
        .iter()
        .find(|claim| claim.name == "cpu")
        .expect("CPU claim");
    assert!(
        cpu.prerequisites
            .iter()
            .any(|item| item == "scaled-roi-radius-and-five-radius-tiling-overlap")
    );
    assert!(
        cpu.prerequisites
            .iter()
            .any(|item| item == "format-and-allocation-failure-copy-through")
    );
    let import = completion
        .claim
        .iter()
        .find(|claim| claim.name == "import")
        .expect("import claim");
    assert!(
        import
            .prerequisites
            .iter()
            .any(|item| item == "opaque-blend-version-and-payload-preservation")
    );

    assert_eq!(
        completion
            .deferral
            .iter()
            .map(|deferral| deferral.name.as_str())
            .collect::<Vec<_>>(),
        [
            "native-source-deletion",
            "scaled-roi-and-tiling-execution",
            "allocation-failure-copy-through",
            "typed-native-blend-payload",
            "opencl-runtime-resource-strategy",
        ]
    );
    assert!(completion.deferral.iter().all(|deferral| {
        !deferral.source_path.is_empty() && !deferral.reason.trim().is_empty()
    }));
}

#[test]
fn colorreconstruct_override_uses_the_native_identity_and_typed_payloads() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let source = fs::read_to_string(&path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&source).expect("parse typed operation overrides");
    let operation = overrides
        .operation
        .iter()
        .find(|operation| operation.name == "colorreconstruct")
        .expect("Color Reconstruction override");

    assert!(
        overrides
            .operation
            .iter()
            .all(|operation| operation.name != "colorreconstruction")
    );
    assert_eq!(operation.module_version, Some(3));
    assert_eq!(operation.parameter_size, Some(20));
    assert_eq!(
        operation.parameter_layout_hash.as_deref(),
        Some("3756c4815d337d3909424f2833160234eb848b3d538059a748efd4f6c8ae98b8")
    );
    assert_eq!(operation.default_order, Some(69));
    assert_eq!(operation.cpu_implementation.as_deref(), Some("process"));
    assert_eq!(operation.input_color_space.as_deref(), Some("LabD50"));
    assert_eq!(operation.output_color_space.as_deref(), Some("LabD50"));
    assert_eq!(operation.roi_behavior.as_deref(), Some("identity"));
    assert_eq!(operation.tiling_requirement.as_deref(), Some("full-frame"));
    assert_eq!(operation.multi_instance, Some(true));
    assert_eq!(operation.supports_blend_masks, Some(true));

    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/darktable-operations.toml");
    let manifest_source =
        fs::read_to_string(manifest_path).expect("read committed operation manifest");
    let manifest =
        parse_operation_manifest(&manifest_source).expect("parse committed operation manifest");
    let committed = manifest
        .operations
        .iter()
        .find(|candidate| candidate.name == "colorreconstruct")
        .expect("committed Color Reconstruction operation");
    assert_eq!(
        committed.module_version,
        operation.module_version.expect("version")
    );
    assert_eq!(
        committed.parameter_size,
        operation.parameter_size.expect("size")
    );
    assert_eq!(
        committed.parameter_layout_hash,
        operation
            .parameter_layout_hash
            .as_deref()
            .expect("layout hash")
    );
    assert_eq!(
        committed.default_order,
        operation.default_order.expect("order")
    );
    assert_eq!(
        committed.multi_instance,
        operation.multi_instance.expect("instances")
    );
    assert_eq!(
        committed.supports_blend_masks,
        operation.supports_blend_masks.expect("blend masks")
    );
    assert_eq!(
        committed.input_color_space,
        operation.input_color_space.as_deref().expect("input color")
    );
    assert_eq!(
        committed.output_color_space,
        operation
            .output_color_space
            .as_deref()
            .expect("output color")
    );
    assert_eq!(
        committed
            .parameter_versions
            .iter()
            .map(|version| (version.version, version.byte_size, version.decoder.as_str()))
            .collect::<Vec<_>>(),
        operation
            .parameter_versions
            .as_ref()
            .expect("override versions")
            .iter()
            .map(|version| (version.version, version.byte_size, version.decoder.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        committed
            .migrations
            .iter()
            .map(|migration| (migration.from_version, migration.to_version))
            .collect::<Vec<_>>(),
        operation
            .migrations
            .as_ref()
            .expect("override migrations")
            .iter()
            .map(|migration| (migration.from_version, migration.to_version))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        operation.parameter_decoder.as_deref(),
        Some("rusttable.colorreconstruct.decode.v3")
    );
    assert_eq!(
        operation
            .opencl_programs
            .as_ref()
            .expect("Color Reconstruction OpenCL program")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["colorreconstruction"]
    );
    assert_eq!(
        operation
            .opencl_kernels
            .as_ref()
            .expect("Color Reconstruction OpenCL kernels")
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "colorreconstruction_zero",
            "colorreconstruction_splat",
            "colorreconstruction_blur_line",
            "colorreconstruction_slice",
        ]
    );

    let evidence = operation
        .evidence
        .as_ref()
        .expect("Color Reconstruction evidence");
    let registration = evidence
        .iter()
        .find(|item| item.field == "registration")
        .expect("Color Reconstruction registration evidence");
    assert_eq!(
        registration.evidence.source_path.as_deref(),
        Some("src/iop/CMakeLists.txt")
    );
    assert_eq!(
        (
            registration.evidence.line_start,
            registration.evidence.line_end
        ),
        (Some(72), Some(72))
    );
    for (field, source_path, line_start, line_end) in [
        ("layout", "src/iop/colorreconstruction.c", 54, 61),
        ("contract", "src/iop/colorreconstruction.c", 216, 673),
        ("gpu", "src/iop/colorreconstruction.c", 675, 1072),
        (
            "gpu-kernels",
            "data/kernels/colorreconstruction.cl",
            50,
            273,
        ),
        ("ui", "src/iop/colorreconstruction.c", 1133, 1254),
        ("colorspace", "src/iop/colorreconstruction.c", 135, 140),
        ("ordering", "src/common/iop_order.c", 646, 650),
        ("tiling", "src/iop/colorreconstruction.c", 123, 128),
        (
            "tiling-callback",
            "src/iop/colorreconstruction.c",
            1076,
            1130,
        ),
    ] {
        let item = evidence
            .iter()
            .find(|item| item.field == field)
            .unwrap_or_else(|| panic!("missing {field} evidence"));
        assert_eq!(item.evidence.source_path.as_deref(), Some(source_path));
        assert_eq!(
            (item.evidence.line_start, item.evidence.line_end),
            (Some(line_start), Some(line_end))
        );
    }

    let versions = operation
        .parameter_versions
        .as_ref()
        .expect("Color Reconstruction parameter versions");
    assert_eq!(
        versions
            .iter()
            .map(|version| (version.version, version.byte_size))
            .collect::<Vec<_>>(),
        [(1, 12), (2, 16), (3, 20)]
    );
    assert!(versions.iter().all(|version| {
        version.decoder == format!("rusttable.colorreconstruct.decode.v{}", version.version)
            && !version.opaque_blocking
            && version.fixture_id
                == format!("operation.colorreconstruct.params.v{}", version.version)
            && version.evidence.source_path.as_deref() == Some("src/iop/colorreconstruction.c")
    }));
    assert_eq!(
        versions
            .iter()
            .map(|version| {
                (
                    version.version,
                    version.evidence.line_start,
                    version.evidence.line_end,
                )
            })
            .collect::<Vec<_>>(),
        [
            (1, Some(160), Some(165)),
            (2, Some(182), Some(188)),
            (3, Some(54), Some(61))
        ]
    );
    assert_eq!(
        versions
            .iter()
            .map(|version| {
                let codec = version.codec.as_ref().expect("typed parameter codec");
                (
                    version.version,
                    codec.byte_size,
                    codec.decoder.as_str(),
                    codec.encoder.as_str(),
                    codec
                        .fields
                        .iter()
                        .map(|field| (field.name.as_str(), field.kind.as_str(), field.offset))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
        [
            (
                1,
                12,
                "rusttable.colorreconstruct.decode.v1",
                "rusttable.colorreconstruct.encode.v1",
                vec![
                    ("threshold", "f32", 0),
                    ("spatial", "f32", 4),
                    ("range", "f32", 8),
                ],
            ),
            (
                2,
                16,
                "rusttable.colorreconstruct.decode.v2",
                "rusttable.colorreconstruct.encode.v2",
                vec![
                    ("threshold", "f32", 0),
                    ("spatial", "f32", 4),
                    ("range", "f32", 8),
                    ("precedence", "i32", 12),
                ],
            ),
            (
                3,
                20,
                "rusttable.colorreconstruct.decode.v3",
                "rusttable.colorreconstruct.encode.v3",
                vec![
                    ("threshold", "f32", 0),
                    ("spatial", "f32", 4),
                    ("range", "f32", 8),
                    ("hue", "f32", 12),
                    ("precedence", "i32", 16),
                ],
            ),
        ]
    );

    assert!(versions.iter().all(|version| {
        let codec = version.codec.as_ref().expect("typed parameter codec");
        codec.byte_order == "little"
            && codec.preserves_padding
            && codec.format == format!("darktable.iop.colorreconstruct.v{}", version.version)
            && version.abi_layouts.len() == 3
            && version.abi_layouts.iter().all(|layout| {
                layout.endianness == "little"
                    && layout.pointer_width == 64
                    && layout.total_size == version.byte_size
                    && layout.alignment == 4
                    && layout.layout_hash == canonical_layout_hash(layout)
            })
    }));
    let expected_abi_fields = [
        ["threshold", "spatial", "range"].as_slice(),
        ["threshold", "spatial", "range", "precedence"].as_slice(),
        ["threshold", "spatial", "range", "hue", "precedence"].as_slice(),
    ];
    for (version, expected_fields) in versions.iter().zip(expected_abi_fields) {
        assert!(version.abi_layouts.iter().all(|layout| {
            layout
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .eq(expected_fields.iter().copied())
        }));
    }
    let current = versions.last().expect("Color Reconstruction v3");
    assert!(current.abi_layouts.iter().all(|layout| {
        layout.total_size == 20
            && layout.alignment == 4
            && layout.layout_hash == canonical_layout_hash(layout)
            && layout
                .fields
                .iter()
                .map(|field| {
                    (
                        field.name.as_str(),
                        field.offset,
                        field.size,
                        field.alignment,
                    )
                })
                .eq([
                    ("threshold", 0, 4, 4),
                    ("spatial", 4, 4, 4),
                    ("range", 8, 4, 4),
                    ("hue", 12, 4, 4),
                    ("precedence", 16, 4, 4),
                ])
    }));
    assert_eq!(
        operation
            .migrations
            .as_ref()
            .expect("direct Color Reconstruction migrations")
            .iter()
            .map(|migration| {
                (
                    migration.from_version,
                    migration.to_version,
                    migration.strategy.as_str(),
                    migration.fixture_id.as_str(),
                    migration.evidence.source_path.as_deref(),
                    migration.evidence.line_start,
                    migration.evidence.line_end,
                )
            })
            .collect::<Vec<_>>(),
        [
            (
                1,
                3,
                "reference-legacy-params",
                "operation.colorreconstruct.migration.v1-v3",
                Some("src/iop/colorreconstruction.c"),
                Some(158),
                Some(179),
            ),
            (
                2,
                3,
                "reference-legacy-params",
                "operation.colorreconstruct.migration.v2-v3",
                Some("src/iop/colorreconstruction.c"),
                Some(180),
                Some(202),
            ),
        ]
    );

    let color = operation
        .color_contract
        .as_ref()
        .expect("Color Reconstruction color contract");
    assert_eq!(color.input.value, "LabD50");
    assert_eq!(color.output.value, "LabD50");
    let capabilities = operation
        .capability_contract
        .as_ref()
        .expect("Color Reconstruction capability contract");
    assert!(capabilities.supports_shared_blending);
    assert!(capabilities.supports_drawn_masks);
    let roi = operation
        .roi_contract
        .as_ref()
        .expect("Color Reconstruction ROI contract");
    assert_eq!(roi.behavior, "identity");
    assert_eq!(roi.full_analysis, "required");
    let tiling = operation
        .tiling_contract
        .as_ref()
        .expect("Color Reconstruction tiling contract");
    assert_eq!(tiling.class, "full-frame");
    assert_eq!(tiling.overlap, 0);
}

#[test]
fn colorreconstruct_completion_gates_claims_and_blocks_source_deletion() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let source = fs::read_to_string(&path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&source).expect("parse typed operation overrides");
    let completion = overrides.colorreconstruct_completion;

    assert_eq!(
        completion.schema,
        "rusttable.colorreconstruct-completion.v1"
    );
    assert_eq!(
        completion.authoritative_rusttable_baseline,
        "d8628e8103989bc4ef06dbfb9fd01f3809f884bf"
    );
    assert_eq!(completion.source_content_commit, PINNED_COMMIT);
    assert_eq!(completion.canonical_identity, "colorreconstruct");
    assert_eq!(completion.rust_id, "rusttable.colorreconstruct");
    assert_eq!(completion.native_source, "src/iop/colorreconstruction.c");
    assert_eq!(
        completion.native_kernel_source,
        "data/kernels/colorreconstruction.cl"
    );
    assert_eq!(completion.status, "non-deletion-milestone");
    assert_eq!(
        completion.parameter_fixture_ids,
        [
            "operation.colorreconstruct.params.v1",
            "operation.colorreconstruct.params.v2",
            "operation.colorreconstruct.params.v3",
        ]
    );
    assert_eq!(
        completion.migration_fixture_ids,
        [
            "operation.colorreconstruct.migration.v1-v3",
            "operation.colorreconstruct.migration.v2-v3",
        ]
    );
    assert_eq!(
        (
            completion.canonical_predecessor.as_str(),
            completion.canonical_order,
            completion.canonical_successor.as_str(),
        ),
        ("vignette", 69, "finalscale")
    );
    assert_eq!(
        completion.retained_registrations,
        [
            "src/iop/CMakeLists.txt:72",
            "data/kernels/programs.conf:16",
            "src/common/iop_order.c:647",
        ]
    );
    for dependency in [
        "src/common/colorspaces_inline_conversions.h",
        "src/common/opencl.h",
        "src/develop/imageop.h",
        "src/develop/imageop_gui.h",
        "src/develop/tiling.h",
        "data/kernels/colorreconstruction.cl",
    ] {
        assert!(
            completion
                .retained_dependencies
                .iter()
                .any(|item| item == dependency)
        );
    }

    assert_eq!(
        completion
            .claim
            .iter()
            .map(|claim| claim.name.as_str())
            .collect::<Vec<_>>(),
        ["cpu", "gpu", "ui", "import", "pipeline"]
    );
    assert!(
        completion.claim.iter().all(|claim| {
            claim.status == "prerequisite-gated" && !claim.prerequisites.is_empty()
        })
    );
    let cpu = completion
        .claim
        .iter()
        .find(|claim| claim.name == "cpu")
        .expect("CPU claim");
    for prerequisite in [
        "canonical-colorreconstruct-identity",
        "preview-to-full-frozen-grid-hash-lock-and-zoom-routing",
        "allocation-failure-log-and-copy-through",
    ] {
        assert!(cpu.prerequisites.iter().any(|item| item == prerequisite));
    }
    let pipeline = completion
        .claim
        .iter()
        .find(|claim| claim.name == "pipeline")
        .expect("pipeline claim");
    assert!(
        pipeline
            .prerequisites
            .iter()
            .any(|item| item == "no-tile-execution-despite-four-sigma-planner-overlap")
    );

    assert_eq!(
        completion
            .deferral
            .iter()
            .map(|deferral| deferral.name.as_str())
            .collect::<Vec<_>>(),
        [
            "native-source-deletion",
            "native-kernel-deletion",
            "preview-full-frozen-grid-sharing",
            "full-frame-tiling-and-memory-accounting",
            "allocation-failure-copy-through",
            "typed-native-blend-payload",
            "monochrome-and-editor-lifecycle",
            "opencl-runtime-resource-strategy",
        ]
    );
    assert!(completion.deferral.iter().all(|deferral| {
        !deferral.source_path.is_empty() && !deferral.reason.trim().is_empty()
    }));
}

#[test]
fn colorzones_codegen_override_matches_the_lossless_codec() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/operation-overrides.toml");
    let source = fs::read_to_string(&path).expect("read operation overrides");
    let overrides: OperationOverridesFile =
        toml::from_str(&source).expect("parse typed operation overrides");
    let colorzones = overrides
        .operation
        .iter()
        .find(|operation| operation.name == "colorzones")
        .expect("Color Zones override");

    assert_eq!(colorzones.parameter_size, Some(520));
    let versions = colorzones
        .parameter_versions
        .as_ref()
        .expect("Color Zones parameter versions");
    assert_eq!(
        versions
            .iter()
            .map(|version| (version.version, version.byte_size))
            .collect::<Vec<_>>(),
        [(1, 148), (2, 196), (3, 200), (4, 516), (5, 520)]
    );
    assert!(versions.iter().all(|version| {
        version.decoder == format!("rusttable.colorzones.decode.v{}", version.version)
            && !version.opaque_blocking
    }));
    let current = versions.last().expect("Color Zones current override");
    assert_eq!(current.abi_layouts.len(), 3);
    let codec = current
        .codec
        .as_ref()
        .expect("Color Zones v5 codec override");
    assert_eq!(codec.byte_size, 520);
    assert_eq!(codec.decoder, "rusttable.colorzones.decode.v5");
    assert_eq!(
        colorzones
            .migrations
            .as_ref()
            .expect("Color Zones direct migrations")
            .iter()
            .map(|migration| (migration.from_version, migration.to_version))
            .collect::<Vec<_>>(),
        [(1, 5), (2, 5), (3, 5), (4, 5)]
    );
}

#[test]
fn colorzones_contract_matches_the_lossless_cpu_port() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/darktable-operations.toml");
    let source = fs::read_to_string(&path).expect("read committed operation manifest");
    let manifest = parse_operation_manifest(&source).expect("parse committed operation manifest");
    let colorzones = manifest
        .operations
        .iter()
        .find(|operation| operation.name == "colorzones")
        .expect("Color Zones contract");

    assert_eq!(colorzones.module_version, 5);
    assert_eq!(colorzones.parameter_size, 520);
    assert_eq!(colorzones.input_color_space, "LabD50");
    assert_eq!(colorzones.output_color_space, "LabD50");
    assert_eq!(colorzones.roi_behavior, "identity");
    assert_eq!(colorzones.tiling_requirement, "tiled");
    assert_eq!(
        colorzones
            .parameter_versions
            .iter()
            .map(|version| (version.version, version.byte_size))
            .collect::<Vec<_>>(),
        [(1, 148), (2, 196), (3, 200), (4, 516), (5, 520)]
    );
    assert!(colorzones.parameter_versions.iter().all(|version| {
        version.decoder == format!("rusttable.colorzones.decode.v{}", version.version)
            && !version.opaque_blocking
    }));

    let current = colorzones
        .parameter_versions
        .last()
        .expect("Color Zones current parameters");
    assert_eq!(current.abi_layouts.len(), 3);
    assert!(current.abi_layouts.iter().all(|layout| {
        layout.total_size == 520
            && layout.alignment == 4
            && layout.layout_hash == canonical_layout_hash(layout)
    }));
    assert_eq!(
        current.abi_layouts[0]
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.type_name.as_str()))
            .collect::<Vec<_>>(),
        [
            ("channel", "dt_iop_colorzones_channel_t"),
            ("curve", "dt_iop_colorzones_node_t[3][20]"),
            ("curve_num_nodes", "int[3]"),
            ("curve_type", "int[3]"),
            ("strength", "float"),
            ("mode", "dt_iop_colorzones_modes_t"),
            ("splines_version", "int"),
        ]
    );
    let codec = current.codec.as_ref().expect("Color Zones typed v5 codec");
    assert_eq!(codec.byte_size, 520);
    assert_eq!(codec.decoder, "rusttable.colorzones.decode.v5");
    assert_eq!(codec.encoder, "rusttable.colorzones.encode.v5");
    assert_eq!(
        colorzones
            .migrations
            .iter()
            .map(|migration| (migration.from_version, migration.to_version))
            .collect::<Vec<_>>(),
        [(1, 5), (2, 5), (3, 5), (4, 5)]
    );
    assert_eq!(
        codec
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        [
            "channel",
            "curve",
            "curve_num_nodes",
            "curve_type",
            "strength",
            "mode",
            "splines_version",
        ]
    );
}

#[test]
fn colorzones_capability_closure_includes_cpu_and_gpu() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../architecture/operation-capabilities.json");
    let source = fs::read_to_string(&path).expect("read operation capability closure");
    let artifact: serde_json::Value =
        serde_json::from_str(&source).expect("parse operation capability closure");
    let entries = artifact["entries"]
        .as_array()
        .expect("operation capability entries");

    for identity in [
        "darktable:colorzones:src/iop/colorzones.c:v5",
        "rusttable.colorzones",
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry["identity"] == identity)
            .expect("Color Zones capability entry");
        assert_eq!(entry["rust_id"], "rusttable.colorzones");
        assert_eq!(entry["status"], "Implemented");
        assert_eq!(entry["cpu_supported"], true);
        assert_eq!(entry["gpu_supported"], true);
    }
}
