//! Safe `.cube` and `.3dl` readers ported from `src/iop/lut3d.c`.
//!
//! Hald PNG and GMIC/GMZ are deliberately not accepted here.  They require
//! the shared PNG decoder and the native GMIC runtime, respectively.

use std::fmt;
use std::fs;
use std::path::Path;

const MAX_TOKEN_LENGTH: usize = 50;
const CUBE_MAX_LEVEL: usize = 256;

/// A validated RGB CLUT in native R-fastest order.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    pub(crate) level: usize,
    pub(crate) values: Vec<[f32; 3]>,
}

impl Lut3d {
    pub(crate) fn new(level: usize, values: Vec<[f32; 3]>) -> Result<Self, Lut3dParseError> {
        if level < 2 {
            return Err(Lut3dParseError::InvalidLevel(level));
        }
        let expected = level
            .checked_mul(level)
            .and_then(|square| square.checked_mul(level))
            .ok_or(Lut3dParseError::ArithmeticOverflow)?;
        if values.len() != expected {
            return Err(Lut3dParseError::WrongRecordCount {
                expected,
                actual: values.len(),
            });
        }
        Ok(Self { level, values })
    }

    #[must_use]
    pub const fn level(&self) -> usize {
        self.level
    }

    #[must_use]
    pub fn values(&self) -> &[[f32; 3]] {
        &self.values
    }

    pub(crate) fn value(&self, index: usize) -> [f32; 3] {
        self.values[index]
    }

    /// Dispatches only the native text extensions implemented by this leaf.
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Lut3dParseError> {
        let path = path.as_ref();
        let extension = path.to_str().and_then(|value| {
            if value.ends_with(".cube") || value.ends_with(".CUBE") {
                Some("cube")
            } else if value.ends_with(".3dl") || value.ends_with(".3DL") {
                Some("3dl")
            } else {
                None
            }
        });
        let extension = extension.ok_or(Lut3dParseError::UnsupportedFormat)?;
        let contents = fs::read_to_string(path).map_err(Lut3dParseError::Io)?;
        match extension {
            "cube" => Self::parse_cube(&contents),
            "3dl" => Self::parse_3dl(&contents),
            _ => Err(Lut3dParseError::UnsupportedFormat),
        }
    }

    /// Parses the native `.cube` directives and R-fastest RGB records.
    pub fn parse_cube(contents: &str) -> Result<Self, Lut3dParseError> {
        let mut level = None;
        let mut expected = None;
        let mut values = Vec::new();

        for (line_index, line) in contents.lines().enumerate() {
            let line_number = line_index + 1;
            let tokens = tokenize(line, line_number)?;
            if tokens.is_empty() {
                continue;
            }

            // Native skips titles and any other line whose first token starts
            // with uppercase T before interpreting the remaining line.
            if tokens[0].starts_with('T') {
                continue;
            }

            match tokens[0] {
                "DOMAIN_MIN" => {
                    require_token_count(&tokens, 4, line_number)?;
                    for token in &tokens[1..] {
                        let value = parse_float(token, line_number)?;
                        if value != 0.0 {
                            return Err(Lut3dParseError::UnsupportedDomain {
                                directive: "DOMAIN_MIN",
                                expected: 0.0,
                            });
                        }
                    }
                }
                "DOMAIN_MAX" => {
                    require_token_count(&tokens, 4, line_number)?;
                    for token in &tokens[1..] {
                        let value = parse_float(token, line_number)?;
                        if value != 1.0 {
                            return Err(Lut3dParseError::UnsupportedDomain {
                                directive: "DOMAIN_MAX",
                                expected: 1.0,
                            });
                        }
                    }
                }
                "LUT_1D_SIZE" => return Err(Lut3dParseError::OneDimensionalLut),
                "LUT_3D_SIZE" => {
                    require_token_count(&tokens, 2, line_number)?;
                    if level.is_some() {
                        return Err(Lut3dParseError::DuplicateSize(line_number));
                    }
                    let parsed = parse_usize(tokens[1], line_number)?;
                    if !(2..=CUBE_MAX_LEVEL).contains(&parsed) {
                        return Err(Lut3dParseError::InvalidLevel(parsed));
                    }
                    let record_count = checked_cube_size(parsed)?;
                    let mut reserved = Vec::new();
                    reserved
                        .try_reserve_exact(record_count)
                        .map_err(|_| Lut3dParseError::AllocationFailure)?;
                    values = reserved;
                    level = Some(parsed);
                    expected = Some(record_count);
                }
                _ if tokens.len() == 3 => {
                    let expected = expected.ok_or(Lut3dParseError::SizeNotDefined(line_number))?;
                    if values.len() >= expected {
                        return Err(Lut3dParseError::ExtraRecords(line_number));
                    }
                    values.push([
                        parse_sample(tokens[0], line_number)?,
                        parse_sample(tokens[1], line_number)?,
                        parse_sample(tokens[2], line_number)?,
                    ]);
                }
                _ => {
                    // Match the native parser's tolerance for non-data
                    // metadata lines whose token count is not three.
                }
            }
        }

        let level = level.ok_or(Lut3dParseError::SizeNotDefined(0))?;
        let expected = expected.ok_or(Lut3dParseError::SizeNotDefined(0))?;
        if values.len() != expected {
            return Err(Lut3dParseError::WrongRecordCount {
                expected,
                actual: values.len(),
            });
        }
        Self::new(level, values)
    }

    /// Parses the native shaper header and remaps blue-fast file order to
    /// native R-fastest storage.
    pub fn parse_3dl(contents: &str) -> Result<Self, Lut3dParseError> {
        let mut lines = contents.lines().enumerate();
        let (header_line, header_tokens) = loop {
            let Some((line_index, line)) = lines.next() else {
                return Err(Lut3dParseError::MissingHeader);
            };
            let tokens = tokenize(line, line_index + 1)?;
            if !tokens.is_empty() {
                break (line_index + 1, tokens);
            }
        };

        if header_tokens.len() <= 3 {
            return Err(Lut3dParseError::InvalidHeader(header_line));
        }
        let minimum = parse_integer(header_tokens[0], header_line)?;
        let maximum = parse_integer(header_tokens[header_tokens.len() - 1], header_line)?;
        if minimum < 0 || maximum < 0 || maximum <= minimum {
            return Err(Lut3dParseError::InvalidHeader(header_line));
        }
        if maximum < 128 {
            let maximum =
                u64::try_from(maximum).map_err(|_| Lut3dParseError::InvalidHeader(header_line))?;
            return Err(Lut3dParseError::InvalidMaximum(maximum));
        }

        let level = header_tokens.len();
        if level < 2 {
            return Err(Lut3dParseError::InvalidLevel(level));
        }
        let expected = checked_cube_size(level)?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(expected)
            .map_err(|_| Lut3dParseError::AllocationFailure)?;
        values.resize(expected, [0.0; 3]);

        let mut record_count = 0_usize;
        let mut max_value = 0_u64;
        for (line_index, line) in lines {
            let line_number = line_index + 1;
            let tokens = tokenize(line, line_number)?;
            if tokens.is_empty() {
                continue;
            }
            if tokens.len() != 3 {
                return Err(Lut3dParseError::WrongTokenCount {
                    line: line_number,
                    expected: 3,
                    actual: tokens.len(),
                });
            }
            if record_count >= expected {
                return Err(Lut3dParseError::ExtraRecords(line_number));
            }

            let red_value = parse_nonnegative_integer(tokens[0], line_number)?;
            let green_value = parse_nonnegative_integer(tokens[1], line_number)?;
            let blue_value = parse_nonnegative_integer(tokens[2], line_number)?;
            max_value = max_value.max(red_value).max(green_value).max(blue_value);

            // The native file order is blue-fast.  Convert it to
            // red + level*green + level²*blue before normalization.
            let level_squared = level * level;
            let red = record_count / level_squared;
            let remainder = record_count - red * level_squared;
            let green = remainder / level;
            let blue = remainder - green * level;
            let destination = red + level * green + level_squared * blue;
            values[destination] = [red_value as f32, green_value as f32, blue_value as f32];
            record_count += 1;
        }

        if record_count != expected {
            return Err(Lut3dParseError::WrongRecordCount {
                expected,
                actual: record_count,
            });
        }
        if max_value < 128 {
            return Err(Lut3dParseError::InvalidMaximum(max_value));
        }

        // Native searches for the smallest power of two strictly greater than
        // max_value, with 16-bit normalization as the ceiling.
        let mut normalization_ceiling = 1_u64;
        while normalization_ceiling < max_value && normalization_ceiling < 65_536 {
            normalization_ceiling <<= 1;
        }
        let normalizer = 1.0_f32 / (normalization_ceiling - 1) as f32;
        for value in &mut values {
            for channel in value {
                *channel = (*channel * normalizer).clamp(0.0, 1.0);
            }
        }
        Self::new(level, values)
    }
}

fn tokenize(line: &str, line_number: usize) -> Result<Vec<&str>, Lut3dParseError> {
    let useful = line.split('#').next().unwrap_or_default();
    let tokens: Vec<_> = useful.split_whitespace().collect();
    if tokens.iter().any(|token| token.len() > MAX_TOKEN_LENGTH) {
        return Err(Lut3dParseError::TokenTooLong(line_number));
    }
    Ok(tokens)
}

fn require_token_count(
    tokens: &[&str],
    expected: usize,
    line_number: usize,
) -> Result<(), Lut3dParseError> {
    if tokens.len() == expected {
        Ok(())
    } else {
        Err(Lut3dParseError::WrongTokenCount {
            line: line_number,
            expected,
            actual: tokens.len(),
        })
    }
}

fn parse_float(token: &str, line: usize) -> Result<f32, Lut3dParseError> {
    token
        .parse::<f32>()
        .map_err(|_| Lut3dParseError::MalformedNumber { line })
}

#[allow(clippy::cast_possible_truncation)]
fn parse_sample(token: &str, line: usize) -> Result<f32, Lut3dParseError> {
    // Native `_dt_atof` accumulates into a double before the cube value is
    // narrowed to float by the destination buffer.  Keep that intermediate
    // precision so decimal values near an f32 rounding midpoint follow the
    // native conversion rather than Rust's direct f32 parser.
    let parsed = token
        .parse::<f64>()
        .map_err(|_| Lut3dParseError::MalformedNumber { line })?;
    let value = parsed as f32;
    // Preserve the existing fail-closed behavior for finite values that
    // overflow the f32 representation while retaining the native handling of
    // explicit non-finite tokens below.
    if parsed.is_finite() && !value.is_finite() {
        return Err(Lut3dParseError::MalformedNumber { line });
    }
    if !value.is_finite() {
        return Err(Lut3dParseError::NonFiniteSample(line));
    }
    Ok(value)
}

fn parse_usize(token: &str, line: usize) -> Result<usize, Lut3dParseError> {
    token
        .parse::<usize>()
        .map_err(|_| Lut3dParseError::MalformedInteger { line })
}

fn parse_integer(token: &str, line: usize) -> Result<i64, Lut3dParseError> {
    token
        .parse::<i64>()
        .map_err(|_| Lut3dParseError::MalformedInteger { line })
}

fn parse_nonnegative_integer(token: &str, line: usize) -> Result<u64, Lut3dParseError> {
    let value = parse_integer(token, line)?;
    u64::try_from(value).map_err(|_| Lut3dParseError::MalformedInteger { line })
}

fn checked_cube_size(level: usize) -> Result<usize, Lut3dParseError> {
    level
        .checked_mul(level)
        .and_then(|square| square.checked_mul(level))
        .ok_or(Lut3dParseError::ArithmeticOverflow)
}

#[derive(Debug)]
pub enum Lut3dParseError {
    Io(std::io::Error),
    UnsupportedFormat,
    MissingHeader,
    InvalidHeader(usize),
    InvalidLevel(usize),
    InvalidMaximum(u64),
    ArithmeticOverflow,
    AllocationFailure,
    SizeNotDefined(usize),
    DuplicateSize(usize),
    OneDimensionalLut,
    ExtraRecords(usize),
    WrongRecordCount {
        expected: usize,
        actual: usize,
    },
    WrongTokenCount {
        line: usize,
        expected: usize,
        actual: usize,
    },
    TokenTooLong(usize),
    MalformedNumber {
        line: usize,
    },
    MalformedInteger {
        line: usize,
    },
    NonFiniteSample(usize),
    UnsupportedDomain {
        directive: &'static str,
        expected: f32,
    },
}

impl fmt::Display for Lut3dParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not read LUT: {error}"),
            Self::UnsupportedFormat => {
                formatter.write_str("LUT format is not supported by this leaf")
            }
            Self::MissingHeader => formatter.write_str("3DL shaper header is missing"),
            Self::InvalidHeader(line) => write!(formatter, "invalid 3DL header on line {line}"),
            Self::InvalidLevel(level) => {
                write!(formatter, "LUT level {level} is outside the safe range")
            }
            Self::InvalidMaximum(value) => {
                write!(formatter, "3DL maximum value {value} is below 128")
            }
            Self::ArithmeticOverflow => formatter.write_str("LUT dimensions overflow usize"),
            Self::AllocationFailure => formatter.write_str("LUT allocation failed"),
            Self::SizeNotDefined(line) => {
                write!(formatter, "cube LUT size is not defined near line {line}")
            }
            Self::DuplicateSize(line) => {
                write!(formatter, "cube LUT size is repeated on line {line}")
            }
            Self::OneDimensionalLut => formatter.write_str("1D cube LUTs are not supported"),
            Self::ExtraRecords(line) => write!(formatter, "extra LUT record on line {line}"),
            Self::WrongRecordCount { expected, actual } => {
                write!(formatter, "LUT has {actual} records; expected {expected}")
            }
            Self::WrongTokenCount {
                line,
                expected,
                actual,
            } => write!(
                formatter,
                "line {line} has {actual} tokens; expected {expected}"
            ),
            Self::TokenTooLong(line) => {
                write!(formatter, "token on line {line} exceeds 50 characters")
            }
            Self::MalformedNumber { line } => {
                write!(formatter, "malformed floating-point value on line {line}")
            }
            Self::MalformedInteger { line } => {
                write!(formatter, "malformed integer on line {line}")
            }
            Self::NonFiniteSample(line) => {
                write!(formatter, "non-finite LUT sample on line {line}")
            }
            Self::UnsupportedDomain {
                directive,
                expected,
            } => {
                write!(
                    formatter,
                    "{directive} must use {expected} for every channel"
                )
            }
        }
    }
}

impl std::error::Error for Lut3dParseError {}
