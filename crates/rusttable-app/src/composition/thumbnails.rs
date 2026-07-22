//! Generation-checked publication of the shared lighttable and filmstrip thumbnails.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
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
    requested: BTreeMap<PhotoId, u64>,
    published: BTreeSet<PhotoId>,
}

impl ThumbnailLifecycle {
    fn begin(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.requested.clear();
        self.generation
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn reconcile(&mut self, photo_ids: &[PhotoId], is_terminal: impl Fn(PhotoId) -> bool) {
        for photo_id in photo_ids {
            if is_terminal(*photo_id) {
                self.requested.remove(photo_id);
                self.published.insert(*photo_id);
            } else {
                self.published.remove(photo_id);
            }
        }
    }

    fn needs_request(&self, photo_id: PhotoId) -> bool {
        !self.requested.contains_key(&photo_id) && !self.published.contains(&photo_id)
    }

    fn invalidate(&mut self, photo_id: PhotoId) {
        self.requested.remove(&photo_id);
        self.published.remove(&photo_id);
    }

    fn request(&mut self, photo_ids: &[PhotoId], generation: u64) {
        self.published
            .retain(|photo_id| !photo_ids.contains(photo_id));
        self.requested.extend(
            photo_ids
                .iter()
                .copied()
                .map(|photo_id| (photo_id, generation)),
        );
    }

    fn publish(&mut self, photo_id: PhotoId, generation: u64) {
        if self.requested.get(&photo_id) == Some(&generation) {
            self.requested.remove(&photo_id);
            self.published.insert(photo_id);
        }
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
    let candidate_photo_ids = shell.lighttable_thumbnail_photo_ids();
    lifecycle
        .borrow_mut()
        .reconcile(&candidate_photo_ids, |photo_id| {
            shell.photo_thumbnail_has_terminal_state(photo_id)
        });
    let has_new_request = candidate_photo_ids
        .iter()
        .any(|photo_id| lifecycle.borrow().needs_request(*photo_id));
    if !has_new_request {
        return;
    }
    let photo_ids = candidate_photo_ids
        .into_iter()
        .filter(|photo_id| !lifecycle.borrow().published.contains(photo_id))
        .collect::<Vec<_>>();
    if photo_ids.is_empty() {
        return;
    }
    let generation = lifecycle.borrow_mut().begin();
    lifecycle.borrow_mut().request(&photo_ids, generation);
    for photo_id in &photo_ids {
        shell.set_photo_thumbnail_loading(*photo_id);
    }

    let (sender, receiver) = mpsc::channel();
    let worker_photo_ids = photo_ids.clone();
    let worker = thread::Builder::new()
        .name("rusttable-thumbnails".to_owned())
        .spawn(move || {
            let Ok(mut controller) = GtkThumbnailController::open(
                catalog_path,
                source_root,
                default_thumbnail_cache_root(),
            ) else {
                for photo_id in worker_photo_ids {
                    let _ = sender.send(ThumbnailWorkerMessage::Failed(photo_id));
                }
                let _ = sender.send(ThumbnailWorkerMessage::Finished);
                return;
            };
            for photo_id in worker_photo_ids {
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
        for photo_id in &photo_ids {
            shell.set_photo_thumbnail_failed(*photo_id);
            lifecycle.borrow_mut().publish(*photo_id, generation);
        }
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
                    lifecycle
                        .borrow_mut()
                        .publish(thumbnail.photo_id(), generation);
                }
                Ok(ThumbnailWorkerMessage::Failed(photo_id)) => {
                    if lifecycle.borrow().is_current(generation) {
                        shell.set_photo_thumbnail_failed(photo_id);
                        lifecycle.borrow_mut().publish(photo_id, generation);
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

/// Invalidates the active filmstrip image before scheduling the post-edit render.
pub(super) fn refresh_active_thumbnail(
    shell: &GtkShell,
    catalog: &GtkCatalogController,
    lifecycle: &Rc<RefCell<ThumbnailLifecycle>>,
) {
    if let Some(photo_id) = shell
        .darkroom_panel_target()
        .map(rusttable_ui::DarkroomPanelTarget::photo_id)
    {
        lifecycle.borrow_mut().invalidate(photo_id);
        shell.set_photo_thumbnail_loading(photo_id);
    }
    start_workspace_thumbnails(shell, catalog, lifecycle);
}

#[cfg(test)]
mod tests {
    use super::ThumbnailLifecycle;
    use rusttable_core::PhotoId;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn a_new_refresh_rejects_late_thumbnail_results() {
        let mut lifecycle = ThumbnailLifecycle::default();
        let first = lifecycle.begin();
        let second = lifecycle.begin();

        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }

    #[test]
    fn recreated_loading_tiles_reconcile_published_thumbnails_and_request_again() {
        let photo_id = id(1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[photo_id], generation);

        lifecycle.publish(photo_id, generation);
        assert!(!lifecycle.needs_request(photo_id));

        lifecycle.reconcile(&[photo_id], |_| false);
        assert!(lifecycle.needs_request(photo_id));

        let next_generation = lifecycle.begin();
        lifecycle.request(&[photo_id], next_generation);
        lifecycle.publish(photo_id, generation);
        assert!(!lifecycle.needs_request(photo_id));
        lifecycle.publish(photo_id, next_generation);
        assert!(!lifecycle.needs_request(photo_id));
    }

    #[test]
    fn late_publication_cannot_complete_a_new_request_for_the_same_photo() {
        let photo_id = id(2);
        let mut lifecycle = ThumbnailLifecycle::default();
        let first_generation = lifecycle.begin();
        lifecycle.request(&[photo_id], first_generation);

        let second_generation = lifecycle.begin();
        lifecycle.request(&[photo_id], second_generation);
        lifecycle.publish(photo_id, first_generation);
        assert_eq!(lifecycle.requested.get(&photo_id), Some(&second_generation));
        assert!(!lifecycle.published.contains(&photo_id));

        lifecycle.publish(photo_id, second_generation);
        assert!(!lifecycle.needs_request(photo_id));
    }

    #[test]
    fn invalidating_an_edited_photo_forces_a_new_thumbnail_request() {
        let photo_id = id(1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[photo_id], generation);
        lifecycle.publish(photo_id, generation);

        lifecycle.invalidate(photo_id);

        assert!(lifecycle.needs_request(photo_id));
    }
}
