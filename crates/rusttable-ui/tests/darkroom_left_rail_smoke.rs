#![forbid(unsafe_code)]

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::DarkroomSelectionState;
use rusttable_ui::{
    DarkroomPanelTarget, GtkShell, PhotoCardViewModel, PhotoDetailViewModel,
    PhotoWorkspaceViewModel, PresentationText, ViewportGeneration, WorkspaceRole,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the darkroom left-rail smoke test");
    cold_launch_left_rail_survives_selected_raw_mode_switch();
    println!("Darkroom left-rail GTK smoke passed");
}

fn cold_launch_left_rail_survives_selected_raw_mode_switch() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.darkroom-left-rail"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("test GTK application must start before constructing windows");
    let shell = GtkShell::new(&application);
    let photo_id = PhotoId::new(946).expect("test photo id");
    let title = PresentationText::new("Alex_Benes.RAF").expect("test title");
    let workspace = PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(photo_id, title.clone(), None)],
        vec![PhotoDetailViewModel::new(photo_id, title, Vec::new())],
    )
    .expect("test workspace");

    shell.set_photo_workspace(&workspace);
    assert!(
        shell.open_photo(photo_id),
        "selected RAW must open in darkroom"
    );
    shell.begin_darkroom_selection(photo_id, ViewportGeneration::new(1));
    let expected = shell.darkroom_panel_target().expect("darkroom target");
    assert_eq!(expected.photo_id(), photo_id);

    assert_left_rail_is_populated(&shell, expected);
    shell.show_workspace(WorkspaceRole::Lighttable);
    assert_left_rail_is_populated(&shell, expected);
    shell.show_workspace(WorkspaceRole::Darkroom);
    assert_left_rail_is_populated(&shell, expected);
}

fn assert_left_rail_is_populated(shell: &GtkShell, expected: DarkroomPanelTarget) {
    assert_eq!(shell.darkroom_panel_target(), Some(expected));
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    let scroll = find_widget(&rail, "darkroom-left-module-scroll")
        .expect("darkroom left-rail scroll")
        .downcast::<gtk4::ScrolledWindow>()
        .expect("darkroom left-rail scroller");
    let content = scroll.child().expect("darkroom left-rail content");
    let modules = find_widget(&content, "darkroom-left-modules").expect("darkroom left modules");
    let direct_children = child_names(&modules);
    for id in [
        "darkroom-navigation",
        "darkroom-snapshots",
        "darkroom-history",
        "darkroom-image-information",
        "darkroom-left-controller-modules",
    ] {
        assert!(
            direct_children.iter().any(|name| name == id),
            "left rail lost existing section {id}: {direct_children:?}"
        );
    }
    for id in [
        "darkroom-navigation-actions",
        "darkroom-snapshots-actions",
        "darkroom-history-actions",
        "darkroom-image-information-actions",
    ] {
        assert!(
            find_widget(&rail, id).is_some(),
            "left rail lost affordance {id}"
        );
    }
    for id in [
        "darkroom-navigation",
        "darkroom-snapshots",
        "darkroom-history",
        "darkroom-image-information",
    ] {
        let expander = find_widget(&rail, id)
            .expect("left-rail section")
            .downcast::<gtk4::Expander>()
            .expect("left-rail section expander");
        assert!(
            expander.label_widget().is_some(),
            "section lost title row {id}"
        );
    }
    assert!(matches!(
        shell.darkroom_preview().selection_state(),
        DarkroomSelectionState::Selected(photo_id) if photo_id == expected.photo_id()
    ));
    assert!(direct_children.len() >= 5, "left rail must not be blank");
}

fn child_names(widget: &gtk4::Widget) -> Vec<String> {
    let mut names = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        names.push(current.widget_name().to_string());
        child = current.next_sibling();
    }
    names
}

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
