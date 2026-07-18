use std::fmt;
use std::fs;
use std::path::Path;

pub const CASES: [&str; 2] = [
    "photo_build_and_iterate_128_assets",
    "render_256x256_two_step_pipeline",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseConfig {
    pub name: String,
    pub warmup_count: u64,
    pub sample_count: u64,
    pub work_units: u64,
    pub calibration_iterations: u64,
    pub limit_ppm: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Io,
    MissingSchema,
    UnsupportedSchema,
    InvalidHeader,
    InvalidRow,
    DuplicateCase,
    MissingCase,
    UnknownCase,
    ZeroValue,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io => "cannot read performance budget configuration",
            Self::MissingSchema => "performance budget schema is missing",
            Self::UnsupportedSchema => "performance budget schema is unsupported",
            Self::InvalidHeader => "performance budget header is invalid",
            Self::InvalidRow => "performance budget row is invalid",
            Self::DuplicateCase => "performance budget case is duplicated",
            Self::MissingCase => "performance budget case is missing",
            Self::UnknownCase => "performance budget case is unknown",
            Self::ZeroValue => "performance budget counts and limit must be nonzero",
        })
    }
}

impl std::error::Error for ConfigError {}

pub fn read(path: &Path) -> Result<Vec<CaseConfig>, ConfigError> {
    parse(&fs::read_to_string(path).map_err(|_| ConfigError::Io)?)
}

pub fn parse(text: &str) -> Result<Vec<CaseConfig>, ConfigError> {
    let mut lines = text.lines();
    let schema = lines.next().ok_or(ConfigError::MissingSchema)?;
    if schema.split('\t').collect::<Vec<_>>() != ["schema_version", "1"] {
        if schema.starts_with("schema_version\t") {
            return Err(ConfigError::UnsupportedSchema);
        }
        return Err(ConfigError::MissingSchema);
    }
    if lines.next()
        != Some("case\twarmup_count\tsample_count\twork_units\tcalibration_iterations\tlimit_ppm")
    {
        return Err(ConfigError::InvalidHeader);
    }
    let mut result = Vec::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(ConfigError::InvalidRow);
        }
        let name = fields[0].to_owned();
        if !CASES.contains(&name.as_str()) {
            return Err(ConfigError::UnknownCase);
        }
        if result.iter().any(|case: &CaseConfig| case.name == name) {
            return Err(ConfigError::DuplicateCase);
        }
        let values = fields[1..]
            .iter()
            .map(|value| value.parse::<u64>().map_err(|_| ConfigError::InvalidRow))
            .collect::<Result<Vec<_>, _>>()?;
        if values.contains(&0) {
            return Err(ConfigError::ZeroValue);
        }
        result.push(CaseConfig {
            name,
            warmup_count: values[0],
            sample_count: values[1],
            work_units: values[2],
            calibration_iterations: values[3],
            limit_ppm: values[4],
        });
    }
    for expected in CASES {
        if !result.iter().any(|case| case.name == expected) {
            return Err(ConfigError::MissingCase);
        }
    }
    if result.len() != CASES.len() {
        return Err(ConfigError::InvalidRow);
    }
    Ok(result)
}
