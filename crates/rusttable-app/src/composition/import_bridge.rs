use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rusttable_ui::ImportRequest;

use super::{
    CollectionController, GtkCatalogController, MacApplicationBridge, dispatch_open_request,
};

pub(super) fn dispatch_import_request(
    _shell: &rusttable_ui::GtkShell,
    native_bridge: &Rc<RefCell<MacApplicationBridge>>,
    active_shell: &Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
    active_catalog: &Rc<RefCell<Option<Rc<RefCell<GtkCatalogController>>>>>,
    active_collection: &Rc<RefCell<Option<CollectionController>>>,
    request: &ImportRequest,
) {
    let paths = expand_import_paths(request.paths(), request.recursive());
    let delivery = native_bridge.borrow_mut().receive_paths(paths);
    if let Some(open_request) = delivery.request().cloned() {
        dispatch_open_request(
            &open_request,
            active_shell,
            active_catalog,
            active_collection,
        );
    }
}

fn expand_import_paths(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_import_files(path, recursive, &mut expanded);
        } else {
            expanded.push(path.clone());
        }
    }
    expanded
}

fn collect_import_files(path: &std::path::Path, recursive: bool, output: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_import_files(&path, true, output);
            }
        } else {
            output.push(path);
        }
    }
}
