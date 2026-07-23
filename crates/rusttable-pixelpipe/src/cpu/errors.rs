use std::fmt;

use super::{CpuPixelpipeError, CpuTileAssemblyError};

impl fmt::Display for CpuPixelpipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => error.fmt(formatter),
            Self::UnsupportedInputEncoding { actual } => {
                write!(formatter, "CPU pixelpipe does not accept {actual:?} input")
            }
            Self::UnsupportedProfileTransform { profile } => write!(
                formatter,
                "CPU pixelpipe has no pure-Rust transform for authoritative ICC profile {profile}"
            ),
            Self::SourceColorPlan(message) => {
                write!(formatter, "CPU source-color plan failed: {message}")
            }
            Self::InputBridge { source } => write!(formatter, "invalid CPU input bridge: {source}"),
            Self::Evaluation { source } => {
                write!(formatter, "CPU operation evaluation failed: {source}")
            }
            Self::OutputBoundary { source } => {
                write!(formatter, "invalid CPU output boundary: {source}")
            }
            Self::TilePlan { source } => write!(formatter, "invalid CPU tile plan: {source}"),
            Self::TileAssembly { source } => {
                write!(formatter, "invalid CPU tile assembly: {source}")
            }
            Self::MaskEvaluation { source } => {
                write!(formatter, "CPU mask graph evaluation failed: {source}")
            }
            Self::MaskBinding { source } => {
                write!(formatter, "CPU mask tile binding failed: {source}")
            }
        }
    }
}

impl std::error::Error for CpuPixelpipeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled(_)
            | Self::UnsupportedInputEncoding { .. }
            | Self::UnsupportedProfileTransform { .. }
            | Self::SourceColorPlan(_) => None,
            Self::InputBridge { source } | Self::OutputBoundary { source } => Some(source),
            Self::Evaluation { source } => Some(source),
            Self::TilePlan { source } => Some(source),
            Self::TileAssembly { source } => Some(source),
            Self::MaskEvaluation { source } => Some(source),
            Self::MaskBinding { source } => Some(source),
        }
    }
}

impl fmt::Display for CpuTileAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PixelIndexOverflow => formatter.write_str("CPU tile pixel index overflowed"),
            Self::PixelIndexExceedsPlatform { index } => {
                write!(
                    formatter,
                    "CPU tile pixel index {index} exceeds this platform"
                )
            }
            Self::RowEndOverflow => formatter.write_str("CPU tile row end overflowed"),
            Self::SourceRowOutsideInput => {
                formatter.write_str("CPU tile source row is out of bounds")
            }
            Self::DestinationRowOutsideOutput => {
                formatter.write_str("CPU tile destination row is out of bounds")
            }
            Self::TileUnavailable => formatter.write_str("CPU tile grid omitted a planned tile"),
            Self::TileOutputDimensionsMismatch => {
                formatter.write_str("CPU tile output dimensions do not match its input tile")
            }
        }
    }
}

impl std::error::Error for CpuTileAssemblyError {}
