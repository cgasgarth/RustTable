use std::env;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::schema::ReferenceIdentityReceipt;

pub(crate) const DEFAULT_FLAGS: &[&str] = &[
    "--configdir",
    "--cachedir",
    "--datadir",
    "--library",
    "--disable-opencl",
    "--width",
    "--height",
    "--icc-type",
    "--icc",
    "--out-ext",
];

static NEXT_SANDBOX: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencePin {
    pub version: String,
    pub commit: String,
    pub data_dir: PathBuf,
    #[serde(default)]
    pub required_flags: Vec<String>,
    #[serde(default = "default_log_ruleset")]
    pub normalized_log_ruleset: u32,
}

impl ReferencePin {
    /// Reads the checked-in reference identity and resolves its data directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or the pin is invalid.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ReferenceProbeError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|error| ReferenceProbeError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let mut pin: Self = toml::from_str(&source).map_err(|error| ReferenceProbeError::Pin {
            message: error.to_string(),
        })?;
        pin.validate()?;
        if pin.data_dir.is_relative() {
            let base = path.parent().unwrap_or_else(|| Path::new("."));
            pin.data_dir = base.join(&pin.data_dir);
        }
        Ok(pin)
    }

    /// Validates the exact semantic-version and commit pin.
    ///
    /// # Errors
    ///
    /// Returns an error when the version, commit, ruleset, or flags are invalid.
    pub fn validate(&self) -> Result<(), ReferenceProbeError> {
        let parts = self.version.split('.').collect::<Vec<_>>();
        if parts.len() != 3
            || parts
                .iter()
                .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
        {
            return Err(ReferenceProbeError::Pin {
                message: format!(
                    "reference version is not an exact semantic version: {}",
                    self.version
                ),
            });
        }
        if self.commit.len() != 40 || !self.commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ReferenceProbeError::Pin {
                message: "reference commit must be a 40-character hexadecimal SHA".to_owned(),
            });
        }
        if self.normalized_log_ruleset != 1 {
            return Err(ReferenceProbeError::Pin {
                message: format!(
                    "unsupported normalized log ruleset {}",
                    self.normalized_log_ruleset
                ),
            });
        }
        let flags = self.flags();
        if flags.iter().any(String::is_empty) {
            return Err(ReferenceProbeError::Pin {
                message: "required CLI flags cannot be empty".to_owned(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub(crate) fn flags(&self) -> Vec<String> {
        if self.required_flags.is_empty() {
            DEFAULT_FLAGS
                .iter()
                .map(|flag| (*flag).to_owned())
                .collect()
        } else {
            self.required_flags.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceIdentity {
    pub executable: PathBuf,
    pub version: String,
    pub commit: String,
    pub data_dir: PathBuf,
    pub required_flags: Vec<String>,
    pub normalized_log_ruleset: u32,
    pub executable_hash: String,
    pub data_bundle_hash: String,
    pub target_triple: String,
    pub c_abi_model: String,
    pub build_option_hash: String,
}

impl ReferenceIdentity {
    pub(crate) fn receipt(&self) -> ReferenceIdentityReceipt {
        ReferenceIdentityReceipt {
            version: self.version.clone(),
            commit: self.commit.clone(),
            executable_hash: self.executable_hash.clone(),
            data_bundle_hash: self.data_bundle_hash.clone(),
            target_triple: self.target_triple.clone(),
            c_abi_model: self.c_abi_model.clone(),
            build_option_hash: self.build_option_hash.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityProbe {
    executable: PathBuf,
    pin: ReferencePin,
}

impl CapabilityProbe {
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>, pin: ReferencePin) -> Self {
        Self {
            executable: executable.into(),
            pin,
        }
    }

    /// Checks the executable, pinned identity, isolated database flags, and data directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the executable, identity, flags, data directory, or isolation is invalid.
    pub fn probe(&self) -> Result<ReferenceIdentity, ReferenceProbeError> {
        self.pin.validate()?;
        let executable = fs::canonicalize(&self.executable).map_err(|error| {
            ReferenceProbeError::Executable {
                path: self.executable.clone(),
                message: error.to_string(),
            }
        })?;
        if !executable.is_file() {
            return Err(ReferenceProbeError::Executable {
                path: executable,
                message: "path is not a file".to_owned(),
            });
        }
        if !self.pin.data_dir.is_dir() {
            return Err(ReferenceProbeError::MissingDataDirectory {
                path: self.pin.data_dir.clone(),
            });
        }

        let sandbox = ProbeSandbox::new()?;
        let isolation = isolation_arguments(
            &self.pin.flags(),
            &self.pin.data_dir,
            &sandbox.config,
            &sandbox.cache,
            &sandbox.library,
        )?;
        let version = probe_command(&executable, &isolation, "--version", &sandbox)?;
        let identity = parse_identity(&version, &self.pin)?;
        let help = probe_command(&executable, &isolation, "--help", &sandbox)?;
        let help = String::from_utf8_lossy(&help);
        for flag in self.pin.flags() {
            if !help.split_whitespace().any(|word| word == flag) {
                return Err(ReferenceProbeError::UnsupportedFlag { flag });
            }
        }
        if !sandbox.library.starts_with(&sandbox.root) {
            return Err(ReferenceProbeError::Isolation {
                message: "probe library path escaped its temporary directory".to_owned(),
            });
        }
        Ok(ReferenceIdentity {
            executable,
            version: identity.0,
            commit: identity.1,
            data_dir: self.pin.data_dir.clone(),
            required_flags: self.pin.flags(),
            normalized_log_ruleset: self.pin.normalized_log_ruleset,
            executable_hash: file_hash(&self.executable)?,
            data_bundle_hash: directory_hash(&self.pin.data_dir)?,
            target_triple: target_triple().to_owned(),
            c_abi_model: target_triple().to_owned(),
            build_option_hash: hash_text("rusttable-reference-build-options-v1"),
        })
    }
}

fn file_hash(path: &Path) -> Result<String, ReferenceProbeError> {
    let bytes = fs::read(path).map_err(|error| ReferenceProbeError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Ok(hash_bytes(&bytes))
}

fn directory_hash(path: &Path) -> Result<String, ReferenceProbeError> {
    let mut files = Vec::new();
    collect_files(path, path, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = Sha256::new();
    for (relative, bytes) in files {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update([0]);
    }
    Ok(format_digest(hasher.finalize()))
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), ReferenceProbeError> {
    let entries = fs::read_dir(directory).map_err(|error| ReferenceProbeError::Io {
        path: directory.to_path_buf(),
        message: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| ReferenceProbeError::Io {
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let bytes = fs::read(&path).map_err(|error| ReferenceProbeError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
            files.push((relative, bytes));
        }
    }
    Ok(())
}

fn hash_text(value: &str) -> String {
    hash_bytes(value.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    format_digest(Sha256::digest(bytes))
}

fn format_digest(digest: impl AsRef<[u8]>) -> String {
    let mut formatted = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        write!(formatted, "{byte:02x}").expect("writing a digest to a String cannot fail");
    }
    formatted
}

const fn target_triple() -> &'static str {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_arch = "aarch64", target_os = "macos"),
        all(target_arch = "x86_64", target_os = "macos")
    )))]
    {
        "unsupported-target"
    }
}

pub(crate) fn isolation_arguments(
    flags: &[String],
    data_dir: &Path,
    config: &Path,
    cache: &Path,
    library: &Path,
) -> Result<Vec<String>, ReferenceProbeError> {
    let mut arguments = Vec::new();
    for flag in flags {
        arguments.push(flag.clone());
        match flag.as_str() {
            "--configdir" => arguments.push(config.display().to_string()),
            "--cachedir" => arguments.push(cache.display().to_string()),
            "--datadir" => arguments.push(data_dir.display().to_string()),
            "--library" => arguments.push(library.display().to_string()),
            "--disable-opencl" => {}
            "--width" | "--height" => arguments.push("1".to_owned()),
            "--icc-type" | "--icc" => arguments.push("srgb".to_owned()),
            "--out-ext" => arguments.push("png".to_owned()),
            other => {
                return Err(ReferenceProbeError::UnsupportedFlag {
                    flag: other.to_owned(),
                });
            }
        }
    }
    Ok(arguments)
}

fn parse_identity(
    bytes: &[u8],
    pin: &ReferencePin,
) -> Result<(String, String), ReferenceProbeError> {
    let text = String::from_utf8_lossy(bytes);
    let has_version = text.split_whitespace().any(|token| token == pin.version);
    let has_commit = text.split_whitespace().any(|token| token == pin.commit);
    if !has_version || !has_commit {
        return Err(ReferenceProbeError::IdentityMismatch {
            expected_version: pin.version.clone(),
            expected_commit: pin.commit.clone(),
            actual: text.trim().to_owned(),
        });
    }
    Ok((pin.version.clone(), pin.commit.clone()))
}

fn probe_command(
    executable: &Path,
    isolation: &[String],
    argument: &str,
    sandbox: &ProbeSandbox,
) -> Result<Vec<u8>, ReferenceProbeError> {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("PATH", default_path())
        .env("HOME", &sandbox.home)
        .env("XDG_CONFIG_HOME", &sandbox.config)
        .env("XDG_CACHE_HOME", &sandbox.cache)
        .env("XDG_DATA_HOME", &sandbox.data)
        .env("RUSTTABLE_OPENCL_CACHE", &sandbox.opencl)
        .env("TMPDIR", &sandbox.tmp)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("TZ", "UTC")
        .args(isolation)
        .arg(argument);
    let output = command
        .output()
        .map_err(|error| ReferenceProbeError::Spawn {
            executable: executable.to_path_buf(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ReferenceProbeError::ProbeExit {
            argument: argument.to_owned(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let mut bytes = output.stdout;
    bytes.extend_from_slice(&output.stderr);
    Ok(bytes)
}

pub(crate) struct ProbeSandbox {
    pub(crate) root: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) cache: PathBuf,
    pub(crate) data: PathBuf,
    pub(crate) opencl: PathBuf,
    pub(crate) library: PathBuf,
    pub(crate) tmp: PathBuf,
}

impl ProbeSandbox {
    fn new() -> Result<Self, ReferenceProbeError> {
        let number = NEXT_SANDBOX.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "rusttable-reference-probe-sandbox-{}-{number}",
            std::process::id()
        ));
        let sandbox = Self {
            home: root.join("home"),
            config: root.join("config"),
            cache: root.join("cache"),
            data: root.join("data"),
            opencl: root.join("opencl"),
            library: root.join("library.db"),
            tmp: root.join("tmp"),
            root,
        };
        for path in [
            &sandbox.home,
            &sandbox.config,
            &sandbox.cache,
            &sandbox.data,
            &sandbox.opencl,
            &sandbox.tmp,
        ] {
            fs::create_dir_all(path).map_err(|error| ReferenceProbeError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
        }
        Ok(sandbox)
    }
}

impl Drop for ProbeSandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn default_path() -> &'static str {
    if cfg!(windows) {
        r"C:\Windows\System32;C:\Windows"
    } else {
        "/usr/bin:/bin"
    }
}

fn default_log_ruleset() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceProbeError {
    Io {
        path: PathBuf,
        message: String,
    },
    Pin {
        message: String,
    },
    Executable {
        path: PathBuf,
        message: String,
    },
    MissingDataDirectory {
        path: PathBuf,
    },
    Spawn {
        executable: PathBuf,
        message: String,
    },
    ProbeExit {
        argument: String,
        code: Option<i32>,
        stderr: String,
    },
    IdentityMismatch {
        expected_version: String,
        expected_commit: String,
        actual: String,
    },
    UnsupportedFlag {
        flag: String,
    },
    Isolation {
        message: String,
    },
}

impl fmt::Display for ReferenceProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(
                formatter,
                "reference probe I/O at {}: {message}",
                path.display()
            ),
            Self::Pin { message } => write!(formatter, "invalid reference pin: {message}"),
            Self::Executable { path, message } => write!(
                formatter,
                "invalid reference executable {}: {message}",
                path.display()
            ),
            Self::MissingDataDirectory { path } => write!(
                formatter,
                "reference data directory is missing: {}",
                path.display()
            ),
            Self::Spawn {
                executable,
                message,
            } => write!(
                formatter,
                "cannot execute reference {}: {message}",
                executable.display()
            ),
            Self::ProbeExit {
                argument,
                code,
                stderr,
            } => write!(formatter, "reference {argument} exited {code:?}: {stderr}"),
            Self::IdentityMismatch {
                expected_version,
                expected_commit,
                actual,
            } => write!(
                formatter,
                "reference identity mismatch; expected {expected_version} {expected_commit}, got {actual}"
            ),
            Self::UnsupportedFlag { flag } => {
                write!(formatter, "reference does not support required flag {flag}")
            }
            Self::Isolation { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ReferenceProbeError {}
