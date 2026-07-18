use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::mapping::iop_issue;
use crate::operation_model::{
    HistoryCompatibility, Operation, OperationManifest, OperationOverride, OperationOverrideFile,
    ParameterMigration, ParameterVersion,
};
use crate::operation_validate::validate_operation_manifest;
use crate::scan::ScanError;

/// Extracts the registered IOPs and their persisted compatibility metadata.
///
/// # Errors
///
/// Returns an error when the override file, source tree, or extracted manifest is invalid.
pub fn scan_operations(source: &Path, overrides: &Path) -> Result<OperationManifest, ScanError> {
    let text = fs::read_to_string(overrides).map_err(|error| ScanError::Io {
        path: overrides.display().to_string(),
        message: error.to_string(),
    })?;
    scan_operations_with_overrides(source, &text)
}

/// Extracts operations from a source tree using caller-supplied TOML overrides.
///
/// # Errors
///
/// Returns an error when a registration, source declaration, override, or manifest invariant is invalid.
pub fn scan_operations_with_overrides(
    source: &Path,
    overrides: &str,
) -> Result<OperationManifest, ScanError> {
    if !source.is_dir() {
        return Err(ScanError::MissingSurface {
            path: source.display().to_string(),
        });
    }
    let entries = parse_overrides(overrides)?;
    let cmake_path = source.join("src/iop/CMakeLists.txt");
    let cmake = read(source, &cmake_path)?;
    let programs = opencl_registry(source)?;
    let mut operations = Vec::new();
    for (order, line) in cmake.lines().enumerate() {
        let tokens = cmake_tokens(line);
        if tokens.first().map(String::as_str) != Some("add_iop") {
            continue;
        }
        let Some(name) = tokens.get(1) else { continue };
        let Some(file) = tokens.get(2) else {
            return Err(ScanError::OperationExtraction {
                operation: name.clone(),
                message: "add_iop has no source file".to_owned(),
            });
        };
        if operations
            .iter()
            .any(|operation: &Operation| operation.name == *name)
        {
            continue;
        }
        let relative = format!("src/iop/{file}");
        let path = source.join(&relative);
        let content = read(source, &path)?;
        let mut operation = extract_operation(name, &relative, order, &content, &programs);
        operation.default_enabled = tokens.iter().any(|token| token == "DEFAULT_VISIBLE");
        if let Some(override_entry) = entries.iter().find(|entry| entry.name == *name) {
            apply_override(&mut operation, override_entry);
        }
        operations.push(operation);
    }
    operations.sort_by(|left, right| left.name.cmp(&right.name));
    let manifest = OperationManifest {
        schema_version: 1,
        source_commit: reference_commit(source),
        history: history_contract(),
        operations,
    };
    validate_operation_manifest(&manifest)?;
    Ok(manifest)
}

fn extract_operation(
    name: &str,
    relative: &str,
    order: usize,
    content: &str,
    programs: &[String],
) -> Operation {
    let (module_version, parameter_type) =
        introspection(content).unwrap_or((1, "opaque".to_owned()));
    let versions = parameter_versions(content, name, module_version, &parameter_type);
    let migrations = migrations(content, name, module_version, &versions);
    let opencl_programs = opencl_programs(content, programs);
    let opencl_kernels = opencl_kernels(content);
    let issue = iop_issue(name)
        .first()
        .copied()
        .unwrap_or("0235")
        .to_owned();
    Operation {
        name: name.to_owned(),
        reference_path: relative.to_owned(),
        module_version,
        parameter_size: versions.last().map_or(0, |version| version.byte_size),
        parameter_layout_hash: layout_hash(content, parameter_type.as_str()),
        default_enabled: content.contains("DEFAULT_VISIBLE"),
        default_order: order,
        group: extract_group(content),
        tags: extract_tags(content),
        multi_instance: content.contains("IOP_FLAGS_ALLOW_MULTI_INSTANCE"),
        supports_blend_masks: content.contains("blendop") || content.contains("blend_params"),
        input_color_space: extract_colorspace(content, "input"),
        output_color_space: extract_colorspace(content, "output"),
        roi_behavior: roi_behavior(content),
        tiling_requirement: if content.contains("process_tiling") {
            "tiled".to_owned()
        } else {
            "scanline".to_owned()
        },
        cpu_implementation: if content.contains("process_tiling") {
            "process_tiling".to_owned()
        } else {
            "process".to_owned()
        },
        opencl_programs,
        opencl_kernels,
        parameter_versions: versions,
        migrations,
        preset_sources: preset_sources(content),
        owning_issue: issue,
    }
}

fn introspection(content: &str) -> Option<(u32, String)> {
    let marker = "DT_MODULE_INTROSPECTION(";
    let start = content.find(marker)? + marker.len();
    let rest = &content[start..];
    let end = rest.find(')')?;
    let mut parts = rest[..end].split(',').map(str::trim);
    let version = parts.next()?.parse().ok()?;
    let ty = parts.next()?.to_owned();
    Some((version, ty))
}

fn parameter_versions(
    content: &str,
    name: &str,
    current: u32,
    parameter_type: &str,
) -> Vec<ParameterVersion> {
    let mut versions = (1..=current)
        .map(|version| ParameterVersion {
            version,
            byte_size: 0,
            layout_hash: layout_hash(content, &format!("{parameter_type}:{version}")),
            decoder: "opaque".to_owned(),
            opaque_blocking: true,
            fixture_id: format!("operation.{name}.params.v{version}"),
        })
        .collect::<Vec<_>>();
    if versions.is_empty() {
        versions.push(ParameterVersion {
            version: 1,
            byte_size: 0,
            layout_hash: layout_hash(content, parameter_type),
            decoder: "opaque".to_owned(),
            opaque_blocking: true,
            fixture_id: format!("operation.{name}.params.v1"),
        });
    }
    versions
}

fn migrations(
    content: &str,
    name: &str,
    current: u32,
    versions: &[ParameterVersion],
) -> Vec<ParameterMigration> {
    let mut result = Vec::new();
    for from in 1..current {
        let strategy = if content.contains(&format!("old_version == {from}")) {
            "reference-legacy-params"
        } else {
            "opaque-blocking"
        };
        result.push(ParameterMigration {
            from_version: from,
            to_version: from + 1,
            strategy: strategy.to_owned(),
            fixture_id: format!("operation.{name}.migration.v{from}-v{}", from + 1),
        });
    }
    if versions.len() == 1 && content.contains("legacy_params") {
        result.push(ParameterMigration {
            from_version: 0,
            to_version: current,
            strategy: "reference-legacy-params".to_owned(),
            fixture_id: format!("operation.{name}.migration.legacy-v{current}"),
        });
    }
    result
}

fn layout_hash(content: &str, parameter_type: &str) -> String {
    let normalized = content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let mut hasher = Sha256::new();
    hasher.update(parameter_type.as_bytes());
    hasher.update([0]);
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn extract_group(content: &str) -> String {
    content
        .find("default_group")
        .and_then(|start| {
            content[start..]
                .find("IOP_GROUP_")
                .map(|offset| start + offset)
        })
        .and_then(|start| {
            content[start..]
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .next()
        })
        .unwrap_or("IOP_GROUP_UNSPECIFIED")
        .trim_start_matches("IOP_GROUP_")
        .to_ascii_lowercase()
}

fn extract_tags(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for marker in ["IOP_FLAGS_", "IOP_GROUP_"] {
        for token in content.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
            if token.starts_with(marker) && !tags.contains(&token.to_owned()) {
                tags.push(token.to_owned());
            }
        }
    }
    tags.sort();
    tags
}

fn extract_colorspace(content: &str, direction: &str) -> String {
    let needle = if direction == "input" {
        "IOP_CS_IN"
    } else {
        "IOP_CS_OUT"
    };
    content
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .find(|token| token.contains(needle))
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

fn roi_behavior(content: &str) -> String {
    if content.contains("modify_roi_in") && content.contains("modify_roi_out") {
        "expands-and-reduces".to_owned()
    } else if content.contains("modify_roi_in") {
        "expands".to_owned()
    } else if content.contains("modify_roi_out") {
        "reduces".to_owned()
    } else {
        "identity".to_owned()
    }
}

fn opencl_programs(content: &str, programs: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for line in content.lines() {
        if let Some((_, comment)) = line.split_once("//")
            && let Some(file) = comment
                .split_whitespace()
                .map(|token| token.trim_matches(|character| matches!(character, ',' | ';' | ')')))
                .find(|token| {
                    Path::new(token)
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("cl"))
                })
        {
            result.push(file.trim_end_matches(".cl").to_owned());
        }
    }
    for line in content.lines() {
        let Some(start) = line.find("const int program") else {
            continue;
        };
        let Some(value) = line[start..]
            .split('=')
            .nth(1)
            .map(str::trim_start)
            .and_then(|rest| {
                rest.split(|character: char| !character.is_ascii_digit())
                    .next()
            })
            .and_then(|number| number.parse::<usize>().ok())
        else {
            continue;
        };
        if let Some(program) = programs.get(value) {
            result.push(program.clone());
        }
    }
    result.sort();
    result.dedup();
    result
}

fn opencl_registry(source: &Path) -> Result<Vec<String>, ScanError> {
    let contents = read(source, &source.join("data/kernels/programs.conf"))?;
    let mut programs = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        let Some(file) = fields.next() else { continue };
        let Some(index) = fields.next().and_then(|value| value.parse::<usize>().ok()) else {
            continue;
        };
        if !Path::new(file)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cl"))
        {
            continue;
        }
        if programs.len() <= index {
            programs.resize(index + 1, String::new());
        }
        file.trim_end_matches(".cl")
            .clone_into(&mut programs[index]);
    }
    if programs.iter().any(String::is_empty) {
        return Err(ScanError::OperationExtraction {
            operation: "opencl".to_owned(),
            message: "programs.conf contains a sparse registry".to_owned(),
        });
    }
    Ok(programs)
}

fn opencl_kernels(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    for marker in ["dt_opencl_create_kernel"] {
        let mut rest = content;
        while let Some(offset) = rest.find(marker) {
            rest = &rest[offset + marker.len()..];
            if let Some(start) = rest.find('"') {
                let tail = &rest[start + 1..];
                if let Some(end) = tail.find('"') {
                    result.push(tail[..end].to_owned());
                    rest = &tail[end + 1..];
                }
            }
        }
    }
    result.sort();
    result.dedup();
    result
}

fn preset_sources(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| line.contains("preset") || line.contains("PRESET"))
        .map(|line| line.trim().to_owned())
        .take(8)
        .collect()
}

fn history_contract() -> HistoryCompatibility {
    HistoryCompatibility {
        database_table: "history".to_owned(),
        database_fields: vec![
            "operation".to_owned(),
            "enabled".to_owned(),
            "instance".to_owned(),
            "multi_priority".to_owned(),
            "blendop_version".to_owned(),
        ],
        xmp_fields: vec![
            "darktable: operation".to_owned(),
            "darktable: enabled".to_owned(),
            "darktable: multi_name".to_owned(),
            "darktable: multi_priority".to_owned(),
            "darktable: blendop_version".to_owned(),
        ],
        enabled_rule: "zero-or-one persisted boolean; unknown values block decoding".to_owned(),
        instance_rule: "preserve darktable operation name and instance name verbatim".to_owned(),
        blend_rule: "retain blend version and opaque parameters when unsupported".to_owned(),
        ordering_rule: "stable database order, then multi_priority, then source registration order"
            .to_owned(),
    }
}

fn parse_overrides(contents: &str) -> Result<Vec<OperationOverride>, ScanError> {
    if contents.trim().is_empty() {
        return Ok(Vec::new());
    }
    toml::from_str::<OperationOverrideFile>(contents)
        .map(|file| file.operations)
        .map_err(|error| ScanError::InvalidOverrides {
            message: error.to_string(),
        })
}

fn apply_override(operation: &mut Operation, entry: &OperationOverride) {
    if let Some(value) = entry.module_version {
        operation.module_version = value;
    }
    if let Some(value) = entry.parameter_size {
        operation.parameter_size = value;
    }
    if let Some(value) = &entry.parameter_layout_hash {
        operation.parameter_layout_hash.clone_from(value);
    }
    if let Some(value) = entry.parameter_decoder.as_ref()
        && let Some(version) = operation.parameter_versions.last_mut()
    {
        version.decoder.clone_from(value);
        version.opaque_blocking = value == "opaque";
    }
    if let Some(value) = entry.default_enabled {
        operation.default_enabled = value;
    }
    if let Some(value) = entry.default_order {
        operation.default_order = value;
    }
    if let Some(value) = &entry.group {
        operation.group.clone_from(value);
    }
    if let Some(value) = &entry.tags {
        operation.tags.clone_from(value);
    }
    if let Some(value) = entry.multi_instance {
        operation.multi_instance = value;
    }
    if let Some(value) = entry.supports_blend_masks {
        operation.supports_blend_masks = value;
    }
    if let Some(value) = &entry.input_color_space {
        operation.input_color_space.clone_from(value);
    }
    if let Some(value) = &entry.output_color_space {
        operation.output_color_space.clone_from(value);
    }
    if let Some(value) = &entry.roi_behavior {
        operation.roi_behavior.clone_from(value);
    }
    if let Some(value) = &entry.tiling_requirement {
        operation.tiling_requirement.clone_from(value);
    }
    if let Some(value) = &entry.cpu_implementation {
        operation.cpu_implementation.clone_from(value);
    }
    if let Some(value) = &entry.opencl_programs {
        operation.opencl_programs.clone_from(value);
    }
    if let Some(value) = &entry.opencl_kernels {
        operation.opencl_kernels.clone_from(value);
    }
    if let Some(value) = &entry.parameter_versions {
        operation.parameter_versions.clone_from(value);
    }
    if let Some(value) = &entry.migrations {
        operation.migrations.clone_from(value);
    }
    if let Some(value) = &entry.preset_sources {
        operation.preset_sources.clone_from(value);
    }
    if let Some(value) = &entry.owning_issue {
        operation.owning_issue.clone_from(value);
    }
}

fn cmake_tokens(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let body = line[open + 1..].trim_end_matches(')').trim();
    std::iter::once(line[..open].trim().to_owned())
        .chain(
            body.split_whitespace()
                .map(|token| token.trim_matches('"').to_owned()),
        )
        .collect()
}

fn read(source: &Path, path: &Path) -> Result<String, ScanError> {
    fs::read_to_string(path).map_err(|error| ScanError::Io {
        path: path
            .strip_prefix(source)
            .unwrap_or(path)
            .display()
            .to_string(),
        message: error.to_string(),
    })
}

fn reference_commit(source: &Path) -> String {
    std::process::Command::new("git")
        .args([
            "-C",
            &source.display().to_string(),
            "rev-parse",
            "--verify",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "fixture".to_owned(), |value| value.trim().to_owned())
}
