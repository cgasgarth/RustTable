use gtk4::gdk;

use super::lighttable::WorkspaceRenderHandle;
use super::{LighttableSelectionAction, SelectionModifiers};
use crate::gui::NavigationDirection;

pub(super) fn handle_lighttable_key(
    render: &WorkspaceRenderHandle,
    key: gdk::Key,
    modifiers: gdk::ModifierType,
) -> gtk4::glib::Propagation {
    let modifiers = SelectionModifiers::new(
        modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            || modifiers.contains(gdk::ModifierType::SUPER_MASK),
        modifiers.contains(gdk::ModifierType::SHIFT_MASK),
    );
    let action = match key {
        gdk::Key::Left => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::Previous,
            modifiers,
        }),
        gdk::Key::Right => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::Next,
            modifiers,
        }),
        gdk::Key::Up => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::RowPrevious,
            modifiers,
        }),
        gdk::Key::Down => Some(LighttableSelectionAction::Move {
            direction: NavigationDirection::RowNext,
            modifiers,
        }),
        gdk::Key::Escape => Some(LighttableSelectionAction::Clear),
        gdk::Key::Return | gdk::Key::KP_Enter => {
            render.open_focused();
            return gtk4::glib::Propagation::Stop;
        }
        gdk::Key::minus | gdk::Key::KP_Subtract => Some(LighttableSelectionAction::SetZoom(
            render.interaction.borrow().zoom().smaller(),
        )),
        gdk::Key::plus | gdk::Key::KP_Add => Some(LighttableSelectionAction::SetZoom(
            render.interaction.borrow().zoom().larger(),
        )),
        _ => None,
    };
    let Some(action) = action else {
        return gtk4::glib::Propagation::Proceed;
    };
    let zoom_changed = matches!(action, LighttableSelectionAction::SetZoom(_));
    let selected = match action {
        LighttableSelectionAction::Move {
            direction,
            modifiers,
        } => render.move_focus(direction, modifiers),
        action => render.interaction.borrow_mut().apply(action),
    };
    if zoom_changed {
        render.rerender_current();
    } else {
        render.sync_selection_styles();
        if selected.is_some() {
            render.focus_selected();
        }
    }
    gtk4::glib::Propagation::Stop
}
