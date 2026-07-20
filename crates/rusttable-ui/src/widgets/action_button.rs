use iced::advanced::{Layout, Renderer as _, Shell, Widget, layout, mouse, renderer, widget};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Background, Color, Element, Event, Length, Rectangle, Shadow, Shrink, Size};

use crate::input::{InputIntent, UiMessage};
use crate::theme;

pub(crate) struct ActionButton<'a> {
    inner: Element<'a, UiMessage>,
    action: UiMessage,
    focused: bool,
    width: Length,
    height: Length,
}

impl<'a> ActionButton<'a> {
    pub(crate) fn new(
        content: impl Into<Element<'a, UiMessage>>,
        action: UiMessage,
        focused: bool,
    ) -> Self {
        Self {
            inner: iced::widget::button(content).on_press(action).into(),
            action,
            focused,
            width: Shrink,
            height: Shrink,
        }
    }

    pub(crate) fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub(crate) fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

pub fn action_button<'a>(
    content: impl Into<Element<'a, UiMessage>>,
    action: UiMessage,
    focused: bool,
) -> Element<'a, UiMessage> {
    ActionButton::new(content, action, focused).into()
}

pub fn sized_action_button<'a>(
    content: impl Into<Element<'a, UiMessage>>,
    action: UiMessage,
    focused: bool,
    width: impl Into<Length>,
    height: impl Into<Length>,
) -> Element<'a, UiMessage> {
    ActionButton::new(content, action, focused)
        .width(width)
        .height(height)
        .into()
}

impl<'a> From<ActionButton<'a>> for Element<'a, UiMessage> {
    fn from(button: ActionButton<'a>) -> Self {
        Self::new(button)
    }
}

impl Widget<UiMessage, iced::Theme, iced::Renderer> for ActionButton<'_> {
    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::contained(limits, self.width, self.height, |limits| {
            self.inner
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, limits)
        })
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout.children().next().unwrap(),
            cursor,
            viewport,
        );
        if self.focused {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: theme::focus_outline(),
                    shadow: Shadow::default(),
                    snap: true,
                },
                Background::Color(Color::TRANSPARENT),
            );
        }
    }

    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::stateless()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::None
    }

    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.inner));
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.inner.as_widget_mut().operate(
            &mut tree.children[0],
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, UiMessage>,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            shell,
            viewport,
        );
        if shell.is_event_captured() || !self.focused {
            return;
        }

        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            repeat,
            ..
        }) = event
        else {
            return;
        };
        if *repeat {
            return;
        }

        let Some(message) = key_message(key, *modifiers, self.action) else {
            return;
        };
        shell.publish(message);
        shell.capture_event();
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }
}

fn key_message(key: &Key, modifiers: Modifiers, action: UiMessage) -> Option<UiMessage> {
    if modifiers.control() || modifiers.alt() || modifiers.command() || modifiers.logo() {
        return None;
    }

    let intent = match key {
        Key::Named(Named::Tab) => {
            if modifiers.shift() {
                InputIntent::FocusPrevious
            } else {
                InputIntent::FocusNext
            }
        }
        Key::Named(Named::Enter | Named::Space) => {
            return Some(action);
        }
        Key::Character(value) if value == " " => return Some(action),
        Key::Named(Named::ArrowDown | Named::ArrowRight) => InputIntent::FocusNextPhoto,
        Key::Character(value) if value == "]" => InputIntent::FocusNextPhoto,
        Key::Named(Named::ArrowUp | Named::ArrowLeft) => InputIntent::FocusPreviousPhoto,
        Key::Character(value) if value == "[" => InputIntent::FocusPreviousPhoto,
        Key::Named(Named::Escape) => InputIntent::Escape,
        _ => return None,
    };
    Some(UiMessage::Input(intent))
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Key, Modifiers, key::Named};

    use super::key_message;
    use crate::input::{InputIntent, UiMessage};

    #[test]
    fn supported_keys_map_once_and_modifiers_are_filtered() {
        let action = UiMessage::ToggleSidebar;
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::default(), action),
            Some(UiMessage::Input(InputIntent::FocusNext))
        );
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::SHIFT, action),
            Some(UiMessage::Input(InputIntent::FocusPrevious))
        );
        assert_eq!(
            key_message(&Key::Named(Named::Enter), Modifiers::default(), action),
            Some(action)
        );
        assert_eq!(
            key_message(&Key::Named(Named::Escape), Modifiers::default(), action),
            Some(UiMessage::Input(InputIntent::Escape))
        );
        assert_eq!(
            key_message(&Key::Named(Named::ArrowDown), Modifiers::default(), action),
            Some(UiMessage::Input(InputIntent::FocusNextPhoto))
        );
        assert_eq!(
            key_message(&Key::Named(Named::ArrowUp), Modifiers::default(), action),
            Some(UiMessage::Input(InputIntent::FocusPreviousPhoto))
        );
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::CTRL, action),
            None
        );
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::ALT, action),
            None
        );
    }
}
