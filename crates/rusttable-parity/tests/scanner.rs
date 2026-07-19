use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_parity::{
    ScanError, parse_manifest, render_manifest, render_receipt, scan_darktable_with_issue_index,
    scan_darktable_with_overrides, validate_manifest,
};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn discovers_all_reference_surfaces_and_is_byte_deterministic() {
    let fixture = Fixture::new();
    let first = scan_darktable_with_overrides(&fixture.root, Fixture::overrides()).unwrap();
    let second = scan_darktable_with_overrides(&fixture.root, Fixture::overrides()).unwrap();
    let first_bytes = render_manifest(&first).unwrap();
    let second_bytes = render_manifest(&second).unwrap();

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(parse_manifest(&first_bytes).unwrap(), first);
    for id in [
        "iop.rawprepare",
        "view.darkroom",
        "lib.import",
        "format.jpeg",
        "storage.disk",
        "lua.gui",
        "build-option.USE_LUA",
        "opencl.basic",
    ] {
        assert!(
            first
                .capabilities
                .iter()
                .any(|capability| capability.id == id),
            "{id}"
        );
    }
    assert!(first.summary.iter().any(|group| group.priority == "P2"));
}

#[test]
fn inventory_receipt_is_deterministic_and_reports_zero_failures() {
    let fixture = Fixture::new();
    let manifest = scan_darktable_with_overrides(&fixture.root, Fixture::overrides()).unwrap();
    let first = render_receipt(&manifest).unwrap();
    let second = render_receipt(&manifest).unwrap();
    assert_eq!(first, second);
    let receipt: toml::Value = toml::from_str(&first).unwrap();
    assert_eq!(
        receipt["unmapped_capabilities"].as_array().unwrap().len(),
        0
    );
    assert_eq!(receipt["stale_issue_numbers"].as_array().unwrap().len(), 0);
    assert_eq!(
        receipt["evidence_incomplete_capabilities"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(receipt["capability_count"].as_integer(), Some(9));
}

#[test]
fn adding_a_registered_module_fails_until_it_has_an_explicit_mapping() {
    let fixture = Fixture::new();
    append(
        fixture.root.join("src/iop/CMakeLists.txt"),
        "add_iop(newmodule \"newmodule.c\")\n",
    );
    write(
        fixture.root.join("src/iop/newmodule.c"),
        "int newmodule(void) { return 0; }\n",
    );

    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, Fixture::overrides()),
        Err(ScanError::UnmappedDiscoveredModule { id, .. }) if id == "iop.newmodule"
    ));
}

#[test]
fn missing_reference_paths_fail_closed() {
    let fixture = Fixture::new();
    append(
        fixture.root.join("src/iop/CMakeLists.txt"),
        "add_iop(missing \"missing.c\")\n",
    );

    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, Fixture::overrides()),
        Err(ScanError::MissingReferencePath { path }) if path == "src/iop/missing.c"
    ));
}

#[test]
fn unregistered_opencl_programs_fail_closed() {
    let fixture = Fixture::new();
    write(
        fixture.root.join("data/kernels/unregistered.cl"),
        "__kernel void unregistered() {}\n",
    );

    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, Fixture::overrides()),
        Err(ScanError::UnregisteredOpenclProgram { id, .. }) if id == "opencl.unregistered"
    ));
}

#[test]
fn invalid_status_and_zero_issue_number_are_rejected() {
    let fixture = Fixture::new();
    let invalid_status = "[[override]]\nid = \"domain.extra\"\nreference_path = \"src/common/darktable.c\"\nstatus = \"deferred\"\nbehavioral_evidence = [\"unit\"]\nacceptance_test_id = \"test\"\nreason = \"test\"\n";
    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, invalid_status),
        Err(ScanError::InvalidStatus { value, .. }) if value == "deferred"
    ));

    let invalid = invalid_status.replace("deferred", "required");
    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, &invalid),
        Err(ScanError::MissingOwnership { id }) if id == "domain.extra"
    ));
}

#[test]
fn duplicate_ids_and_masking_overrides_are_rejected() {
    let fixture = Fixture::new();
    let duplicate = "[[override]]\nid = \"domain.catalog-library\"\nreference_path = \"src/common/darktable.c\"\nstatus = \"required\"\nbehavioral_evidence = [\"unit\"]\nacceptance_test_id = \"test\"\nreason = \"one\"\n[[override]]\nid = \"domain.catalog-library\"\nreference_path = \"src/common/collection.c\"\nstatus = \"required\"\nbehavioral_evidence = [\"unit\"]\nacceptance_test_id = \"test\"\nreason = \"two\"\n";
    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, duplicate),
        Err(ScanError::DuplicateCapabilityId { id }) if id == "domain.catalog-library"
    ));

    let masking = "[[override]]\nid = \"iop.rawprepare\"\nreference_path = \"src/common/darktable.c\"\nstatus = \"required\"\nbehavioral_evidence = [\"unit\"]\nacceptance_test_id = \"test\"\nreason = \"mask\"\n";
    assert!(matches!(
        scan_darktable_with_overrides(&fixture.root, masking),
        Err(ScanError::MaskingOverride { id }) if id == "iop.rawprepare"
    ));
}

#[test]
fn manifest_validation_rejects_unknown_status_after_toml_parse() {
    let manifest = "schema_version = 2\nsource_commit = \"fixture\"\n\n[[capabilities]]\nid = \"iop.rawprepare\"\nreference_path = \"src/iop/rawprepare.c\"\nreference_symbol = \"rawprepare\"\ncategory = \"darkroom\"\nstatus = \"unknown\"\nstructural_evidence = [\"reference-scan:iop\"]\nbehavioral_evidence = [\"unit\"]\nacceptance_test_id = \"test\"\n[[capabilities.ownership]]\nissue_number = 302\nrole = \"implementation\"\nmilestone = \"Processing Pipeline & Color\"\npriority = \"P2\"\n";
    let parsed = parse_manifest(manifest).unwrap();
    assert!(matches!(
        validate_manifest(&parsed),
        Err(ScanError::InvalidStatus { value, .. }) if value == "unknown"
    ));
}

#[test]
fn issue_index_rejects_missing_duplicate_stale_and_superseded_ownership() {
    let fixture = Fixture::new();
    let index = || include_str!("../../../architecture/darktable-issue-index.toml");

    let missing = index().replacen(
        "capability_id = \"iop.rawprepare\"\nissue_number = 302\n",
        "capability_id = \"iop.rawprepare\"\nissue_number = 395\n",
        1,
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &missing),
        Err(ScanError::StaleIssue {
            issue_number: 395,
            ..
        })
    ));

    let duplicate = format!(
        "{}\n[[ownership]]\ncapability_id = \"iop.rawprepare\"\nissue_number = 302\nrole = \"implementation\"\ncategory = \"darkroom\"\nstatus = \"required\"\nstructural_evidence = [\"reference-scan:iop\"]\nbehavioral_evidence = [\"parity:iop.rawprepare\"]\nacceptance_test_id = \"capability.iop.rawprepare\"\n",
        index()
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &duplicate),
        Err(ScanError::DuplicateOwnership { id, issue_number: 302, .. }) if id == "iop.rawprepare"
    ));

    let stale = index().replacen(
        "title = \"Implement the rawprepare operation for sensor normalization\"",
        "title = \"[0302] Implement the rawprepare operation for sensor normalization\"",
        1,
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &stale),
        Err(ScanError::StaleIssue {
            issue_number: 302,
            ..
        })
    ));

    let superseded = index().replacen(
        "state = \"open\"\nmilestone = \"Processing Pipeline & Color\"\npriority = \"P2\"\nparent_issue = 158\ncapability_ids = [\"iop.rawprepare\"]",
        "state = \"closed\"\nstate_reason = \"not_planned\"\nmilestone = \"Processing Pipeline & Color\"\npriority = \"P2\"\nparent_issue = 158\ncapability_ids = [\"iop.rawprepare\"]",
        1,
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &superseded),
        Err(ScanError::StaleIssue {
            issue_number: 302,
            ..
        })
    ));

    let duplicate_issue = format!(
        "{}\n[[issues]]\nnumber = 302\ntitle = \"Implement the rawprepare operation for sensor normalization\"\nstate = \"open\"\nmilestone = \"Processing Pipeline & Color\"\npriority = \"P2\"\nparent_issue = 158\ncapability_ids = [\"iop.rawprepare\"]\n",
        index()
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &duplicate_issue),
        Err(ScanError::InvalidIssueIndex { message }) if message.contains("duplicate issue metadata")
    ));
}

#[test]
fn issue_index_rejects_structural_only_behavioral_evidence_and_unknown_capability() {
    let fixture = Fixture::new();
    let index = include_str!("../../../architecture/darktable-issue-index.toml");
    let structural_only = index.replacen(
        "behavioral_evidence = [\"parity:iop.rawprepare\"]",
        "behavioral_evidence = [\"reference-scan:iop\"]",
        1,
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &structural_only),
        Err(ScanError::InvalidIssueIndex { message }) if message.contains("structural evidence")
    ));

    let unknown = index.replacen(
        "capability_ids = [\"iop.rawprepare\"]",
        "capability_ids = [\"iop.rawprepare\", \"iop.unknown\"]",
        1,
    );
    let unknown = format!(
        "{unknown}\n[[ownership]]\ncapability_id = \"iop.unknown\"\nissue_number = 302\nrole = \"implementation\"\ncategory = \"darkroom\"\nstatus = \"required\"\nstructural_evidence = [\"reference-scan:iop\"]\nbehavioral_evidence = [\"parity:iop.unknown\"]\nacceptance_test_id = \"capability.iop.unknown\"\n"
    );
    assert!(matches!(
        scan_darktable_with_issue_index(&fixture.root, Fixture::overrides(), &unknown),
        Err(ScanError::UnknownIssueCapability { id }) if id == "iop.unknown"
    ));
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rusttable-parity-{number}"));
        let _ = fs::remove_dir_all(&root);
        for directory in [
            "src/iop",
            "src/libs",
            "src/views",
            "src/imageio/format",
            "src/imageio/storage",
            "src/lua",
            "src/common",
            "data/kernels",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        write(
            root.join("src/iop/CMakeLists.txt"),
            "add_iop(rawprepare \"rawprepare.c\")\n",
        );
        write(
            root.join("src/iop/rawprepare.c"),
            "int rawprepare(void) { return 0; }\n",
        );
        write(
            root.join("src/libs/CMakeLists.txt"),
            "add_library(import MODULE \"import.c\")\n",
        );
        write(
            root.join("src/libs/import.c"),
            "int import(void) { return 0; }\n",
        );
        write(
            root.join("src/views/CMakeLists.txt"),
            "add_library(darkroom MODULE \"darkroom.c\")\n",
        );
        write(
            root.join("src/views/darkroom.c"),
            "int darkroom(void) { return 0; }\n",
        );
        write(
            root.join("src/imageio/format/CMakeLists.txt"),
            "add_library(jpeg MODULE \"jpeg.c\")\n",
        );
        write(
            root.join("src/imageio/format/jpeg.c"),
            "int jpeg(void) { return 0; }\n",
        );
        write(
            root.join("src/imageio/storage/CMakeLists.txt"),
            "set(MODULES disk)\n",
        );
        write(
            root.join("src/imageio/storage/disk.c"),
            "int disk(void) { return 0; }\n",
        );
        write(
            root.join("src/lua/init.c"),
            "static lua_CFunction init_funcs[] = { dt_lua_init_gui, NULL };\n",
        );
        write(
            root.join("DefineOptions.cmake"),
            "option(USE_LUA \"Build Lua\" ON)\n",
        );
        write(root.join("data/kernels/programs.conf"), "basic.cl 2\n");
        write(
            root.join("data/kernels/basic.cl"),
            "__kernel void basic() {}\n",
        );
        write(
            root.join("src/common/darktable.c"),
            "int darktable(void) { return 0; }\n",
        );
        write(
            root.join("src/common/collection.c"),
            "int collection(void) { return 0; }\n",
        );
        Self { root }
    }

    fn overrides() -> &'static str {
        "[[override]]\nid = \"domain.catalog-library\"\nreference_path = \"src/common/darktable.c\"\nstatus = \"required\"\nbehavioral_evidence = [\"fixture\"]\nacceptance_test_id = \"fixture.domain.catalog-library\"\nreason = \"cross-cutting\"\n"
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write(path: impl AsRef<Path>, contents: &str) {
    fs::write(path, contents).unwrap();
}

fn append(path: impl AsRef<Path>, contents: &str) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(contents.as_bytes()).unwrap();
}
