use iced::widget::{column, container, image, row, scrollable, text};
use iced::{ContentFit, Element, Fill, Length};
use rusttable_core::product_name;

use crate::input::{BasicEditControl, BasicEditIntent, FocusTarget, InputIntent, UiMessage};
use crate::library::LibraryState;
use crate::navigation::{NavigationIntent, WorkspaceRoute};
use crate::presentation::{
    BasicEditField, BasicEditSaveState, BasicEditValues, PhotoCardViewModel, PhotoDetailViewModel,
    Rgba8PreviewMetadata, SelectedPreviewState,
};
use crate::state::UiState;
use crate::theme::{
    CONTENT_PADDING, HEADER_HEIGHT, PHOTO_CARD_HEIGHT, PHOTO_CARD_WIDTH, PHOTO_GRID_COLUMNS,
    PHOTO_GRID_SPACING, REGION_SPACING, SIDEBAR_WIDTH,
};
use crate::widgets::{action_button, sized_action_button};

const SELECTED_PREVIEW_SURFACE_WIDTH: f32 = 320.0;
const SELECTED_PREVIEW_SURFACE_HEIGHT: f32 = 240.0;

#[must_use]
pub fn view(state: &UiState) -> Element<'_, UiMessage> {
    let toggle_label = if state.sidebar_visible() {
        "Hide sidebar"
    } else {
        "Show sidebar"
    };
    let header = container(
        row![
            text(product_name()),
            action_button(
                text(toggle_label),
                UiMessage::ToggleSidebar,
                state.is_focused(FocusTarget::SidebarToggle),
            )
        ]
        .spacing(REGION_SPACING),
    )
    .width(Fill)
    .height(Length::Fixed(HEADER_HEIGHT));
    let workspace_content = match state.route() {
        WorkspaceRoute::Library => library_content(state),
        WorkspaceRoute::PhotoDetail(photo_id) => detail_content(state, photo_id),
    };
    let workspace = container(column![text("Workspace"), workspace_content])
        .width(Fill)
        .height(Fill);
    let body = if state.sidebar_visible() {
        row![
            container(column![
                text("Sidebar"),
                action_button(
                    text("Library"),
                    UiMessage::Navigate(NavigationIntent::ShowLibrary),
                    state.is_focused(FocusTarget::Library),
                ),
            ])
            .width(Length::Fixed(SIDEBAR_WIDTH)),
            workspace,
        ]
        .spacing(REGION_SPACING)
    } else {
        row![workspace]
    };

    column![header, body]
        .width(Fill)
        .height(Fill)
        .padding(CONTENT_PADDING)
        .spacing(REGION_SPACING)
        .into()
}

fn library_content(state: &UiState) -> Element<'_, UiMessage> {
    let library = match state.library_state() {
        LibraryState::Loading => column![text("Library"), text("Loading library")].into(),
        LibraryState::Empty => column![text("Library"), text("No photos in this catalog")].into(),
        LibraryState::Ready(workspace) => ready_library_content(state, workspace),
        LibraryState::Failed(kind) => {
            let projection = kind.projection();
            column![
                text("Library"),
                text(projection.title()),
                text(projection.detail()),
                action_button(
                    text("Retry library"),
                    UiMessage::RetryLibrary,
                    state.is_focused(FocusTarget::RetryLibrary),
                ),
            ]
            .into()
        }
    };
    let mut content = column![
        action_button(
            text("Import files…"),
            UiMessage::ImportFiles,
            state.is_focused(FocusTarget::ImportFiles),
        ),
        text("Drop PNG, JPEG, or TIFF files here"),
        library,
    ]
    .spacing(REGION_SPACING);
    if state.import_panel().is_visible() {
        content = content.push(import_panel(state));
    }
    content.into()
}

fn import_panel(state: &UiState) -> Element<'_, UiMessage> {
    let panel = state.import_panel();
    let rows = panel.rows().map(|item| {
        let mut row = row![text(format!(
            "{} — {}",
            item.alias().as_str(),
            item.state().label()
        ))]
        .spacing(REGION_SPACING);
        if item.state().can_retry() {
            row = row.push(action_button(
                text(format!("Retry {}", item.alias().as_str())),
                UiMessage::RetryImport(item.item_id()),
                state.is_focused(FocusTarget::RetryImport(item.item_id())),
            ));
        }
        if !panel.active() {
            row = row.push(action_button(
                text(format!("Remove {}", item.alias().as_str())),
                UiMessage::RemoveImportResult(item.item_id()),
                state.is_focused(FocusTarget::RemoveImportResult(item.item_id())),
            ));
        }
        row.into()
    });
    let action = if panel.active() {
        action_button(
            text("Cancel import"),
            UiMessage::CancelImport,
            state.is_focused(FocusTarget::CancelImport),
        )
    } else {
        action_button(
            text("Close import results"),
            UiMessage::CloseImportPanel,
            state.is_focused(FocusTarget::CloseImportPanel),
        )
    };
    column![
        text("Import progress"),
        column(rows).spacing(REGION_SPACING),
        action
    ]
    .spacing(REGION_SPACING)
    .into()
}

fn ready_library_content<'a>(
    state: &'a UiState,
    workspace: &'a crate::presentation::PhotoWorkspaceViewModel,
) -> Element<'a, UiMessage> {
    let cards: Vec<_> = workspace.cards().collect();
    if cards.is_empty() {
        return column![text("Library"), text("No photos in this catalog")].into();
    }

    let rows = cards.chunks(PHOTO_GRID_COLUMNS).map(|cards| {
        cards
            .iter()
            .fold(row![], |row, card| row.push(photo_card(state, card)))
            .spacing(PHOTO_GRID_SPACING)
            .into()
    });
    let grid = scrollable(column(rows).spacing(PHOTO_GRID_SPACING))
        .width(Fill)
        .height(Fill);

    column![
        text("Library"),
        text(format!("{} catalog photos", cards.len())),
        grid
    ]
    .spacing(REGION_SPACING)
    .into()
}

fn photo_card<'a>(state: &UiState, card: &'a PhotoCardViewModel) -> Element<'a, UiMessage> {
    let mut content = column![text("Select to preview"), text(card.title().as_str())];
    if let Some(secondary) = card.secondary() {
        content = content.push(text(secondary.as_str()));
    }

    sized_action_button(
        content,
        UiMessage::Navigate(NavigationIntent::ShowPhoto(card.id())),
        state.is_focused(FocusTarget::PhotoCard(card.id())),
        Length::Fixed(PHOTO_CARD_WIDTH),
        Length::Fixed(PHOTO_CARD_HEIGHT),
    )
}

fn detail_content(state: &UiState, photo_id: rusttable_core::PhotoId) -> Element<'_, UiMessage> {
    let back = action_button(
        text("Back to library"),
        UiMessage::Navigate(NavigationIntent::ShowLibrary),
        state.is_focused(FocusTarget::BackToLibrary),
    );
    let Some(workspace) = state.library_state().ready_workspace() else {
        return column![
            text("Photo detail"),
            text(photo_id.to_string()),
            text("Photo unavailable"),
            back
        ]
        .into();
    };
    let Some(detail) = workspace.detail(photo_id) else {
        return column![
            text("Photo detail"),
            text(photo_id.to_string()),
            text("Photo unavailable"),
            back
        ]
        .into();
    };

    let preview = action_button(
        selected_preview_content(detail.selected_preview()),
        UiMessage::Navigate(NavigationIntent::ShowLibrary),
        state.is_focused(FocusTarget::Preview(detail.id())),
    );
    let save_rendered_copy = action_button(
        text("Save rendered copy…"),
        UiMessage::SaveRenderedCopy(detail.id()),
        state.is_focused(FocusTarget::SaveRenderedCopy(detail.id())),
    );
    let export_status = state
        .export_status(detail.id())
        .map_or_else(|| column![].into(), |status| text(status.as_str()).into());
    let inspector = basic_edit_inspector(state, detail.id());
    detail_view(
        detail,
        preview,
        save_rendered_copy,
        export_status,
        inspector,
        back,
    )
}

fn detail_view<'a>(
    detail: &'a PhotoDetailViewModel,
    preview: Element<'a, UiMessage>,
    save_rendered_copy: Element<'a, UiMessage>,
    export_status: Element<'a, UiMessage>,
    inspector: Element<'a, UiMessage>,
    back: Element<'a, UiMessage>,
) -> Element<'a, UiMessage> {
    let facts = column(detail.facts().map(|fact| {
        row![text(fact.label().as_str()), text(fact.value().as_str())]
            .spacing(REGION_SPACING)
            .into()
    }))
    .spacing(REGION_SPACING);
    let preview_and_facts = column![preview, facts].spacing(REGION_SPACING);

    let heading = row![text("Photo detail"), save_rendered_copy, back].spacing(REGION_SPACING);
    column![
        heading,
        export_status,
        text(detail.title().as_str()),
        row![preview_and_facts, inspector].spacing(REGION_SPACING),
    ]
    .into()
}

fn basic_edit_inspector(
    state: &UiState,
    photo_id: rusttable_core::PhotoId,
) -> Element<'_, UiMessage> {
    let Some(inspector) = state
        .basic_edit()
        .filter(|inspector| inspector.photo_id() == photo_id)
    else {
        return column![].into();
    };
    let values = inspector.values();
    let fields = BasicEditField::ALL
        .into_iter()
        .map(|field| basic_edit_field_row(state, values, field));
    let status = match inspector.save_state() {
        BasicEditSaveState::Clean => "No unsaved changes",
        BasicEditSaveState::Unsaved => "Unsaved edit",
        BasicEditSaveState::Saving => "Saving edit",
        BasicEditSaveState::Failed => "Save failed; unsaved edit retained",
        BasicEditSaveState::Conflict => "Edit changed elsewhere; reload or reapply your draft",
    };
    column![
        text("Basic edit inspector"),
        text("Exposure and RGB gain"),
        column(fields).spacing(REGION_SPACING),
        basic_edit_control(
            state,
            BasicEditControl::Undo,
            "Undo edit".to_owned(),
            BasicEditIntent::Undo,
        ),
        basic_edit_control(
            state,
            BasicEditControl::Redo,
            "Redo edit".to_owned(),
            BasicEditIntent::Redo,
        ),
        action_button(
            text("Reset edit"),
            UiMessage::Input(InputIntent::BasicEdit(BasicEditIntent::Reset)),
            state.is_focused(FocusTarget::BasicEdit(BasicEditControl::Reset)),
        ),
        basic_edit_control(
            state,
            BasicEditControl::Reload,
            "Reload edit".to_owned(),
            BasicEditIntent::Reload,
        ),
        basic_edit_control(
            state,
            BasicEditControl::Reapply,
            "Reapply draft".to_owned(),
            BasicEditIntent::Reapply,
        ),
        action_button(
            text("Save edit"),
            UiMessage::Input(InputIntent::BasicEdit(BasicEditIntent::Commit)),
            state.is_focused(FocusTarget::BasicEdit(BasicEditControl::Commit)),
        ),
        text(status),
    ]
    .spacing(REGION_SPACING)
    .into()
}

fn basic_edit_field_row<'a>(
    state: &UiState,
    values: BasicEditValues,
    field: BasicEditField,
) -> Element<'a, UiMessage> {
    let value = format!(
        "{} ({}): {}",
        field.label(),
        field.unit(),
        values.display_value(field)
    );
    row![
        text(value),
        basic_edit_control(
            state,
            BasicEditControl::Decrement(field),
            format!("Decrease {}", field.label()),
            BasicEditIntent::Decrement(field),
        ),
        basic_edit_control(
            state,
            BasicEditControl::Increment(field),
            format!("Increase {}", field.label()),
            BasicEditIntent::Increment(field),
        ),
    ]
    .spacing(REGION_SPACING)
    .into()
}

fn basic_edit_control<'a>(
    state: &UiState,
    control: BasicEditControl,
    label: String,
    intent: BasicEditIntent,
) -> Element<'a, UiMessage> {
    action_button(
        text(label),
        UiMessage::Input(InputIntent::BasicEdit(intent)),
        state.is_focused(FocusTarget::BasicEdit(control)),
    )
}

fn selected_preview_content(state: &SelectedPreviewState) -> Element<'_, UiMessage> {
    match state {
        SelectedPreviewState::Loading => column![text("Preview"), text("Loading preview")].into(),
        SelectedPreviewState::Ready(metadata) => ready_preview_content(metadata),
        SelectedPreviewState::Unavailable => {
            column![text("Preview"), text("Preview unavailable")].into()
        }
        SelectedPreviewState::Failed(failure) => column![
            text("Preview"),
            text("Preview failed"),
            text(failure.detail().as_str()),
        ]
        .into(),
    }
}

fn ready_preview_content(metadata: &Rgba8PreviewMetadata) -> Element<'_, UiMessage> {
    let dimensions = metadata.dimensions();
    let handle = image::Handle::from_rgba(
        dimensions.width(),
        dimensions.height(),
        metadata.pixels().to_vec(),
    );
    let surface = image::Image::new(handle)
        .width(Length::Fixed(SELECTED_PREVIEW_SURFACE_WIDTH))
        .height(Length::Fixed(SELECTED_PREVIEW_SURFACE_HEIGHT))
        .content_fit(ContentFit::Contain);

    column![
        text("Preview"),
        text("RGBA8 preview"),
        surface,
        text(format!(
            "{} × {} pixels",
            dimensions.width(),
            dimensions.height()
        )),
        text(metadata.status().as_str()),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::{Settings, Size};
    use rusttable_core::PhotoId;

    use super::view;
    use crate::input::UiMessage;
    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::navigation::NavigationIntent;
    use crate::presentation::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
        PreviewDimensions, Rgba8PreviewMetadata, SelectedPreviewFailure, SelectedPreviewState,
    };
    use crate::state::UiState;
    use crate::{ImportPanelViewModel, ImportRowState, ImportRowViewModel};

    fn text(value: &str) -> PresentationText {
        PresentationText::new(value).expect("test text is valid")
    }

    fn ready_workspace() -> PhotoWorkspaceViewModel {
        let photo_id = PhotoId::new(1).expect("test photo ID is non-zero");
        PhotoWorkspaceViewModel::new(
            vec![PhotoCardViewModel::new(photo_id, text("Photo 1"), None)],
            vec![PhotoDetailViewModel::new(
                photo_id,
                text("Photo 1"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid")
    }

    fn workspace_with_selected_preview(preview: SelectedPreviewState) -> PhotoWorkspaceViewModel {
        let photo_id = PhotoId::new(1).expect("test photo ID is non-zero");
        PhotoWorkspaceViewModel::new(
            vec![PhotoCardViewModel::new(photo_id, text("Photo 1"), None)],
            vec![
                PhotoDetailViewModel::new(photo_id, text("Photo 1"), Vec::new())
                    .with_selected_preview(preview),
            ],
        )
        .expect("test workspace is valid")
    }

    fn selected_photo_state(preview: SelectedPreviewState) -> UiState {
        let photo_id = PhotoId::new(1).expect("test photo ID is non-zero");
        let mut state = UiState::with_photo_workspace(workspace_with_selected_preview(preview));
        let _ = state.handle(UiMessage::Navigate(NavigationIntent::ShowPhoto(photo_id)));
        state
    }

    #[test]
    fn empty_library_projection_is_explicit() -> Result<(), iced_test::Error> {
        let state = UiState::default();
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Workspace")?;
        simulator.find("Library")?;
        simulator.find("No photos in this catalog")?;

        Ok(())
    }

    #[test]
    fn missing_detail_projection_is_explicit() -> Result<(), iced_test::Error> {
        let photo_id = PhotoId::new(99).expect("test photo ID is non-zero");
        let mut state = UiState::default();
        let _ = state.handle(UiMessage::Navigate(NavigationIntent::ShowPhoto(photo_id)));
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Photo detail")?;
        simulator.find(photo_id.to_string().as_str())?;
        simulator.find("Photo unavailable")?;
        simulator.find("Back to library")?;

        Ok(())
    }

    #[test]
    fn loading_library_projection_has_no_photo_cards() -> Result<(), iced_test::Error> {
        let state = UiState::with_library_state(LibraryState::Loading);
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Loading library")?;
        assert!(simulator.find("No photos in this catalog").is_err());
        assert!(simulator.find("Photo 1").is_err());
        Ok(())
    }

    #[test]
    fn failed_library_projections_are_safe_and_card_free() -> Result<(), iced_test::Error> {
        for (kind, detail) in [
            (
                LibraryFailureKind::CatalogLocationUnavailable,
                "The catalog location is unavailable.",
            ),
            (
                LibraryFailureKind::RepositoryUnavailable,
                "The catalog repository is unavailable.",
            ),
            (
                LibraryFailureKind::CorruptPersistedCatalog,
                "The persisted catalog is corrupt.",
            ),
            (
                LibraryFailureKind::PresentationConversionFailed,
                "A catalog record could not be shown.",
            ),
        ] {
            let state = UiState::with_library_state(LibraryState::Failed(kind));
            let mut simulator =
                Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

            simulator.find("Library unavailable")?;
            simulator.find(detail)?;
            assert!(simulator.find("Photo 1").is_err());
            assert!(simulator.find("Loading library").is_err());
        }
        Ok(())
    }

    #[test]
    fn ready_library_projection_preserves_photo_cards() -> Result<(), iced_test::Error> {
        let state = UiState::with_library_state(LibraryState::Ready(ready_workspace()));
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Photo 1")?;
        assert!(simulator.find("Basic edit inspector").is_err());
        assert!(simulator.find("Loading library").is_err());
        assert!(simulator.find("No photos in this catalog").is_err());
        Ok(())
    }

    #[test]
    fn selected_preview_rgba8_surface_renders_in_photo_detail() -> Result<(), iced_test::Error> {
        let dimensions = PreviewDimensions::new(2, 1).expect("non-zero dimensions");
        let state = selected_photo_state(SelectedPreviewState::Ready(
            Rgba8PreviewMetadata::new(
                dimensions,
                text("Edited preview ready"),
                vec![255, 0, 0, 255, 0, 0, 255, 255],
            )
            .expect("valid RGBA8 pixels"),
        ));
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("RGBA8 preview")?;
        simulator.find("2 × 1 pixels")?;
        simulator.find("Edited preview ready")?;
        assert!(simulator.find("Preview unavailable").is_err());
        simulator.snapshot(&iced::Theme::Dark)?;
        Ok(())
    }

    #[test]
    fn selected_photo_renders_the_basic_edit_inspector() -> Result<(), iced_test::Error> {
        let state = selected_photo_state(SelectedPreviewState::Unavailable);
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Basic edit inspector")?;
        simulator.find("Exposure (stops): 0.00")?;
        simulator.find("Decrease Exposure")?;
        simulator.find("Increase Exposure")?;
        simulator.find("Red gain (gain): 1.000")?;
        simulator.find("Green gain (gain): 1.000")?;
        simulator.find("Blue gain (gain): 1.000")?;
        simulator.find("Reset edit")?;
        simulator.find("Save edit")?;
        simulator.find("No unsaved changes")?;

        Ok(())
    }

    #[test]
    fn selected_preview_loading_unavailable_and_failed_states_are_explicit()
    -> Result<(), iced_test::Error> {
        for (preview, expected) in [
            (SelectedPreviewState::Loading, "Loading preview"),
            (SelectedPreviewState::Unavailable, "Preview unavailable"),
            (
                SelectedPreviewState::Failed(SelectedPreviewFailure::new(text(
                    "The preview could not be decoded.",
                ))),
                "The preview could not be decoded.",
            ),
        ] {
            let state = selected_photo_state(preview);
            let mut simulator =
                Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

            simulator.find("Preview")?;
            simulator.find(expected)?;
        }
        Ok(())
    }

    #[test]
    fn raster_import_panel_exposes_progress_retry_remove_and_close_actions()
    -> Result<(), iced_test::Error> {
        let mut state = UiState::default();
        state.set_import_panel(ImportPanelViewModel::new(
            vec![ImportRowViewModel::new(
                1,
                text("photo.png"),
                ImportRowState::Failed,
            )],
            false,
        ));
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&state));

        simulator.find("Import files…")?;
        simulator.find("Drop PNG, JPEG, or TIFF files here")?;
        simulator.find("Import progress")?;
        simulator.find("photo.png — Import failed")?;
        simulator.find("Retry photo.png")?;
        simulator.find("Remove photo.png")?;
        simulator.find("Close import results")?;
        Ok(())
    }
}
