mod catalog_preview;

pub use catalog_preview::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService};

use crate::gtk_controller::GtkCatalogController;
use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewState};
use crate::lifecycle::run_with_bootstrap;
use gtk4::gio::prelude::{ApplicationExt, ApplicationExtManual};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Error returned when GTK terminates `RustTable` unsuccessfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopRunError {
    exit_code: u8,
}

impl fmt::Display for DesktopRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "RustTable GTK4 exited with status {}",
            self.exit_code
        )
    }
}

impl std::error::Error for DesktopRunError {}

/// Starts the GTK4 `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error when GTK terminates the application with a failure exit
/// code.
pub fn run() -> Result<(), DesktopRunError> {
    let preflight = crate::platform::startup_preflight();
    run_with_bootstrap(
        rusttable_diagnostics::install,
        || {
            if !preflight.is_supported() {
                return Ok(());
            }

            let application = gtk4::Application::builder()
                .application_id("com.cgasgarth.rusttable")
                .build();
            application.connect_activate(|application| {
                let catalog_controller =
                    Rc::new(RefCell::new(GtkCatalogController::load_persisted()));
                let shell = rusttable_ui::GtkShell::new(application);
                let workspace = catalog_controller.borrow().state().workspace().cloned();
                if let Some(workspace) = workspace.as_ref() {
                    shell.set_photo_workspace(workspace);
                }
                let selection_controller = Rc::clone(&catalog_controller);
                let preview_shell = shell.clone();
                shell.set_photo_selected_handler(move |photo_id| {
                    if !selection_controller.borrow_mut().select_photo(photo_id) {
                        return;
                    }
                    let catalog = selection_controller.borrow();
                    install_selected_preview(&preview_shell, &catalog);
                });
                shell.present();
            });
            let exit_code = application.run();
            if exit_code == gtk4::glib::ExitCode::SUCCESS {
                Ok(())
            } else {
                Err(DesktopRunError {
                    exit_code: exit_code.get(),
                })
            }
        },
        |warning| eprintln!("{warning}"),
    )
}

fn install_selected_preview(shell: &rusttable_ui::GtkShell, catalog: &GtkCatalogController) {
    let preview = GtkPreviewController::new().render_selected(catalog);
    let GtkPreviewState::Ready(preview) = preview else {
        shell.darkroom_preview().clear_texture();
        return;
    };

    let Ok(dimensions) = rusttable_ui::PreviewDimensions::new(
        preview.dimensions().width(),
        preview.dimensions().height(),
    ) else {
        shell.darkroom_preview().clear_texture();
        return;
    };
    let Ok(status) = rusttable_ui::PresentationText::new("rendered") else {
        shell.darkroom_preview().clear_texture();
        return;
    };
    let Ok(metadata) =
        rusttable_ui::Rgba8PreviewMetadata::new(dimensions, status, preview.pixels().to_vec())
    else {
        shell.darkroom_preview().clear_texture();
        return;
    };
    if shell.darkroom_preview().set_rgba8(&metadata).is_err() {
        shell.darkroom_preview().clear_texture();
    }
}
