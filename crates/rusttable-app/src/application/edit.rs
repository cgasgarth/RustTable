use std::path::PathBuf;

use iced::Task;
use rusttable_core::{FiniteF64, PhotoId};
use rusttable_ui::{
    input::BasicEditIntent,
    presentation::{BasicEditField, BasicEditValues},
};

use crate::workspace::{
    BasicEditDraft, BasicEditValueError, commit_basic_edit_at_path,
    preview_loader::load_selected_edit,
};

use super::{Message, Shell, WorkspaceRoute, start_preview};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditLoadResult {
    Ready(BasicEditDraft),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditCommitResult {
    Ready(BasicEditDraft),
    Failed,
}

pub(super) fn start_load(catalog_path: Option<PathBuf>, photo_id: PhotoId) -> Task<Message> {
    Task::perform(
        async move {
            catalog_path
                .as_deref()
                .and_then(|path| load_selected_edit(path, photo_id).ok())
                .and_then(|edit| BasicEditDraft::from_edit(&edit).ok())
                .map_or(EditLoadResult::Failed, EditLoadResult::Ready)
        },
        move |result| Message::EditLoaded { photo_id, result },
    )
}

pub(super) fn apply_loaded(shell: &mut Shell, photo_id: PhotoId, result: &EditLoadResult) {
    let EditLoadResult::Ready(draft) = result else {
        return;
    };
    if draft.photo_id() == photo_id && shell.ui.route() == WorkspaceRoute::PhotoDetail(photo_id) {
        shell.basic_edit = Some(draft.clone());
        shell.ui.set_basic_edit_values(photo_id, values(draft));
    }
}

pub(super) fn handle_intent(shell: &mut Shell, intent: BasicEditIntent) -> Task<Message> {
    let Some(draft) = shell.basic_edit.as_ref() else {
        return Task::none();
    };
    let photo_id = draft.photo_id();
    if shell.ui.route() != WorkspaceRoute::PhotoDetail(photo_id) {
        return Task::none();
    }
    if matches!(intent, BasicEditIntent::Commit) {
        return start_commit(
            shell.catalog_path.as_ref().ok().cloned(),
            photo_id,
            draft.clone(),
        );
    }
    let Some(draft) = shell.basic_edit.as_mut() else {
        return Task::none();
    };
    if adjust(draft, intent).is_ok() {
        shell.ui.set_basic_edit_values(photo_id, values(draft));
    }
    Task::none()
}

pub(super) fn apply_committed(
    shell: &mut Shell,
    photo_id: PhotoId,
    result: &EditCommitResult,
) -> Task<Message> {
    let EditCommitResult::Ready(draft) = result else {
        return Task::none();
    };
    if draft.photo_id() != photo_id || shell.ui.route() != WorkspaceRoute::PhotoDetail(photo_id) {
        return Task::none();
    }
    shell.basic_edit = Some(draft.clone());
    shell.ui.set_basic_edit_values(photo_id, values(draft));
    start_preview(shell, photo_id)
}

fn start_commit(
    catalog_path: Option<PathBuf>,
    photo_id: PhotoId,
    draft: BasicEditDraft,
) -> Task<Message> {
    Task::perform(
        async move {
            catalog_path
                .as_deref()
                .and_then(|path| commit_basic_edit_at_path(path, &draft).ok())
                .and_then(|edit| BasicEditDraft::from_edit(&edit).ok())
                .map_or(EditCommitResult::Failed, EditCommitResult::Ready)
        },
        move |result| Message::EditCommitted { photo_id, result },
    )
}

fn adjust(draft: &mut BasicEditDraft, intent: BasicEditIntent) -> Result<(), BasicEditValueError> {
    match intent {
        BasicEditIntent::Increment(field) => adjust_field(draft, field, true),
        BasicEditIntent::Decrement(field) => adjust_field(draft, field, false),
        BasicEditIntent::Reset => {
            draft.set_exposure_stops(0.0)?;
            draft.set_rgb_red(1.0)?;
            draft.set_rgb_green(1.0)?;
            draft.set_rgb_blue(1.0)
        }
        BasicEditIntent::Commit => Ok(()),
    }
}

fn adjust_field(
    draft: &mut BasicEditDraft,
    field: BasicEditField,
    increase: bool,
) -> Result<(), BasicEditValueError> {
    let (current, minimum, maximum, step) = match field {
        BasicEditField::Exposure => (draft.exposure_stops(), -5.0, 5.0, 0.01),
        BasicEditField::RedGain => (draft.rgb_red(), 0.0, 2.0, 0.001),
        BasicEditField::GreenGain => (draft.rgb_green(), 0.0, 2.0, 0.001),
        BasicEditField::BlueGain => (draft.rgb_blue(), 0.0, 2.0, 0.001),
    };
    let next = (current + if increase { step } else { -step }).clamp(minimum, maximum);
    match field {
        BasicEditField::Exposure => draft.set_exposure_stops(next),
        BasicEditField::RedGain => draft.set_rgb_red(next),
        BasicEditField::GreenGain => draft.set_rgb_green(next),
        BasicEditField::BlueGain => draft.set_rgb_blue(next),
    }
}

fn values(draft: &BasicEditDraft) -> BasicEditValues {
    BasicEditValues::from_finite(
        finite(draft.exposure_stops()),
        finite(draft.rgb_red()),
        finite(draft.rgb_green()),
        finite(draft.rgb_blue()),
    )
}

fn finite(value: f64) -> FiniteF64 {
    FiniteF64::new(value).expect("validated basic edit values are finite")
}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        Edit, EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
        ParameterValue, PhotoId, Revision,
    };
    use rusttable_ui::{input::BasicEditIntent, presentation::BasicEditField};

    use super::{BasicEditDraft, adjust, values};

    fn draft() -> BasicEditDraft {
        let scalar = |value| ParameterValue::Scalar(super::finite(value));
        let edit = Edit::from_parts(
            EditId::new(1).expect("test edit ID is non-zero"),
            PhotoId::new(2).expect("test photo ID is non-zero"),
            Revision::ZERO,
            Revision::ZERO,
            [
                operation(10, "rusttable.exposure", [("stops", scalar(0.0))]),
                operation(
                    20,
                    "rusttable.rgb_gain",
                    [
                        ("red", scalar(1.0)),
                        ("green", scalar(1.0)),
                        ("blue", scalar(1.0)),
                    ],
                ),
            ],
        )
        .expect("test edit is valid");
        BasicEditDraft::from_edit(&edit).expect("test edit has a basic draft")
    }

    fn operation<const N: usize>(
        id: u128,
        key: &'static str,
        parameters: [(&'static str, ParameterValue); N],
    ) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).expect("test operation ID is non-zero"),
            OperationKey::new(key).expect("test operation key is valid"),
            true,
            OperationOpacity::ONE,
            parameters.into_iter().map(|(name, value)| {
                (
                    ParameterName::new(name).expect("test parameter name is valid"),
                    value,
                )
            }),
        )
        .expect("test operation is valid")
    }

    #[test]
    fn adjustment_clamps_at_the_domain_bounds() {
        let mut draft = draft();
        draft
            .set_exposure_stops(4.999)
            .expect("test exposure is in range");

        adjust(
            &mut draft,
            BasicEditIntent::Increment(BasicEditField::Exposure),
        )
        .expect("increment remains in range");

        assert_eq!(
            values(&draft).display_value(BasicEditField::Exposure),
            "5.00"
        );
    }

    #[test]
    fn reset_projects_the_full_domain_draft_back_to_defaults() {
        let mut draft = draft();
        draft.set_exposure_stops(2.5).expect("in range");
        draft.set_rgb_red(0.5).expect("in range");
        draft.set_rgb_green(1.5).expect("in range");
        draft.set_rgb_blue(0.25).expect("in range");

        adjust(&mut draft, BasicEditIntent::Reset).expect("defaults are valid");

        let projected = values(&draft);
        assert_eq!(projected.display_value(BasicEditField::Exposure), "0.00");
        assert_eq!(projected.display_value(BasicEditField::RedGain), "1.000");
        assert_eq!(projected.display_value(BasicEditField::GreenGain), "1.000");
        assert_eq!(projected.display_value(BasicEditField::BlueGain), "1.000");
    }
}
