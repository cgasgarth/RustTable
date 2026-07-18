use iced::advanced::{
    Clipboard, Layout, Renderer as _, Shell, Widget, layout, mouse, renderer, widget,
};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Background, Color, Element, Event, Length, Rectangle, Shadow, Size};

use crate::app::Message;
use crate::input::InputIntent;
use crate::theme;

pub(crate) struct ActionButton<'a> {
    inner: Element<'a, Message>,
    action: Message,
    focused: bool,
    width: Length,
    height: Length,
}

impl<'a> ActionButton<'a> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message>>,
        action: Message,
        focused: bool,
    ) -> Self {
        Self {
            inner: iced::widget::button(content).on_press(action).into(),
            action,
            focused,
            width: Length::Shrink,
            height: Length::Shrink,
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

pub(crate) fn action_button<'a>(
    content: impl Into<Element<'a, Message>>,
    action: Message,
    focused: bool,
) -> Element<'a, Message> {
    ActionButton::new(content, action, focused).into()
}

pub(crate) fn sized_action_button<'a>(
    content: impl Into<Element<'a, Message>>,
    action: Message,
    focused: bool,
    width: impl Into<Length>,
    height: impl Into<Length>,
) -> Element<'a, Message> {
    ActionButton::new(content, action, focused)
        .width(width)
        .height(height)
        .into()
}

impl<'a> From<ActionButton<'a>> for Element<'a, Message> {
    fn from(button: ActionButton<'a>) -> Self {
        Self::new(button)
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for ActionButton<'_> {
    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn size_hint(&self) -> Size<Length> {
        self.size()
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

    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(self.inner.as_widget())]
    }

    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.inner));
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
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
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

fn key_message(key: &Key, modifiers: Modifiers, action: Message) -> Option<Message> {
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
        Key::Named(Named::Escape) => InputIntent::Escape,
        _ => return None,
    };
    Some(Message::Input(intent))
}

#[cfg(test)]
mod tests {
    use iced::keyboard::{Key, Modifiers, key::Named};

    use super::key_message;
    use crate::app::Message;
    use crate::input::InputIntent;

    #[test]
    fn supported_keys_map_once_and_modifiers_are_filtered() {
        let action = Message::ToggleSidebar;
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::default(), action),
            Some(Message::Input(InputIntent::FocusNext))
        );
        assert_eq!(
            key_message(&Key::Named(Named::Tab), Modifiers::SHIFT, action),
            Some(Message::Input(InputIntent::FocusPrevious))
        );
        assert_eq!(
            key_message(&Key::Named(Named::Enter), Modifiers::default(), action),
            Some(action)
        );
        assert_eq!(
            key_message(&Key::Named(Named::Escape), Modifiers::default(), action),
            Some(Message::Input(InputIntent::Escape))
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
