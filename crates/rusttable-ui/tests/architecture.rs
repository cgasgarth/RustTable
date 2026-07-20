#![forbid(unsafe_code)]

const UI_SOURCES: &[&str] = &[
    include_str!("../src/external_editor/model.rs"),
    include_str!("../src/external_editor/controller.rs"),
    include_str!("../src/external_editor/view.rs"),
    include_str!("../src/viewport_presentation.rs"),
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
