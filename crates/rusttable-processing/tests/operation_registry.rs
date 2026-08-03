use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::descriptor::OperationFlags;
use rusttable_processing::{
    CpuExecutionRoute, DefinitionAvailability, EvaluationError, FactoryError, FiniteF32, LinearRgb,
    OperationDefinition, OperationUiAvailability, PipelineStepIndex, ProcessingOperationKind,
    RasterDimensions, RegistryClosure, RegistryValidationError, builtin_registry,
};

#[test]
fn operation_ui_availability_distinguishes_full_partial_and_unavailable() {
    let available = OperationUiAvailability::Available;
    let partial = OperationUiAvailability::PartiallyAvailable {
        reason: "custom editor only".to_owned(),
        deferred_responsibilities: vec!["operation.ui.deferred".to_owned()],
    };
    let unavailable = OperationUiAvailability::Unavailable {
        reason: "UI not implemented".to_owned(),
    };

    assert!(available.is_available());
    assert!(available.is_usable());
    assert!(!available.is_partial());
    assert_eq!(available.reason(), None);
    assert!(available.deferred_responsibilities().is_empty());

    assert!(!partial.is_available());
    assert!(partial.is_usable());
    assert!(partial.is_partial());
    assert_eq!(partial.reason(), Some("custom editor only"));
    assert_eq!(
        partial.deferred_responsibilities(),
        &["operation.ui.deferred".to_owned()]
    );

    assert!(!unavailable.is_available());
    assert!(!unavailable.is_usable());
    assert!(!unavailable.is_partial());
    assert_eq!(unavailable.reason(), Some("UI not implemented"));
    assert!(unavailable.deferred_responsibilities().is_empty());
}

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new(key).expect("operation key"),
        true,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("operation")
}

#[test]
fn tonecurve_materializes_but_routes_execution_to_lab_pixelpipe() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.tonecurve")
        .expect("Tone Curve definition");
    assert!(definition.availability().is_available());
    assert_eq!(
        definition.cpu_execution_route(),
        Some(CpuExecutionRoute::LabD50Pixelpipe)
    );
    assert!(
        !definition
            .cpu()
            .expect("Tone Curve CPU factory")
            .execution_route()
            .generic_executor_available()
    );

    let capability = registry
        .capability(
            "rusttable.tonecurve",
            &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
            rusttable_color::ColorEncoding::LabD50,
            Some("preview"),
        )
        .expect("Tone Curve capability");
    assert!(capability.available);
    assert_eq!(capability.cpu_route, definition.cpu_execution_route());

    let operation = registry
        .materialize_operation(
            "rusttable.tonecurve",
            OperationId::new(52).expect("operation ID"),
        )
        .expect("Tone Curve materialization remains available");
    let prepared = registry
        .prepare_cpu(&operation)
        .expect("Tone Curve preparation");
    assert_eq!(
        prepared.execution_route(),
        CpuExecutionRoute::LabD50Pixelpipe
    );
    let finite = FiniteF32::new(0.25).expect("finite pixel");
    let mut pixels = [LinearRgb::new(finite, finite, finite)];
    let error = prepared
        .execute(
            PipelineStepIndex::new(0),
            &mut pixels,
            RasterDimensions::new(1, 1).expect("dimensions"),
            0,
        )
        .expect_err("generic evaluation must not approximate the Lab route");
    assert!(error.to_string().contains("Lab D50 pixelpipe route"));
}

#[test]
fn highpass_materializes_but_routes_execution_to_lab_pixelpipe() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.highpass")
        .expect("Highpass definition");
    assert!(definition.availability().is_available());
    assert_eq!(
        definition.descriptor().roi,
        rusttable_processing::descriptor::RoiKind::Neighborhood
    );
    assert_eq!(
        definition.cpu_execution_route(),
        Some(CpuExecutionRoute::LabD50Pixelpipe)
    );
    assert!(
        !definition
            .cpu()
            .expect("Highpass CPU factory")
            .execution_route()
            .generic_executor_available()
    );
    assert!(
        definition
            .evidence_ids()
            .contains(&"iop.highpass.cpu.lab-d50-box-filter".to_owned())
    );
    assert!(definition.gpu().is_none());
    assert_eq!(
        definition.ui_availability(),
        &OperationUiAvailability::PartiallyAvailable {
            reason: "the generic two-slider editor is usable, but native shared blend/mask/outer-blend controls and their action persistence remain deferred"
                .to_owned(),
            deferred_responsibilities: [
                "iop.highpass.ui.shared-blend-mask-controls",
                "iop.highpass.ui.outer-blend-controls",
                "iop.highpass.persistence.native-shared-blend-mask-and-outer-blend",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    );
    assert!(!definition.ui_availability().is_available());
    assert!(definition.ui_availability().is_usable());
    assert!(definition.ui_availability().is_partial());
    assert_eq!(
        definition.descriptor().mask_blend,
        rusttable_processing::descriptor::MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        }
    );

    let capability = registry
        .capability(
            "rusttable.highpass",
            &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
            rusttable_color::ColorEncoding::LabD50,
            Some("preview"),
        )
        .expect("Highpass capability");
    assert!(capability.available);
    assert_eq!(capability.cpu_route, definition.cpu_execution_route());

    let operation = registry
        .materialize_operation(
            "rusttable.highpass",
            OperationId::new(53).expect("operation ID"),
        )
        .expect("Highpass materialization remains available");
    let prepared = registry
        .prepare_cpu(&operation)
        .expect("Highpass preparation");
    assert_eq!(
        prepared.execution_route(),
        CpuExecutionRoute::LabD50Pixelpipe
    );
    let finite = FiniteF32::new(0.25).expect("finite pixel");
    let mut pixels = [LinearRgb::new(finite, finite, finite)];
    let error = prepared
        .execute(
            PipelineStepIndex::new(0),
            &mut pixels,
            RasterDimensions::new(1, 1).expect("dimensions"),
            0,
        )
        .expect_err("generic evaluation must not approximate the Lab route");
    assert_eq!(
        error,
        EvaluationError::OperationExecution {
            step_index: PipelineStepIndex::new(0),
            operation_id: operation.id(),
            reason: "operation requires the Lab D50 pixelpipe route".to_owned(),
        }
    );
}

#[test]
fn larger_tonal_set_registers_typed_cpu_factories_with_honest_capabilities() {
    let registry = builtin_registry();
    let colortransfer = registry
        .definition("rusttable.colortransfer")
        .expect("Color Transfer definition");
    assert!(
        colortransfer
            .descriptor()
            .flags
            .contains(OperationFlags::FULL_IMAGE)
    );
    assert!(
        colortransfer
            .descriptor()
            .flags
            .contains(OperationFlags::ANALYSIS)
    );
    assert!(colortransfer.gpu().is_none());
    assert!(!colortransfer.ui_availability().is_usable());

    let colormapping = registry
        .definition("rusttable.colormapping")
        .expect("Color Mapping definition");
    assert!(
        colormapping
            .descriptor()
            .flags
            .contains(OperationFlags::TILEABLE)
    );
    assert!(!colormapping.descriptor().mask_blend.consumes_mask);
    assert!(colormapping.gpu().is_none());
    assert!(!colormapping.ui_availability().is_usable());

    let transfer = registry
        .prepare_cpu(&operation(50, "rusttable.colortransfer", &[]))
        .expect("default Color Transfer factory");
    assert!(matches!(
        transfer.operation().kind(),
        ProcessingOperationKind::ColorTransfer { .. }
    ));
    let mapping = registry
        .prepare_cpu(&operation(51, "rusttable.colormapping", &[]))
        .expect("default Color Mapping factory");
    assert!(matches!(
        mapping.operation().kind(),
        ProcessingOperationKind::ColorMapping { .. }
    ));
}

fn missing_cpu_definition() -> OperationDefinition {
    let builtin = &builtin_registry().definitions()[0];
    OperationDefinition::new(
        builtin.descriptor().clone(),
        None,
        builtin.gpu().cloned(),
        builtin.migrations().to_vec(),
        builtin.identity().clone(),
        builtin.evidence_ids().to_vec(),
    )
}

fn migration_gap_definition() -> OperationDefinition {
    let builtin = &builtin_registry().definitions()[0];
    let mut descriptor = builtin.descriptor().clone();
    descriptor.migration.source_versions = vec![1, 2];
    descriptor.migration.target_version = 2;
    OperationDefinition::new(
        descriptor,
        builtin.cpu(),
        builtin.gpu().cloned(),
        Vec::new(),
        builtin.identity().clone(),
        builtin.evidence_ids().to_vec(),
    )
}

#[test]
fn operation_registry_executes_all_first_party_operations_through_factories() {
    let registry = builtin_registry();
    let cases = [
        operation(1, "rusttable.exposure", &[("stops", 0.5)]),
        operation(2, "rusttable.linear_offset", &[("value", 0.25)]),
        operation(
            3,
            "rusttable.rgb_gain",
            &[("red", 1.0), ("green", 0.75), ("blue", 0.5)],
        ),
        operation(
            4,
            "rusttable.bloom",
            &[("size", 0.0), ("threshold", 0.0), ("strength", 25.0)],
        ),
        operation(
            5,
            "rusttable.soften",
            &[
                ("size", 0.0),
                ("saturation", 100.0),
                ("brightness", 0.33),
                ("amount", 50.0),
            ],
        ),
    ];

    let prepared = cases
        .iter()
        .map(|operation| registry.prepare_cpu(operation).expect("factory"))
        .collect::<Vec<_>>();
    assert!(matches!(
        prepared[0].operation().kind(),
        ProcessingOperationKind::Exposure { .. }
    ));
    assert!(matches!(
        prepared[1].operation().kind(),
        ProcessingOperationKind::LinearOffset { .. }
    ));
    assert!(matches!(
        prepared[2].operation().kind(),
        ProcessingOperationKind::RgbGain { .. }
    ));
    assert!(matches!(
        prepared[3].operation().kind(),
        ProcessingOperationKind::Bloom { .. }
    ));
    assert!(matches!(
        prepared[4].operation().kind(),
        ProcessingOperationKind::Soften { .. }
    ));
}

#[test]
fn colorreconstruct_registry_keeps_deferred_capabilities_unavailable() {
    let definition = builtin_registry()
        .definition("rusttable.colorreconstruct")
        .expect("Color Reconstruction registry seam");
    let descriptor = definition.descriptor();
    assert_eq!(descriptor.id.compatibility_name, "colorreconstruct");
    assert!(definition.availability().is_available());
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_some());
    assert!(!descriptor.flags.contains(OperationFlags::BLENDING));
    assert!(!descriptor.flags.contains(OperationFlags::MASKS));
    assert_eq!(
        descriptor.mask_blend,
        rusttable_processing::descriptor::MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: true,
        }
    );

    assert_eq!(
        definition.ui_availability(),
        &OperationUiAvailability::PartiallyAvailable {
            reason: "the Color Reconstruction parameter editor is usable, but native UI adjuncts remain deferred"
                .to_owned(),
            deferred_responsibilities: [
                "iop.colorreconstruct.ui.shared-blending-and-drawn-masks",
                "iop.colorreconstruct.ui.monochrome-applicability",
                "iop.colorreconstruct.ui.preview-grid-lifecycle",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    );
    assert!(!definition.ui_availability().is_available());
    assert!(definition.ui_availability().is_usable());
    assert!(definition.ui_availability().is_partial());
}

#[test]
fn crop_keeps_cpu_execution_but_fails_closed_for_ui_context() {
    let definition = builtin_registry()
        .definition("rusttable.crop")
        .expect("Crop registry seam");
    assert!(definition.availability().is_available());
    assert!(definition.cpu().is_some());
    assert_eq!(
        definition.ui_availability(),
        &OperationUiAvailability::Unavailable {
            reason: "Crop editing requires transformed crop-stage preview context".to_owned(),
        }
    );
    assert!(!definition.ui_availability().is_available());
    assert!(!definition.ui_availability().is_usable());
    assert_eq!(
        definition.ui_availability().reason(),
        Some("Crop editing requires transformed crop-stage preview context")
    );
}

#[test]
fn enlargecanvas_keeps_cpu_geometry_without_gpu_or_generic_ui_claims() {
    let definition = builtin_registry()
        .definition("rusttable.enlargecanvas")
        .expect("Enlarge Canvas registry seam");
    let descriptor = definition.descriptor();

    assert!(definition.availability().is_available());
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
    assert!(descriptor.capability.fallback_to_cpu);
    assert!(descriptor.capability.gpu_tier.is_none());
    assert!(!descriptor.capability.deterministic_gpu);
    assert!(descriptor.capability.required_features.is_empty());
    assert!(descriptor.capability.required_formats.is_empty());
    assert!(!descriptor.flags.contains(OperationFlags::DETERMINISTIC_GPU));
    assert!(descriptor.ui.is_none());
    let closure = RegistryClosure::from_registry(builtin_registry()).expect("registry closure");
    let capability = closure
        .entries
        .iter()
        .find(|entry| entry.identity == "rusttable.enlargecanvas")
        .expect("Enlarge Canvas capability");
    assert!(capability.cpu_supported);
    assert!(!capability.gpu_supported);
    assert!(capability.cpu_fallback);
    assert_eq!(
        definition.ui_availability(),
        &OperationUiAvailability::Unavailable {
            reason: "Enlarge Canvas color-picker and source-shaped GTK interactions are not ported"
                .to_owned(),
        }
    );
}

#[test]
fn operation_registry_keeps_unknown_imported_identity_opaque() {
    let error = builtin_registry()
        .prepare_cpu(&operation(7, "rusttable.unknown", &[]))
        .expect_err("unknown operation must not be constructed");
    assert!(matches!(
        error,
        rusttable_processing::RegistryLookupError::UnknownOperation(_)
    ));
}

#[test]
fn operation_registry_rejects_definition_without_cpu_factory() {
    let error = rusttable_processing::RegistrySnapshot::try_new(&[missing_cpu_definition])
        .expect_err("missing CPU must be rejected");
    assert!(
        error
            .findings()
            .iter()
            .any(|finding| matches!(finding, RegistryValidationError::MissingCpu(_)))
    );
}

#[test]
fn operation_registry_rejects_migration_gap() {
    let error = rusttable_processing::RegistrySnapshot::try_new(&[migration_gap_definition])
        .expect_err("migration gap must be rejected");
    assert!(
        error
            .findings()
            .iter()
            .any(|finding| matches!(finding, RegistryValidationError::MigrationGap(_)))
    );
}

#[test]
fn operation_registry_reports_factory_errors_with_operation_context() {
    let error = builtin_registry()
        .prepare_cpu(&operation(8, "rusttable.exposure", &[]))
        .expect_err("missing parameter");
    assert!(matches!(
        error,
        rusttable_processing::RegistryLookupError::Factory {
            source,
            ..
        } if matches!(source.as_ref(), FactoryError::Operation(_))
    ));
}

#[test]
fn operation_registry_preserves_darktable_declaration_order_for_ui_projections() {
    let ids = builtin_registry()
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), builtin_registry().definitions().len());
    assert_eq!(
        &ids[..9],
        [
            "exposure",
            "basicadj",
            "linear-offset",
            "rgbgain",
            "invert",
            "defringe",
            "highpass",
            "sharpen",
            "clahe"
        ]
    );
    assert_eq!(ids.last(), Some(&"colorout"));
}

#[test]
fn sharpen_registry_is_cpu_only_lab_neighborhood_and_ui_unavailable() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.sharpen")
        .expect("Sharpen registry seam");
    let descriptor = definition.descriptor();
    assert_eq!(descriptor.id.compatibility_name, "sharpen");
    assert_eq!(descriptor.id.schema_version, 1);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        ["radius", "amount", "threshold"]
    );
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::Neighborhood
    );
    assert_eq!(descriptor.tiling.overlap_pixels, 0);
    assert_eq!(
        descriptor.io.input.encodings,
        [rusttable_color::ColorEncoding::LabD50]
    );
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
    assert!(!definition.availability().is_available());
    assert!(!definition.ui_availability().is_usable());
    assert!(
        definition
            .evidence_ids()
            .contains(&"iop.sharpen.cpu.dynamic-neighborhood-overlap".to_owned())
    );
    assert!(
        registry
            .capability(
                "rusttable.sharpen",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LabD50,
                Some("full"),
            )
            .is_some_and(|capability| !capability.available)
    );
    assert!(
        registry
            .capability(
                "rusttable.sharpen",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("full"),
            )
            .is_some_and(|capability| !capability.available)
    );
}

#[test]
fn censorize_is_registry_visible_and_cpu_qualified() {
    let definition = builtin_registry()
        .definition("rusttable.censorize")
        .expect("censorize registry seam");
    assert_eq!(definition.descriptor().parameters.len(), 4);
    assert!(definition.availability().is_available());
    assert!(
        builtin_registry()
            .capability(
                "rusttable.censorize",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("preview"),
            )
            .is_some_and(|capability| capability.available)
    );
}

#[test]
fn clahe_registry_is_descriptor_visible_and_cpu_qualified() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.clahe")
        .expect("CLAHE registry seam");
    let descriptor = definition.descriptor();
    assert_eq!(descriptor.id.compatibility_name, "clahe");
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::DEPRECATED)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::HIDDEN)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::STYLE_ELIGIBLE)
    );
    assert!(definition.cpu().is_some());
    assert!(definition.availability().is_available());
    let radius = descriptor
        .parameters
        .iter()
        .find(|parameter| parameter.id == "radius")
        .expect("radius descriptor");
    assert_eq!(
        radius.kind,
        rusttable_processing::descriptor::ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 256.0,
        }
    );
    assert_eq!(
        radius.default,
        rusttable_processing::descriptor::ParameterDefault::Scalar(64.0)
    );
    assert!(
        registry
            .capability(
                "rusttable.clahe",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("full"),
            )
            .is_some_and(|capability| capability.available)
    );
}

#[test]
fn liquify_registry_exposes_geometry_and_explicit_cpu_fallback() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.liquify")
        .expect("liquify registry seam");
    assert_eq!(definition.descriptor().id.compatibility_name, "liquify");
    assert!(
        definition
            .descriptor()
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::GEOMETRY)
    );
    assert!(
        definition
            .descriptor()
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MASKS)
    );
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
    assert!(
        registry
            .capability(
                "rusttable.liquify",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("preview"),
            )
            .is_some_and(|capability| capability.available)
    );
}

#[test]
fn colorzones_registry_exposes_dedicated_gpu_fallback_and_source_order() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.colorzones")
        .expect("Color Zones registry seam");
    assert_eq!(definition.descriptor().id.compatibility_name, "colorzones");
    assert_eq!(definition.descriptor().id.schema_version, 5);
    assert_eq!(
        definition.availability(),
        &DefinitionAvailability::Available
    );
    let deferred_responsibilities = [
        "iop.colorzones.ui.picker-lifecycle",
        "iop.colorzones.ui.operation-local-histogram",
        "iop.colorzones.ui.display-selection",
        "iop.colorzones.ui.presets",
        "iop.colorzones.ui.global-shortcuts-hold-mode",
        "iop.colorzones.ui.durable-gui-preferences",
        "iop.colorzones.ui.pending-import-materialization",
    ];
    assert_eq!(
        definition.ui_availability(),
        &OperationUiAvailability::PartiallyAvailable {
            reason: "the Color Zones custom editor is usable, but native UI responsibilities remain deferred"
                .to_owned(),
            deferred_responsibilities: deferred_responsibilities
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    );
    assert!(!definition.ui_availability().is_available());
    assert!(definition.ui_availability().is_usable());
    assert!(definition.ui_availability().is_partial());
    assert!(definition.cpu().is_some());
    let gpu = definition.gpu().expect("dedicated Color Zones GPU binding");
    assert_eq!(
        gpu.binding_id(),
        rusttable_processing::COLORZONES_WGPU_PASS_ID
    );
    assert_eq!(gpu.tier(), rusttable_processing::COLORZONES_GPU_TIER);
    assert!(definition.descriptor().capability.fallback_to_cpu);
    assert_eq!(
        definition
            .migrations()
            .iter()
            .map(|migration| (migration.from_version(), migration.to_version()))
            .collect::<Vec<_>>(),
        [(1, 5), (2, 5), (3, 5), (4, 5)]
    );

    let order = registry
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let colorzones = order
        .iter()
        .position(|name| *name == "colorzones")
        .expect("Color Zones order entry");
    assert_eq!(order[colorzones - 1], "vibrance");
    assert!(order[colorzones + 1..].contains(&"bloom"));
}
