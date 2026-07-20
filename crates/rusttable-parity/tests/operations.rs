use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use rusttable_parity::{
    CodecField, EnumValue, ParameterCodec, ParameterValue, decode_parameter, encode_parameter,
    render_operation_manifest, scan_operations_with_overrides, validate_operation_manifest,
};

fn fixture_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rusttable-parity-{name}-{}", std::process::id()))
}

fn evidence(field: &str, path: &str) -> String {
    format!(
        "[[operation.evidence]]\nfield = \"{field}\"\nsource_commit = \"fixture\"\nsource_path = \"{path}\"\nline_start = 1\nline_end = 1\nreason = \"fixture source declaration\"\nreviewer = \"cgasgarth\"\nevidence_hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n"
    )
}

fn version_evidence() -> &'static str {
    "source_commit = \"fixture\"\nsource_path = \"src/iop/zeta.c\"\nline_start = 1\nline_end = 1\nreason = \"fixture AST layout\"\nreviewer = \"cgasgarth\"\nevidence_hash = \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"\n"
}

fn overrides() -> String {
    let mut value =
        String::from("[[operation]]\nname = \"alpha\"\ntiling_requirement = \"full-frame\"\n");
    for field in ["registration", "layout", "contract"] {
        value.push_str(&evidence(field, "src/iop/alpha.c"));
    }
    value.push_str(&semantic_contract("alpha", "identity", "full-frame"));
    value.push_str(
        "\n[[operation]]\nname = \"zeta\"\nmodule_version = 2\nparameter_size = 8\nparameter_layout_hash = \"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"\n",
    );
    for field in ["registration", "layout", "contract", "gpu"] {
        value.push_str(&evidence(field, "src/iop/zeta.c"));
    }
    value.push_str(&semantic_contract("zeta", "identity", "tiled"));
    for version in [1, 2] {
        write!(
            value,
            "\n[[operation.parameter_versions]]\nversion = {version}\nbyte_size = 8\nlayout_hash = \"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\"\ndecoder = \"fixture\"\nopaque_blocking = false\nfixture_id = \"operation.zeta.params.v{version}\"\n[operation.parameter_versions.evidence]\n{}",
            version_evidence()
        )
        .expect("writing fixture override");
    }
    value.push_str(
        "\n[[operation.migrations]]\nfrom_version = 1\nto_version = 2\nstrategy = \"direct\"\nfixture_id = \"operation.zeta.migration.v1-v2\"\n[operation.migrations.evidence]\n",
    );
    value.push_str(version_evidence());
    value.push_str(&layout_blocks("operation.parameter_versions.abi_layouts"));
    value.push_str(
        "\n[operation.parameter_versions.codec]\nbyte_size = 8\ndecoder = \"fixture.v2.decode\"\nencoder = \"fixture.v2.encode\"\nbyte_order = \"little\"\npreserves_padding = true\nformat = \"rusttable.operation.v1\"\n[[operation.parameter_versions.codec.fields]]\nname = \"value\"\nkind = \"i32\"\noffset = 0\nsize = 4\n[[operation.parameter_versions.codec.fields]]\nname = \"reserved\"\nkind = \"bytes\"\noffset = 4\nsize = 4\n",
    );
    value.push_str(&layout_blocks("operation.abi_layouts"));
    value
}

fn semantic_contract(name: &str, behavior: &str, tiling: &str) -> String {
    format!(
        "\n[operation.color_contract.input]\nmode = \"unconditional\"\nvalue = \"scene-linear\"\nevidence = [{{ source_commit = \"fixture\", source_path = \"src/iop/{name}.c\", line_start = 1, line_end = 1, reason = \"callback fixture\", reviewer = \"cgasgarth\", evidence_hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" }}]\n[operation.color_contract.output]\nmode = \"unconditional\"\nvalue = \"scene-linear\"\nevidence = [{{ source_commit = \"fixture\", source_path = \"src/iop/{name}.c\", line_start = 1, line_end = 1, reason = \"callback fixture\", reviewer = \"cgasgarth\", evidence_hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" }}]\n[operation.capability_contract]\nsupports_shared_blending = false\nsupports_drawn_masks = false\npublishes_raster_mask = false\nconsumes_raster_mask = false\nflags = []\n[operation.roi_contract]\nbehavior = \"{behavior}\"\noverlap = \"none\"\nfull_analysis = \"not-required\"\ngeometry = \"none\"\nfast_pipe = \"not-supported\"\nscale = \"preserve\"\n[operation.tiling_contract]\nclass = \"{tiling}\"\noverlap = 0\n",
    )
}

fn layout_blocks(path: &str) -> String {
    let mut value = String::new();
    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ] {
        write!(
            value,
            "\n[[{path}]]\ntarget = \"{target}\"\nc_abi_model = \"{target}\"\nendianness = \"little\"\npointer_width = 64\ntotal_size = 8\nalignment = 4\nlayout_hash = \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n[[{path}.fields]]\nname = \"value\"\ntype_name = \"i32\"\noffset = 0\nsize = 4\nalignment = 4\n[[{path}.fields]]\nname = \"reserved\"\ntype_name = \"uint8_t[4]\"\narray_extent = 4\noffset = 4\nsize = 4\nalignment = 1\n"
        )
        .expect("write ABI fixture");
    }
    value
}

#[test]
fn operation_scan_is_deterministic_and_uses_only_explicit_evidence() {
    let root = fixture_root("operations");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/iop")).expect("fixture iop directory");
    fs::create_dir_all(root.join("data/kernels")).expect("fixture kernel directory");
    fs::write(root.join("compile_commands.json"), "[]\n").expect("fixture compile commands");
    fs::write(
        root.join("src/iop/CMakeLists.txt"),
        "add_iop(zeta \"zeta.c\" DEFAULT_VISIBLE)\nadd_iop(alpha \"alpha.c\")\n",
    )
    .expect("fixture registration");
    fs::write(root.join("data/kernels/programs.conf"), "basic.cl 0\n").expect("fixture programs");
    fs::write(
        root.join("data/kernels/basic.cl"),
        "kernel void basic() {}\n",
    )
    .expect("fixture kernel");
    fs::write(
        root.join("src/iop/zeta.c"),
        "DT_MODULE_INTROSPECTION(2, dt_iop_zeta_params_t)\n// a comment mentions preset, fake.cl, and dt_opencl_create_kernel(program, \"comment_kernel\")\nint legacy_params(int old_version) { if(old_version == 1) return 0; }\nvoid process_tiling(void) {}\nvoid init_global(void) { const int program = 0; dt_opencl_create_kernel(program, \"basic\"); }\n",
    )
    .expect("fixture zeta");
    fs::write(root.join("src/iop/alpha.c"), "void process(void) {}\n").expect("fixture alpha");

    let manifest = scan_operations_with_overrides(&root, &overrides()).expect("scan fixture");
    assert_eq!(manifest.operations.len(), 2);
    assert_eq!(manifest.operations[0].name, "alpha");
    let zeta = &manifest.operations[1];
    assert!(zeta.default_enabled);
    assert_eq!(zeta.module_version, 2);
    assert_eq!(zeta.parameter_versions.len(), 2);
    assert_eq!(zeta.migrations.len(), 1);
    assert_eq!(zeta.opencl_programs, vec!["basic"]);
    assert_eq!(zeta.opencl_kernels, vec!["basic"]);
    assert!(zeta.preset_sources.is_empty());

    let first = render_operation_manifest(&manifest).expect("render fixture");
    let second = render_operation_manifest(&manifest).expect("render fixture twice");
    assert_eq!(first, second);
    assert!(first.starts_with("# GENERATED FILE:"));
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn operation_scan_rejects_fabricated_versions_and_invalid_evidence() {
    let root = fixture_root("operation-negative");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/iop")).expect("fixture iop directory");
    fs::create_dir_all(root.join("data/kernels")).expect("fixture kernel directory");
    fs::write(root.join("compile_commands.json"), "[]\n").expect("fixture compile commands");
    fs::write(
        root.join("src/iop/CMakeLists.txt"),
        "add_iop(zeta \"zeta.c\")\nadd_iop(alpha \"alpha.c\")\n",
    )
    .expect("fixture registration");
    fs::write(root.join("src/iop/alpha.c"), "void process(void) {}\n").expect("fixture alpha");
    fs::write(
        root.join("src/iop/zeta.c"),
        "DT_MODULE_INTROSPECTION(2, dt_iop_zeta_params_t)\n",
    )
    .expect("fixture zeta");
    fs::write(root.join("data/kernels/programs.conf"), "basic.cl 0\n").expect("fixture programs");
    fs::write(
        root.join("data/kernels/basic.cl"),
        "kernel void basic() {}\n",
    )
    .expect("fixture kernel");

    let fabricated = overrides().replace(
        "fixture_id = \"operation.zeta.params.v1\"",
        "fixture_id = \"\"",
    );
    assert!(scan_operations_with_overrides(&root, &fabricated).is_err());

    let missing_layout_evidence = overrides().replace(
        "field = \"layout\"\nsource_commit = \"fixture\"\nsource_path = \"src/iop/zeta.c\"",
        "field = \"not-layout\"\nsource_commit = \"fixture\"\nsource_path = \"src/iop/zeta.c\"",
    );
    assert!(scan_operations_with_overrides(&root, &missing_layout_evidence).is_err());

    let unknown_program = overrides().replace(
        "parameter_layout_hash = \"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"",
        "parameter_layout_hash = \"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"\nopencl_programs = [\"not-registered\"]",
    );
    assert!(scan_operations_with_overrides(&root, &unknown_program).is_err());
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn current_parameter_codec_round_trips_typed_fields_and_padding() {
    let codec = ParameterCodec {
        byte_size: 16,
        decoder: "fixture.v1.decode".to_owned(),
        encoder: "fixture.v1.encode".to_owned(),
        byte_order: "little".to_owned(),
        fields: vec![
            CodecField {
                name: "enabled".to_owned(),
                kind: "bool".to_owned(),
                offset: 0,
                size: 1,
                array_extent: None,
                enum_values: Vec::new(),
            },
            CodecField {
                name: "mode".to_owned(),
                kind: "enum_i32".to_owned(),
                offset: 4,
                size: 4,
                array_extent: None,
                enum_values: vec![EnumValue {
                    name: "Mode::Fast".to_owned(),
                    value: 2,
                }],
            },
            CodecField {
                name: "gains".to_owned(),
                kind: "f32".to_owned(),
                offset: 8,
                size: 8,
                array_extent: Some(2),
                enum_values: Vec::new(),
            },
        ],
        preserves_padding: true,
        format: "rusttable.operation.v1".to_owned(),
    };
    let payload = [
        1, 0xaa, 0xbb, 0xcc, 2, 0, 0, 0, 0, 0, 0x80, 0x3f, 0, 0, 0, 0x40,
    ];
    let mut decoded = decode_parameter(&codec, &payload).expect("decode typed payload");
    assert_eq!(decoded.fields["enabled"], ParameterValue::Boolean(true));
    assert_eq!(
        decoded.fields["gains"],
        ParameterValue::Array(vec![ParameterValue::Float(1.0), ParameterValue::Float(2.0)])
    );
    decoded
        .fields
        .insert("mode".to_owned(), ParameterValue::Signed(2));
    assert_eq!(
        encode_parameter(&codec, &decoded).expect("encode typed payload"),
        payload
    );

    let truncated = decode_parameter(&codec, &payload[..15]).expect_err("truncate must fail");
    assert!(truncated.to_string().contains("expected 16"));
}

#[test]
fn semantic_mutations_fail_closed() {
    let root = fixture_root("operation-mutations");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/iop")).expect("fixture iop directory");
    fs::create_dir_all(root.join("data/kernels")).expect("fixture kernel directory");
    fs::write(root.join("compile_commands.json"), "[]\n").expect("fixture compile commands");
    fs::write(
        root.join("src/iop/CMakeLists.txt"),
        "add_iop(zeta \"zeta.c\")\nadd_iop(alpha \"alpha.c\")\n",
    )
    .expect("fixture registration");
    fs::write(
        root.join("src/iop/zeta.c"),
        "void process_tiling(void) {}\n",
    )
    .expect("fixture operation");
    fs::write(root.join("src/iop/alpha.c"), "void process(void) {}\n").expect("fixture alpha");
    fs::write(root.join("data/kernels/programs.conf"), "basic.cl 0\n").expect("fixture programs");
    fs::write(
        root.join("data/kernels/basic.cl"),
        "kernel void basic() {}\n",
    )
    .expect("fixture kernel");

    let manifest = scan_operations_with_overrides(&root, &overrides()).expect("scan fixture");
    let mut unknown_color = manifest.clone();
    unknown_color.operations[1].color_contract.input.value = "unknown".to_owned();
    assert!(validate_operation_manifest(&unknown_color).is_err());

    let mut opaque_current = manifest.clone();
    let current = opaque_current.operations[1]
        .parameter_versions
        .last_mut()
        .expect("current version");
    current.decoder = "opaque".to_owned();
    current.opaque_blocking = true;
    assert!(validate_operation_manifest(&opaque_current).is_err());

    let mut renamed_kernel = manifest;
    renamed_kernel.operations[1].opencl_kernels = vec!["renamed".to_owned()];
    assert!(validate_operation_manifest(&renamed_kernel).is_err());
    fs::remove_dir_all(root).expect("remove fixture");
}
