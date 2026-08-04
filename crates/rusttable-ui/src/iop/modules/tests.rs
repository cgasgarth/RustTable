use super::*;
use crate::presentation::darkroom_controls::DarkroomControlKind;
use rusttable_core::OperationId;

fn module(id: &str, side: DarkroomModuleSide) -> DarkroomModuleViewModel {
    let slider = DarkroomControlViewModel::slider(
        format!("{id}-amount"),
        "Amount",
        0.0,
        1.0,
        0.01,
        0.5,
        0.0,
    )
    .expect("valid slider");
    DarkroomModuleViewModel::new(
        id,
        id,
        side,
        true,
        true,
        true,
        Revision::from_u64(7),
        vec![slider],
    )
    .expect("valid module")
}

#[test]
fn colorreconstruct_focus_order_includes_every_native_control() {
    let operation_id = OperationId::new(37).expect("operation id");
    let template = reference_modules()
        .expect("source-derived reference modules")
        .module("colorreconstruct")
        .expect("Color Reconstruction template")
        .clone();
    let state = crate::iop::colorreconstruct::ColorReconstructionGtkState::new(
        operation_id,
        Revision::from_u64(7),
        crate::iop::colorreconstruct::ColorReconstructionEditorState::default(),
        false,
        true,
        true,
    );
    let module = template.with_colorreconstruct_editor_state(state);

    assert_eq!(
        module.focus_order(),
        [
            "colorreconstruct-disclosure",
            "colorreconstruct-enabled",
            "colorreconstruct-reset",
            "colorreconstruct-threshold",
            "colorreconstruct-spatial",
            "colorreconstruct-range",
            "colorreconstruct-precedence",
            "colorreconstruct-hue",
        ]
    );
}

#[test]
fn soften_projection_mounts_source_controls_without_generic_fallback() {
    let operation_id = OperationId::new(53).expect("operation id");
    let template = reference_modules()
        .expect("source-derived reference modules")
        .module("soften")
        .expect("Soften template")
        .clone();
    let state = crate::iop::soften::SoftenGtkState::new(
        operation_id,
        Revision::from_u64(11),
        crate::iop::soften::SoftenEditorState::default(),
        false,
        true,
        false,
    );
    let module = template
        .with_operation_instance(operation_id, 0, 1)
        .with_soften_editor_state(state);

    assert!(module.has_soften_custom_editor());
    assert_eq!(module.soften_editor_state(), Some(&state));
    assert_eq!(module.controls().controls().count(), 0);
    assert_eq!(
        module.focus_order(),
        [
            "soften-disclosure",
            "soften-enabled",
            "soften-reset",
            "soften-size",
            "soften-saturation",
            "soften-brightness",
            "soften-amount",
        ]
    );
}

#[test]
fn colorreconstruct_projection_keeps_deferred_surfaces_partial_and_hidden() {
    let modules = reference_modules().expect("source-derived reference modules");
    let template = modules
        .module("colorreconstruct")
        .expect("Color Reconstruction template");
    assert!(template.availability().is_supported());
    assert!(template.availability().is_partial());
    assert_eq!(
        template
            .availability()
            .deferred_responsibilities()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "iop.colorreconstruct.ui.shared-blending-and-drawn-masks",
            "iop.colorreconstruct.ui.monochrome-applicability",
            "iop.colorreconstruct.ui.preview-grid-lifecycle",
        ]
    );
    assert!(template.has_colorreconstruct_custom_editor());
    assert!(template.controls().controls().next().is_none());
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "This source-parity regression keeps the complete persisted-instance contract together."
)]
fn persisted_instances_keep_compatibility_identity_but_require_exact_operation_targets() {
    let first_id = OperationId::new(41).expect("first operation id");
    let second_id = OperationId::new(73).expect("second operation id");
    let templates = reference_modules().expect("source-derived reference modules");
    let template = templates
        .module("colorcontrast")
        .expect("Color Contrast template")
        .clone();
    assert_eq!(
        template.focus_order(),
        [
            "colorcontrast-disclosure",
            "colorcontrast-enabled",
            "colorcontrast-reset",
            "colorcontrast-a-steepness-widget",
            "colorcontrast-b-steepness-widget",
        ],
        "a sole instance retains the established GTK names"
    );
    let first = template.clone().with_operation_instance(first_id, 0, 2);
    let second = template.with_operation_instance(second_id, 1, 2);
    let mut modules =
        DarkroomModulesViewModel::new(vec![first, second]).expect("distinct operation instances");

    assert!(
        modules.module("colorcontrast").is_none(),
        "compatibility-only lookup must not select an arbitrary instance"
    );
    assert_eq!(
        modules
            .instances("colorcontrast")
            .map(DarkroomModuleViewModel::operation_id)
            .collect::<Vec<_>>(),
        [Some(first_id), Some(second_id)]
    );
    let first_widget_id = modules
        .module_target("colorcontrast", Some(first_id))
        .expect("first exact target")
        .widget_id();
    let second_widget_id = modules
        .module_target("colorcontrast", Some(second_id))
        .expect("second exact target")
        .widget_id();
    assert_eq!(
        first_widget_id,
        format!("colorcontrast-instance-{first_id}")
    );
    assert_eq!(
        second_widget_id,
        format!("colorcontrast-instance-{second_id}")
    );
    assert_eq!(
        modules
            .module_target("colorcontrast", Some(first_id))
            .expect("first exact target")
            .focus_order(),
        [
            format!("{first_widget_id}-disclosure"),
            format!("{first_widget_id}-enabled"),
            format!("{first_widget_id}-actions"),
            format!("{first_widget_id}-reset"),
            format!("{first_widget_id}-a-steepness-widget"),
            format!("{first_widget_id}-b-steepness-widget"),
        ]
    );
    assert_eq!(
        modules
            .module_target("colorcontrast", Some(second_id))
            .expect("second exact target")
            .focus_order(),
        [
            format!("{second_widget_id}-disclosure"),
            format!("{second_widget_id}-enabled"),
            format!("{second_widget_id}-actions"),
            format!("{second_widget_id}-reset"),
            format!("{second_widget_id}-a-steepness-widget"),
            format!("{second_widget_id}-b-steepness-widget"),
        ]
    );
    for operation_id in [first_id, second_id] {
        assert_eq!(
            modules
                .module_target("colorcontrast", Some(operation_id))
                .expect("exact operation target")
                .controls()
                .controls()
                .map(|control| control.id().as_str())
                .collect::<Vec<_>>(),
            ["colorcontrast-a-steepness", "colorcontrast-b-steepness"],
            "GTK identity must not leak into persisted control mapping"
        );
    }

    let second = modules
        .module_target_mut("colorcontrast", Some(second_id))
        .expect("second exact target");
    let error = second
        .apply(DarkroomModuleAction::Enable {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
            enabled: false,
        })
        .expect_err("bound panels reject compatibility-only actions");
    assert!(matches!(error, DarkroomModuleError::WrongOperation { .. }));

    let error = second
        .apply(DarkroomModuleAction::Enable {
            module_id: "colorcontrast".to_owned(),
            operation_id: Some(first_id),
            expected_revision: Revision::ZERO,
            enabled: false,
        })
        .expect_err("forged operation target");
    assert!(matches!(error, DarkroomModuleError::WrongOperation { .. }));
}

#[test]
fn multi_instance_model_actions_enforce_exact_source_boundaries() {
    let first_id = OperationId::new(811).expect("first Vibrance operation id");
    let second_id = OperationId::new(812).expect("second Vibrance operation id");
    let template = reference_modules()
        .expect("source-derived reference modules")
        .module("vibrance")
        .expect("Vibrance template")
        .clone();
    let mut first = template.clone().with_operation_instance(first_id, 0, 2);
    let mut second = template.clone().with_operation_instance(second_id, 1, 2);

    assert!(first.supports_multi_instance());
    assert!(first.can_add_instance());
    assert!(first.can_delete_instance());

    let revision = first
        .apply(DarkroomModuleAction::NewInstance {
            module_id: "vibrance".to_owned(),
            operation_id: Some(first_id),
            expected_revision: Revision::ZERO,
        })
        .expect("new instance advances the processing revision");
    assert_eq!(revision, Revision::from_u64(1));

    for action in [
        DarkroomModuleAction::DuplicateInstance {
            module_id: "vibrance".to_owned(),
            operation_id: Some(second_id),
            expected_revision: Revision::ZERO,
        },
        DarkroomModuleAction::MoveInstanceUp {
            module_id: "vibrance".to_owned(),
            operation_id: Some(second_id),
            expected_revision: Revision::ZERO,
        },
        DarkroomModuleAction::MoveInstanceDown {
            module_id: "vibrance".to_owned(),
            operation_id: Some(second_id),
            expected_revision: Revision::ZERO,
        },
    ] {
        let (expected_action, expected_reason) = match &action {
            DarkroomModuleAction::DuplicateInstance { .. } => (
                "duplicate instance",
                "the current edit model cannot copy native blend and mask state",
            ),
            DarkroomModuleAction::MoveInstanceUp { .. } => (
                "move up",
                "the current edit model cannot apply native adjacent-module ordering",
            ),
            DarkroomModuleAction::MoveInstanceDown { .. } => (
                "move down",
                "the current edit model cannot apply native adjacent-module ordering",
            ),
            _ => unreachable!("test constructs only gated instance actions"),
        };
        let error = second
            .apply(action)
            .expect_err("unfaithful instance action stays gated");
        assert!(matches!(
            error,
            DarkroomModuleError::InstanceActionUnavailable {
                action,
                reason,
                ..
            } if action == expected_action && reason == expected_reason
        ));
        assert_eq!(second.revision(), Revision::ZERO);
    }

    let mut sole = template.with_operation_instance(first_id, 0, 1);
    let error = sole
        .apply(DarkroomModuleAction::DeleteInstance {
            module_id: "vibrance".to_owned(),
            operation_id: Some(first_id),
            expected_revision: Revision::ZERO,
        })
        .expect_err("the final same-key instance cannot be deleted");
    assert!(matches!(
        error,
        DarkroomModuleError::InstanceActionUnavailable {
            action: "delete",
            ..
        }
    ));
}

#[test]
fn stale_module_action_and_control_validation_are_visible() {
    let mut model = module("exposure", DarkroomModuleSide::Right);
    model
        .set_control(
            Revision::from_u64(7),
            "exposure-amount",
            DarkroomControlValue::Slider(0.75),
        )
        .expect("typed control update");
    let error = model
        .set_enabled(Revision::from_u64(7), false)
        .expect_err("stale");
    assert!(matches!(error, DarkroomModuleError::StaleRevision { .. }));
    assert!(matches!(model.status(), DarkroomModuleStatus::Stale { .. }));
}

#[test]
fn action_routing_covers_controls_and_keeps_focus_order_deterministic() {
    let mut model = DarkroomModuleViewModel::new(
        "exposure",
        "Exposure",
        DarkroomModuleSide::Right,
        true,
        true,
        true,
        Revision::from_u64(7),
        vec![
            DarkroomControlViewModel::slider("amount", "Amount", 0.0, 1.0, 0.01, 0.5, 0.0)
                .expect("valid slider"),
            DarkroomControlViewModel::choice("method", "Method", ["balanced", "preserve"], 0)
                .expect("valid choice"),
            DarkroomControlViewModel::toggle("protect", "Protect", false, true)
                .expect("valid toggle"),
        ],
    )
    .expect("valid module");
    let mut revision = Revision::from_u64(7);
    revision = model
        .apply(DarkroomModuleAction::Disclosure {
            module_id: "exposure".to_owned(),
            operation_id: None,
            expected_revision: revision,
            expanded: false,
        })
        .expect("disclosure action");
    revision = model
        .apply(DarkroomModuleAction::Enable {
            module_id: "exposure".to_owned(),
            operation_id: None,
            expected_revision: revision,
            enabled: false,
        })
        .expect("enable action");
    for (id, value) in [
        ("amount", DarkroomControlValue::Slider(0.75)),
        ("method", DarkroomControlValue::Choice(1)),
        ("protect", DarkroomControlValue::Toggle(true)),
    ] {
        revision = model
            .apply(DarkroomModuleAction::Control {
                module_id: "exposure".to_owned(),
                operation_id: None,
                expected_revision: revision,
                id: id.to_owned(),
                value,
            })
            .expect("control action");
    }
    revision = model
        .apply(DarkroomModuleAction::Reset {
            module_id: "exposure".to_owned(),
            operation_id: None,
            expected_revision: revision,
        })
        .expect("reset action");
    assert_eq!(revision, Revision::from_u64(12));
    assert!(
        model.enabled(),
        "native module reset enables the reset module"
    );
    assert_eq!(
        model.focus_order(),
        [
            "exposure-disclosure",
            "exposure-enabled",
            "exposure-reset",
            "amount-widget",
            "method-widget",
            "protect-widget",
        ]
    );
    assert_eq!(
        model.controls().control("amount").expect("amount").value(),
        DarkroomControlValue::Slider(0.0)
    );
    assert_eq!(
        model.controls().control("method").expect("method").value(),
        DarkroomControlValue::Choice(0)
    );
    assert!(matches!(model.status(), DarkroomModuleStatus::Ready));
}

#[test]
fn module_search_matches_title_and_id_without_case_or_whitespace_surprises() {
    let module = module("color-balance", DarkroomModuleSide::Right);

    assert!(module_matches_query(&module, ""));
    assert!(module_matches_query(
        &module,
        "  COLOR  ".trim().to_ascii_lowercase().as_str()
    ));
    assert!(module_matches_query(&module, "balance"));
    assert!(!module_matches_query(&module, "exposure"));
}

#[test]
fn darkroom_search_covers_static_panels_and_has_explicit_empty_behavior() {
    for (query, title, id) in [
        (" exposure ", "Exposure", "exposure"),
        ("rgb ai", "RGB AI denoise", "rgb-denoise"),
        ("raw denoise", "RAW AI denoise", "raw-denoise"),
        ("mask-manager", "Mask manager", "mask-manager"),
        ("retouch", "Multiscale retouch", "multiscale-retouch"),
    ] {
        assert!(
            search_matches(query, title, id, &[]),
            "query {query} should match"
        );
    }
    assert!(search_matches("", "Mask manager", "mask-manager", &[]));
    assert!(!search_matches(
        "does-not-exist",
        "Mask manager",
        "mask-manager",
        &[]
    ));
}

#[test]
fn reference_modules_expose_registry_controls_and_deprecated_filter_data() {
    let modules = reference_modules().expect("reference module snapshot");
    assert_eq!(
        modules
            .right_modules()
            .map(DarkroomModuleViewModel::id)
            .collect::<Vec<_>>(),
        vec![
            "exposure",
            "basicadj",
            "linear-offset",
            "rgbgain",
            "invert",
            "defringe",
            "highpass",
            "sharpen",
            "clahe",
            "dither",
            "grain",
            "colortransfer",
            "colormapping",
            "rgblevels",
            "basecurve",
            "tonecurve",
            "colisa",
            "agx",
            "levels",
            "relight",
            "colorcorrection",
            "colorcontrast",
            "velvia",
            "vibrance",
            "colorzones",
            "channelmixer",
            "shadhi",
            "temperature",
            "bloom",
            "soften",
            "censorize",
            "vignette",
            "graduatednd",
            "crop",
            "clipping",
            "rasterfile",
            "borders",
            "overlay",
            "watermark",
            "flip",
            "rotatepixels",
            "scalepixels",
            "finalscale",
            "enlargecanvas",
            "ashift",
            "lenscorrection",
            "liquify",
            "mask_manager",
            "retouch",
            "spots",
            "highlights",
            "colorreconstruct",
            "colorin",
            "primaries",
            "colorout",
        ]
    );
    assert!(modules.module("bloom").is_some());
    assert!(modules.module("soften").is_some());
    assert!(modules.module("dither").is_some());
    assert!(modules.module("borders").is_some());
    assert!(modules.module("overlay").is_some());
    assert!(modules.module("watermark").is_some());
    let invert = modules.module("invert").expect("invert module");
    assert!(invert.availability().is_deprecated());
    assert!(invert.status_text().contains("Deprecated"));
    let defringe = modules.module("defringe").expect("defringe module");
    assert!(defringe.availability().is_deprecated());
    assert!(defringe.availability().is_supported());
    assert!(defringe.enabled());
    assert!(defringe.status_text().contains("Deprecated"));
    assert!(!DarkroomModuleGroup::Active.matches(defringe));
    assert!(DarkroomModuleGroup::Deprecated.matches(defringe));
    assert!(bloom_has_typed_sliders(&modules));
    let graduatednd = modules.module("graduatednd").expect("graduated ND");
    assert_eq!(graduatednd.presets().len(), 13);
    let minimum = graduatednd
        .controls()
        .control("graduatednd-density")
        .expect("density")
        .slider_spec()
        .expect("density slider")
        .minimum();
    assert!((minimum + 8.0).abs() < f64::EPSILON);
    let vignette = modules.module("vignette").expect("vignette");
    assert!(vignette.controls().control("vignette-center-x").is_some());
    assert!(vignette.availability().is_supported());
}

#[test]
fn deprecated_groups_follow_native_enabled_visibility() {
    let mut vibrance = reference_modules()
        .expect("reference module snapshot")
        .module("vibrance")
        .expect("Vibrance module")
        .clone();
    assert!(vibrance.availability().is_deprecated());
    assert!(!vibrance.enabled());
    assert!(!DarkroomModuleGroup::Active.matches(&vibrance));
    assert!(!DarkroomModuleGroup::Color.matches(&vibrance));
    assert!(!DarkroomModuleGroup::Grading.matches(&vibrance));
    assert!(DarkroomModuleGroup::Deprecated.matches(&vibrance));

    let revision = vibrance.revision();
    vibrance
        .set_enabled(revision, true)
        .expect("deprecated compatibility module remains usable");
    assert!(DarkroomModuleGroup::Active.matches(&vibrance));
    assert!(DarkroomModuleGroup::Color.matches(&vibrance));
    assert!(DarkroomModuleGroup::Grading.matches(&vibrance));
    assert!(DarkroomModuleGroup::Deprecated.matches(&vibrance));
}

#[test]
fn censorize_projects_exact_controls_and_is_cpu_supported() {
    let modules = reference_modules().expect("reference module snapshot");
    let censorize = modules.module("censorize").expect("censorize module");
    assert!(censorize.availability().is_supported());
    assert!(censorize.enabled());
    assert_eq!(
        censorize
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        [
            "censorize-radius-1",
            "censorize-pixelate",
            "censorize-radius-2",
            "censorize-noise"
        ]
    );
    for id in [
        "censorize-radius-1",
        "censorize-pixelate",
        "censorize-radius-2",
    ] {
        let slider = censorize
            .controls()
            .control(id)
            .expect("radius control")
            .slider_spec()
            .expect("radius slider");
        assert_float_eq(slider.minimum(), 0.0);
        assert_float_eq(slider.maximum(), 500.0);
        assert_float_eq(slider.default_value(), 0.0);
    }
    let noise = censorize
        .controls()
        .control("censorize-noise")
        .expect("noise control")
        .slider_spec()
        .expect("noise slider");
    assert_float_eq(noise.minimum(), 0.0);
    assert_float_eq(noise.maximum(), 1.0);
    assert_float_eq(noise.default_value(), 0.0);
}

#[test]
fn defringe_descriptor_projects_exact_v1_controls_and_qualifies_processing() {
    let modules = reference_modules().expect("reference modules");
    let defringe = modules.module("defringe").expect("defringe");
    assert_eq!(
        defringe
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["defringe-radius", "defringe-threshold", "defringe-mode"]
    );
    for (id, minimum, maximum, default) in [
        ("defringe-radius", 0.5, 20.0, 4.0),
        ("defringe-threshold", 0.5, 128.0, 20.0),
    ] {
        let slider = defringe
            .controls()
            .control(id)
            .expect("defringe slider")
            .slider_spec()
            .expect("slider metadata");
        assert_float_eq(slider.minimum(), minimum);
        assert_float_eq(slider.maximum(), maximum);
        assert_float_eq(slider.default_value(), default);
    }
    let mode = defringe.controls().control("defringe-mode").expect("mode");
    assert_eq!(
        mode.choices()
            .map(crate::presentation::PresentationText::as_str)
            .collect::<Vec<_>>(),
        ["global_average", "local_average", "static"]
    );
    assert_eq!(mode.value(), DarkroomControlValue::Choice(0));
    assert!(defringe.availability().is_supported());
}

#[test]
fn clahe_descriptor_projects_exact_v1_controls_and_cpu_state() {
    let modules = reference_modules().expect("reference modules");
    let clahe = modules.module("clahe").expect("CLAHE");
    assert_eq!(clahe.title(), "Old Local Contrast");
    assert!(clahe.availability().is_deprecated());
    assert!(!clahe.availability().is_unsupported());
    assert!(clahe.availability().is_supported());
    assert!(clahe.status_text().contains("Deprecated"));
    assert_eq!(
        clahe
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["clahe-radius", "clahe-slope"]
    );
    for (id, minimum, maximum, default) in [
        ("clahe-radius", 0.0, 256.0, 64.0),
        ("clahe-slope", 1.0, 3.0, 1.25),
    ] {
        let slider = clahe
            .controls()
            .control(id)
            .expect("CLAHE slider")
            .slider_spec()
            .expect("slider metadata");
        assert_float_eq(slider.minimum(), minimum);
        assert_float_eq(slider.maximum(), maximum);
        assert_float_eq(slider.default_value(), default);
    }
    assert!(!DarkroomModuleGroup::Active.matches(clahe));
    assert!(DarkroomModuleGroup::Deprecated.matches(clahe));
}

#[test]
fn velvia_projects_the_source_module_and_slider_presentation() {
    let modules = reference_modules().expect("reference modules");
    let velvia = modules.module("velvia").expect("Velvia module");

    assert_eq!(velvia.title(), "velvia");
    assert_eq!(velvia.aliases().collect::<Vec<_>>(), ["saturation"]);
    assert_eq!(
        velvia.group_keys().collect::<Vec<_>>(),
        ["group.color", "group.grading"]
    );
    assert!(!velvia.enabled());
    assert!(!velvia.expanded());
    assert!(velvia.is_style_eligible());
    assert!(!velvia.is_favorite());
    assert_eq!(velvia.presets().len(), 0);
    assert!(DarkroomModuleGroup::Color.matches(velvia));
    assert!(DarkroomModuleGroup::Grading.matches(velvia));
    assert!(!DarkroomModuleGroup::Active.matches(velvia));
    assert!(module_matches_query(velvia, "saturation"));

    let expected = [
        (
            "velvia-strength",
            "strength",
            (0.0, 100.0),
            25.0,
            1.0,
            2,
            "%",
            "the strength of saturation boost",
        ),
        (
            "velvia-bias",
            "mid-tones bias",
            (0.0, 1.0),
            1.0,
            0.01,
            2,
            "",
            "how much to spare highlights and shadows",
        ),
    ];
    for (id, label, range, default, step, digits, suffix, tooltip) in expected {
        let control = velvia.controls().control(id).expect("Velvia control");
        assert_eq!(control.label().as_str(), label);
        let slider = control.slider_spec().expect("Velvia slider");
        assert_eq!((slider.minimum(), slider.maximum()), range);
        assert_float_eq(slider.default_value(), default);
        assert_float_eq(slider.value(), default);
        assert_float_eq(slider.step(), step);
        let source = control
            .source_mapped_slider_spec()
            .expect("source-qualified Bauhaus slider");
        assert_eq!(source.digits(), digits);
        assert!(source.automatic_step());
        assert_eq!(source.suffix(), suffix);
        assert_eq!(source.tooltip(), tooltip);
    }
}

#[test]
fn vibrance_projects_exact_deprecated_source_module_and_amount_slider() {
    let modules = reference_modules().expect("reference modules");
    let vibrance = modules.module("vibrance").expect("Vibrance module");
    let descriptor = rusttable_processing::builtin_registry()
        .definition("rusttable.vibrance")
        .expect("Vibrance definition")
        .descriptor();

    assert_eq!(vibrance.title(), "vibrance");
    assert_eq!(vibrance.aliases().collect::<Vec<_>>(), ["saturation"]);
    assert_eq!(
        vibrance.group_keys().collect::<Vec<_>>(),
        ["group.color", "group.grading"]
    );
    assert!(!vibrance.enabled());
    assert!(!vibrance.expanded());
    assert!(vibrance.is_style_eligible());
    assert!(vibrance.availability().is_deprecated());
    assert!(vibrance.availability().is_supported());
    assert_eq!(
        vibrance.availability().reason(),
        Some(
            "this module is deprecated. please use the vibrance slider in the color balance rgb module instead."
        )
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MULTI_INSTANCE),
        "omitting Darktable's ONE_INSTANCE flag enables native multi-instance behavior"
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::BLENDING)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MASKS),
        "Darktable enables masks when SUPPORTS_BLENDING is present without NO_MASKS"
    );
    assert!(!DarkroomModuleGroup::Color.matches(vibrance));
    assert!(!DarkroomModuleGroup::Grading.matches(vibrance));
    assert!(!DarkroomModuleGroup::Active.matches(vibrance));
    assert!(DarkroomModuleGroup::Deprecated.matches(vibrance));
    assert!(module_matches_query(vibrance, "saturation"));
    assert_eq!(
        vibrance
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["vibrance-amount"]
    );

    let amount = vibrance
        .controls()
        .control("vibrance-amount")
        .expect("native amount control");
    assert_eq!(amount.label().as_str(), "vibrance");
    let slider = amount.slider_spec().expect("Vibrance slider");
    assert_eq!((slider.minimum(), slider.maximum()), (0.0, 100.0));
    assert_float_eq(slider.default_value(), 25.0);
    assert_float_eq(slider.value(), 25.0);
    assert_float_eq(slider.step(), 1.0);
    let source = amount
        .source_mapped_slider_spec()
        .expect("source-qualified Bauhaus slider");
    assert_eq!(source.digits(), 2);
    assert!(source.automatic_step());
    assert_eq!(source.suffix(), "%");
    assert_eq!(source.tooltip(), "the amount of vibrance");
}

#[test]
fn vibrance_reset_enables_and_preserves_exact_operation_target() {
    let operation_id = OperationId::new(611).expect("Vibrance operation id");
    let mut vibrance = reference_modules()
        .expect("reference modules")
        .module("vibrance")
        .expect("Vibrance module")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    vibrance
        .reconcile_operation(
            Revision::from_u64(8),
            false,
            [(
                "vibrance-amount".to_owned(),
                DarkroomControlValue::Slider(125.0),
            )],
        )
        .expect("finite persisted outlier remains projectable");

    let wrong = vibrance
        .apply(DarkroomModuleAction::Reset {
            module_id: "vibrance".to_owned(),
            operation_id: None,
            expected_revision: Revision::from_u64(8),
        })
        .expect_err("persisted panels reject an ambiguous operation target");
    assert!(matches!(wrong, DarkroomModuleError::WrongOperation { .. }));

    vibrance
        .apply(DarkroomModuleAction::Reset {
            module_id: "vibrance".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::from_u64(8),
        })
        .expect("native reset");
    assert!(vibrance.enabled());
    assert_eq!(
        vibrance
            .controls()
            .control("vibrance-amount")
            .expect("reset amount")
            .value(),
        DarkroomControlValue::Slider(25.0)
    );
}

#[test]
fn colorcorrection_projects_one_atomic_grid_one_slider_and_gates_unpersistable_presets() {
    let modules = reference_modules().expect("reference modules");
    let module = modules
        .module("colorcorrection")
        .expect("Color Correction module");
    assert_eq!(module.title(), "color correction");
    assert_eq!(
        module.group_keys().collect::<Vec<_>>(),
        ["group.color", "group.grading"]
    );
    assert!(!module.enabled());
    assert!(!module.expanded());
    assert!(module.is_style_eligible());
    assert_eq!(
        module
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["colorcorrection-saturation"],
        "hia, hib, loa, and lob are one grid state, not invented sliders"
    );
    let saturation = module
        .controls()
        .control("colorcorrection-saturation")
        .expect("native saturation slider");
    let slider = saturation.slider_spec().expect("slider contract");
    assert_eq!((slider.minimum(), slider.maximum()), (-3.0, 3.0));
    assert_float_eq(slider.default_value(), 1.0);
    assert_float_eq(slider.step(), 0.01);
    let source = saturation
        .source_mapped_slider_spec()
        .expect("source slider presentation");
    assert_eq!(source.digits(), 2);
    assert!(source.automatic_step());
    assert_eq!(source.tooltip(), "set the global saturation");
    assert_eq!(
        module.color_correction_grid(),
        Some(crate::iop::colorcorrection::ColorCorrectionGridState::DEFAULT)
    );
    assert_eq!(module.presets().len(), 0);
    assert_eq!(
        module.presets_unavailable_reason(),
        Some(
            "Color Correction presets require RGB-display blend state, which the current edit model cannot persist"
        )
    );
    let source_presets = rusttable_processing::operations::colorcorrection::presets();
    assert_eq!(
        source_presets
            .iter()
            .map(|preset| preset.name)
            .collect::<Vec<_>>(),
        ["warm tone", "warming filter", "cooling filter"],
        "source-derived processing definitions remain intact behind the UI gate"
    );
    assert!(source_presets.iter().all(|preset| preset.enabled));
    assert_eq!(
        f64::from(source_presets[2].parameters.lob).to_bits(),
        (-0.0_f64).to_bits(),
        "native cooling-filter -0.0 survives in the source-derived definition"
    );
}

#[test]
fn colorcorrection_grid_action_is_exact_targeted_and_advances_once() {
    let operation_id = OperationId::new(901).expect("Color Correction operation id");
    let mut module = reference_modules()
        .expect("reference modules")
        .module("colorcorrection")
        .expect("Color Correction")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    assert!(!module.enabled());
    let grid = crate::iop::colorcorrection::ColorCorrectionGridState::new(-0.95, 4.5, 3.55, 0.0)
        .expect("warming grid");
    let revision = module
        .apply(DarkroomModuleAction::ColorCorrectionGrid {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            grid,
        })
        .expect("atomic grid action");
    assert_eq!(revision, Revision::from_u64(1));
    assert!(
        module.enabled(),
        "native history insertion enables Color Correction on the first grid edit"
    );
    assert_eq!(module.color_correction_grid(), Some(grid));
    assert_eq!(
        module
            .focus_order()
            .into_iter()
            .filter(|id| id.ends_with("-grid") || id.ends_with("saturation-widget"))
            .collect::<Vec<_>>(),
        ["colorcorrection-grid", "colorcorrection-saturation-widget"]
    );

    let wrong = module
        .apply(DarkroomModuleAction::ColorCorrectionGrid {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(OperationId::new(902).expect("wrong operation id")),
            expected_revision: revision,
            grid: crate::iop::colorcorrection::ColorCorrectionGridState::DEFAULT,
        })
        .expect_err("exact persisted target is required");
    assert!(matches!(wrong, DarkroomModuleError::WrongOperation { .. }));
    assert_eq!(module.color_correction_grid(), Some(grid));
    assert_eq!(module.revision(), revision);
}

#[test]
fn colorcorrection_saturation_edit_enables_disabled_exact_instance() {
    let operation_id = OperationId::new(902).expect("Color Correction operation id");
    let mut module = reference_modules()
        .expect("reference modules")
        .module("colorcorrection")
        .expect("Color Correction")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    assert!(!module.enabled());

    let revision = module
        .apply(DarkroomModuleAction::Control {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            id: "colorcorrection-saturation".to_owned(),
            value: DarkroomControlValue::Slider(0.25),
        })
        .expect("native saturation edit");

    assert_eq!(revision, Revision::from_u64(1));
    assert!(module.enabled());
    assert_eq!(
        module
            .controls()
            .control("colorcorrection-saturation")
            .expect("saturation")
            .value(),
        DarkroomControlValue::Slider(0.25)
    );
}

#[test]
fn colorcorrection_parameter_reset_defaults_five_parameters_and_enables_once() {
    let operation_id = OperationId::new(904).expect("Color Correction operation id");
    let mut module = reference_modules()
        .expect("reference modules")
        .module("colorcorrection")
        .expect("Color Correction")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    module
        .reconcile_operation(
            Revision::from_u64(7),
            false,
            [(
                "colorcorrection-saturation".to_owned(),
                DarkroomControlValue::Slider(2.25),
            )],
        )
        .expect("persisted saturation");
    module
        .reconcile_color_correction_grid(
            Revision::from_u64(7),
            crate::iop::colorcorrection::ColorCorrectionGridState::new(1.0, 2.0, 3.0, 4.0)
                .expect("persisted grid"),
        )
        .expect("persisted grid projection");

    let revision = module
        .apply(DarkroomModuleAction::ColorCorrectionResetParameters {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::from_u64(7),
        })
        .expect("source-specific parameter reset");

    assert_eq!(revision, Revision::from_u64(8));
    assert!(module.enabled());
    assert_eq!(
        module.color_correction_grid(),
        Some(crate::iop::colorcorrection::ColorCorrectionGridState::DEFAULT)
    );
    assert_eq!(
        module
            .controls()
            .control("colorcorrection-saturation")
            .expect("saturation")
            .value(),
        DarkroomControlValue::Slider(1.0)
    );
}

#[test]
fn colorcorrection_preset_action_is_rejected_until_blend_state_is_persistable() {
    let operation_id = OperationId::new(903).expect("Color Correction operation id");
    let mut module = reference_modules()
        .expect("reference modules")
        .module("colorcorrection")
        .expect("Color Correction")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    assert!(!module.enabled());
    let error = module
        .apply(DarkroomModuleAction::Preset {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            preset_id: "cooling filter".to_owned(),
        })
        .expect_err("incomplete Color Correction preset cannot reach production");
    assert!(matches!(
        error,
        DarkroomModuleError::Unsupported { module_id, reason }
            if module_id == "colorcorrection"
                && reason.contains("RGB-display blend state")
    ));
    assert_eq!(module.revision(), Revision::ZERO);
    assert!(!module.enabled());
    assert_eq!(
        module
            .controls()
            .control("colorcorrection-saturation")
            .expect("saturation")
            .value(),
        DarkroomControlValue::Slider(1.0)
    );
    assert_eq!(
        module.color_correction_grid(),
        Some(crate::iop::colorcorrection::ColorCorrectionGridState::DEFAULT)
    );
}

#[test]
fn colorcontrast_projects_only_the_two_source_gui_sliders() {
    let modules = reference_modules().expect("reference modules");
    let colorcontrast = modules
        .module("colorcontrast")
        .expect("Color Contrast module");
    let descriptor = rusttable_processing::builtin_registry()
        .definition("rusttable.colorcontrast")
        .expect("Color Contrast definition")
        .descriptor();

    assert_eq!(colorcontrast.title(), "color contrast");
    assert_eq!(colorcontrast.aliases().collect::<Vec<_>>(), ["saturation"]);
    assert_eq!(
        colorcontrast.group_keys().collect::<Vec<_>>(),
        ["group.color", "group.grading"]
    );
    assert!(!colorcontrast.enabled());
    assert!(!colorcontrast.expanded());
    assert!(colorcontrast.is_style_eligible());
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MULTI_INSTANCE)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::BLENDING)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MASKS)
    );
    assert!(!colorcontrast.is_favorite());
    assert_eq!(colorcontrast.presets().len(), 0);
    assert!(DarkroomModuleGroup::Color.matches(colorcontrast));
    assert!(DarkroomModuleGroup::Grading.matches(colorcontrast));
    assert!(!DarkroomModuleGroup::Active.matches(colorcontrast));
    assert!(module_matches_query(colorcontrast, "saturation"));
    assert_eq!(
        colorcontrast
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["colorcontrast-a-steepness", "colorcontrast-b-steepness"],
        "native gui_init does not expose offsets or unbound"
    );

    let expected = [
        (
            "colorcontrast-a-steepness",
            "green-magenta contrast",
            "steepness of the a* curve in Lab\nlower values desaturate greens and magenta while higher saturate them",
        ),
        (
            "colorcontrast-b-steepness",
            "blue-yellow contrast",
            "steepness of the b* curve in Lab\nlower values desaturate blues and yellows while higher saturate them",
        ),
    ];
    for (id, label, tooltip) in expected {
        let control = colorcontrast
            .controls()
            .control(id)
            .expect("Color Contrast control");
        assert_eq!(control.label().as_str(), label);
        let slider = control.slider_spec().expect("Color Contrast slider");
        assert_eq!((slider.minimum(), slider.maximum()), (0.0, 5.0));
        assert_float_eq(slider.default_value(), 1.0);
        assert_float_eq(slider.value(), 1.0);
        assert_float_eq(slider.step(), 0.05);
        let source = control
            .source_mapped_slider_spec()
            .expect("source-qualified Bauhaus slider");
        assert_eq!(source.digits(), 2);
        assert!(source.automatic_step());
        assert_eq!(source.suffix(), "");
        assert_eq!(source.tooltip(), tooltip);
    }
    for hidden in [
        "colorcontrast-a-offset",
        "colorcontrast-b-offset",
        "colorcontrast-unbound",
    ] {
        assert!(colorcontrast.controls().control(hidden).is_none());
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "This source-parity regression keeps Color Contrast metadata and outlier persistence together."
)]
fn colorcontrast_actions_preserve_source_metadata_and_persisted_outliers() {
    let mut colorcontrast = reference_modules()
        .expect("reference modules")
        .module("colorcontrast")
        .expect("Color Contrast module")
        .clone();

    colorcontrast
        .reconcile_operation(
            Revision::from_u64(5),
            true,
            [
                (
                    "colorcontrast-a-steepness".to_owned(),
                    DarkroomControlValue::Slider(5.5),
                ),
                (
                    "colorcontrast-b-steepness".to_owned(),
                    DarkroomControlValue::Slider(-0.5),
                ),
            ],
        )
        .expect("finite persisted source values remain projectable");
    assert_eq!(
        colorcontrast
            .controls()
            .control("colorcontrast-a-steepness")
            .expect("a* steepness")
            .value(),
        DarkroomControlValue::Slider(5.5)
    );
    assert_eq!(
        colorcontrast
            .controls()
            .control("colorcontrast-b-steepness")
            .expect("b* steepness")
            .value(),
        DarkroomControlValue::Slider(-0.5)
    );
    for id in ["colorcontrast-a-steepness", "colorcontrast-b-steepness"] {
        assert!(
            colorcontrast
                .controls()
                .control(id)
                .expect("projected control")
                .source_mapped_slider_spec()
                .is_some()
        );
    }

    let error = colorcontrast
        .apply(DarkroomModuleAction::Control {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: Revision::from_u64(5),
            id: "colorcontrast-a-steepness".to_owned(),
            value: DarkroomControlValue::Slider(5.5),
        })
        .expect_err("new GTK input remains constrained to the source UI range");
    assert!(matches!(
        error,
        DarkroomModuleError::Control(DarkroomControlError::Validation(
            ControlValidationError::SliderValueOutOfRange { .. }
        ))
    ));

    let mut revision = colorcontrast.revision();
    revision = colorcontrast
        .apply(DarkroomModuleAction::Enable {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: revision,
            enabled: false,
        })
        .expect("disable Color Contrast");
    colorcontrast
        .apply(DarkroomModuleAction::Disclosure {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: revision,
            expanded: true,
        })
        .expect("expand Color Contrast");
    assert!(!colorcontrast.enabled());
    assert!(colorcontrast.expanded());

    colorcontrast
        .apply(DarkroomModuleAction::Reset {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: revision,
        })
        .expect("reset disabled Color Contrast");
    assert!(
        colorcontrast.enabled(),
        "native reset enables disabled Color Contrast"
    );
    for id in ["colorcontrast-a-steepness", "colorcontrast-b-steepness"] {
        assert_eq!(
            colorcontrast
                .controls()
                .control(id)
                .expect("reset Color Contrast control")
                .value(),
            DarkroomControlValue::Slider(1.0)
        );
    }
}

#[test]
fn velvia_actions_and_persisted_state_projection_preserve_source_metadata() {
    let mut velvia = reference_modules()
        .expect("reference modules")
        .module("velvia")
        .expect("Velvia module")
        .clone();

    let mut revision = velvia
        .apply(DarkroomModuleAction::Enable {
            module_id: "velvia".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
            enabled: true,
        })
        .expect("enable Velvia");
    revision = velvia
        .apply(DarkroomModuleAction::Disclosure {
            module_id: "velvia".to_owned(),
            operation_id: None,
            expected_revision: revision,
            expanded: true,
        })
        .expect("expand Velvia");
    for (id, value) in [("velvia-strength", 60.0), ("velvia-bias", 0.4)] {
        revision = velvia
            .apply(DarkroomModuleAction::Control {
                module_id: "velvia".to_owned(),
                operation_id: None,
                expected_revision: revision,
                id: id.to_owned(),
                value: DarkroomControlValue::Slider(value),
            })
            .expect("apply Velvia slider action");
    }
    assert!(velvia.enabled());
    assert!(velvia.expanded());
    assert!(DarkroomModuleGroup::Active.matches(&velvia));
    assert_eq!(
        velvia
            .controls()
            .control("velvia-strength")
            .expect("strength")
            .value(),
        DarkroomControlValue::Slider(60.0)
    );
    assert_eq!(
        velvia
            .controls()
            .control("velvia-bias")
            .expect("bias")
            .value(),
        DarkroomControlValue::Slider(0.4)
    );

    velvia
        .reconcile_operation(
            Revision::from_u64(20),
            false,
            [
                (
                    "velvia-strength".to_owned(),
                    DarkroomControlValue::Slider(35.0),
                ),
                ("velvia-bias".to_owned(), DarkroomControlValue::Slider(0.8)),
            ],
        )
        .expect("persisted Velvia projection");
    assert_eq!(velvia.revision(), Revision::from_u64(20));
    assert!(!velvia.enabled());
    for id in ["velvia-strength", "velvia-bias"] {
        assert!(
            velvia
                .controls()
                .control(id)
                .expect("projected control")
                .source_mapped_slider_spec()
                .is_some(),
            "state projection must preserve {id} Bauhaus qualification"
        );
    }
}

#[test]
fn velvia_persisted_values_can_exceed_ui_bounds_but_new_input_cannot() {
    let mut velvia = reference_modules()
        .expect("reference modules")
        .module("velvia")
        .expect("Velvia module")
        .clone();
    velvia
        .reconcile_operation(
            Revision::from_u64(5),
            true,
            [
                (
                    "velvia-strength".to_owned(),
                    DarkroomControlValue::Slider(101.0),
                ),
                (
                    "velvia-bias".to_owned(),
                    DarkroomControlValue::Slider(-0.01),
                ),
            ],
        )
        .expect("finite native values remain projectable");
    assert_eq!(
        velvia
            .controls()
            .control("velvia-strength")
            .expect("strength")
            .value(),
        DarkroomControlValue::Slider(101.0)
    );
    assert_eq!(
        velvia
            .controls()
            .control("velvia-bias")
            .expect("bias")
            .value(),
        DarkroomControlValue::Slider(-0.01)
    );

    let error = velvia
        .apply(DarkroomModuleAction::Control {
            module_id: "velvia".to_owned(),
            operation_id: None,
            expected_revision: Revision::from_u64(5),
            id: "velvia-strength".to_owned(),
            value: DarkroomControlValue::Slider(101.0),
        })
        .expect_err("new GTK input remains constrained to the source UI range");
    assert!(matches!(
        error,
        DarkroomModuleError::Control(DarkroomControlError::Validation(
            ControlValidationError::SliderValueOutOfRange { .. }
        ))
    ));
}

fn assert_float_eq(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < f64::EPSILON);
}

#[test]
fn cpu_qualified_censorize_accepts_enable_reset_and_control_actions() {
    for action in [
        DarkroomModuleAction::Enable {
            module_id: "censorize".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
            enabled: true,
        },
        DarkroomModuleAction::Reset {
            module_id: "censorize".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
        },
        DarkroomModuleAction::Control {
            module_id: "censorize".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
            id: "censorize-noise".to_owned(),
            value: DarkroomControlValue::Slider(0.5),
        },
    ] {
        let mut module = reference_modules()
            .expect("reference modules")
            .module("censorize")
            .expect("censorize")
            .clone();
        assert!(module.apply(action).is_ok());
        assert_eq!(module.revision(), Revision::from_u64(1));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn censorize_gtk_panel_exposes_sensitive_controls() {
    if gtk4::init().is_err() {
        return;
    }
    let module = reference_modules()
        .expect("reference modules")
        .module("censorize")
        .expect("censorize")
        .clone();
    let panel = build_module_panel(&module);
    let root: gtk4::Widget = panel.upcast();
    assert_eq!(root.widget_name(), "censorize");
    for id in [
        "censorize-enabled",
        "censorize-radius-1-widget",
        "censorize-pixelate-widget",
        "censorize-radius-2-widget",
        "censorize-noise-widget",
        "censorize-reset",
    ] {
        assert!(
            find_widget(&root, id).is_some_and(|widget| widget.is_sensitive()),
            "qualified control {id} must be sensitive"
        );
    }
    let status = find_widget(&root, "censorize-status")
        .expect("status widget")
        .downcast::<gtk4::Label>()
        .expect("status label");
    assert!(!status.text().contains("backend is unqualified until #477"));
}

#[cfg(target_os = "linux")]
#[test]
fn clahe_gtk_panel_exposes_imported_controls_as_unavailable() {
    if gtk4::init().is_err() {
        return;
    }
    let module = reference_modules()
        .expect("reference modules")
        .module("clahe")
        .expect("CLAHE")
        .clone();
    let panel = build_module_panel(&module);
    let root: gtk4::Widget = panel.upcast();
    for id in ["clahe-enabled", "clahe-radius-widget", "clahe-slope-widget"] {
        assert!(
            find_widget(&root, id).is_some_and(|widget| !widget.is_sensitive()),
            "unqualified control {id} must remain insensitive"
        );
    }
    let status = find_widget(&root, "clahe-status")
        .expect("status widget")
        .downcast::<gtk4::Label>()
        .expect("status label");
    assert!(status.text().contains("Unavailable"));
    assert!(status.text().contains("#473"));
}

#[cfg(target_os = "linux")]
#[test]
fn unavailable_resettable_module_keeps_reset_insensitive() {
    if gtk4::init().is_err() {
        return;
    }
    let module = reference_modules()
        .expect("reference modules")
        .module("colorcontrast")
        .expect("Color Contrast")
        .clone()
        .with_availability(DarkroomModuleAvailability::Unsupported {
            reason: "test backend unavailable".to_owned(),
        });
    let panel = build_module_panel(&module);
    let root: gtk4::Widget = panel.upcast();
    for id in [
        "colorcontrast-enabled",
        "colorcontrast-reset",
        "colorcontrast-a-steepness-widget",
        "colorcontrast-b-steepness-widget",
    ] {
        assert!(
            find_widget(&root, id).is_some_and(|widget| !widget.is_sensitive()),
            "unavailable control {id} must remain insensitive"
        );
    }
}

#[cfg(target_os = "linux")]
fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn bloom_has_typed_sliders(modules: &DarkroomModulesViewModel) -> bool {
    modules
        .module("bloom")
        .expect("bloom module")
        .controls()
        .controls()
        .all(|control| control.kind() == DarkroomControlKind::Slider)
}
