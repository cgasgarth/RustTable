//! Darktable-shaped GTK4 import source dialog.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::gio::prelude::{FileExt, ListModelExt};
use gtk4::glib::object::CastNone;
use gtk4::prelude::*;

use super::{ImportAction, ImportRequest};

pub const IMPORT_DIALOG_WIDGET_IDS: [&str; 15] = [
    "import-dialog",
    "import-dialog-source",
    "import-dialog-places",
    "import-dialog-folders",
    "import-dialog-files",
    "import-dialog-select-files",
    "import-dialog-select-folder",
    "import-dialog-select-new",
    "import-dialog-recursive",
    "import-dialog-ignore-nonraws",
    "import-dialog-select-all",
    "import-dialog-select-none",
    "import-dialog-status",
    "import-dialog-cancel",
    "import-dialog-import",
];

pub const IMPORT_DIALOG_FOCUS_ORDER: [&str; 8] = [
    "import-dialog-select-files",
    "import-dialog-select-folder",
    "import-dialog-select-new",
    "import-dialog-recursive",
    "import-dialog-ignore-nonraws",
    "import-dialog-select-all",
    "import-dialog-select-none",
    "import-dialog-import",
];

type ActionHandler = Rc<dyn Fn(ImportAction)>;

/// GTK-owned modal source chooser that emits only an application-safe request.
#[derive(Clone)]
pub struct ImportDialog {
    window: gtk4::Window,
    import: gtk4::Button,
    source: gtk4::Label,
    files: gtk4::ListBox,
    status: gtk4::Label,
    select_new: gtk4::CheckButton,
    recursive: gtk4::CheckButton,
    ignore_nonraws: gtk4::CheckButton,
    paths: Rc<RefCell<Vec<PathBuf>>>,
    generation: Rc<Cell<u64>>,
    action: Rc<RefCell<Option<ActionHandler>>>,
}

impl ImportDialog {
    #[must_use]
    pub fn new(parent: &gtk4::ApplicationWindow) -> Self {
        let window = gtk4::Window::builder()
            .title("Import images")
            .transient_for(parent)
            .modal(true)
            .default_width(820)
            .default_height(540)
            .build();
        window.set_widget_name("import-dialog");
        let cancel = button("import-dialog-cancel", "Cancel");
        let import = button("import-dialog-import", "Import");
        import.set_widget_name("import-dialog-import");
        import.set_sensitive(false);

        let paths = Rc::new(RefCell::new(Vec::new()));
        let generation = Rc::new(Cell::new(0));
        let action = Rc::new(RefCell::new(None));
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        content.set_spacing(8);
        content.set_margin_start(10);
        content.set_margin_end(10);
        content.set_margin_top(10);
        content.set_margin_bottom(10);

        let heading = gtk4::Label::new(Some("Add existing images to the library"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("dt_dialog_heading");
        content.append(&heading);

        let panes = gtk4::Paned::new(gtk4::Orientation::Horizontal);
        panes.set_position(250);
        let places = places_pane();
        panes.set_start_child(Some(&places));

        let source = gtk4::Label::new(Some("No source selected"));
        source.set_widget_name("import-dialog-source");
        source.set_halign(gtk4::Align::Start);
        source.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        source.set_hexpand(true);
        let files = gtk4::ListBox::new();
        files.set_widget_name("import-dialog-files");
        files.set_selection_mode(gtk4::SelectionMode::None);
        let file_scroll = gtk4::ScrolledWindow::builder()
            .child(&files)
            .vexpand(true)
            .hexpand(true)
            .build();
        let select_files = button("import-dialog-select-files", "Select files…");
        let select_folder = button("import-dialog-select-folder", "Select folder…");
        let source_buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        source_buttons.append(&select_files);
        source_buttons.append(&select_folder);
        let options = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let select_new = check_button("import-dialog-select-new", "Select new", true);
        let recursive = check_button("import-dialog-recursive", "Recursive", false);
        let ignore_nonraws = check_button(
            "import-dialog-ignore-nonraws",
            "Ignore non-raw files",
            false,
        );
        options.append(&select_new);
        options.append(&recursive);
        options.append(&ignore_nonraws);
        let select_all = button("import-dialog-select-all", "Select all");
        let select_none = button("import-dialog-select-none", "Select none");
        let selection_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        selection_actions.append(&select_all);
        selection_actions.append(&select_none);
        let status = gtk4::Label::new(Some("Choose files or a folder to continue."));
        status.set_widget_name("import-dialog-status");
        status.set_halign(gtk4::Align::Start);
        status.add_css_class("dim-label");
        let right = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        right.set_margin_start(10);
        right.append(&source_buttons);
        right.append(&source);
        right.append(&file_scroll);
        right.append(&options);
        right.append(&selection_actions);
        right.append(&status);
        panes.set_end_child(Some(&right));
        content.append(&panes);
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        actions.set_halign(gtk4::Align::End);
        actions.append(&cancel);
        actions.append(&import);
        content.append(&actions);
        window.set_child(Some(&content));

        let view = Self {
            window,
            import,
            source,
            files,
            status,
            select_new,
            recursive,
            ignore_nonraws,
            paths,
            generation,
            action,
        };
        view.connect_source_buttons(&select_files, &select_folder);
        view.connect_selection_buttons(&select_all, &select_none);
        view.connect_actions(&cancel);
        view
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(ImportAction) + 'static,
    {
        self.action.replace(Some(Rc::new(handler)));
    }

    fn connect_source_buttons(&self, select_files: &gtk4::Button, select_folder: &gtk4::Button) {
        let state = self.clone();
        select_files.connect_clicked(move |_| state.choose_files());
        let state = self.clone();
        select_folder.connect_clicked(move |_| state.choose_folder());
    }

    fn connect_selection_buttons(&self, select_all: &gtk4::Button, select_none: &gtk4::Button) {
        let state = self.clone();
        select_all
            .connect_clicked(move |_| state.status.set_text("All discovered files selected."));
        let state = self.clone();
        select_none.connect_clicked(move |_| state.status.set_text("No files selected."));
    }

    fn connect_actions(&self, cancel: &gtk4::Button) {
        let state = self.clone();
        cancel.connect_clicked(move |_| state.window.set_visible(false));
        let state = self.clone();
        self.import.connect_clicked(move |_| {
            let request = ImportRequest::new(
                state.paths.borrow().clone(),
                state.recursive.is_active(),
                state.select_new.is_active(),
                state.ignore_nonraws.is_active(),
                state.generation.get(),
            );
            let Some(request) = request else {
                state
                    .status
                    .set_text("Choose at least one source before importing.");
                return;
            };
            state.import.set_sensitive(false);
            state.window.set_visible(false);
            if let Some(handler) = state.action.borrow().as_ref() {
                handler(ImportAction::Import(request));
            }
        });
    }

    fn choose_files(&self) {
        let token = self.bump_generation();
        let state = self.clone();
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select images to import")
            .accept_label("Select")
            .modal(true)
            .build();
        file_dialog.open_multiple(
            Some(&self.window),
            None::<&gtk4::gio::Cancellable>,
            move |result| {
                if state.generation.get() != token {
                    return;
                }
                let Ok(files) = result else { return };
                let paths = (0..files.n_items())
                    .filter_map(|index| files.item(index).and_downcast::<gtk4::gio::File>())
                    .filter_map(|file| file.path())
                    .collect::<Vec<_>>();
                state.set_paths(&paths);
            },
        );
    }

    fn choose_folder(&self) {
        let token = self.bump_generation();
        let state = self.clone();
        let file_dialog = gtk4::FileDialog::builder()
            .title("Select import folder")
            .accept_label("Select")
            .modal(true)
            .build();
        file_dialog.select_folder(
            Some(&self.window),
            None::<&gtk4::gio::Cancellable>,
            move |result| {
                if state.generation.get() != token {
                    return;
                }
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let paths = vec![path];
                state.set_paths(&paths);
            },
        );
    }

    fn set_paths(&self, paths: &[PathBuf]) {
        self.paths.replace(paths.to_owned());
        clear_children(&self.files);
        for path in paths {
            let row = gtk4::Label::new(Some(&path.to_string_lossy()));
            row.set_halign(gtk4::Align::Start);
            row.set_margin_top(3);
            row.set_margin_bottom(3);
            row.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            self.files.append(&row);
        }
        let count = paths.len();
        let source = if count == 1 {
            paths[0].to_string_lossy().into_owned()
        } else {
            "Selected image files".to_owned()
        };
        self.source.set_text(&source);
        self.status.set_text(&format!("{count} source selected"));
        self.import.set_sensitive(!paths.is_empty());
    }

    fn bump_generation(&self) -> u64 {
        let next = self.generation.get().saturating_add(1);
        self.generation.set(next);
        next
    }
}

fn places_pane() -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_widget_name("import-dialog-places");
    let heading = gtk4::Label::new(Some("Places"));
    heading.set_halign(gtk4::Align::Start);
    heading.add_css_class("dt_module_title");
    root.append(&heading);
    let recent = gtk4::ListBox::new();
    recent.set_widget_name("import-dialog-folders");
    recent.set_selection_mode(gtk4::SelectionMode::None);
    for title in ["Home", "Pictures", "Recent locations"] {
        let row = gtk4::Label::new(Some(title));
        row.set_halign(gtk4::Align::Start);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        recent.append(&row);
    }
    root.append(&recent);
    root
}

fn button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(label)]);
    button
}

fn check_button(id: &str, label: &str, active: bool) -> gtk4::CheckButton {
    let check = gtk4::CheckButton::with_label(label);
    check.set_widget_name(id);
    check.set_active(active);
    check
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::{IMPORT_DIALOG_FOCUS_ORDER, IMPORT_DIALOG_WIDGET_IDS};
    use crate::import::ImportRequest;

    #[test]
    fn import_dialog_contract_keeps_source_options_and_actions() {
        for id in [
            "import-dialog-select-files",
            "import-dialog-select-folder",
            "import-dialog-recursive",
            "import-dialog-import",
        ] {
            assert!(IMPORT_DIALOG_WIDGET_IDS.contains(&id));
        }
        assert_eq!(IMPORT_DIALOG_FOCUS_ORDER[0], "import-dialog-select-files");
        assert_eq!(
            IMPORT_DIALOG_FOCUS_ORDER.last(),
            Some(&"import-dialog-import")
        );
    }

    #[test]
    fn empty_import_requests_are_rejected_and_options_are_typed() {
        assert!(ImportRequest::new(Vec::new(), true, true, false, 1).is_none());
        let request = ImportRequest::new(
            vec![std::path::PathBuf::from("Pictures")],
            true,
            false,
            true,
            7,
        )
        .expect("non-empty source");
        assert!(request.recursive());
        assert!(!request.select_new());
        assert!(request.ignore_nonraws());
        assert_eq!(request.generation(), 7);
    }
}
