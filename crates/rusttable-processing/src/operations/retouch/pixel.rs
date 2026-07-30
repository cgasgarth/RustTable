#![allow(clippy::missing_errors_doc)]

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetouchPixel {
    pub(crate) channels: [f32; 3],
}

impl RetouchPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self {
            channels: [red, green, blue],
        }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 3] {
        self.channels
    }

    #[must_use]
    pub(crate) fn add(self, other: Self) -> Self {
        Self::new(
            self.channels[0] + other.channels[0],
            self.channels[1] + other.channels[1],
            self.channels[2] + other.channels[2],
        )
    }

    #[must_use]
    pub(crate) fn sub(self, other: Self) -> Self {
        Self::new(
            self.channels[0] - other.channels[0],
            self.channels[1] - other.channels[1],
            self.channels[2] - other.channels[2],
        )
    }

    #[must_use]
    pub(crate) fn scale(self, value: f32) -> Self {
        Self::new(
            self.channels[0] * value,
            self.channels[1] * value,
            self.channels[2] * value,
        )
    }

    #[must_use]
    pub(crate) fn mix(self, other: Self, value: f32) -> Self {
        self.scale(1.0 - value).add(other.scale(value))
    }
}
