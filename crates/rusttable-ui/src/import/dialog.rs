//! Darktable-shaped GTK4 import source dialog.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::gio::prelude::{FileExt, ListModelExt};
use gtk4::prelude::*;

use super::{
    ImportAction, ImportPlace, ImportRequest, ImportSourceModel, ImportSourceState,
    MAX_IMPORT_SOURCE_ROWS,
};

pub const IMPORT_DIALOG_WIDGET_IDS: [&str; 18] = [
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
    "import-dialog-source-row",
    "import-dialog-source-row-selection",
    "import-dialog-place-row",
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
    model: Rc<RefCell<ImportSourceModel>>,
    source_roots: Rc<RefCell<Vec<PathBuf>>>,
    existing_paths: Rc<RefCell<BTreeSet<PathBuf>>>,
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
        import.set_sensitive(false);

        let model = Rc::new(RefCell::new(ImportSourceModel::default()));
        let source_roots = Rc::new(RefCell::new(Vec::new()));
        let existing_paths = Rc::new(RefCell::new(BTreeSet::new()));
        let generation = Rc::new(Cell::new(0));
        let action = Rc::new(RefCell::new(None));
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
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
        let (places, place_rows) = places_pane();
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
        let (select_new, recursive, ignore_nonraws, options) = option_buttons();
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
            model,
            source_roots,
            existing_paths,
            generation,
            action,
        };
        view.connect_source_buttons(&select_files, &select_folder);
        view.connect_selection_buttons(&select_all, &select_none);
        view.connect_option_buttons();
        view.connect_places(&place_rows);
        view.connect_actions(&cancel);
        view
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Reconciles the typed source-row `new` flag with the current catalog.
    pub fn set_existing_paths(&self, paths: impl IntoIterator<Item = PathBuf>) {
        self.existing_paths.replace(paths.into_iter().collect());
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

    fn connect_places(&self, rows: &[(gtk4::ListBoxRow, ImportPlace)]) {
        for (row, place) in rows {
            let state = self.clone();
            let path = place.path().to_path_buf();
            row.connect_activate(move |_| state.set_source_roots(std::slice::from_ref(&path)));
        }
    }

    fn connect_selection_buttons(&self, select_all: &gtk4::Button, select_none: &gtk4::Button) {
        let state = self.clone();
        select_all.connect_clicked(move |_| {
            state.model.borrow_mut().select_all();
            state.render_rows();
        });
        let state = self.clone();
        select_none.connect_clicked(move |_| {
            state.model.borrow_mut().select_none();
            state.render_rows();
        });
    }

    fn connect_option_buttons(&self) {
        let state = self.clone();
        self.select_new.connect_toggled(move |button| {
            state.model.borrow_mut().set_select_new(button.is_active());
            state.render_rows();
        });
        let state = self.clone();
        self.ignore_nonraws.connect_toggled(move |button| {
            state
                .model
                .borrow_mut()
                .set_ignore_nonraws(button.is_active());
            state.render_rows();
        });
        let state = self.clone();
        self.recursive.connect_toggled(move |button| {
            state.model.borrow_mut().set_recursive(button.is_active());
            let roots = state.source_roots.borrow().clone();
            if !roots.is_empty() {
                state.set_source_roots(&roots);
            }
        });
    }

    fn connect_actions(&self, cancel: &gtk4::Button) {
        let state = self.clone();
        cancel.connect_clicked(move |_| state.window.set_visible(false));
        let state = self.clone();
        self.import.connect_clicked(move |_| {
            let model = state.model.borrow();
            let Some(request) = ImportRequest::new(
                model.effective_paths(),
                model.recursive(),
                model.select_new(),
                model.ignore_nonraws(),
                model.generation(),
            ) else {
                state.status.set_text("Select at least one file to import.");
                return;
            };
            drop(model);
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
                state.set_source_roots(&paths);
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
                state.set_source_roots(std::slice::from_ref(&path));
            },
        );
    }

    fn set_source_roots(&self, roots: &[PathBuf]) {
        let token = self.bump_generation();
        self.source_roots.replace(roots.to_owned());
        let mut model = self.model.borrow_mut();
        model.begin(token);
        let existing = self.existing_paths.borrow();
        match discover_paths(roots, model.recursive(), &existing) {
            Ok(paths) => model.replace_rows(token, paths),
            Err(detail) => model.fail(detail),
        }
        drop(existing);
        drop(model);
        let source = roots.first().map_or_else(
            || "No source selected".to_owned(),
            |path| {
                if roots.len() == 1 {
                    path.display().to_string()
                } else {
                    format!("{} selected image files", roots.len())
                }
            },
        );
        self.source.set_text(&source);
        self.render_rows();
    }

    fn render_rows(&self) {
        clear_children(&self.files);
        let model = self.model.borrow();
        let state_text = match model.state() {
            ImportSourceState::Error { detail } => format!("{} · {detail}", model.state().label()),
            state => state.label().to_owned(),
        };
        self.status.set_text(&state_text);
        for row in model.rows() {
            let selection = gtk4::CheckButton::with_label(row.label());
            selection.set_widget_name("import-dialog-source-row-selection");
            selection.set_active(model.row_is_effectively_selected(row));
            selection.set_sensitive(!model.ignore_nonraws() || row.is_raw());
            selection.set_tooltip_text(Some(&row.path().display().to_string()));
            selection.update_property(&[Property::Label("Select import source row")]);
            let state = self.clone();
            let path = row.path().to_path_buf();
            selection.connect_toggled(move |button| {
                state
                    .model
                    .borrow_mut()
                    .set_selected(&path, button.is_active());
                state.update_import_sensitivity();
            });
            let list_row = gtk4::ListBoxRow::new();
            list_row.set_widget_name("import-dialog-source-row");
            list_row.set_child(Some(&selection));
            self.files.append(&list_row);
        }
        drop(model);
        self.update_import_sensitivity();
    }

    fn update_import_sensitivity(&self) {
        let has_selection = !self.model.borrow().effective_paths().is_empty();
        self.import.set_sensitive(has_selection);
        if has_selection {
            let count = self.model.borrow().effective_paths().len();
            self.status
                .set_text(&format!("{count} source row(s) selected."));
        }
    }

    fn bump_generation(&self) -> u64 {
        let next = self.generation.get().saturating_add(1);
        self.generation.set(next);
        next
    }
}

fn discover_paths(
    roots: &[PathBuf],
    recursive: bool,
    existing: &BTreeSet<PathBuf>,
) -> Result<Vec<(PathBuf, bool)>, String> {
    let mut paths = Vec::new();
    for root in roots {
        if root.is_dir() {
            collect_directory(root, recursive, &mut paths)?;
        } else if root.is_file() {
            paths.push(root.clone());
        }
    }
    paths.sort();
    paths.dedup();
    paths.truncate(MAX_IMPORT_SOURCE_ROWS);
    Ok(paths
        .into_iter()
        .map(|path| {
            let new = !existing.contains(&path);
            (path, new)
        })
        .collect())
}

fn collect_directory(
    root: &Path,
    recursive: bool,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let entries =
        std::fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("{}: {error}", root.display()))?
            .path();
        if path.is_dir() && recursive {
            collect_directory(&path, true, output)?;
        } else if path.is_file() {
            output.push(path);
            if output.len() >= MAX_IMPORT_SOURCE_ROWS {
                break;
            }
        }
    }
    Ok(())
}

fn places_pane() -> (gtk4::Box, Vec<(gtk4::ListBoxRow, ImportPlace)>) {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    root.set_widget_name("import-dialog-places");
    let heading = gtk4::Label::new(Some("Places"));
    heading.set_halign(gtk4::Align::Start);
    heading.add_css_class("dt_module_title");
    root.append(&heading);
    let recent = gtk4::ListBox::new();
    recent.set_widget_name("import-dialog-folders");
    recent.set_selection_mode(gtk4::SelectionMode::None);
    let places = typed_places();
    let mut rows = Vec::new();
    for place in places {
        let row = gtk4::ListBoxRow::new();
        row.set_widget_name("import-dialog-place-row");
        let label = gtk4::Label::new(Some(place.label()));
        label.set_halign(gtk4::Align::Start);
        label.set_margin_top(4);
        label.set_margin_bottom(4);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        row.set_child(Some(&label));
        recent.append(&row);
        rows.push((row, place));
    }
    root.append(&recent);
    (root, rows)
}

fn typed_places() -> Vec<ImportPlace> {
    let mut places = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if home.is_dir() {
            places.push(ImportPlace::new("Home", home.clone(), false));
        }
        let pictures = home.join("Pictures");
        if pictures.is_dir() {
            places.push(ImportPlace::new("Pictures", pictures, false));
        }
    }
    let mut recent_paths = BTreeSet::new();
    for info in gtk4::RecentManager::default().items() {
        if !info.exists() || !info.is_local() {
            continue;
        }
        let file = gtk4::gio::File::for_uri(info.uri().as_str());
        let Some(path) = file.path() else { continue };
        let location = if path.is_dir() {
            path
        } else {
            path.parent().map_or(path.clone(), Path::to_path_buf)
        };
        if location.is_dir() && recent_paths.insert(location.clone()) {
            places.push(ImportPlace::new(
                info.display_name().as_str(),
                location,
                true,
            ));
        }
        if recent_paths.len() >= 8 {
            break;
        }
    }
    places
}

fn button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(label)]);
    button
}

fn option_buttons() -> (
    gtk4::CheckButton,
    gtk4::CheckButton,
    gtk4::CheckButton,
    gtk4::Box,
) {
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
    (select_new, recursive, ignore_nonraws, options)
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
    #[cfg(target_os = "linux")]
    use std::cell::RefCell;
    #[cfg(target_os = "linux")]
    use std::fs;
    #[cfg(target_os = "linux")]
    use std::rc::Rc;
    #[cfg(target_os = "linux")]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(target_os = "linux")]
    use gtk4::prelude::ButtonExt;

    use super::{IMPORT_DIALOG_FOCUS_ORDER, IMPORT_DIALOG_WIDGET_IDS};
    #[cfg(target_os = "linux")]
    use crate::import::ImportAction;
    use crate::import::ImportRequest;

    #[cfg(target_os = "linux")]
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn import_dialog_contract_keeps_typed_source_rows_and_actions() {
        for id in [
            "import-dialog-select-files",
            "import-dialog-select-folder",
            "import-dialog-source-row",
            "import-dialog-select-new",
            "import-dialog-ignore-nonraws",
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
            vec![std::path::PathBuf::from("Pictures/photo.jpg")],
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

    #[cfg(target_os = "linux")]
    #[test]
    fn gtk_source_selection_emits_the_typed_import_action() {
        if gtk4::init().is_err() {
            return;
        }
        let application = gtk4::Application::new(
            Some("com.cgasgarth.rusttable.test.import-dialog"),
            Default::default(),
        );
        let parent = gtk4::ApplicationWindow::new(&application);
        let dialog = super::super::dialog::ImportDialog::new(&parent);
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let source = std::env::temp_dir().join(format!(
            "rusttable-import-dialog-{}/deterministic-xpro2.raf",
            number
        ));
        fs::create_dir_all(source.parent().expect("fixture parent")).expect("fixture parent");
        fs::write(&source, b"synthetic fixture").expect("fixture source");

        let received = Rc::new(RefCell::new(None));
        dialog.connect_action({
            let received = Rc::clone(&received);
            move |action| {
                received.replace(Some(action));
            }
        });
        dialog.set_source_roots(std::slice::from_ref(&source));
        assert_eq!(
            dialog.model.borrow().effective_paths(),
            vec![source.clone()]
        );

        dialog.import.emit_clicked();

        let action = received.borrow().clone().expect("typed import action");
        let ImportAction::Import(request) = action else {
            panic!("source selection must emit ImportAction::Import");
        };
        assert_eq!(request.paths(), std::slice::from_ref(&source));
        assert!(request.select_new());
        assert!(!request.recursive());
        assert!(!request.ignore_nonraws());
        assert_eq!(request.generation(), 1);

        fs::remove_dir_all(
            source
                .parent()
                .and_then(std::path::Path::parent)
                .expect("fixture directory"),
        )
        .expect("remove fixture directory");
    }
}
