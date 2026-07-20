#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputFormat {
    Jpeg,
    Png,
    Tiff,
}

impl InputFormat {
    /// Returns the extensions advertised for this decoder.
    #[must_use]
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Tiff => &["tif", "tiff"],
        }
    }

    /// Selects a decoder capability from a normalized filename extension.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        let extension = extension.trim_start_matches('.');
        SUPPORTED_INPUT_FORMATS.into_iter().find(|format| {
            format
                .extensions()
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
    }
}

pub const SUPPORTED_INPUT_FORMATS: [InputFormat; 3] =
    [InputFormat::Jpeg, InputFormat::Png, InputFormat::Tiff];

/// All extensions advertised by the standard decoder registry in stable order.
pub const SUPPORTED_INPUT_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "tif", "tiff"];
