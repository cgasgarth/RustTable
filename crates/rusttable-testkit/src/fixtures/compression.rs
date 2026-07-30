use std::fmt;
use std::io::{self, Read};

use flate2::read::GzDecoder;

use super::manifest::Compression;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecompressionReport {
    output_size: u64,
}

impl DecompressionReport {
    #[must_use]
    pub const fn output_size(self) -> u64 {
        self.output_size
    }
}

pub(crate) fn preflight(
    compression: Compression,
    bytes: &[u8],
    max_output_bytes: u64,
    max_ratio: u64,
) -> Result<DecompressionReport, CompressionError> {
    if max_output_bytes == 0 || max_ratio == 0 {
        return Err(CompressionError::ZeroLimit);
    }
    let compressed_size = u64::try_from(bytes.len()).map_err(|_| CompressionError::Overflow)?;
    let report = match compression {
        Compression::None => DecompressionReport {
            output_size: compressed_size,
        },
        Compression::Gzip => gzip_size(bytes, max_output_bytes)?,
        Compression::Zip => zip_size(bytes, max_output_bytes)?,
    };
    if report.output_size > max_output_bytes {
        return Err(CompressionError::OutputLimit {
            limit: max_output_bytes,
            actual: report.output_size,
        });
    }
    let ratio_limit = compressed_size
        .max(1)
        .checked_mul(max_ratio)
        .ok_or(CompressionError::Overflow)?;
    if report.output_size > ratio_limit {
        return Err(CompressionError::RatioLimit {
            limit: max_ratio,
            compressed: compressed_size,
            decompressed: report.output_size,
        });
    }
    Ok(report)
}

pub(crate) fn decompressed_bytes(
    compression: Compression,
    bytes: &[u8],
    max_output_bytes: u64,
) -> Result<Vec<u8>, CompressionError> {
    match compression {
        Compression::None => Ok(bytes.to_vec()),
        Compression::Gzip => {
            let read_limit = max_output_bytes
                .checked_add(1)
                .ok_or(CompressionError::Overflow)?;
            let mut decoder = GzDecoder::new(bytes);
            let mut output = Vec::new();
            decoder
                .by_ref()
                .take(read_limit)
                .read_to_end(&mut output)
                .map_err(|error| CompressionError::Io(error.to_string()))?;
            let size = u64::try_from(output.len()).map_err(|_| CompressionError::Overflow)?;
            if size > max_output_bytes {
                return Err(CompressionError::OutputLimit {
                    limit: max_output_bytes,
                    actual: size,
                });
            }
            Ok(output)
        }
        Compression::Zip => Err(CompressionError::Malformed(
            "ZIP member content is not decoded for privacy scanning",
        )),
    }
}

fn gzip_size(bytes: &[u8], limit: u64) -> Result<DecompressionReport, CompressionError> {
    let read_limit = limit.checked_add(1).ok_or(CompressionError::Overflow)?;
    let mut decoder = GzDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .by_ref()
        .take(read_limit)
        .read_to_end(&mut output)
        .map_err(|error| CompressionError::Io(error.to_string()))?;
    let output_size = u64::try_from(output.len()).map_err(|_| CompressionError::Overflow)?;
    if output_size > limit {
        return Err(CompressionError::OutputLimit {
            limit,
            actual: output_size,
        });
    }
    Ok(DecompressionReport { output_size })
}

fn zip_size(bytes: &[u8], limit: u64) -> Result<DecompressionReport, CompressionError> {
    let eocd = find_end_of_central_directory(bytes)?;
    let entry_count = u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]);
    let central_size = u32::from_le_bytes([
        bytes[eocd + 12],
        bytes[eocd + 13],
        bytes[eocd + 14],
        bytes[eocd + 15],
    ]);
    let central_offset = u32::from_le_bytes([
        bytes[eocd + 16],
        bytes[eocd + 17],
        bytes[eocd + 18],
        bytes[eocd + 19],
    ]);
    let start = usize::try_from(central_offset).map_err(|_| CompressionError::Overflow)?;
    let end = start
        .checked_add(usize::try_from(central_size).map_err(|_| CompressionError::Overflow)?)
        .ok_or(CompressionError::Overflow)?;
    if end > bytes.len() {
        return Err(CompressionError::Malformed(
            "central directory exceeds source",
        ));
    }
    let mut cursor = start;
    let mut total = 0u64;
    for _ in 0..entry_count {
        let header_end = cursor.checked_add(46).ok_or(CompressionError::Overflow)?;
        if header_end > end || bytes.get(cursor..cursor + 4) != Some(b"PK\x01\x02") {
            return Err(CompressionError::Malformed(
                "invalid central directory entry",
            ));
        }
        let flags = u16::from_le_bytes([bytes[cursor + 8], bytes[cursor + 9]]);
        if flags & 1 != 0 {
            return Err(CompressionError::EncryptedArchive);
        }
        let uncompressed = u32::from_le_bytes([
            bytes[cursor + 24],
            bytes[cursor + 25],
            bytes[cursor + 26],
            bytes[cursor + 27],
        ]);
        if uncompressed == u32::MAX {
            return Err(CompressionError::Malformed("ZIP64 sizes are not supported"));
        }
        total = total
            .checked_add(u64::from(uncompressed))
            .ok_or(CompressionError::Overflow)?;
        if total > limit {
            return Err(CompressionError::OutputLimit {
                limit,
                actual: total,
            });
        }
        let name_len = usize::from(u16::from_le_bytes([bytes[cursor + 28], bytes[cursor + 29]]));
        let extra_len = usize::from(u16::from_le_bytes([bytes[cursor + 30], bytes[cursor + 31]]));
        let comment_len = usize::from(u16::from_le_bytes([bytes[cursor + 32], bytes[cursor + 33]]));
        cursor = header_end
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or(CompressionError::Overflow)?;
        if cursor > end {
            return Err(CompressionError::Malformed(
                "central directory entry exceeds source",
            ));
        }
    }
    Ok(DecompressionReport { output_size: total })
}

fn find_end_of_central_directory(bytes: &[u8]) -> Result<usize, CompressionError> {
    let start = bytes.len().saturating_sub(65_557);
    bytes[start..]
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .map(|offset| start + offset)
        .filter(|offset| offset.checked_add(22).is_some_and(|end| end <= bytes.len()))
        .ok_or(CompressionError::Malformed("ZIP end record is missing"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompressionError {
    ZeroLimit,
    Overflow,
    Io(String),
    OutputLimit {
        limit: u64,
        actual: u64,
    },
    RatioLimit {
        limit: u64,
        compressed: u64,
        decompressed: u64,
    },
    Malformed(&'static str),
    EncryptedArchive,
}

impl fmt::Display for CompressionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => formatter.write_str("compression limits must be nonzero"),
            Self::Overflow => formatter.write_str("compression size arithmetic overflowed"),
            Self::Io(message) => {
                write!(formatter, "compressed fixture could not be read: {message}")
            }
            Self::OutputLimit { limit, actual } => write!(
                formatter,
                "decompressed size {actual} exceeds limit {limit}"
            ),
            Self::RatioLimit {
                limit,
                compressed,
                decompressed,
            } => write!(
                formatter,
                "compression ratio {decompressed}/{compressed} exceeds limit {limit}"
            ),
            Self::Malformed(message) => {
                write!(formatter, "malformed compressed fixture: {message}")
            }
            Self::EncryptedArchive => formatter.write_str("encrypted ZIP fixtures are not allowed"),
        }
    }
}

impl std::error::Error for CompressionError {}

impl From<io::Error> for CompressionError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
