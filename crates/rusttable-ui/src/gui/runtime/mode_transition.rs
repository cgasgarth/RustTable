//! Shared selected-photo binding for Darktable's lighttable → darkroom transition.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rusttable_core::PhotoId;

use super::{DarkroomView, LighttableInteractionState};
use crate::presentation::PhotoDetailViewModel;

/// Binds the selected lighttable photo to the existing darkroom preview surface.
pub(super) fn sync_darkroom_selection(
    darkroom: &DarkroomView,
    interaction: &Rc<RefCell<LighttableInteractionState>>,
    photo_details: &Rc<RefCell<BTreeMap<PhotoId, PhotoDetailViewModel>>>,
) {
    let Some(photo_id) = interaction.borrow().selected_in_order().next() else {
        return;
    };
    let Some(detail) = photo_details.borrow().get(&photo_id).cloned() else {
        return;
    };
    darkroom.set_detail(&detail);
    darkroom.set_status(&format!("selected · {}", detail.title().as_str()));
}
