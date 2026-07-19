use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use rusttable_parity::{render_operation_manifest, scan_operations_with_overrides};

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
    let mut value = String::from("[[operation]]\nname = \"alpha\"\nowning_issue_number = 448\n");
    for field in ["registration", "layout", "contract", "ownership"] {
        value.push_str(&evidence(field, "src/iop/alpha.c"));
    }
    value.push_str(
        "\n[[operation]]\nname = \"zeta\"\nowning_issue_number = 448\nmodule_version = 2\nparameter_size = 8\nparameter_layout_hash = \"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\"\n",
    );
    for field in ["registration", "layout", "contract", "ownership", "gpu"] {
        value.push_str(&evidence(field, "src/iop/zeta.c"));
    }
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
fn operation_scan_rejects_fabricated_versions_and_missing_ownership() {
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

    let missing_ownership =
        overrides().replace("owning_issue_number = 448", "owning_issue_number = 0");
    assert!(scan_operations_with_overrides(&root, &missing_ownership).is_err());

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
