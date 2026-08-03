use std::collections::{BTreeMap, BTreeSet};

use super::{
    CompatHistoryHash, CompatHistoryStep, CompatModuleInstance, CompatModuleOrder,
    DARKTABLE_ORDER_RULES, DarktableOperationManifest, EnabledState, Finding, FindingCode,
    HistoryLimits, HistoryOrderSource, HistoryRows, ModuleInstanceId, ModuleOrderEntry,
    ModuleOrderRule, ModuleOrderVersion, OpaquePayload, Severity, SourceRowKey, finding,
};

// Keep this separate from `Operation::default_order`: the operation scanner
// records CMake registration ordinals there, while a built-in `module_order`
// row names one of Darktable's versioned pixel-pipeline tables.
//
// Migration oracle from pinned darktable
// `cfe57f3bbf5269bfacf31e832267279caa6938ad:src/common/iop_order.c::legacy_order`.

const LEGACY_BUILT_IN_ORDER: &[&str] = &[
    "rawprepare",
    "invert",
    "temperature",
    "rasterfile",
    "highlights",
    "cacorrect",
    "hotpixels",
    "rawdenoise",
    "demosaic",
    "mask_manager",
    "denoiseprofile",
    "tonemap",
    "exposure",
    "spots",
    "retouch",
    "lens",
    "cacorrectrgb",
    "ashift",
    "liquify",
    "rotatepixels",
    "scalepixels",
    "flip",
    "enlargecanvas",
    "clipping",
    "toneequal",
    "crop",
    "overlay",
    "graduatednd",
    "basecurve",
    "bilateral",
    "profile_gamma",
    "hazeremoval",
    "colorin",
    "channelmixerrgb",
    "diffuse",
    "censorize",
    "negadoctor",
    "blurs",
    "basicadj",
    "primaries",
    "colorreconstruct",
    "colorchecker",
    "defringe",
    "equalizer",
    "vibrance",
    "colorharmonizer",
    "colorbalance",
    "colorequal",
    "colorbalancergb",
    "colorize",
    "colortransfer",
    "colormapping",
    "bloom",
    "nlmeans",
    "globaltonemap",
    "shadhi",
    "atrous",
    "bilat",
    "colorzones",
    "lowlight",
    "monochrome",
    "sigmoid",
    "agx",
    "filmic",
    "filmicrgb",
    "colisa",
    "zonesystem",
    "tonecurve",
    "levels",
    "rgblevels",
    "rgbcurve",
    "relight",
    "colorcorrection",
    "sharpen",
    "lowpass",
    "highpass",
    "grain",
    "lut3d",
    "colorcontrast",
    "colorout",
    "channelmixer",
    "soften",
    "vignette",
    "splittoning",
    "velvia",
    "clahe",
    "finalscale",
    "overexposed",
    "rawoverexposed",
    "dither",
    "borders",
    "watermark",
    "gamma",
];

// Migration oracle from pinned darktable
// `cfe57f3bbf5269bfacf31e832267279caa6938ad:src/common/iop_order.c::v30_order`.
const V30_BUILT_IN_ORDER: &[&str] = &[
    "rawprepare",
    "invert",
    "temperature",
    "rasterfile",
    "highlights",
    "cacorrect",
    "hotpixels",
    "rawdenoise",
    "demosaic",
    "denoiseprofile",
    "bilateral",
    "rotatepixels",
    "scalepixels",
    "lens",
    "cacorrectrgb",
    "hazeremoval",
    "ashift",
    "flip",
    "enlargecanvas",
    "overlay",
    "clipping",
    "liquify",
    "spots",
    "retouch",
    "exposure",
    "mask_manager",
    "tonemap",
    "toneequal",
    "crop",
    "graduatednd",
    "profile_gamma",
    "equalizer",
    "colorin",
    "channelmixerrgb",
    "diffuse",
    "censorize",
    "negadoctor",
    "blurs",
    "primaries",
    "nlmeans",
    "colorchecker",
    "defringe",
    "atrous",
    "lowpass",
    "highpass",
    "sharpen",
    "colortransfer",
    "colormapping",
    "channelmixer",
    "basicadj",
    "colorharmonizer",
    "colorbalance",
    "colorequal",
    "colorbalancergb",
    "rgbcurve",
    "rgblevels",
    "basecurve",
    "filmic",
    "sigmoid",
    "agx",
    "filmicrgb",
    "lut3d",
    "colisa",
    "tonecurve",
    "levels",
    "shadhi",
    "zonesystem",
    "globaltonemap",
    "relight",
    "bilat",
    "colorcorrection",
    "colorcontrast",
    "velvia",
    "vibrance",
    "colorzones",
    "bloom",
    "colorize",
    "lowlight",
    "monochrome",
    "grain",
    "soften",
    "splittoning",
    "vignette",
    "colorreconstruct",
    "colorout",
    "clahe",
    "finalscale",
    "overexposed",
    "rawoverexposed",
    "dither",
    "borders",
    "watermark",
    "gamma",
];

// Migration oracle from pinned darktable
// `cfe57f3bbf5269bfacf31e832267279caa6938ad:src/common/iop_order.c::v30_jpg_order`.
const V30_JPEG_BUILT_IN_ORDER: &[&str] = &[
    "rawprepare",
    "invert",
    "temperature",
    "rasterfile",
    "highlights",
    "cacorrect",
    "hotpixels",
    "rawdenoise",
    "demosaic",
    "colorin",
    "denoiseprofile",
    "bilateral",
    "rotatepixels",
    "scalepixels",
    "lens",
    "cacorrectrgb",
    "hazeremoval",
    "ashift",
    "flip",
    "enlargecanvas",
    "overlay",
    "clipping",
    "liquify",
    "spots",
    "retouch",
    "exposure",
    "mask_manager",
    "tonemap",
    "toneequal",
    "crop",
    "graduatednd",
    "profile_gamma",
    "equalizer",
    "channelmixerrgb",
    "diffuse",
    "censorize",
    "negadoctor",
    "blurs",
    "primaries",
    "nlmeans",
    "colorchecker",
    "defringe",
    "atrous",
    "lowpass",
    "highpass",
    "sharpen",
    "colortransfer",
    "colormapping",
    "channelmixer",
    "basicadj",
    "colorharmonizer",
    "colorbalance",
    "colorequal",
    "colorbalancergb",
    "rgbcurve",
    "rgblevels",
    "basecurve",
    "filmic",
    "agx",
    "sigmoid",
    "filmicrgb",
    "lut3d",
    "colisa",
    "tonecurve",
    "levels",
    "shadhi",
    "zonesystem",
    "globaltonemap",
    "relight",
    "bilat",
    "colorcorrection",
    "colorcontrast",
    "velvia",
    "vibrance",
    "colorzones",
    "bloom",
    "colorize",
    "lowlight",
    "monochrome",
    "grain",
    "soften",
    "splittoning",
    "vignette",
    "colorreconstruct",
    "colorout",
    "clahe",
    "finalscale",
    "overexposed",
    "rawoverexposed",
    "dither",
    "borders",
    "watermark",
    "gamma",
];

// Migration oracle from pinned darktable
// `cfe57f3bbf5269bfacf31e832267279caa6938ad:src/common/iop_order.c::v50_order`.
const V50_BUILT_IN_ORDER: &[&str] = &[
    "rawprepare",
    "invert",
    "temperature",
    "rasterfile",
    "highlights",
    "cacorrect",
    "hotpixels",
    "rawdenoise",
    "demosaic",
    "denoiseprofile",
    "bilateral",
    "rotatepixels",
    "scalepixels",
    "lens",
    "cacorrectrgb",
    "hazeremoval",
    "ashift",
    "flip",
    "enlargecanvas",
    "overlay",
    "clipping",
    "liquify",
    "spots",
    "retouch",
    "exposure",
    "mask_manager",
    "tonemap",
    "toneequal",
    "crop",
    "graduatednd",
    "profile_gamma",
    "equalizer",
    "colorin",
    "channelmixerrgb",
    "diffuse",
    "censorize",
    "negadoctor",
    "blurs",
    "primaries",
    "nlmeans",
    "colorchecker",
    "defringe",
    "atrous",
    "lowpass",
    "highpass",
    "sharpen",
    "colortransfer",
    "colormapping",
    "channelmixer",
    "basicadj",
    "colorharmonizer",
    "colorbalance",
    "colorequal",
    "colorbalancergb",
    "rgbcurve",
    "rgblevels",
    "basecurve",
    "filmic",
    "sigmoid",
    "agx",
    "filmicrgb",
    "lut3d",
    "colisa",
    "tonecurve",
    "levels",
    "shadhi",
    "zonesystem",
    "globaltonemap",
    "relight",
    "bilat",
    "colorcorrection",
    "colorcontrast",
    "velvia",
    "vibrance",
    "colorzones",
    "bloom",
    "colorize",
    "lowlight",
    "monochrome",
    "grain",
    "soften",
    "splittoning",
    "vignette",
    "colorreconstruct",
    "finalscale",
    "colorout",
    "clahe",
    "overexposed",
    "rawoverexposed",
    "dither",
    "borders",
    "watermark",
    "gamma",
];

// Migration oracle from pinned darktable
// `cfe57f3bbf5269bfacf31e832267279caa6938ad:src/common/iop_order.c::v50_jpg_order`.
const V50_JPEG_BUILT_IN_ORDER: &[&str] = &[
    "rawprepare",
    "invert",
    "temperature",
    "rasterfile",
    "highlights",
    "cacorrect",
    "hotpixels",
    "rawdenoise",
    "demosaic",
    "colorin",
    "denoiseprofile",
    "bilateral",
    "rotatepixels",
    "scalepixels",
    "lens",
    "cacorrectrgb",
    "hazeremoval",
    "ashift",
    "flip",
    "enlargecanvas",
    "overlay",
    "clipping",
    "liquify",
    "spots",
    "retouch",
    "exposure",
    "mask_manager",
    "tonemap",
    "toneequal",
    "crop",
    "graduatednd",
    "profile_gamma",
    "equalizer",
    "channelmixerrgb",
    "diffuse",
    "censorize",
    "negadoctor",
    "blurs",
    "primaries",
    "nlmeans",
    "colorchecker",
    "defringe",
    "atrous",
    "lowpass",
    "highpass",
    "sharpen",
    "colortransfer",
    "colormapping",
    "channelmixer",
    "basicadj",
    "colorharmonizer",
    "colorbalance",
    "colorequal",
    "colorbalancergb",
    "rgbcurve",
    "rgblevels",
    "basecurve",
    "filmic",
    "sigmoid",
    "agx",
    "filmicrgb",
    "lut3d",
    "colisa",
    "tonecurve",
    "levels",
    "shadhi",
    "zonesystem",
    "globaltonemap",
    "relight",
    "bilat",
    "colorcorrection",
    "colorcontrast",
    "velvia",
    "vibrance",
    "colorzones",
    "bloom",
    "colorize",
    "lowlight",
    "monochrome",
    "grain",
    "soften",
    "splittoning",
    "vignette",
    "colorreconstruct",
    "finalscale",
    "colorout",
    "clahe",
    "overexposed",
    "rawoverexposed",
    "dither",
    "borders",
    "watermark",
    "gamma",
];

pub(super) fn decode_module_order(
    image_id: i64,
    rows: &HistoryRows,
    limits: &HistoryLimits,
    findings: &mut Vec<Finding>,
    manifest: &DarktableOperationManifest,
) -> Option<CompatModuleOrder> {
    let records = rows
        .module_orders
        .iter()
        .filter(|row| row.image_id == image_id)
        .collect::<Vec<_>>();
    let raw = records.first()?;
    if records.len() > 1 {
        finding(
            findings,
            FindingCode::MultipleModuleOrderRows,
            Severity::Blocking,
            raw.source_row,
            "multiple module_order rows provide competing order evidence",
        );
    }
    let Some(raw_version) = raw.version else {
        finding(
            findings,
            FindingCode::InvalidModuleOrderVersion,
            Severity::Blocking,
            raw.source_row,
            "module_order.version is NULL",
        );
        return None;
    };
    let version = ModuleOrderVersion::decode(raw_version);
    if matches!(version, ModuleOrderVersion::Unknown(_)) {
        finding(
            findings,
            FindingCode::InvalidModuleOrderVersion,
            Severity::Blocking,
            raw.source_row,
            "module_order.version is not a supported Darktable order kind",
        );
    }
    let raw_list = OpaquePayload::from_optional(raw.operation_list.as_deref());
    let entries = if raw.operation_list.is_some() {
        parse_order_list(
            raw.operation_list.as_deref().unwrap_or_default(),
            limits.max_module_order_entries,
            raw.source_row,
            findings,
            manifest,
        )
    } else {
        Vec::new()
    };
    if matches!(version, ModuleOrderVersion::Custom) && raw.operation_list.is_none() {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            raw.source_row,
            "custom module order has no serialized operation list",
        );
    }
    Some(CompatModuleOrder {
        source: SourceRowKey::new("main.module_order", raw.source_row),
        version,
        raw_list,
        entries,
        rules: DARKTABLE_ORDER_RULES
            .iter()
            .map(|(before, after)| ModuleOrderRule {
                before: before.as_bytes().to_vec(),
                after: after.as_bytes().to_vec(),
            })
            .collect(),
    })
}

fn parse_order_list(
    raw: &[u8],
    max_entries: usize,
    source_row: u64,
    findings: &mut Vec<Finding>,
    manifest: &DarktableOperationManifest,
) -> Vec<ModuleOrderEntry> {
    let Ok(text) = std::str::from_utf8(raw) else {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            source_row,
            "module_order.iop_list is not valid UTF-8",
        );
        return Vec::new();
    };
    let tokens = text.split(',').collect::<Vec<_>>();
    if tokens.len() % 2 != 0 || tokens.iter().any(|token| token.is_empty()) {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            source_row,
            "module_order.iop_list is not operation,instance pairs",
        );
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for (ordinal, pair) in tokens.chunks(2).enumerate() {
        if ordinal >= max_entries {
            finding(
                findings,
                FindingCode::HistoryOrderLimit,
                Severity::Blocking,
                source_row,
                "module_order entry limit truncated the order projection",
            );
            break;
        }
        let operation = pair[0].as_bytes().to_vec();
        let Ok(instance) = pair[1].parse::<i64>() else {
            finding(
                findings,
                FindingCode::InvalidModuleOrderList,
                Severity::Blocking,
                source_row,
                "module_order instance is not an integer",
            );
            continue;
        };
        if !seen.insert((operation.clone(), instance)) {
            finding(
                findings,
                FindingCode::DuplicateModuleOrderEntry,
                Severity::Blocking,
                source_row,
                "module_order repeats an operation instance",
            );
        }
        if manifest.get(pair[0]).is_none() {
            finding(
                findings,
                FindingCode::UnknownModuleOrderOperation,
                Severity::Blocking,
                source_row,
                "module_order contains an operation absent from the manifest",
            );
        }
        entries.push(ModuleOrderEntry {
            ordinal,
            operation,
            instance,
        });
    }
    entries
}

pub(super) fn order_instances(
    instances: &[CompatModuleInstance],
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    manifest: &DarktableOperationManifest,
    findings: &mut Vec<Finding>,
) -> (Vec<ModuleInstanceId>, Option<HistoryOrderSource>, bool) {
    let Some(order) = module_order else {
        // Native selects a built-in default here from display-referred state and
        // the image flags before loading history. Those inputs are not part of
        // `HistoryRows`, so the physical history-number sequence is retention
        // order only and cannot prove executable module order.
        let mut ordered = instances.to_vec();
        ordered.sort_by_key(|instance| first_step_number(instance.id, steps));
        return (
            ordered.into_iter().map(|instance| instance.id).collect(),
            None,
            false,
        );
    };
    if !order.entries.is_empty() {
        let mut result = Vec::new();
        for entry in &order.entries {
            if let Some(instance) = instances.iter().find(|instance| {
                instance.operation.raw_name == entry.operation
                    && instance.multi_priority.unwrap_or(0) == entry.instance
            }) && !result.contains(&instance.id)
            {
                result.push(instance.id);
            }
        }
        for instance in instances {
            if !result.contains(&instance.id) {
                finding(
                    findings,
                    FindingCode::MissingModuleOrderEntry,
                    Severity::Blocking,
                    instance.first_source.row(),
                    "history module instance is absent from serialized module_order",
                );
            }
        }
        check_order_rules(
            &result,
            instances,
            &order.rules,
            findings,
            order.source.row(),
        );
        return (
            result,
            Some(if matches!(order.version, ModuleOrderVersion::Custom) {
                HistoryOrderSource::CustomModuleOrder
            } else {
                HistoryOrderSource::BuiltInModuleOrder
            }),
            !findings
                .iter()
                .any(|finding| finding.code == FindingCode::MissingModuleOrderEntry),
        );
    }
    let (result, proven) = built_in_order(instances, manifest, order.version);
    if !proven {
        finding(
            findings,
            FindingCode::ModuleOrderConflict,
            Severity::Blocking,
            order.source.row(),
            "built-in module_order version has no complete source-derived order proof",
        );
    }
    check_order_rules(
        &result,
        instances,
        &order.rules,
        findings,
        order.source.row(),
    );
    (result, Some(HistoryOrderSource::BuiltInModuleOrder), proven)
}

fn built_in_order(
    instances: &[CompatModuleInstance],
    manifest: &DarktableOperationManifest,
    version: ModuleOrderVersion,
) -> (Vec<ModuleInstanceId>, bool) {
    let native_order = match version {
        ModuleOrderVersion::Legacy => Some(LEGACY_BUILT_IN_ORDER),
        ModuleOrderVersion::V30 => Some(V30_BUILT_IN_ORDER),
        ModuleOrderVersion::V30Jpeg => Some(V30_JPEG_BUILT_IN_ORDER),
        ModuleOrderVersion::V50 => Some(V50_BUILT_IN_ORDER),
        ModuleOrderVersion::V50Jpeg => Some(V50_JPEG_BUILT_IN_ORDER),
        ModuleOrderVersion::Custom | ModuleOrderVersion::Unknown(_) => None,
    };
    let mut ordered = instances.to_vec();
    ordered.sort_by_key(|instance| {
        let name = instance.operation.name.as_deref().unwrap_or_default();
        (
            native_order
                .and_then(|order| order.iter().position(|candidate| *candidate == name))
                .unwrap_or(usize::MAX),
            name.to_owned(),
            instance.multi_priority.unwrap_or(i64::MAX),
            instance.first_source.row(),
        )
    });
    let mut seen_operations = BTreeSet::new();
    let proven = native_order.is_some_and(|order| {
        ordered.iter().all(|instance| {
            instance.multi_priority.unwrap_or(0) == 0
                && instance.operation.name.as_deref().is_some_and(|name| {
                    manifest.get(name).is_some()
                        && order.contains(&name)
                        && seen_operations.insert(name)
                })
        })
    });
    (
        ordered.into_iter().map(|instance| instance.id).collect(),
        proven,
    )
}

fn check_order_rules(
    order: &[ModuleInstanceId],
    instances: &[CompatModuleInstance],
    rules: &[ModuleOrderRule],
    findings: &mut Vec<Finding>,
    source_row: u64,
) {
    for rule in rules {
        let before = order.iter().position(|id| {
            instances
                .iter()
                .find(|instance| instance.id == *id)
                .is_some_and(|instance| instance.operation.raw_name == rule.before)
        });
        let after = order.iter().position(|id| {
            instances
                .iter()
                .find(|instance| instance.id == *id)
                .is_some_and(|instance| instance.operation.raw_name == rule.after)
        });
        if let (Some(before), Some(after)) = (before, after)
            && before >= after
        {
            finding(
                findings,
                FindingCode::ModuleOrderConflict,
                Severity::Blocking,
                source_row,
                "module_order violates a persisted Darktable operation-order rule",
            );
        }
    }
}

fn first_step_number(id: ModuleInstanceId, steps: &[CompatHistoryStep]) -> i64 {
    steps
        .iter()
        .filter(|step| step.instance_id == id)
        .map(|step| step.num)
        .min()
        .unwrap_or(i64::MAX)
}

pub(super) fn decode_hash(
    image_id: i64,
    rows: &HistoryRows,
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    history_end: Option<i64>,
    findings: &mut Vec<Finding>,
) -> Option<CompatHistoryHash> {
    let records = rows
        .hashes
        .iter()
        .filter(|row| row.image_id == image_id)
        .collect::<Vec<_>>();
    let raw = records.first()?;
    if records.len() > 1 {
        finding(
            findings,
            FindingCode::MultipleHistoryHashRows,
            Severity::Blocking,
            raw.source_row,
            "multiple history_hash rows provide competing hash state",
        );
    }
    let hash = CompatHistoryHash {
        source: SourceRowKey::new("main.history_hash", raw.source_row),
        basic: OpaquePayload::from_optional(raw.basic_hash.as_deref()),
        auto: OpaquePayload::from_optional(raw.auto_hash.as_deref()),
        current: OpaquePayload::from_optional(raw.current_hash.as_deref()),
        mipmap: OpaquePayload::from_optional(raw.mipmap_hash.as_deref()),
        current_matches: None,
    };
    let expected = darktable_hash(steps, module_order, history_end);
    let current_matches = raw
        .current_hash
        .as_deref()
        .map(|current| current == expected.as_slice());
    if current_matches == Some(false) {
        finding(
            findings,
            FindingCode::HistoryHashMismatch,
            Severity::Warning,
            raw.source_row,
            "history_hash.current_hash disagrees with decoded selected history evidence",
        );
    }
    Some(CompatHistoryHash {
        current_matches,
        ..hash
    })
}

fn darktable_hash(
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    history_end: Option<i64>,
) -> [u8; 16] {
    let mut digest = md5::Context::new();
    let endpoint = history_end.unwrap_or(i64::MAX);
    let mut max_num_by_key = BTreeMap::<(Vec<u8>, i64), i64>::new();
    for step in steps.iter().filter(|step| step.num <= endpoint) {
        max_num_by_key
            .entry((
                step.operation.raw_name.clone(),
                step.multi_priority.unwrap_or(0),
            ))
            .and_modify(|max_num| *max_num = (*max_num).max(step.num))
            .or_insert(step.num);
    }
    let mut selected = steps
        .iter()
        .filter(|step| {
            step.num <= endpoint
                && matches!(step.enabled, EnabledState::Enabled)
                && max_num_by_key
                    .get(&(
                        step.operation.raw_name.clone(),
                        step.multi_priority.unwrap_or(0),
                    ))
                    .is_some_and(|max_num| *max_num == step.num)
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|step| step.num);
    for step in selected {
        digest.consume(&step.operation.raw_name);
        digest.consume(&step.operation_params.bytes);
        digest.consume(&step.blend_params.bytes);
    }
    if let Some(order) = module_order {
        let version = match order.version {
            ModuleOrderVersion::Custom => 0_i32,
            ModuleOrderVersion::Legacy => 1,
            ModuleOrderVersion::V30 => 2,
            ModuleOrderVersion::V30Jpeg => 3,
            ModuleOrderVersion::V50 => 4,
            ModuleOrderVersion::V50Jpeg => 5,
            ModuleOrderVersion::Unknown(value) => i32::try_from(value).unwrap_or_else(|_| {
                if value.is_negative() {
                    i32::MIN
                } else {
                    i32::MAX
                }
            }),
        };
        digest.consume(version.to_ne_bytes());
        if matches!(order.version, ModuleOrderVersion::Custom) {
            digest.consume(&order.raw_list.bytes);
        }
    }
    digest.finalize().0
}
