#![forbid(unsafe_code)]

use rusttable_ui::{AI_MODELS_FOCUS_ORDER, NEURAL_RESTORE_FOCUS_ORDER};

const UI_SOURCES: &[&str] = &[
    include_str!("../src/external_editor/model.rs"),
    include_str!("../src/external_editor/controller.rs"),
    include_str!("../src/external_editor/view.rs"),
    include_str!("../src/viewport_presentation.rs"),
    include_str!("../src/ai_models/model.rs"),
    include_str!("../src/ai_models/controller.rs"),
    include_str!("../src/ai_models/view.rs"),
    include_str!("../src/neural_restore/model.rs"),
    include_str!("../src/neural_restore/controller.rs"),
    include_str!("../src/neural_restore/view.rs"),
];

#[test]
fn external_editor_and_presentation_ui_keep_side_effects_behind_ports() {
    for source in UI_SOURCES {
        for forbidden in [
            "std::fs",
            "std::process",
            "Command::new",
            "rusttable_catalog",
            "rusttable_render",
            "rusttable_export",
            "iced::",
        ] {
            assert!(
                !source.contains(forbidden),
                "UI source crossed a service boundary: {forbidden}"
            );
        }
    }
}

#[test]
fn external_editor_focus_fixture_has_a_status_and_confirmation_path() {
    assert!(include_str!("../src/external_editor/view.rs").contains("external-editor-status"));
    assert!(include_str!("../src/external_editor/view.rs").contains("external-editor-confirm"));
}

#[test]
fn ai_surfaces_have_stable_keyboard_and_status_contracts() {
    for order in [
        AI_MODELS_FOCUS_ORDER.as_slice(),
        NEURAL_RESTORE_FOCUS_ORDER.as_slice(),
    ] {
        let unique = order
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), order.len());
        assert!(order.iter().any(|id| id.ends_with("status")));
        assert!(order.iter().any(|id| id.ends_with("cancel")));
        assert!(!order.iter().any(|id| id.contains("path")));
    }
}
