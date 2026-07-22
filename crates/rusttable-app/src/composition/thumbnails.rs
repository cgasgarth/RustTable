//! Generation-checked publication of the shared lighttable and filmstrip thumbnails.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::gtk_controller::{GtkCatalogController, GtkCatalogState};
use crate::gtk_thumbnail_controller::{
    GtkThumbnail, GtkThumbnailController, default_thumbnail_cache_root,
};
use gtk4::glib::{self, ControlFlow};
use rusttable_core::PhotoId;
use rusttable_ui::GtkShell;

#[derive(Debug, Default)]
pub(super) struct ThumbnailLifecycle {
    generation: u64,
    requested: BTreeSet<PhotoId>,
}

impl ThumbnailLifecycle {
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

enum ThumbnailWorkerMessage {
    Ready(GtkThumbnail),
    Failed(PhotoId),
    Finished,
}

pub(super) fn start_workspace_thumbnails(
    shell: &GtkShell,
    catalog: &GtkCatalogController,
    lifecycle: &Rc<RefCell<ThumbnailLifecycle>>,
) {
    let GtkCatalogState::Ready(ready) = catalog.state() else {
        return;
    };
    let catalog_path = ready.location().catalog_path().to_path_buf();
    let source_root = ready.location().source_root().to_path_buf();
    let photo_ids = shell
        .lighttable_thumbnail_photo_ids()
        .into_iter()
        .filter(|photo_id| !lifecycle.borrow().requested.contains(photo_id))
        .collect::<Vec<_>>();
    if photo_ids.is_empty() {
        return;
    }
    let generation = lifecycle.borrow_mut().begin();
    lifecycle
        .borrow_mut()
        .requested
        .extend(photo_ids.iter().copied());
    for photo_id in &photo_ids {
        shell.set_photo_thumbnail_loading(*photo_id);
    }

    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("rusttable-thumbnails".to_owned())
        .spawn(move || {
            let Ok(mut controller) = GtkThumbnailController::open(
                catalog_path,
                source_root,
                default_thumbnail_cache_root(),
            ) else {
                for photo_id in photo_ids {
                    let _ = sender.send(ThumbnailWorkerMessage::Failed(photo_id));
                }
                let _ = sender.send(ThumbnailWorkerMessage::Finished);
                return;
            };
            for photo_id in photo_ids {
                let message = controller
                    .render_with_generation(photo_id, generation)
                    .map_or_else(
                        |_| ThumbnailWorkerMessage::Failed(photo_id),
                        ThumbnailWorkerMessage::Ready,
                    );
                if sender.send(message).is_err() {
                    return;
                }
            }
            let _ = sender.send(ThumbnailWorkerMessage::Finished);
        });
    if worker.is_err() {
        return;
    }

    let shell = shell.clone();
    let lifecycle = Rc::clone(lifecycle);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        loop {
            match receiver.try_recv() {
                Ok(ThumbnailWorkerMessage::Ready(thumbnail)) => {
                    if !lifecycle.borrow().is_current(generation) {
                        continue;
                    }
                    if shell
                        .set_photo_thumbnail(thumbnail.photo_id(), thumbnail.metadata())
                        .is_err()
                    {
                        shell.set_photo_thumbnail_failed(thumbnail.photo_id());
                    }
                }
                Ok(ThumbnailWorkerMessage::Failed(photo_id)) => {
                    if lifecycle.borrow().is_current(generation) {
                        shell.set_photo_thumbnail_failed(photo_id);
                    }
                }
                Ok(ThumbnailWorkerMessage::Finished) | Err(TryRecvError::Disconnected) => {
                    return ControlFlow::Break;
                }
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::ThumbnailLifecycle;

    #[test]
    fn a_new_refresh_rejects_late_thumbnail_results() {
        let mut lifecycle = ThumbnailLifecycle::default();
        let first = lifecycle.begin();
        let second = lifecycle.begin();

        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }
}
