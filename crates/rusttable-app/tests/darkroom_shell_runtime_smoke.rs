#![forbid(unsafe_code)]

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_ui::gtk_shell::{GtkShell, WorkspaceRole};
use rusttable_ui::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    ViewportGeneration,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the app-shell runtime smoke");
    app_shell_transition_keeps_darkroom_titles_allocated();
    println!("Darkroom app-shell runtime smoke passed");
}

fn app_shell_transition_keeps_darkroom_titles_allocated() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.darkroom-shell-runtime"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("test GTK application must start before constructing windows");
    let shell = GtkShell::new(&application);
    let photo_id = PhotoId::new(949).expect("test photo id");
    let title = PresentationText::new("Alex_Benes.RAF").expect("test title");
    let workspace = PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(photo_id, title.clone(), None)],
        vec![PhotoDetailViewModel::new(photo_id, title, Vec::new())],
    )
    .expect("test workspace");

    shell.set_photo_workspace(&workspace);
    assert!(shell.open_photo(photo_id), "selected photo opens darkroom");
    shell.begin_darkroom_selection(photo_id, ViewportGeneration::new(1));
    shell.present();
    settle_gtk();
    assert_darkroom_titles_are_allocated(&shell);

    shell.show_workspace(WorkspaceRole::Lighttable);
    settle_gtk();
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert_darkroom_titles_are_allocated(&shell);
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn assert_darkroom_titles_are_allocated(shell: &GtkShell) {
    let root: gtk4::Widget = shell.window().clone().upcast();
    let rail = find_widget(&root, "darkroom-left-panel").expect("darkroom left rail");
    for id in [
        "darkroom-navigation",
        "darkroom-snapshots",
        "darkroom-history",
        "darkroom-image-information",
    ] {
        let expander = find_widget(&rail, id)
            .expect("darkroom section")
            .downcast::<gtk4::Expander>()
            .expect("darkroom section expander");
        let title_row = expander.label_widget().expect("darkroom title row");
        let title = find_widget(&title_row, &format!("{id}-label"))
            .expect("darkroom title")
            .downcast::<gtk4::Label>()
            .expect("darkroom title label");
        assert!(
            title_row.allocated_width() > 0
                && title_row.allocated_height() > 0
                && title.allocated_width() > 0
                && title.allocated_height() > 0
                && title.is_visible(),
            "nonzero title allocation required for {id}: row {}x{}, title {}x{}",
            title_row.allocated_width(),
            title_row.allocated_height(),
            title.allocated_width(),
            title.allocated_height()
        );
    }
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
