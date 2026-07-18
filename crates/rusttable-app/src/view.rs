use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Length};
use rusttable_core::product_name;

use crate::app::{Message, Shell};
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
            button(text(toggle_label)).on_press(Message::ToggleSidebar)
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
                button(text("Library")).on_press(Message::Navigate(NavigationIntent::ShowLibrary)),
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
    let cards: Vec<_> = shell.photo_workspace().cards().collect();
    if cards.is_empty() {
        return column![text("Library"), text("No photos")].into();
    }

    let rows = cards.chunks(PHOTO_GRID_COLUMNS).map(|cards| {
        cards
            .iter()
            .fold(row![], |row, card| row.push(photo_card(card)))
            .spacing(PHOTO_GRID_SPACING)
            .into()
    });
    let grid = scrollable(column(rows).spacing(PHOTO_GRID_SPACING))
        .width(Length::Fill)
        .height(Length::Fill);

    column![text("Library"), grid].into()
}

fn photo_card(card: &PhotoCardViewModel) -> Element<'_, Message> {
    let mut content = column![text("Preview unavailable"), text(card.title().as_str())];
    if let Some(secondary) = card.secondary() {
        content = content.push(text(secondary.as_str()));
    }

    button(content)
        .width(Length::Fixed(PHOTO_CARD_WIDTH))
        .height(Length::Fixed(PHOTO_CARD_HEIGHT))
        .on_press(Message::Navigate(NavigationIntent::ShowPhoto(card.id())))
        .into()
}

fn detail_content(shell: &Shell, photo_id: rusttable_core::PhotoId) -> Element<'_, Message> {
    let back =
        button(text("Back to library")).on_press(Message::Navigate(NavigationIntent::ShowLibrary));
    let Some(detail) = shell.photo_workspace().detail(photo_id) else {
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
    back: iced::widget::Button<'a, Message>,
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
    use crate::navigation::NavigationIntent;

    #[test]
    fn empty_library_projection_is_explicit() -> Result<(), iced_test::Error> {
        let shell = Shell::default();
        let mut simulator =
            Simulator::with_size(Settings::default(), Size::new(800.0, 600.0), view(&shell));

        simulator.find("Workspace")?;
        simulator.find("Library")?;
        simulator.find("No photos")?;

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
}
