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
    claim: Vec<BloomClaim>,
    deferral: Vec<BloomDeferral>,
}

#[derive(serde::Deserialize)]
struct BloomClaim {
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
