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
    let object_start = metadata
        .find(&marker)
        .unwrap_or_else(|| panic!("package {package_name} is missing from cargo metadata"));
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
fn processing_has_only_core_as_a_normal_dependency() {
    let metadata = metadata();
    let package = package_object(&metadata, "rusttable-processing");
    let start = package
        .find("\"dependencies\":[")
        .expect("processing metadata should contain dependencies")
        + "\"dependencies\":".len();
    let end = package[start..]
        .find("],\"targets\"")
        .map(|offset| start + offset)
        .expect("processing metadata should contain targets after dependencies");
    let dependencies = &package[start..end];

    assert!(dependencies.contains("\"name\":\"rusttable-core\""));
    assert_eq!(dependencies.matches("\"name\":").count(), 1);
}

#[test]
fn processing_boundary_has_no_integration_dependencies() {
    let metadata = metadata();
    let package = package_object(&metadata, "rusttable-processing");

    for forbidden in [
        "rusttable-catalog",
        "rusttable-image",
        "rusttable-image-io",
        "rusttable-app",
        "iced",
        "gtk",
        "jpeg",
        "png",
        "redb",
        "serde",
        "tokio",
        "rayon",
        "wgpu",
        "bindgen",
        "cmake",
        "ffi",
    ] {
        assert!(
            !package.contains(&format!("\"name\":\"{forbidden}\"")),
            "processing contains forbidden dependency {forbidden}: {package}"
        );
    }
}
