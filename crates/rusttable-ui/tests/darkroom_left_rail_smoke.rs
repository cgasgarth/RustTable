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
    shell.present();
    settle_gtk();
    let expected = shell.darkroom_panel_target().expect("darkroom target");
    assert_eq!(expected.photo_id(), photo_id);

    assert_left_rail_is_populated(&shell, expected);
    shell.show_workspace(WorkspaceRole::Lighttable);
    settle_gtk();
    assert_left_rail_is_populated(&shell, expected);
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert_left_rail_is_populated(&shell, expected);
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

#[allow(clippy::too_many_lines)]
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
    assert_eq!(
        &direct_children[..5],
        [
            "darkroom-navigation",
            "darkroom-image-information",
            "darkroom-history",
            "darkroom-snapshots",
            "darkroom-left-controller-modules",
        ],
        "left rail must follow Darktable container positions"
    );
    for id in [
        "darkroom-image-information",
        "darkroom-history",
        "darkroom-snapshots",
    ] {
        find_widget(&rail, id)
            .expect("resizable left-center module")
            .downcast::<gtk4::Expander>()
            .expect("left-center module expander")
            .set_expanded(true);
    }
    settle_gtk();
    for id in [
        "darkroom-navigation-preview",
        "darkroom-image-information-list",
        "darkroom-history-list",
        "darkroom-snapshots-list",
        "darkroom-left-controller-modules",
    ] {
        assert!(
            find_widget(&rail, id).is_some(),
            "left rail lost existing surface {id}: {:?}",
            descendant_names(&rail)
        );
    }
    for id in [
        "darkroom-navigation-info",
        "darkroom-navigation-actions",
        "darkroom-snapshots-info",
        "darkroom-snapshots-actions",
        "darkroom-history-info",
        "darkroom-history-actions",
        "darkroom-image-information-info",
        "darkroom-image-information-actions",
    ] {
        let affordance = find_widget(&rail, id)
            .unwrap_or_else(|| panic!("left rail lost affordance {id}"))
            .downcast::<gtk4::Button>()
            .expect("accordion affordance is a button");
        assert!(affordance.is_visible(), "accordion affordance hidden {id}");
        assert!(
            !affordance.is_sensitive(),
            "unavailable accordion affordance must remain neutral {id}"
        );
        assert!(
            affordance
                .child()
                .is_some_and(|child| child.is::<gtk4::Image>()),
            "accordion affordance must use a Darktable-shaped symbolic icon {id}"
        );
    }
    for id in [
        "darkroom-navigation",
        "darkroom-image-information",
        "darkroom-history",
        "darkroom-snapshots",
    ] {
        let expected_title = id
            .strip_prefix("darkroom-")
            .expect("darkroom section id")
            .replace('-', " ");
        let section = find_widget(&rail, id).expect("left-rail section");
        let title_row = if id == "darkroom-navigation" {
            assert!(
                !section.is::<gtk4::Expander>(),
                "Darktable navigation is fixed and must not expose a disclosure arrow"
            );
            find_widget(&section, &format!("{id}-title")).expect("fixed navigation title row")
        } else {
            section
                .downcast::<gtk4::Expander>()
                .expect("left-center section expander")
                .label_widget()
                .expect("section title row")
        };
        let title_label = find_widget(&title_row, &format!("{id}-label"))
            .expect("section title label")
            .downcast::<gtk4::Label>()
            .expect("section title widget is a label");
        assert_eq!(
            title_label.text().as_str(),
            expected_title.as_str(),
            "section {id}"
        );
        assert!(title_label.is_visible(), "section label is hidden {id}");
        assert!(
            title_row.allocated_width() > 0
                && title_row.allocated_height() > 0
                && title_label.allocated_width() > 0
                && title_label.allocated_height() > 0,
            "section label has no rendered allocation {id}: {}x{}",
            title_label.allocated_width(),
            title_label.allocated_height()
        );
        assert!(
            title_label.has_css_class("dt_darkroom_section_label"),
            "section label lost Darktable typography class {id}"
        );
        let action_id = format!("{id}-actions");
        assert!(
            find_widget(&title_row, &action_id).is_some(),
            "section lost action affordance {id}"
        );
    }
    let navigation_preview =
        find_widget(&rail, "darkroom-navigation-preview").expect("navigation preview");
    assert_eq!(
        navigation_preview.height_request(),
        200,
        "navigation uses Darktable's configured default graph height"
    );
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

fn descendant_names(widget: &gtk4::Widget) -> Vec<String> {
    let mut names = vec![widget.widget_name().to_string()];
    let mut child = widget.first_child();
    while let Some(current) = child {
        names.extend(descendant_names(&current));
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
