use iced::widget::{button, column, container, row, text};
use iced::{Element, Length};
use rusttable_core::product_name;

use crate::app::{Message, Shell};
use crate::navigation::{NavigationIntent, WorkspaceRoute};
use crate::theme::{CONTENT_PADDING, HEADER_HEIGHT, REGION_SPACING, SIDEBAR_WIDTH};

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
        WorkspaceRoute::Library => column![text("Workspace"), text("Library")],
        WorkspaceRoute::PhotoDetail(photo_id) => column![
            text("Workspace"),
            text("Photo detail"),
            text(photo_id.to_string()),
            button(text("Back to library"))
                .on_press(Message::Navigate(NavigationIntent::ShowLibrary)),
        ],
    };
    let workspace = container(workspace_content)
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
