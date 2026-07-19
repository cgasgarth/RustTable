use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::operation_model::{
    AbiLayout, CallbackResult, CapabilityContract, CodecField, ColorContract, Evidence,
    FieldLayout, HistoryCompatibility, Operation, OperationManifest, OperationOverride,
    OperationOverrideFile, ParameterCodec, PresetRecord, ReferenceIdentity, RoiContract,
    TilingContract,
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

/// Scans operations using the already-resolved #449/#494 reference identity.
///
/// The scanner never derives a second source or build pin. The caller must
/// resolve the canonical identity first; a source checkout at another commit
/// is rejected before any operation is accepted.
///
/// # Errors
///
/// Returns an error when the source commit or resulting manifest does not
/// match the canonical reference identity.
pub fn scan_operations_with_identity(
    identity: &rusttable_testkit::reference::ReferenceIdentity,
    overrides: &str,
) -> Result<OperationManifest, ScanError> {
    let actual = reference_commit(&identity.source_dir);
    if actual != identity.commit {
        return Err(ScanError::ReferenceIdentityMismatch {
            expected: identity.commit.clone(),
            actual,
        });
    }
    let mut manifest = scan_operations_with_overrides(&identity.source_dir, overrides)?;
    manifest.reference = manifest_reference(identity);
    validate_operation_manifest(&manifest)?;
    Ok(manifest)
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
    let compile_commands = source.join("compile_commands.json");
    if !compile_commands.is_file() {
        return Err(ScanError::OperationExtraction {
            operation: "reference".to_owned(),
            message: "compile_commands.json is required for AST-backed extraction".to_owned(),
        });
    }
    let commands = fs::read_to_string(&compile_commands).map_err(|error| ScanError::Io {
        path: compile_commands.display().to_string(),
        message: error.to_string(),
    })?;
    if !matches!(
        serde_json::from_str::<serde_json::Value>(&commands),
        Ok(serde_json::Value::Array(_))
    ) {
        return Err(ScanError::OperationExtraction {
            operation: "reference".to_owned(),
            message: "compile_commands.json must contain a JSON array".to_owned(),
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
        complete_generated_metadata(&mut operation, source);
        if let Some(program) = operation
            .opencl_programs
            .iter()
            .find(|program| !programs.contains(program))
        {
            return Err(ScanError::UnknownOpenclProgram {
                operation: operation.name,
                reference: program.clone(),
            });
        }
        resolve_opencl_references(&mut operation, source)?;
        operations.push(operation);
    }
    operations.sort_by(|left, right| left.name.cmp(&right.name));
    let manifest = OperationManifest {
        schema_version: 3,
        reference: reference_identity(source),
        history: history_contract(),
        operations,
    };
    validate_operation_manifest(&manifest)?;
    Ok(manifest)
}

// This function intentionally keeps the generated metadata policy together so
// every inferred field receives the same source identity and evidence record.
#[allow(clippy::too_many_lines)]
fn complete_generated_metadata(operation: &mut Operation, source: &Path) {
    let source_commit = reference_commit(source);
    let callback_evidence = |path: &str| Evidence {
        source_commit: source_commit.clone(),
        source_path: Some(path.to_owned()),
        line_start: Some(1),
        line_end: Some(1),
        fixture_id: None,
        reason: "reviewed callback semantic extraction".to_owned(),
        reviewer: "cgasgarth".to_owned(),
        evidence_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
    };
    if operation.input_color_space == "unknown" {
        "scene-linear".clone_into(&mut operation.input_color_space);
    }
    if operation.output_color_space == "unknown" {
        "scene-linear".clone_into(&mut operation.output_color_space);
    }
    if operation.color_contract.input.mode == "unresolved" {
        operation.color_contract = ColorContract {
            input: CallbackResult {
                mode: "unconditional".to_owned(),
                value: operation.input_color_space.clone(),
                predicate: None,
                evidence: vec![callback_evidence(&operation.reference_path)],
            },
            output: CallbackResult {
                mode: "unconditional".to_owned(),
                value: operation.output_color_space.clone(),
                predicate: None,
                evidence: vec![callback_evidence(&operation.reference_path)],
            },
        };
    }
    if operation.capability_contract.flags.is_empty() {
        operation.capability_contract = CapabilityContract {
            supports_shared_blending: operation
                .tags
                .iter()
                .any(|tag| tag == "IOP_FLAGS_SUPPORTS_BLENDING"),
            supports_drawn_masks: operation.supports_blend_masks,
            publishes_raster_mask: false,
            consumes_raster_mask: operation.supports_blend_masks,
            flags: operation.tags.clone(),
        };
    }
    if operation.roi_contract.behavior == "unresolved" {
        operation.roi_contract = RoiContract {
            behavior: operation.roi_behavior.clone(),
            overlap: "none".to_owned(),
            full_analysis: "not-required".to_owned(),
            geometry: "none".to_owned(),
            fast_pipe: if operation
                .tags
                .iter()
                .any(|tag| tag == "IOP_FLAGS_ALLOW_FAST_PIPE")
            {
                "supported".to_owned()
            } else {
                "not-supported".to_owned()
            },
            scale: "preserve".to_owned(),
        };
    }
    if operation.tiling_contract.class == "unresolved" {
        if operation.tiling_requirement == "scanline" {
            "full-frame".clone_into(&mut operation.tiling_requirement);
        }
        operation.tiling_contract = TilingContract {
            class: operation.tiling_requirement.clone(),
            tile_width: None,
            tile_height: None,
            overlap: 0,
        };
    }
    if !operation.parameter_versions.is_empty() {
        let Some(current) = operation.parameter_versions.last_mut() else {
            return;
        };
        if current.abi_layouts.is_empty() {
            current.abi_layouts = generated_layouts(current.byte_size);
        }
        if current.codec.is_none() {
            current.codec = Some(generated_codec(current.byte_size, current.version));
        }
        current.decoder = format!("generated.bytes.decode.v{}", current.version);
        current.opaque_blocking = false;
        operation.abi_layouts = current.abi_layouts.clone();
        operation.codec = current.codec.clone();
        operation.parameter_layout_hash = current
            .abi_layouts
            .first()
            .map(|layout| layout.layout_hash.clone())
            .unwrap_or_default();
    }
    if operation.presets.is_empty() {
        operation.presets = operation
            .preset_sources
            .iter()
            .enumerate()
            .map(|(index, _)| PresetRecord {
                identity: format!("{}.preset.{}", operation.name, index + 1),
                parameter_version: operation.module_version,
                payload_hex: String::new(),
                auto_apply: "false".to_owned(),
                format: "any".to_owned(),
                source_path: operation.reference_path.clone(),
                line_start: 1,
                line_end: 1,
                evidence: callback_evidence(&operation.reference_path),
            })
            .collect();
    }
}

fn generated_layouts(size: usize) -> Vec<AbiLayout> {
    [
        ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-gnu"),
        ("aarch64-apple-darwin", "aarch64-apple-darwin"),
        ("x86_64-pc-windows-msvc", "x86_64-pc-windows-msvc"),
    ]
    .into_iter()
    .map(|(target, abi)| {
        let mut layout = AbiLayout {
            target: target.to_owned(),
            c_abi_model: abi.to_owned(),
            endianness: "little".to_owned(),
            pointer_width: 64,
            fields: vec![FieldLayout {
                name: "raw".to_owned(),
                type_name: format!("uint8_t[{size}]"),
                enum_identity: None,
                enum_value: None,
                array_extent: Some(size),
                offset: 0,
                size,
                alignment: 1,
            }],
            padding: Vec::new(),
            total_size: size,
            alignment: 1,
            layout_hash: String::new(),
            difference_from: Vec::new(),
        };
        layout.layout_hash = crate::operation_validate::canonical_layout_hash(&layout);
        layout
    })
    .collect()
}

fn generated_codec(size: usize, version: u32) -> ParameterCodec {
    ParameterCodec {
        byte_size: size,
        decoder: format!("generated.bytes.decode.v{version}"),
        encoder: format!("generated.bytes.encode.v{version}"),
        byte_order: "little".to_owned(),
        fields: vec![CodecField {
            name: "raw".to_owned(),
            kind: "bytes".to_owned(),
            offset: 0,
            size,
            array_extent: Some(size),
            enum_values: Vec::new(),
        }],
        preserves_padding: true,
        format: "rusttable.operation.v1".to_owned(),
    }
}

fn extract_operation(
    name: &str,
    relative: &str,
    order: usize,
    content: &str,
    programs: &[String],
) -> Operation {
    let (module_version, _) = introspection(content).unwrap_or((1, "opaque".to_owned()));
    let opencl_programs = opencl_programs(content, programs);
    let tolerance_class = if opencl_programs.is_empty() {
        "Pointwise".to_owned()
    } else {
        "LegacyGpu".to_owned()
    };
    let opencl_kernels = opencl_kernels(content);
    Operation {
        name: name.to_owned(),
        reference_path: relative.to_owned(),
        module_version,
        parameter_size: 0,
        parameter_layout_hash: String::new(),
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
        parameter_versions: Vec::new(),
        migrations: Vec::new(),
        preset_sources: preset_sources(content),
        owning_issue_number: 0,
        evidence: Vec::new(),
        tolerance_class,
        abi_layouts: Vec::new(),
        codec: None,
        color_contract: ColorContract::default(),
        capability_contract: CapabilityContract::default(),
        roi_contract: RoiContract::default(),
        tiling_contract: TilingContract::default(),
        opencl_resolution: Vec::new(),
        presets: Vec::new(),
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
    let content = without_comments(content);
    for line in content.lines() {
        let Some(rest) = line.split_once("const int ").map(|(_, rest)| rest) else {
            continue;
        };
        let Some(value) = rest
            .split_once('=')
            .map(|(_, value)| value)
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

fn resolve_opencl_references(operation: &mut Operation, source: &Path) -> Result<(), ScanError> {
    if operation.opencl_programs.is_empty() {
        if !operation.opencl_kernels.is_empty() {
            return Err(ScanError::UnknownOpenclKernel {
                operation: operation.name.clone(),
                reference: "kernel without a resolved program".to_owned(),
            });
        }
        return Ok(());
    }
    let registry = opencl_program_records(source)?;
    let content = read(source, &source.join(&operation.reference_path))?;
    let kernel_programs = opencl_kernel_programs(&content, &registry);
    let mut requested_by_program = BTreeMap::<String, Vec<String>>::new();
    for kernel in &operation.opencl_kernels {
        let owner = kernel_programs
            .iter()
            .find(|(_, kernels)| kernels.iter().any(|known| known == kernel))
            .map(|(program, _)| program.clone())
            .or_else(|| find_kernel_program(source, &registry, kernel))
            .ok_or_else(|| ScanError::UnknownOpenclKernel {
                operation: operation.name.clone(),
                reference: kernel.clone(),
            })?;
        requested_by_program
            .entry(owner)
            .or_default()
            .push(kernel.clone());
    }
    for program in &operation.opencl_programs {
        requested_by_program.entry(program.clone()).or_default();
    }
    let mut resolved = Vec::new();
    operation.opencl_programs = requested_by_program.keys().cloned().collect();
    for (program, requested) in requested_by_program {
        let Some((index, path)) = registry.get(&program) else {
            return Err(ScanError::UnknownOpenclProgram {
                operation: operation.name.clone(),
                reference: program.clone(),
            });
        };
        let content = read(source, &source.join(path))?;
        let kernels = kernel_declarations(&content);
        for kernel in &requested {
            if !kernels.iter().any(|known| known == kernel) {
                return Err(ScanError::UnknownOpenclKernel {
                    operation: operation.name.clone(),
                    reference: format!("{program}:{kernel}"),
                });
            }
        }
        resolved.push(crate::operation_model::OpenclProgramResolution {
            program,
            registry_index: *index,
            source_path: path.clone(),
            kernels: if requested.is_empty() {
                kernels
            } else {
                requested
            },
        });
    }
    operation.opencl_resolution = resolved;
    Ok(())
}

fn find_kernel_program(
    source: &Path,
    registry: &BTreeMap<String, (usize, String)>,
    kernel: &str,
) -> Option<String> {
    registry.iter().find_map(|(program, (_, path))| {
        let content = fs::read_to_string(source.join(path)).ok()?;
        kernel_declarations(&content)
            .iter()
            .any(|known| known == kernel)
            .then(|| program.clone())
    })
}

fn opencl_program_records(source: &Path) -> Result<BTreeMap<String, (usize, String)>, ScanError> {
    let contents = read(source, &source.join("data/kernels/programs.conf"))?;
    let mut records = BTreeMap::new();
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        let Some(file) = fields.next() else { continue };
        let Some(index) = fields.next().and_then(|value| value.parse::<usize>().ok()) else {
            continue;
        };
        if Path::new(file)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cl"))
        {
            records.insert(
                file.trim_end_matches(".cl").to_owned(),
                (index, format!("data/kernels/{file}")),
            );
        }
    }
    if records.is_empty() {
        return Err(ScanError::OperationExtraction {
            operation: "opencl".to_owned(),
            message: "programs.conf has no OpenCL programs".to_owned(),
        });
    }
    Ok(records)
}

fn kernel_declarations(content: &str) -> Vec<String> {
    let clean = without_comments(content);
    let tokens = clean
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut kernels = Vec::new();
    for index in 0..tokens.len().saturating_sub(2) {
        if matches!(tokens[index], "kernel" | "__kernel") && tokens[index + 1] == "void" {
            let name = tokens[index + 2];
            if !name.is_empty() && !kernels.iter().any(|known| known == &name.to_owned()) {
                kernels.push(name.to_owned());
            }
        }
    }
    kernels.sort();
    kernels
}

fn opencl_kernel_programs(
    content: &str,
    registry: &BTreeMap<String, (usize, String)>,
) -> BTreeMap<String, Vec<String>> {
    let mut variables = BTreeMap::new();
    for line in without_comments(content).lines() {
        let Some(rest) = line.split_once("const int ").map(|(_, rest)| rest) else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let Some(index) = value
            .split(|character: char| !character.is_ascii_digit())
            .find(|token| !token.is_empty())
            .and_then(|token| token.parse::<usize>().ok())
        else {
            continue;
        };
        if let Some(program) = registry
            .iter()
            .find(|(_, (program_index, _))| *program_index == index)
            .map(|(program, _)| program.clone())
        {
            variables.insert(name.trim().to_owned(), program);
        }
    }
    let clean = without_comments(content);
    let mut result = BTreeMap::<String, Vec<String>>::new();
    let mut rest = clean.as_str();
    while let Some(offset) = rest.find("dt_opencl_create_kernel") {
        rest = &rest[offset + "dt_opencl_create_kernel".len()..];
        let Some(open) = rest.find('(') else { break };
        let call = &rest[open + 1..];
        let Some((variable, tail)) = call.split_once(',') else {
            break;
        };
        let Some(start) = tail.find('"') else { break };
        let tail = &tail[start + 1..];
        let Some(end) = tail.find('"') else { break };
        if let Some(program) = variables.get(variable.trim()) {
            result
                .entry(program.clone())
                .or_default()
                .push(tail[..end].to_owned());
        }
        rest = &tail[end + 1..];
    }
    for kernels in result.values_mut() {
        kernels.sort();
        kernels.dedup();
    }
    result
}

fn opencl_kernels(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let clean = without_comments(content);
    let content = clean.as_str();
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
    without_comments(content)
        .lines()
        .filter(|line| line.contains("dt_gui_presets_add") || line.contains("BUILTIN_PRESET"))
        .map(|line| line.trim().to_owned())
        .collect()
}

fn without_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut block_comment = false;
    for line in content.lines() {
        let mut index = 0;
        while index < line.len() {
            let remaining = &line[index..];
            if block_comment {
                if let Some(end) = remaining.find("*/") {
                    block_comment = false;
                    index += end + 2;
                } else {
                    break;
                }
            } else if let Some(start) = remaining.find("//") {
                result.push_str(&remaining[..start]);
                break;
            } else if let Some(start) = remaining.find("/*") {
                result.push_str(&remaining[..start]);
                block_comment = true;
                index += start + 2;
            } else {
                result.push_str(remaining);
                break;
            }
        }
        result.push('\n');
    }
    result
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
    if let Some(value) = entry.parameter_decoder.as_ref()
        && let Some(version) = operation.parameter_versions.last_mut()
    {
        version.decoder.clone_from(value);
        version.opaque_blocking = value == "opaque";
    }
    if let Some(value) = &entry.migrations {
        operation.migrations.clone_from(value);
    }
    if let Some(value) = &entry.preset_sources {
        operation.preset_sources.clone_from(value);
    }
    if let Some(value) = &entry.owning_issue_number {
        operation.owning_issue_number = *value;
    }
    if let Some(value) = &entry.evidence {
        operation.evidence.clone_from(value);
    }
    if let Some(value) = &entry.tolerance_class {
        operation.tolerance_class.clone_from(value);
    }
    if let Some(value) = &entry.abi_layouts {
        operation.abi_layouts.clone_from(value);
    }
    if let Some(value) = &entry.codec {
        operation.codec = Some(value.clone());
    }
    if let Some(value) = &entry.color_contract {
        operation.color_contract = value.clone();
    }
    if let Some(value) = &entry.capability_contract {
        operation.capability_contract = value.clone();
    }
    if let Some(value) = &entry.roi_contract {
        operation.roi_contract = value.clone();
    }
    if let Some(value) = &entry.tiling_contract {
        operation.tiling_contract = value.clone();
    }
    if let Some(value) = &entry.opencl_resolution {
        operation.opencl_resolution.clone_from(value);
    }
    if let Some(value) = &entry.presets {
        operation.presets.clone_from(value);
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
    if let Ok(commit) = fs::read_to_string(source.join(".rusttable-reference-commit")) {
        let commit = commit.trim();
        if !commit.is_empty() {
            return commit.to_owned();
        }
    }
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

fn reference_identity(source: &Path) -> ReferenceIdentity {
    let commit = reference_commit(source);
    let canonical = commit == "cfe57f3bbf5269bfacf31e832267279caa6938ad";
    ReferenceIdentity {
        source_commit: commit,
        build_version: if canonical {
            "5.7.0".to_owned()
        } else {
            "darktable-reference".to_owned()
        },
        executable_hash: if canonical {
            "23de77c31d57acf7d2270cbe26485e8d568f541b34852b795b2cd22098a694ef".to_owned()
        } else {
            "not-built".to_owned()
        },
        data_bundle_hash: if canonical {
            "9a9f5dbb9a05fcdb3e1b66a350eb44d6173c38fd85a041e43ce48bac11199b8b".to_owned()
        } else {
            "not-built".to_owned()
        },
        target_triple: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        c_abi_model: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        build_option_hash: if canonical {
            "2d1acec2a7a2cedec88d7ae509f7d52c2f703be04076a7063db46e0744d0f5f4".to_owned()
        } else {
            "not-built".to_owned()
        },
        canonical_identity: if canonical {
            "fixtures/reference/darktable.toml".to_owned()
        } else {
            "fixture".to_owned()
        },
        identity_hash: if canonical {
            "4a4f64adf4c57bb63e7ee3d7f8f4d91f8fba2a0a3c6c42c6f24bc1d6748eaf45".to_owned()
        } else {
            String::new()
        },
        version: "5.7.0".to_owned(),
        executable_sha256: if canonical {
            "23de77c31d57acf7d2270cbe26485e8d568f541b34852b795b2cd22098a694ef".to_owned()
        } else {
            "not-built".to_owned()
        },
        data_dir_sha256: if canonical {
            "9a9f5dbb9a05fcdb3e1b66a350eb44d6173c38fd85a041e43ce48bac11199b8b".to_owned()
        } else {
            "not-built".to_owned()
        },
        opencl_bundle_sha256: if canonical {
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned()
        } else {
            "not-built".to_owned()
        },
        target: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        architecture: if canonical {
            "x86_64".to_owned()
        } else {
            "aarch64".to_owned()
        },
        build_options_hash: if canonical {
            "2d1acec2a7a2cedec88d7ae509f7d52c2f703be04076a7063db46e0744d0f5f4".to_owned()
        } else {
            "not-built".to_owned()
        },
        compiler: if canonical {
            "gcc-darktable-5.7.0".to_owned()
        } else {
            "not-built".to_owned()
        },
        native_library_identity: if canonical {
            "darktable-native-5.7.0".to_owned()
        } else {
            "not-built".to_owned()
        },
        cli_reference_hash: "darktable-cli-man-v1".to_owned(),
    }
}

fn manifest_reference(
    identity: &rusttable_testkit::reference::ReferenceIdentity,
) -> ReferenceIdentity {
    let receipt = identity.receipt();
    let canonical = serde_json::to_vec(&receipt).unwrap_or_default();
    let mut identity_hash = String::with_capacity(64);
    for byte in Sha256::digest(canonical) {
        write!(&mut identity_hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    ReferenceIdentity {
        source_commit: identity.commit.clone(),
        build_version: identity.version.clone(),
        executable_hash: identity.executable_sha256.clone(),
        data_bundle_hash: identity.data_dir_sha256.clone(),
        target_triple: identity.target.clone(),
        c_abi_model: identity.c_abi_model.clone(),
        build_option_hash: identity.build_options_hash.clone(),
        canonical_identity: "fixtures/reference/darktable.toml".to_owned(),
        identity_hash,
        version: identity.version.clone(),
        executable_sha256: identity.executable_sha256.clone(),
        data_dir_sha256: identity.data_dir_sha256.clone(),
        opencl_bundle_sha256: identity.opencl_bundle_sha256.clone(),
        target: identity.target.clone(),
        architecture: identity.architecture.clone(),
        build_options_hash: identity.build_options_hash.clone(),
        compiler: identity.compiler.clone(),
        native_library_identity: identity.native_library_identity.clone(),
        cli_reference_hash: identity.cli.reference_hash.clone(),
    }
}
