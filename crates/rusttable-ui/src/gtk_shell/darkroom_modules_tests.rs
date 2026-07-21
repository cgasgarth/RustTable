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
fn reference_modules_expose_registry_controls_and_deprecated_filter_data() {
    let modules = reference_modules().expect("reference module snapshot");
    assert_eq!(
        modules
            .right_modules()
            .map(DarkroomModuleViewModel::id)
            .collect::<Vec<_>>(),
        vec![
            "bloom",
            "soften",
            "invert",
            "dither",
            "graduatednd",
            "vignette"
        ]
    );
    assert!(modules.module("bloom").is_some());
    assert!(modules.module("soften").is_some());
    assert!(modules.module("dither").is_some());
    let invert = modules.module("invert").expect("invert module");
    assert!(invert.availability().is_deprecated());
    assert!(invert.status_text().contains("Deprecated"));
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
    assert!(vignette.availability().is_unsupported());
}

fn bloom_has_typed_sliders(modules: &DarkroomModulesViewModel) -> bool {
    modules
        .module("bloom")
        .expect("bloom module")
        .controls()
        .controls()
        .all(|control| control.kind() == DarkroomControlKind::Slider)
}
