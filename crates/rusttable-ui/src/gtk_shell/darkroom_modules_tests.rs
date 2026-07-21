use super::*;
use crate::presentation::darkroom_controls::DarkroomControlKind;

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
            expected_revision: revision,
            expanded: false,
        })
        .expect("disclosure action");
    revision = model
        .apply(DarkroomModuleAction::Enable {
            module_id: "exposure".to_owned(),
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
                expected_revision: revision,
                id: id.to_owned(),
                value,
            })
            .expect("control action");
    }
    revision = model
        .apply(DarkroomModuleAction::Reset {
            module_id: "exposure".to_owned(),
            expected_revision: revision,
        })
        .expect("reset action");
    assert_eq!(revision, Revision::from_u64(13));
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
            "clahe",
            "dither",
            "grain",
            "relight",
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
            "colorreconstruction",
            "colorin",
            "primaries",
            "colorout",
            "colorcorrection",
        ]
    );
    assert!(modules.module("bloom").is_some());
    assert!(modules.module("soften").is_some());
    assert!(modules.module("dither").is_some());
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

fn assert_float_eq(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < f64::EPSILON);
}

#[test]
fn cpu_qualified_censorize_accepts_enable_reset_and_control_actions() {
    for action in [
        DarkroomModuleAction::Enable {
            module_id: "censorize".to_owned(),
            expected_revision: Revision::ZERO,
            enabled: true,
        },
        DarkroomModuleAction::Reset {
            module_id: "censorize".to_owned(),
            expected_revision: Revision::ZERO,
        },
        DarkroomModuleAction::Control {
            module_id: "censorize".to_owned(),
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
