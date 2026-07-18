use iced::widget::{column, container, row, scrollable, text};
use iced::{Element, Length};
use rusttable_core::product_name;

use crate::action_button::{action_button, sized_action_button};
use crate::app::{Message, Shell};
use crate::input::FocusTarget;
use crate::library::LibraryState;
use crate::navigation::{NavigationIntent, WorkspaceRoute};
use crate::presentation::{PhotoCardViewModel, PhotoDetailViewModel};
use crate::theme::{
    CONTENT_PADDING, HEADER_HEIGHT, PHOTO_CARD_HEIGHT, PHOTO_CARD_WIDTH, PHOTO_GRID_COLUMNS,
    PHOTO_GRID_SPACING, REGION_SPACING, SIDEBAR_WIDTH,
};

pub(crate) fn view(shell: &Shell) -> Element<'_, Message> {
    let toggle_label = if shell.sidebar_visible() {
        "Hide sidebar"
    } else {
        "Show sidebar"
    };
    let header = container(
        row![
            text(product_name()),
            action_button(
                text(toggle_label),
                Message::ToggleSidebar,
                shell.is_focused(FocusTarget::SidebarToggle),
            )
        ]
        .spacing(REGION_SPACING),
    )
    .width(Length::Fill)
    .height(Length::Fixed(HEADER_HEIGHT));
    let workspace_content = match shell.route() {
        WorkspaceRoute::Library => library_content(shell),
        WorkspaceRoute::PhotoDetail(photo_id) => detail_content(shell, photo_id),
    };
    let workspace = container(column![text("Workspace"), workspace_content])
        .width(Length::Fill)
        .height(Length::Fill);
    let body = if shell.sidebar_visible() {
        row![
            container(column![
                text("Sidebar"),
                action_button(
                    text("Library"),
                    Message::Navigate(NavigationIntent::ShowLibrary),
                    shell.is_focused(FocusTarget::Library),
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
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(CONTENT_PADDING)
        .spacing(REGION_SPACING)
        .into()
}

fn library_content(shell: &Shell) -> Element<'_, Message> {
    match shell.library_state() {
        LibraryState::Loading => column![text("Library"), text("Loading library")].into(),
        LibraryState::Empty => column![text("Library"), text("No photos in this catalog")].into(),
        LibraryState::Ready(workspace) => ready_library_content(shell, workspace),
        LibraryState::Failed(kind) => {
            let projection = kind.projection();
            column![
                text("Library"),
                text(projection.title()),
                text(projection.detail()),
            ]
            .into()
        }
    }
}

fn ready_library_content<'a>(
    shell: &'a Shell,
    workspace: &'a crate::presentation::PhotoWorkspaceViewModel,
) -> Element<'a, Message> {
    let cards: Vec<_> = workspace.cards().collect();
    if cards.is_empty() {
        return column![text("Library"), text("No photos in this catalog")].into();
    }

    let rows = cards.chunks(PHOTO_GRID_COLUMNS).map(|cards| {
        cards
            .iter()
            .fold(row![], |row, card| row.push(photo_card(shell, card)))
            .spacing(PHOTO_GRID_SPACING)
            .into()
    });
    let grid = scrollable(column(rows).spacing(PHOTO_GRID_SPACING))
        .width(Length::Fill)
        .height(Length::Fill);

    column![text("Library"), grid].into()
}

fn photo_card<'a>(shell: &Shell, card: &'a PhotoCardViewModel) -> Element<'a, Message> {
    let mut content = column![text("Preview unavailable"), text(card.title().as_str())];
    if let Some(secondary) = card.secondary() {
        content = content.push(text(secondary.as_str()));
    }

    sized_action_button(
        content,
        Message::Navigate(NavigationIntent::ShowPhoto(card.id())),
        shell.is_focused(FocusTarget::PhotoCard(card.id())),
        Length::Fixed(PHOTO_CARD_WIDTH),
        Length::Fixed(PHOTO_CARD_HEIGHT),
    )
}

fn detail_content(shell: &Shell, photo_id: rusttable_core::PhotoId) -> Element<'_, Message> {
    let back = action_button(
        text("Back to library"),
        Message::Navigate(NavigationIntent::ShowLibrary),
        shell.is_focused(FocusTarget::BackToLibrary),
    );
    let Some(workspace) = shell.library_state().ready_workspace() else {
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

    detail_view(detail, back)
}

fn detail_view<'a>(
    detail: &'a PhotoDetailViewModel,
    back: Element<'a, Message>,
) -> Element<'a, Message> {
    let facts = column(detail.facts().map(|fact| {
        row![text(fact.label().as_str()), text(fact.value().as_str())]
            .spacing(REGION_SPACING)
            .into()
    }))
    .spacing(REGION_SPACING);

    column![
        text("Photo detail"),
        text(detail.title().as_str()),
        facts,
        back
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use iced_test::Simulator;
    use iced_test::core::{Settings, Size};
    use rusttable_core::PhotoId;

    use super::view;
    use crate::app::{Message, Shell, update};
    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::navigation::NavigationIntent;
    use crate::presentation::{
        PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    };

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

    #[test]
    fn empty_library_projection_is_explicit() -> Result<(), iced_test::Error> {
        let shell = Shell::default();
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

        simulator.find("Workspace")?;
        simulator.find("Library")?;
        simulator.find("No photos in this catalog")?;

        Ok(())
    }

    #[test]
    fn missing_detail_projection_is_explicit() -> Result<(), iced_test::Error> {
        let photo_id = PhotoId::new(99).expect("test photo ID is non-zero");
        let mut shell = Shell::default();
        let _ = update(
            &mut shell,
            Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
        );
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

        simulator.find("Photo detail")?;
        simulator.find(photo_id.to_string().as_str())?;
        simulator.find("Photo unavailable")?;
        simulator.find("Back to library")?;

        Ok(())
    }

    #[test]
    fn loading_library_projection_has_no_photo_cards() -> Result<(), iced_test::Error> {
        let shell = Shell::with_library_state(LibraryState::Loading);
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

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
            let shell = Shell::with_library_state(LibraryState::Failed(kind));
            let mut simulator =
                Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

            simulator.find("Library unavailable")?;
            simulator.find(detail)?;
            assert!(simulator.find("Photo 1").is_err());
            assert!(simulator.find("Loading library").is_err());
        }
        Ok(())
    }

    #[test]
    fn ready_library_projection_preserves_photo_cards() -> Result<(), iced_test::Error> {
        let shell = Shell::with_library_state(LibraryState::Ready(ready_workspace()));
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

        simulator.find("Photo 1")?;
        assert!(simulator.find("Loading library").is_err());
        assert!(simulator.find("No photos in this catalog").is_err());
        Ok(())
    }
}
