use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::rc::Rc;

use crate::diagnostics::AppDiagnostics;
use rusttable_ui::{ImportRequest, is_raw_path};

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
    diagnostics: &AppDiagnostics,
) {
    let existing = active_catalog
        .borrow()
        .as_ref()
        .map(|catalog| catalog.borrow().existing_source_paths())
        .unwrap_or_default();
    let paths = effective_import_paths(
        expand_import_paths(request.paths(), request.recursive()),
        request,
        &existing,
    );
    if paths.is_empty() {
        return;
    }
    let delivery = native_bridge.borrow_mut().receive_paths(paths);
    if let Some(open_request) = delivery.request().cloned() {
        dispatch_open_request(
            &open_request,
            active_shell,
            active_catalog,
            active_collection,
            diagnostics,
        );
    }
}

fn effective_import_paths(
    paths: Vec<PathBuf>,
    request: &ImportRequest,
    existing: &BTreeSet<PathBuf>,
) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter(|path| !request.ignore_nonraws() || is_raw_path(path))
        .filter(|path| !request.select_new() || !existing.contains(path))
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_import_paths_apply_new_and_raw_filters_before_dispatch() {
        let paths = vec![
            PathBuf::from("new.nef"),
            PathBuf::from("new.jpg"),
            PathBuf::from("old.arw"),
        ];
        let existing = BTreeSet::from([PathBuf::from("old.arw")]);
        let request = ImportRequest::new(paths.clone(), false, true, true, 3).expect("request");
        assert_eq!(
            effective_import_paths(paths, &request, &existing)
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["new.nef"]
        );
    }
}
