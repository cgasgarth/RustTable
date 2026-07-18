use std::process::Command;

fn metadata() -> String {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata should start");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cargo metadata should emit UTF-8")
}

fn package_object<'metadata>(metadata: &'metadata str, package_name: &str) -> &'metadata str {
    let marker = format!("{{\"name\":\"{package_name}\",\"version\"");
    let name_start = metadata
        .find(&marker)
        .unwrap_or_else(|| panic!("package {package_name} is missing from cargo metadata"));
    let object_start = name_start;
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in metadata[object_start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &metadata[object_start..object_start + offset + character.len_utf8()];
                }
            }
            _ => {}
        }
    }
    panic!("package {package_name} metadata object is incomplete");
}

#[test]
fn core_has_no_normal_dependencies() {
    let metadata = metadata();
    let core = package_object(&metadata, "rusttable-core");

    assert!(
        core.contains("\"dependencies\":[]"),
        "core dependencies changed: {core}"
    );
}

#[test]
fn catalog_depends_only_on_core_and_image() {
    let metadata = metadata();
    let catalog = package_object(&metadata, "rusttable-catalog");
    let dependencies_start = catalog
        .find("\"dependencies\":[")
        .expect("catalog metadata should contain dependencies")
        + "\"dependencies\":[".len();
    let dependencies_end = catalog[dependencies_start..]
        .find("],\"targets\"")
        .map(|offset| dependencies_start + offset)
        .expect("catalog metadata should contain targets after dependencies");
    let dependencies = &catalog[dependencies_start..dependencies_end];

    assert!(
        dependencies.contains("\"name\":\"rusttable-core\""),
        "catalog must depend on rusttable-core: {catalog}"
    );
    assert!(
        dependencies.contains("\"name\":\"rusttable-image\""),
        "catalog must depend on rusttable-image: {catalog}"
    );
    assert_eq!(
        dependencies.matches("\"name\":").count(),
        2,
        "catalog must have exactly two normal dependencies: {catalog}"
    );
}

#[test]
fn catalog_boundary_has_no_forbidden_integration_dependencies() {
    let metadata = metadata();
    let catalog = package_object(&metadata, "rusttable-catalog");

    for forbidden in [
        "iced",
        "gtk",
        "bindgen",
        "cmake",
        "database",
        "jpeg",
        "png",
        "processing",
        "ffi",
    ] {
        assert!(
            !catalog.contains(&format!("\"name\":\"{forbidden}\"")),
            "catalog contains forbidden dependency {forbidden}: {catalog}"
        );
    }
}

#[test]
fn image_contract_stays_codec_neutral() {
    let metadata = metadata();
    let image = package_object(&metadata, "rusttable-image");

    assert!(
        image.contains("\"dependencies\":[]"),
        "image dependencies changed: {image}"
    );
    for forbidden in [
        "rusttable-app",
        "rusttable-catalog",
        "rusttable-core",
        "image",
    ] {
        assert!(
            !image.contains(&format!("\"name\":\"{forbidden}\"")),
            "image contains forbidden dependency {forbidden}: {image}"
        );
    }
}

#[test]
fn image_io_contains_the_codec_boundary() {
    let metadata = metadata();
    let image_io = package_object(&metadata, "rusttable-image-io");

    assert!(
        image_io.contains("\"name\":\"rusttable-image\""),
        "image-io must depend on rusttable-image: {image_io}"
    );
    assert!(
        image_io.contains("\"name\":\"image\""),
        "image-io must own the codec dependency: {image_io}"
    );
    for forbidden in [
        "rusttable-app",
        "rusttable-catalog",
        "rusttable-core",
        "iced",
        "gtk",
        "bindgen",
        "cmake",
        "database",
        "processing",
        "ffi",
    ] {
        assert!(
            !image_io.contains(&format!("\"name\":\"{forbidden}\"")),
            "image-io contains forbidden dependency {forbidden}: {image_io}"
        );
    }
}
