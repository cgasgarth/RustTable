use std::path::PathBuf;

use iced::Task;
use rusttable_core::PhotoId;
use rusttable_ui::presentation::BasicEditValues;

use crate::workspace::{BasicEditCommand, preview_loader::load_selected_edit};

use super::{Message, Shell};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditLoadResult {
    Ready(BasicEditValues),
    Failed,
}

pub(super) fn start_load(catalog_path: Option<PathBuf>, photo_id: PhotoId) -> Task<Message> {
    Task::perform(
        async move {
            catalog_path
                .as_deref()
                .and_then(|path| load_selected_edit(path, photo_id).ok())
                .and_then(|edit| BasicEditCommand::from_edit(&edit).ok())
                .map_or(EditLoadResult::Failed, |command| {
                    let values = command.values();
                    EditLoadResult::Ready(BasicEditValues::from_finite(
                        rusttable_core::FiniteF64::new(values.exposure_stops())
                            .expect("validated edit value is finite"),
                        rusttable_core::FiniteF64::new(values.rgb_red())
                            .expect("validated edit value is finite"),
                        rusttable_core::FiniteF64::new(values.rgb_green())
                            .expect("validated edit value is finite"),
                        rusttable_core::FiniteF64::new(values.rgb_blue())
                            .expect("validated edit value is finite"),
                    ))
                })
        },
        move |result| Message::EditLoaded { photo_id, result },
    )
}

pub(super) fn apply_loaded(shell: &mut Shell, photo_id: PhotoId, result: &EditLoadResult) {
    if let EditLoadResult::Ready(values) = result {
        shell.ui.set_basic_edit_values(photo_id, *values);
    }
}
