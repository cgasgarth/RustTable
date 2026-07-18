use std::fmt;
use std::num::NonZeroU128;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdParseError {
    Empty,
    WrongLength,
    NonHex,
    NonCanonical,
    Zero,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "an ID cannot be empty",
            Self::WrongLength => "an ID must contain exactly 32 hexadecimal characters",
            Self::NonHex => "an ID must contain only hexadecimal characters",
            Self::NonCanonical => "an ID must use lowercase hexadecimal characters",
            Self::Zero => "an ID cannot be all zeroes",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for IdParseError {}

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU128);

        impl $name {
            #[must_use]
            pub const fn new(value: u128) -> Option<Self> {
                match NonZeroU128::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            #[must_use]
            pub const fn get(self) -> u128 {
                self.0.get()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{:032x}", self.get())
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_id(value)
            }
        }
    };
}

fn parse_id<T>(value: &str) -> Result<T, IdParseError>
where
    T: FromNonZero,
{
    if value.is_empty() {
        return Err(IdParseError::Empty);
    }
    if value.len() != 32 {
        return Err(IdParseError::WrongLength);
    }
    if value.bytes().any(|byte| !byte.is_ascii_hexdigit()) {
        return Err(IdParseError::NonHex);
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(IdParseError::NonCanonical);
    }
    let raw = u128::from_str_radix(value, 16).map_err(|_| IdParseError::NonHex)?;
    T::from_nonzero(NonZeroU128::new(raw).ok_or(IdParseError::Zero)?)
}

trait FromNonZero: Sized {
    fn from_nonzero(value: NonZeroU128) -> Result<Self, IdParseError>;
}

define_id!(PhotoId);
define_id!(AssetId);
define_id!(EditId);
define_id!(OperationId);

impl FromNonZero for PhotoId {
    fn from_nonzero(value: NonZeroU128) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }
}

impl FromNonZero for AssetId {
    fn from_nonzero(value: NonZeroU128) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }
}

impl FromNonZero for EditId {
    fn from_nonzero(value: NonZeroU128) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }
}

impl FromNonZero for OperationId {
    fn from_nonzero(value: NonZeroU128) -> Result<Self, IdParseError> {
        Ok(Self(value))
    }
}
