use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataText(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataTextError {
    Empty,
    ContainsNul,
    TooLong,
    InvalidUtf8,
}

impl MetadataText {
    /// Creates exact, nonempty UTF-8 metadata text.
    ///
    /// # Errors
    ///
    /// Returns an error for empty text, embedded NUL, or more than 4,096
    /// UTF-8 bytes.
    pub fn new(text: &str) -> Result<Self, MetadataTextError> {
        Self::from_string(text.to_owned())
    }

    /// Creates metadata text from bytes after validating UTF-8.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid UTF-8, empty text, embedded NUL, or more
    /// than 4,096 UTF-8 bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, MetadataTextError> {
        let text = String::from_utf8(bytes).map_err(|_| MetadataTextError::InvalidUtf8)?;
        Self::from_string(text)
    }

    fn from_string(text: String) -> Result<Self, MetadataTextError> {
        if text.is_empty() {
            return Err(MetadataTextError::Empty);
        }
        if text.as_bytes().contains(&0) {
            return Err(MetadataTextError::ContainsNul);
        }
        if text.len() > 4_096 {
            return Err(MetadataTextError::TooLong);
        }
        Ok(Self(text))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub const fn byte_len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Display for MetadataTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "metadata text is empty",
            Self::ContainsNul => "metadata text contains NUL",
            Self::TooLong => "metadata text exceeds 4096 UTF-8 bytes",
            Self::InvalidUtf8 => "metadata text is not valid UTF-8",
        })
    }
}

impl std::error::Error for MetadataTextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PositiveRational {
    numerator: u64,
    denominator: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositiveRationalError {
    ZeroNumerator,
    ZeroDenominator,
}

impl PositiveRational {
    /// Creates a reduced rational with positive numerator and denominator.
    ///
    /// # Errors
    ///
    /// Returns an error when either component is zero.
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, PositiveRationalError> {
        if numerator == 0 {
            return Err(PositiveRationalError::ZeroNumerator);
        }
        if denominator == 0 {
            return Err(PositiveRationalError::ZeroDenominator);
        }
        let divisor = gcd(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }
}

impl fmt::Display for PositiveRationalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroNumerator => "positive rational numerator is zero",
            Self::ZeroDenominator => "positive rational denominator is zero",
        })
    }
}

impl std::error::Error for PositiveRationalError {}

fn gcd(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    LeftTop,
    RightTop,
    RightBottom,
    LeftBottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationError {
    InvalidCode(u8),
}

impl Orientation {
    /// Converts one of the eight standard stored-orientation codes.
    ///
    /// # Errors
    ///
    /// Returns [`OrientationError::InvalidCode`] for every code outside
    /// `1..=8`.
    pub const fn from_u8(code: u8) -> Result<Self, OrientationError> {
        match code {
            1 => Ok(Self::TopLeft),
            2 => Ok(Self::TopRight),
            3 => Ok(Self::BottomRight),
            4 => Ok(Self::BottomLeft),
            5 => Ok(Self::LeftTop),
            6 => Ok(Self::RightTop),
            7 => Ok(Self::RightBottom),
            8 => Ok(Self::LeftBottom),
            invalid => Err(OrientationError::InvalidCode(invalid)),
        }
    }

    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::TopLeft => 1,
            Self::TopRight => 2,
            Self::BottomRight => 3,
            Self::BottomLeft => 4,
            Self::LeftTop => 5,
            Self::RightTop => 6,
            Self::RightBottom => 7,
            Self::LeftBottom => 8,
        }
    }
}

impl fmt::Display for OrientationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCode(code) => write!(formatter, "invalid orientation code {code}"),
        }
    }
}

impl std::error::Error for OrientationError {}
