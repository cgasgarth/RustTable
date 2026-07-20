//! GTK4 realization of the display-independent `RustTable` shell model.

use gtk4::prelude::*;

use super::{ShellLayout, ShellRegion, WorkspaceRole};

/// Reusable `RustTable` GTK application window.
///
/// The caller owns the `gtk4::Application` and its main loop. This type only
/// constructs and presents one application window.
#[derive(Debug, Clone)]
pub struct GtkShell {
    window: gtk4::ApplicationWindow,
    layout: ShellLayout,
    workspace: gtk4::Stack,
}

impl GtkShell {
    /// Creates the standard `RustTable` desktop shell for an activated GTK app.
    ///
    /// GTK itself requires an initialized main-thread application. The pure
    /// [`ShellLayout`] API can be used in tests without that runtime setup.
    #[must_use]
    pub fn new(application: &gtk4::Application) -> Self {
        Self::with_layout(application, ShellLayout::default())
    }

    /// Creates the shell with an explicit initial workspace.
    #[must_use]
    pub fn with_layout(application: &gtk4::Application, layout: ShellLayout) -> Self {
        let window = gtk4::ApplicationWindow::builder()
            .application(application)
            .default_width(1_280)
            .default_height(800)
            .title("RustTable")
            .build();
        let header = header_bar();
        let workspace = workspace_stack(layout.initial_workspace());
        let sidebar = sidebar(&workspace);
        let body = gtk4::Paned::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .start_child(&sidebar)
            .end_child(&workspace)
            .resize_start_child(false)
            .shrink_start_child(false)
            .position(260)
            .build();

        window.set_titlebar(Some(&header));
        window.set_child(Some(&body));

        Self {
            window,
            layout,
            workspace,
        }
    }

    /// Presents the application window without taking ownership of GTK's loop.
    pub fn present(&self) {
        self.window.present();
    }

    /// Returns the stable layout used to construct this runtime shell.
    #[must_use]
    pub const fn layout(&self) -> ShellLayout {
        self.layout
    }

    /// Exposes the application window for the application crate to attach
    /// actions, persistence, and application-specific controllers.
    #[must_use]
    pub fn window(&self) -> &gtk4::ApplicationWindow {
        &self.window
    }

    /// Switches the central placeholder without starting or owning a GTK loop.
    pub fn show_workspace(&self, role: WorkspaceRole) {
        self.workspace.set_visible_child_name(role.stack_name());
    }
}

fn header_bar() -> gtk4::HeaderBar {
    let title = gtk4::Label::new(Some("RustTable"));
    let header = gtk4::HeaderBar::new();
    header.set_widget_name(ShellRegion::Header.identifier());
    header.set_title_widget(Some(&title));
    header.set_show_title_buttons(true);
    header
}

fn sidebar(workspace: &gtk4::Stack) -> gtk4::Box {
    let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    sidebar.set_widget_name(ShellRegion::Sidebar.identifier());
    sidebar.set_margin_top(12);
    sidebar.set_margin_bottom(12);
    sidebar.set_margin_start(12);
    sidebar.set_margin_end(12);
    sidebar.append(&gtk4::Label::new(Some("Collections")));
    for role in [WorkspaceRole::Library, WorkspaceRole::PhotoDetail] {
        let button = gtk4::Button::with_label(role.title());
        let stack = workspace.clone();
        button.connect_clicked(move |_| stack.set_visible_child_name(role.stack_name()));
        sidebar.append(&button);
    }
    sidebar
}

fn workspace_stack(initial_workspace: WorkspaceRole) -> gtk4::Stack {
    let workspace = gtk4::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .build();
    workspace.set_widget_name(ShellRegion::Workspace.identifier());

    for role in [WorkspaceRole::Library, WorkspaceRole::PhotoDetail] {
        let placeholder = workspace_placeholder(role);
        workspace.add_titled(&placeholder, Some(role.stack_name()), role.title());
    }
    workspace.set_visible_child_name(initial_workspace.stack_name());
    workspace
}

fn workspace_placeholder(role: WorkspaceRole) -> gtk4::Box {
    let placeholder = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    placeholder.set_halign(gtk4::Align::Center);
    placeholder.set_valign(gtk4::Align::Center);
    placeholder.append(&gtk4::Label::new(Some(role.title())));
    placeholder.append(&gtk4::Label::new(Some(match role {
        WorkspaceRole::Library => "Catalog browser placeholder",
        WorkspaceRole::PhotoDetail => "Photo editing placeholder",
    })));
    placeholder
}
