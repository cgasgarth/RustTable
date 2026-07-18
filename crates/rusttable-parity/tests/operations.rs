use std::fs;
use std::path::PathBuf;

use rusttable_parity::{render_operation_manifest, scan_operations_with_overrides};

fn fixture_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("rusttable-parity-{name}-{}", std::process::id()))
}

#[test]
fn operation_scan_is_deterministic_and_extracts_legacy_versions() {
    let root = fixture_root("operations");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src/iop")).expect("fixture iop directory");
    fs::create_dir_all(root.join("data/kernels")).expect("fixture kernel directory");
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
        "DT_MODULE_INTROSPECTION(2, dt_iop_zeta_params_t)\nint legacy_params(int old_version) { if(old_version == 1) return 0; }\nvoid process_tiling(void) {}\nvoid init_global(void) { const int program = 0; dt_opencl_create_kernel(program, \"basic\"); }\n",
    )
    .expect("fixture zeta");
    fs::write(root.join("src/iop/alpha.c"), "void process(void) {}\n").expect("fixture alpha");

    let manifest = scan_operations_with_overrides(&root, "").expect("scan fixture");
    assert_eq!(manifest.operations.len(), 2);
    assert_eq!(manifest.operations[0].name, "alpha");
    let zeta = &manifest.operations[1];
    assert!(zeta.default_enabled);
    assert_eq!(zeta.module_version, 2);
    assert_eq!(zeta.parameter_versions.len(), 2);
    assert_eq!(zeta.migrations.len(), 1);
    assert_eq!(zeta.opencl_programs, vec!["basic"]);
    assert_eq!(zeta.opencl_kernels, vec!["basic"]);

    let first = render_operation_manifest(&manifest).expect("render fixture");
    let second = render_operation_manifest(&manifest).expect("render fixture twice");
    assert_eq!(first, second);
    assert!(first.starts_with("# GENERATED FILE:"));
    fs::remove_dir_all(root).expect("remove fixture");
}
