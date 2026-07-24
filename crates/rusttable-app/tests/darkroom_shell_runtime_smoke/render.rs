use gtk4::prelude::*;

pub(super) struct RenderedWidget {
    bytes: Vec<u8>,
    width: usize,
}

impl RenderedWidget {
    pub(super) fn bright_pixels(&self, bounds: gtk4::graphene::Rect) -> usize {
        self.pixels_with_channel_at_least(bounds, 128)
    }

    pub(super) fn pixels_with_channel_at_least(
        &self,
        bounds: gtk4::graphene::Rect,
        threshold: u8,
    ) -> usize {
        let left = bounds.x();
        let top = bounds.y();
        let right = left + bounds.width();
        let bottom = top + bounds.height();
        let (pixels, remainder) = self.bytes.as_chunks::<4>();
        assert!(
            remainder.is_empty(),
            "render texture must contain RGBA pixels"
        );
        pixels
            .iter()
            .enumerate()
            .filter(|(index, pixel)| {
                let x = u16::try_from(index % self.width).expect("render x fits u16");
                let y = u16::try_from(index / self.width).expect("render y fits u16");
                let x = f32::from(x);
                let y = f32::from(y);
                x >= left
                    && x < right
                    && y >= top
                    && y < bottom
                    && pixel[..3]
                        .iter()
                        .copied()
                        .max()
                        .is_some_and(|channel| channel >= threshold)
            })
            .count()
    }

    pub(super) fn pixels_with_channel_at_most(
        &self,
        bounds: gtk4::graphene::Rect,
        threshold: u8,
    ) -> usize {
        let left = bounds.x();
        let top = bounds.y();
        let right = left + bounds.width();
        let bottom = top + bounds.height();
        let (pixels, remainder) = self.bytes.as_chunks::<4>();
        assert!(
            remainder.is_empty(),
            "render texture must contain RGBA pixels"
        );
        pixels
            .iter()
            .enumerate()
            .filter(|(index, pixel)| {
                let x = u16::try_from(index % self.width).expect("render x fits u16");
                let y = u16::try_from(index / self.width).expect("render y fits u16");
                let x = f32::from(x);
                let y = f32::from(y);
                x >= left
                    && x < right
                    && y >= top
                    && y < bottom
                    && pixel[..3].iter().all(|channel| *channel <= threshold)
            })
            .count()
    }
}

pub(super) fn render_widget(widget: &gtk4::Widget) -> RenderedWidget {
    // The automated runtime smoke maps its toplevel transparently so it cannot
    // disturb the desktop. Snapshot the toplevel child directly: compositor
    // opacity is intentionally absent while the complete child paint tree and
    // allocation remain under test.
    let source = widget
        .clone()
        .downcast::<gtk4::Window>()
        .ok()
        .and_then(|window| window.child())
        .unwrap_or_else(|| widget.clone());
    let allocated_width = source.allocated_width();
    let allocated_height = source.allocated_height();
    let width = usize::try_from(allocated_width).expect("positive widget width");
    let height = usize::try_from(allocated_height).expect("positive widget height");
    assert!(width > 0 && height > 0, "widget must be allocated");
    let paintable = gtk4::WidgetPaintable::new(Some(&source));
    let snapshot = gtk4::Snapshot::new();
    paintable.snapshot(
        &snapshot,
        f64::from(allocated_width),
        f64::from(allocated_height),
    );
    let node = snapshot.to_node().expect("render node for mapped rail");
    let renderer = gtk4::gsk::CairoRenderer::new();
    renderer
        .realize(None::<&gtk4::gdk::Surface>)
        .expect("Cairo renderer");
    let width_f32 = f32::from(u16::try_from(allocated_width).expect("render width fits u16"));
    let height_f32 = f32::from(u16::try_from(allocated_height).expect("render height fits u16"));
    let viewport = gtk4::graphene::Rect::new(0.0, 0.0, width_f32, height_f32);
    let texture = renderer.render_texture(&node, Some(&viewport));
    let mut bytes = vec![0; width * height * 4];
    texture.download(&mut bytes, width * 4);
    RenderedWidget { bytes, width }
}

pub(super) fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

pub(super) fn find_widget_with_prefix(root: &gtk4::Widget, prefix: &str) -> Option<gtk4::Widget> {
    if root.widget_name().starts_with(prefix) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget_with_prefix(&current, prefix) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
