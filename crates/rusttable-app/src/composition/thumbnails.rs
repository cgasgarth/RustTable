//! Generation-checked publication of the shared lighttable and filmstrip thumbnails.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::gtk_controller::{GtkCatalogController, GtkCatalogState};
use crate::gtk_thumbnail_controller::{
    GtkThumbnail, GtkThumbnailController, default_thumbnail_cache_root,
};
use gtk4::glib::{self, ControlFlow};
use rusttable_core::{EditId, PhotoId, Revision};
use rusttable_ui::GtkShell;

#[derive(Debug, Default)]
pub(crate) struct ThumbnailLifecycle {
    generation: u64,
    requested: BTreeMap<PhotoId, (ThumbnailTarget, u64)>,
    published: BTreeMap<PhotoId, ThumbnailTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ThumbnailTarget {
    photo_id: PhotoId,
    edit_id: EditId,
    edit_revision: Revision,
}

impl ThumbnailTarget {
    const fn new(photo_id: PhotoId, edit_id: EditId, edit_revision: Revision) -> Self {
        Self {
            photo_id,
            edit_id,
            edit_revision,
        }
    }
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

    fn accepts(&self, target: ThumbnailTarget, generation: u64) -> bool {
        self.is_current(generation)
            && self.requested.get(&target.photo_id) == Some(&(target, generation))
    }

    fn reconcile(
        &mut self,
        targets: &[ThumbnailTarget],
        is_terminal: impl Fn(ThumbnailTarget) -> bool,
    ) {
        for target in targets {
            if is_terminal(*target) {
                self.requested.remove(&target.photo_id);
                self.published.insert(target.photo_id, *target);
            } else {
                self.published.remove(&target.photo_id);
            }
        }
    }

    fn needs_request(&self, target: ThumbnailTarget) -> bool {
        self.requested
            .get(&target.photo_id)
            .is_none_or(|(requested, _)| *requested != target)
            && self.published.get(&target.photo_id) != Some(&target)
    }

    pub(crate) fn invalidate(&mut self, photo_id: PhotoId) {
        // Invalidate only the selected target. A shared worker may still be rendering visible
        // filmstrip neighbors, and those requests remain valid. Publication checks the exact
        // target/generation pair so a late result for this photo cannot restore stale pixels.
        self.requested.remove(&photo_id);
        self.published.remove(&photo_id);
    }

    fn request(&mut self, targets: &[ThumbnailTarget], generation: u64) {
        for target in targets {
            self.published.remove(&target.photo_id);
        }
        self.requested.extend(
            targets
                .iter()
                .map(|target| (target.photo_id, (*target, generation))),
        );
    }

    fn publish(&mut self, target: ThumbnailTarget, generation: u64) {
        if self.requested.get(&target.photo_id) == Some(&(target, generation)) {
            self.requested.remove(&target.photo_id);
            self.published.insert(target.photo_id, target);
        }
    }
}

enum ThumbnailWorkerMessage {
    Ready(GtkThumbnail),
    Failed(ThumbnailTarget),
    Finished,
}

#[expect(
    clippy::too_many_lines,
    reason = "thumbnail scheduling keeps target capture, worker publication, and stale-result rejection together"
)]
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
    let candidate_photo_ids = shell
        .lighttable_thumbnail_photo_ids()
        .into_iter()
        .collect::<Vec<_>>();
    let targets = candidate_photo_ids
        .iter()
        .filter_map(|photo_id| {
            catalog
                .current_edit(*photo_id)
                .ok()
                .flatten()
                .map(|edit| ThumbnailTarget::new(*photo_id, edit.id(), edit.revision()))
        })
        .collect::<Vec<_>>();
    lifecycle.borrow_mut().reconcile(&targets, |target| {
        shell.photo_thumbnail_has_terminal_state(target.photo_id)
            && shell.photo_thumbnail_has_edit_identity(
                target.photo_id,
                target.edit_id,
                target.edit_revision,
            )
    });
    let has_new_request = targets
        .iter()
        .any(|target| lifecycle.borrow().needs_request(*target));
    if !has_new_request {
        return;
    }
    let targets = targets
        .into_iter()
        .filter(|target| lifecycle.borrow().needs_request(*target))
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return;
    }
    let generation = lifecycle.borrow_mut().begin();
    lifecycle.borrow_mut().request(&targets, generation);
    for target in &targets {
        shell.set_photo_thumbnail_loading(target.photo_id);
    }

    let (sender, receiver) = mpsc::channel();
    let worker_targets = targets.clone();
    let worker = thread::Builder::new()
        .name("rusttable-thumbnails".to_owned())
        .spawn(move || {
            let Ok(mut controller) = GtkThumbnailController::open(
                catalog_path,
                source_root,
                default_thumbnail_cache_root(),
            ) else {
                for target in worker_targets {
                    let _ = sender.send(ThumbnailWorkerMessage::Failed(target));
                }
                let _ = sender.send(ThumbnailWorkerMessage::Finished);
                return;
            };
            for target in worker_targets {
                let message = controller
                    .render_with_generation_for_edit(
                        target.photo_id,
                        target.edit_id,
                        target.edit_revision,
                        generation,
                    )
                    .map_or_else(
                        |_| ThumbnailWorkerMessage::Failed(target),
                        ThumbnailWorkerMessage::Ready,
                    );
                if sender.send(message).is_err() {
                    return;
                }
            }
            let _ = sender.send(ThumbnailWorkerMessage::Finished);
        });
    if worker.is_err() {
        for target in &targets {
            shell.set_photo_thumbnail_failed(target.photo_id);
            lifecycle.borrow_mut().publish(*target, generation);
        }
        return;
    }

    let shell = shell.clone();
    let lifecycle = Rc::clone(lifecycle);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        loop {
            match receiver.try_recv() {
                Ok(ThumbnailWorkerMessage::Ready(thumbnail)) => {
                    let target = ThumbnailTarget::new(
                        thumbnail.photo_id(),
                        thumbnail.edit_id(),
                        thumbnail.edit_revision(),
                    );
                    if !lifecycle.borrow().accepts(target, generation) {
                        continue;
                    }
                    if shell
                        .set_photo_thumbnail_for_edit(
                            thumbnail.photo_id(),
                            thumbnail.metadata(),
                            thumbnail.edit_id(),
                            thumbnail.edit_revision(),
                        )
                        .is_err()
                    {
                        shell.set_photo_thumbnail_failed(thumbnail.photo_id());
                    }
                    lifecycle.borrow_mut().publish(target, generation);
                }
                Ok(ThumbnailWorkerMessage::Failed(target)) => {
                    if lifecycle.borrow().accepts(target, generation) {
                        shell.set_photo_thumbnail_failed(target.photo_id);
                        lifecycle.borrow_mut().publish(target, generation);
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
    use super::{ThumbnailLifecycle, ThumbnailTarget};
    use rusttable_core::{EditId, PhotoId, Revision};

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    fn target(photo_id: PhotoId, revision: u64) -> ThumbnailTarget {
        ThumbnailTarget::new(
            photo_id,
            EditId::new(9).expect("edit"),
            Revision::from_u64(revision),
        )
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
    fn invalidating_a_selected_photo_rejects_inflight_results() {
        let photo_id = id(4);
        let selected = target(photo_id, 1);
        let neighbor = target(id(5), 1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[selected, neighbor], generation);

        lifecycle.invalidate(photo_id);
        lifecycle.publish(selected, generation);

        assert!(lifecycle.is_current(generation));
        assert!(!lifecycle.accepts(selected, generation));
        assert!(lifecycle.accepts(neighbor, generation));
        assert!(lifecycle.needs_request(selected));
        assert!(!lifecycle.published.contains_key(&photo_id));
    }

    #[test]
    fn recreated_loading_tiles_reconcile_published_thumbnails_and_request_again() {
        let photo_id = id(1);
        let target = target(photo_id, 1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[target], generation);

        lifecycle.publish(target, generation);
        assert!(!lifecycle.needs_request(target));

        lifecycle.reconcile(&[target], |_| false);
        assert!(lifecycle.needs_request(target));

        let next_generation = lifecycle.begin();
        lifecycle.request(&[target], next_generation);
        lifecycle.publish(target, generation);
        assert!(!lifecycle.needs_request(target));
        lifecycle.publish(target, next_generation);
        assert!(!lifecycle.needs_request(target));
    }

    #[test]
    fn late_publication_cannot_complete_a_new_request_for_the_same_photo() {
        let photo_id = id(2);
        let target = target(photo_id, 1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let first_generation = lifecycle.begin();
        lifecycle.request(&[target], first_generation);

        let second_generation = lifecycle.begin();
        lifecycle.request(&[target], second_generation);
        lifecycle.publish(target, first_generation);
        assert_eq!(
            lifecycle.requested.get(&photo_id),
            Some(&(target, second_generation))
        );
        assert!(!lifecycle.published.contains_key(&photo_id));

        lifecycle.publish(target, second_generation);
        assert!(!lifecycle.needs_request(target));
    }

    #[test]
    fn invalidating_an_edited_photo_forces_a_new_thumbnail_request() {
        let photo_id = id(1);
        let target = target(photo_id, 1);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[target], generation);
        lifecycle.publish(target, generation);

        lifecycle.invalidate(photo_id);

        assert!(lifecycle.needs_request(target));
    }

    #[test]
    fn a_new_edit_identity_rejects_the_previous_published_thumbnail() {
        let photo_id = id(3);
        let old = target(photo_id, 4);
        let new = target(photo_id, 5);
        let mut lifecycle = ThumbnailLifecycle::default();
        let generation = lifecycle.begin();
        lifecycle.request(&[old], generation);
        lifecycle.publish(old, generation);

        lifecycle.reconcile(&[new], |_| false);
        assert!(lifecycle.needs_request(new));
        assert!(lifecycle.needs_request(old));
    }
}
