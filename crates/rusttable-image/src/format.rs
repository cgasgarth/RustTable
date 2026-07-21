#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputFormat {
    Jpeg,
    Png,
    Tiff,
    Raw,
}

impl InputFormat {
    /// Returns the extensions advertised for this decoder.
    #[must_use]
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Png => &["png"],
            Self::Tiff => &["tif", "tiff"],
            Self::Raw => &[
                "arw", "cr2", "crw", "dng", "erf", "kdc", "nef", "nrw", "orf", "pef", "raf", "raw",
                "rw2", "sr2", "srf", "x3f",
            ],
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

pub const SUPPORTED_INPUT_FORMATS: [InputFormat; 4] = [
    InputFormat::Jpeg,
    InputFormat::Png,
    InputFormat::Tiff,
    InputFormat::Raw,
];

/// All extensions advertised by the standard decoder registry in stable order.
pub const SUPPORTED_INPUT_EXTENSIONS: [&str; 21] = [
    "jpg", "jpeg", "png", "tif", "tiff", "arw", "cr2", "crw", "dng", "erf", "kdc", "nef", "nrw",
    "orf", "pef", "raf", "raw", "rw2", "sr2", "srf", "x3f",
];
