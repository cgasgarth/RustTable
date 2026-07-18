#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputFormat {
    Jpeg,
    Png,
    Tiff,
}

pub const SUPPORTED_INPUT_FORMATS: [InputFormat; 3] =
    [InputFormat::Jpeg, InputFormat::Png, InputFormat::Tiff];
