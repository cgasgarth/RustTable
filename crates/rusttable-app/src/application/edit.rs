use std::path::PathBuf;

use iced::Task;
use rusttable_core::{FiniteF64, PhotoId};
use rusttable_ui::{
    input::BasicEditIntent,
    presentation::{BasicEditField, BasicEditValues},
};

use crate::workspace::{
    BasicEditCommitError, BasicEditDraft, BasicEditMutation, BasicEditSession, BasicEditValueError,
    commit_basic_edit_at_path, preview_loader::load_selected_edit,
};

use super::{Message, Shell, WorkspaceRoute, preview};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditLoadResult {
    Ready(BasicEditDraft),
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditCommitResult {
    Ready(BasicEditDraft),
    Conflict,
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
        shell.basic_edit = Some(BasicEditSession::new(draft.clone(), 128));
        shell.ui.set_basic_edit_values(photo_id, values(draft));
    }
}

pub(super) fn handle_intent(shell: &mut Shell, intent: BasicEditIntent) -> Task<Message> {
    let Some(draft) = shell.basic_edit.as_ref() else {
        return Task::none();
    };
    let photo_id = draft.draft().photo_id();
    if shell.ui.route() != WorkspaceRoute::PhotoDetail(photo_id) {
        return Task::none();
    }
    match intent {
        BasicEditIntent::Commit => {
            shell.ui.begin_basic_edit_save(photo_id);
            return start_commit(
                shell.catalog_path.as_ref().ok().cloned(),
                photo_id,
                draft.draft().clone(),
            );
        }
        BasicEditIntent::Reload => {
            return Task::batch([
                preview::start_persisted(shell, photo_id),
                start_load(shell.catalog_path.as_ref().ok().cloned(), photo_id),
            ]);
        }
        BasicEditIntent::Reapply => {
            shell.ui.begin_basic_edit_save(photo_id);
            return start_reapply(
                shell.catalog_path.as_ref().ok().cloned(),
                photo_id,
                draft.draft().clone(),
            );
        }
        BasicEditIntent::Increment(_)
        | BasicEditIntent::Decrement(_)
        | BasicEditIntent::Undo
        | BasicEditIntent::Redo
        | BasicEditIntent::Reset => {}
    }
    let (draft_values, draft_preview) = {
        let Some(session) = shell.basic_edit.as_mut() else {
            return Task::none();
        };
        if adjust(session, intent).is_err() {
            return Task::none();
        }
        (
            values(session.draft()),
            session.draft().replacement_edit().ok(),
        )
    };
    shell.ui.set_basic_edit_draft_values(photo_id, draft_values);
    draft_preview.map_or_else(Task::none, |edit| preview::start_draft(shell, &edit))
}

pub(super) fn apply_committed(
    shell: &mut Shell,
    photo_id: PhotoId,
    result: &EditCommitResult,
) -> Task<Message> {
    let draft = match result {
        EditCommitResult::Ready(draft) => draft,
        EditCommitResult::Conflict => {
            shell.ui.conflict_basic_edit_save(photo_id);
            return Task::none();
        }
        EditCommitResult::Failed => {
            shell.ui.fail_basic_edit_save(photo_id);
            return Task::none();
        }
    };
    if draft.photo_id() != photo_id || shell.ui.route() != WorkspaceRoute::PhotoDetail(photo_id) {
        return Task::none();
    }
    shell.basic_edit = Some(BasicEditSession::new(draft.clone(), 128));
    shell.ui.set_basic_edit_values(photo_id, values(draft));
    preview::start_persisted(shell, photo_id)
}

fn start_commit(
    catalog_path: Option<PathBuf>,
    photo_id: PhotoId,
    draft: BasicEditDraft,
) -> Task<Message> {
    Task::perform(
        async move { commit_at_path(catalog_path.as_deref(), &draft) },
        move |result| Message::EditCommitted { photo_id, result },
    )
}

fn start_reapply(
    catalog_path: Option<PathBuf>,
    photo_id: PhotoId,
    draft: BasicEditDraft,
) -> Task<Message> {
    Task::perform(
        async move {
            let Some(path) = catalog_path.as_deref() else {
                return EditCommitResult::Failed;
            };
            let Ok(current) = load_selected_edit(path, photo_id) else {
                return EditCommitResult::Failed;
            };
            let Ok(mut reloaded) = BasicEditDraft::from_edit(&current) else {
                return EditCommitResult::Failed;
            };
            if apply_values(&mut reloaded, &draft).is_err() {
                return EditCommitResult::Failed;
            }
            commit_at_path(Some(path), &reloaded)
        },
        move |result| Message::EditCommitted { photo_id, result },
    )
}

fn commit_at_path(
    catalog_path: Option<&std::path::Path>,
    draft: &BasicEditDraft,
) -> EditCommitResult {
    let Some(path) = catalog_path else {
        return EditCommitResult::Failed;
    };
    match commit_basic_edit_at_path(path, draft) {
        Ok(edit) => BasicEditDraft::from_edit(&edit)
            .map_or(EditCommitResult::Failed, EditCommitResult::Ready),
        Err(BasicEditCommitError::RevisionConflict { .. }) => EditCommitResult::Conflict,
        Err(_) => EditCommitResult::Failed,
    }
}

fn adjust(
    session: &mut BasicEditSession,
    intent: BasicEditIntent,
) -> Result<bool, BasicEditValueError> {
    let mutation = match intent {
        BasicEditIntent::Increment(field) => adjustment(session.draft(), field, true),
        BasicEditIntent::Decrement(field) => adjustment(session.draft(), field, false),
        BasicEditIntent::Undo => return Ok(session.undo()),
        BasicEditIntent::Redo => return Ok(session.redo()),
        BasicEditIntent::Reset => BasicEditMutation::Reset,
        BasicEditIntent::Commit | BasicEditIntent::Reload | BasicEditIntent::Reapply => {
            return Ok(false);
        }
    };
    session.apply(mutation)
}

fn apply_values(
    target: &mut BasicEditDraft,
    source: &BasicEditDraft,
) -> Result<(), BasicEditValueError> {
    target.set_exposure_stops(source.exposure_stops())?;
    target.set_rgb_red(source.rgb_red())?;
    target.set_rgb_green(source.rgb_green())?;
    target.set_rgb_blue(source.rgb_blue())
}

fn adjustment(draft: &BasicEditDraft, field: BasicEditField, increase: bool) -> BasicEditMutation {
    let (current, minimum, maximum, step) = match field {
        BasicEditField::Exposure => (draft.exposure_stops(), -5.0, 5.0, 0.01),
        BasicEditField::RedGain => (draft.rgb_red(), 0.0, 2.0, 0.001),
        BasicEditField::GreenGain => (draft.rgb_green(), 0.0, 2.0, 0.001),
        BasicEditField::BlueGain => (draft.rgb_blue(), 0.0, 2.0, 0.001),
    };
    let next = (current + if increase { step } else { -step }).clamp(minimum, maximum);
    match field {
        BasicEditField::Exposure => BasicEditMutation::SetExposureStops(next),
        BasicEditField::RedGain => BasicEditMutation::SetRgbRed(next),
        BasicEditField::GreenGain => BasicEditMutation::SetRgbGreen(next),
        BasicEditField::BlueGain => BasicEditMutation::SetRgbBlue(next),
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
    use rusttable_ui::{
        NavigationIntent, PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel,
        PresentationText, WorkspaceRoute,
        input::BasicEditIntent,
        presentation::{BasicEditField, BasicEditSaveState},
    };

    use super::{
        BasicEditDraft, BasicEditSession, EditCommitResult, EditLoadResult, Shell, adjust,
        apply_committed, apply_loaded, handle_intent, values,
    };

    fn draft(photo_id: PhotoId) -> BasicEditDraft {
        let scalar = |value| ParameterValue::Scalar(super::finite(value));
        let edit = Edit::from_parts(
            EditId::new(1).expect("test edit ID is non-zero"),
            photo_id,
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

    fn shell(photo_id: PhotoId) -> Shell {
        let workspace = PhotoWorkspaceViewModel::new(
            vec![PhotoCardViewModel::new(
                photo_id,
                PresentationText::new("Test photo").expect("test title is valid"),
                None,
            )],
            vec![PhotoDetailViewModel::new(
                photo_id,
                PresentationText::new("Test photo").expect("test title is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid");
        let mut shell = Shell::with_photo_workspace(workspace);
        let _ = super::super::update(
            &mut shell,
            super::super::Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
        );
        shell
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
        let mut draft = draft(PhotoId::new(2).expect("test photo ID is non-zero"));
        draft
            .set_exposure_stops(4.999)
            .expect("test exposure is in range");
        let mut session = BasicEditSession::new(draft, 4);

        adjust(
            &mut session,
            BasicEditIntent::Increment(BasicEditField::Exposure),
        )
        .expect("increment remains in range");

        assert_eq!(
            values(session.draft()).display_value(BasicEditField::Exposure),
            "5.00"
        );
    }

    #[test]
    fn reset_projects_the_full_domain_draft_back_to_defaults() {
        let mut draft = draft(PhotoId::new(2).expect("test photo ID is non-zero"));
        draft.set_exposure_stops(2.5).expect("in range");
        draft.set_rgb_red(0.5).expect("in range");
        draft.set_rgb_green(1.5).expect("in range");
        draft.set_rgb_blue(0.25).expect("in range");
        let mut session = BasicEditSession::new(draft, 4);

        adjust(&mut session, BasicEditIntent::Reset).expect("defaults are valid");

        let projected = values(session.draft());
        assert_eq!(projected.display_value(BasicEditField::Exposure), "0.00");
        assert_eq!(projected.display_value(BasicEditField::RedGain), "1.000");
        assert_eq!(projected.display_value(BasicEditField::GreenGain), "1.000");
        assert_eq!(projected.display_value(BasicEditField::BlueGain), "1.000");
    }

    #[test]
    fn editor_intents_mutate_the_app_owned_draft_not_only_the_ui_mirror() {
        let photo_id = PhotoId::new(2).expect("test photo ID is non-zero");
        let mut shell = shell(photo_id);
        apply_loaded(
            &mut shell,
            photo_id,
            &EditLoadResult::Ready(draft(photo_id)),
        );

        let _ = handle_intent(
            &mut shell,
            BasicEditIntent::Increment(BasicEditField::Exposure),
        );

        assert_eq!(
            shell
                .basic_edit
                .as_ref()
                .map(|session| values(session.draft()).display_value(BasicEditField::Exposure)),
            Some("0.01".to_owned())
        );
        assert_eq!(
            shell
                .ui
                .basic_edit()
                .map(|inspector| inspector.values().display_value(BasicEditField::Exposure)),
            Some("0.01".to_owned())
        );
    }

    #[test]
    fn stale_loaded_draft_cannot_replace_the_current_photo() {
        let photo_id = PhotoId::new(2).expect("test photo ID is non-zero");
        let mut shell = shell(photo_id);
        let stale = PhotoId::new(3).expect("test photo ID is non-zero");

        apply_loaded(&mut shell, stale, &EditLoadResult::Ready(draft(stale)));

        assert_eq!(shell.ui.route(), WorkspaceRoute::PhotoDetail(photo_id));
        assert!(shell.basic_edit.is_none());
    }

    #[test]
    fn undo_and_redo_reproject_the_session_draft() {
        let photo_id = PhotoId::new(2).expect("test photo ID is non-zero");
        let mut shell = shell(photo_id);
        apply_loaded(
            &mut shell,
            photo_id,
            &EditLoadResult::Ready(draft(photo_id)),
        );
        let _ = handle_intent(
            &mut shell,
            BasicEditIntent::Increment(BasicEditField::Exposure),
        );
        let _ = handle_intent(&mut shell, BasicEditIntent::Undo);

        assert_eq!(
            shell
                .ui
                .basic_edit()
                .map(|inspector| inspector.values().display_value(BasicEditField::Exposure)),
            Some("0.00".to_owned())
        );

        let _ = handle_intent(&mut shell, BasicEditIntent::Redo);
        assert_eq!(
            shell
                .ui
                .basic_edit()
                .map(|inspector| inspector.values().display_value(BasicEditField::Exposure)),
            Some("0.01".to_owned())
        );
    }

    #[test]
    fn conflict_retains_the_unsaved_draft_and_exposes_recovery_actions() {
        let photo_id = PhotoId::new(2).expect("test photo ID is non-zero");
        let mut shell = shell(photo_id);
        apply_loaded(
            &mut shell,
            photo_id,
            &EditLoadResult::Ready(draft(photo_id)),
        );
        let _ = handle_intent(
            &mut shell,
            BasicEditIntent::Increment(BasicEditField::Exposure),
        );

        let _ = apply_committed(&mut shell, photo_id, &EditCommitResult::Conflict);

        let inspector = shell.ui.basic_edit().expect("selected edit inspector");
        assert_eq!(inspector.save_state(), BasicEditSaveState::Conflict);
        assert_eq!(
            inspector.values().display_value(BasicEditField::Exposure),
            "0.01"
        );
    }
}
