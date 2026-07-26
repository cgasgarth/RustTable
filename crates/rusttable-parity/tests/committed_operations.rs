use std::fs;
use std::path::Path;

use rusttable_parity::{
    OperationOverride, canonical_layout_hash, parse_operation_manifest, validate_operation_manifest,
};

const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(serde::Deserialize)]
struct OperationOverridesFile {
    operation: Vec<OperationOverride>,
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
