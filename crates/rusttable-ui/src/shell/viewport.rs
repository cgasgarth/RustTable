use iced::widget::shader::{self, Pipeline, Primitive, Shader};
use iced::{Element, Rectangle, Size, mouse};

use super::model::UiMessage;
use super::tasks::TaskGeneration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureHandle(u64);

impl TextureHandle {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationReceipt {
    generation: TaskGeneration,
    texture: Option<TextureHandle>,
    zero_copy_compatible: bool,
}

impl PresentationReceipt {
    #[must_use]
    pub const fn new(
        generation: TaskGeneration,
        texture: Option<TextureHandle>,
        zero_copy_compatible: bool,
    ) -> Self {
        Self {
            generation,
            texture,
            zero_copy_compatible,
        }
    }

    #[must_use]
    pub const fn generation(self) -> TaskGeneration {
        self.generation
    }

    #[must_use]
    pub const fn texture(self) -> Option<TextureHandle> {
        self.texture
    }

    #[must_use]
    pub const fn is_zero_copy_compatible(self) -> bool {
        self.zero_copy_compatible
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportFailure {
    DeviceLost,
    IncompatibleTexture,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ViewportState {
    generation: TaskGeneration,
    size: Size,
    scissor: Rectangle,
    receipt: Option<PresentationReceipt>,
    failure: Option<ViewportFailure>,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            generation: TaskGeneration::zero(),
            size: Size::new(1.0, 1.0),
            scissor: Rectangle::with_size(Size::new(1.0, 1.0)),
            receipt: None,
            failure: None,
        }
    }
}

impl ViewportState {
    pub fn resize(&mut self, size: Size) {
        self.size = Size::new(size.width.max(1.0), size.height.max(1.0));
        self.scissor = Rectangle::with_size(self.size);
    }

    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub const fn scissor(&self) -> Rectangle {
        self.scissor
    }

    #[must_use]
    pub const fn receipt(&self) -> Option<PresentationReceipt> {
        self.receipt
    }

    #[must_use]
    pub const fn failure(&self) -> Option<ViewportFailure> {
        self.failure
    }

    pub fn present(&mut self, receipt: PresentationReceipt) -> bool {
        if receipt.generation() < self.generation {
            return false;
        }
        if receipt.texture().is_none() && !receipt.is_zero_copy_compatible() {
            self.failure = Some(ViewportFailure::IncompatibleTexture);
            return false;
        }
        self.generation = receipt.generation();
        self.receipt = Some(receipt);
        self.failure = None;
        true
    }

    pub fn device_lost(&mut self) {
        self.failure = Some(ViewportFailure::DeviceLost);
        self.receipt = None;
    }
}

#[derive(Debug, Clone)]
pub struct ViewportProgram {
    state: ViewportState,
}

impl ViewportProgram {
    #[must_use]
    pub const fn new(state: ViewportState) -> Self {
        Self { state }
    }
}

#[derive(Debug, Default)]
pub struct ViewportWidgetState;

#[derive(Debug, Clone, Copy)]
pub struct ViewportPrimitive {
    receipt: Option<PresentationReceipt>,
    scissor: Rectangle,
    placeholder: bool,
}

#[derive(Debug, Default)]
pub struct ViewportPipeline;

impl Pipeline for ViewportPipeline {
    fn new(
        _device: &iced::wgpu::Device,
        _queue: &iced::wgpu::Queue,
        _format: iced::wgpu::TextureFormat,
    ) -> Self {
        Self
    }
}

impl Primitive for ViewportPrimitive {
    type Pipeline = ViewportPipeline;

    fn prepare(
        &self,
        _pipeline: &mut Self::Pipeline,
        _device: &iced::wgpu::Device,
        _queue: &iced::wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &iced::advanced::graphics::Viewport,
    ) {
        let _ = (self.receipt, self.scissor, self.placeholder);
    }
}

impl shader::Program<UiMessage> for ViewportProgram {
    type State = ViewportWidgetState;
    type Primitive = ViewportPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: mouse::Cursor,
        bounds: Rectangle,
    ) -> Self::Primitive {
        ViewportPrimitive {
            receipt: self.state.receipt(),
            scissor: Rectangle::with_size(Size::new(
                self.state.scissor().width.min(bounds.width),
                self.state.scissor().height.min(bounds.height),
            )),
            placeholder: self.state.failure().is_some(),
        }
    }
}

#[must_use]
pub fn viewport(state: ViewportState) -> Element<'static, UiMessage> {
    Shader::new(ViewportProgram::new(state))
        .width(iced::Fill)
        .height(iced::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use iced::Size;

    use super::{PresentationReceipt, TextureHandle, ViewportFailure, ViewportState};
    use crate::shell::TaskGeneration;

    #[test]
    fn viewport_rejects_stale_receipts_and_preserves_zero_copy_path() {
        let mut viewport = ViewportState::default();
        viewport.resize(Size::new(800.0, 600.0));
        assert!(viewport.present(PresentationReceipt::new(
            TaskGeneration::new(2),
            Some(TextureHandle::new(9)),
            true,
        )));
        assert!(!viewport.present(PresentationReceipt::new(
            TaskGeneration::new(1),
            Some(TextureHandle::new(8)),
            true,
        )));
        assert_eq!(
            viewport.receipt().expect("receipt").texture(),
            Some(TextureHandle::new(9))
        );
        assert_eq!(viewport.failure(), None);
    }

    #[test]
    fn viewport_device_loss_degrades_to_placeholder() {
        let mut viewport = ViewportState::default();
        viewport.device_lost();
        assert_eq!(viewport.failure(), Some(ViewportFailure::DeviceLost));
        assert_eq!(viewport.receipt(), None);
    }
}
