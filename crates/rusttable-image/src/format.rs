#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputFormat {
    Jpeg,
    Png,
}

pub const SUPPORTED_INPUT_FORMATS: [InputFormat; 2] = [InputFormat::Jpeg, InputFormat::Png];
