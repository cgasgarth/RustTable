//! Runtime platform policy and GPU-capability contracts.
//!
//! This module deliberately contains no operating-system bindings or WGPU types.  It models the
//! small, privacy-safe vocabulary that the application composition layer and the later GPU
//! service share.

#![allow(clippy::struct_excessive_bools)]

use std::{collections::BTreeSet, fmt};

mod target;

const MAX_BUILD_ID_BYTES: usize = 128;
const MAX_TARGET_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperatingSystem {
    Linux,
    MacOs,
    Windows,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuArchitecture {
    X86_64,
    Aarch64,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinuxLibc {
    Gnu,
    Musl,
    Unknown,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WindowSystem {
    Wayland,
    X11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GraphicsBackend {
    Vulkan,
    OpenGl,
    Metal,
    Direct3D12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ApplicationMode {
    Desktop,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SupportLevel {
    SupportedGpuCandidate,
    SupportedCpuOnly,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuFallbackPolicy {
    Required,
    Forbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OsVersion {
    major: u32,
    minor: u32,
    patch: u32,
    build: Option<u32>,
}

impl OsVersion {
    #[must_use]
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
            build: None,
        }
    }

    #[must_use]
    pub const fn with_build(self, build: u32) -> Self {
        Self {
            build: Some(build),
            ..self
        }
    }

    /// Parses `major.minor.patch[-build]` without accepting unbounded native strings.
    ///
    /// # Errors
    ///
    /// Returns an error when any component is non-numeric or the version has too many components.
    pub fn parse(value: &str) -> Result<Self, VersionParseError> {
        let (core, build) = value
            .split_once('-')
            .map_or((value, None), |(core, build)| (core, Some(build)));
        if core.is_empty() || core.split('.').count() > 3 {
            return Err(VersionParseError);
        }
        let mut parts = core
            .split('.')
            .map(|part| part.parse::<u32>().map_err(|_| VersionParseError));
        let major = parts.next().ok_or(VersionParseError)??;
        let minor = parts.next().unwrap_or(Ok(0))?;
        let patch = parts.next().unwrap_or(Ok(0))?;
        if parts.next().is_some() {
            return Err(VersionParseError);
        }
        let build = build
            .map(|value| value.parse::<u32>().map_err(|_| VersionParseError))
            .transpose()?;
        Ok(Self {
            major,
            minor,
            patch,
            build,
        })
    }

    #[must_use]
    pub const fn major(self) -> u32 {
        self.major
    }

    #[must_use]
    pub const fn minor(self) -> u32 {
        self.minor
    }

    #[must_use]
    pub const fn patch(self) -> u32 {
        self.patch
    }

    #[must_use]
    pub const fn build(self) -> Option<u32> {
        self.build
    }
}

impl fmt::Display for OsVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(build) = self.build {
            write!(formatter, "-{build}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionParseError;

impl fmt::Display for VersionParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("version is not a bounded dotted numeric value")
    }
}

impl std::error::Error for VersionParseError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetTriple(String);

impl TargetTriple {
    /// # Errors
    ///
    /// Returns an error when the target is empty, too long, or contains a control character.
    pub fn new(value: impl Into<String>) -> Result<Self, TargetTripleError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_TARGET_BYTES || value.chars().any(char::is_control)
        {
            return Err(TargetTripleError);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetTripleError;

impl fmt::Display for TargetTripleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("target triple is empty, too long, or contains a control character")
    }
}

impl std::error::Error for TargetTripleError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformIdentity {
    operating_system: OperatingSystem,
    architecture: CpuArchitecture,
    target: TargetTriple,
    os_version: Option<OsVersion>,
    libc: LinuxLibc,
    libc_version: Option<OsVersion>,
    window_systems: Vec<WindowSystem>,
    headless: bool,
    application_build: String,
}

impl PlatformIdentity {
    /// Builds a privacy-safe runtime identity from normalized values.
    ///
    /// # Errors
    ///
    /// Returns an error when the identity contains invalid build, libc, or mode data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operating_system: OperatingSystem,
        architecture: CpuArchitecture,
        target: TargetTriple,
        os_version: Option<OsVersion>,
        libc: LinuxLibc,
        libc_version: Option<OsVersion>,
        window_systems: Vec<WindowSystem>,
        headless: bool,
        application_build: impl Into<String>,
    ) -> Result<Self, PlatformIdentityError> {
        let application_build = application_build.into();
        if application_build.is_empty()
            || application_build.len() > MAX_BUILD_ID_BYTES
            || application_build.chars().any(char::is_control)
        {
            return Err(PlatformIdentityError::InvalidBuildIdentity);
        }
        let mut systems = window_systems;
        systems.sort_unstable();
        systems.dedup();
        if headless && !systems.is_empty() {
            return Err(PlatformIdentityError::HeadlessHasWindowSystem);
        }
        if operating_system != OperatingSystem::Linux
            && (libc != LinuxLibc::NotApplicable || libc_version.is_some())
        {
            return Err(PlatformIdentityError::NonLinuxLibc);
        }
        if operating_system == OperatingSystem::Linux
            && libc == LinuxLibc::Gnu
            && libc_version.is_none()
        {
            return Err(PlatformIdentityError::MissingLibcVersion);
        }
        Ok(Self {
            operating_system,
            architecture,
            target,
            os_version,
            libc,
            libc_version,
            window_systems: systems,
            headless,
            application_build,
        })
    }

    #[must_use]
    pub const fn operating_system(&self) -> OperatingSystem {
        self.operating_system
    }

    #[must_use]
    pub const fn architecture(&self) -> CpuArchitecture {
        self.architecture
    }

    #[must_use]
    pub fn target(&self) -> &TargetTriple {
        &self.target
    }

    #[must_use]
    pub const fn os_version(&self) -> Option<OsVersion> {
        self.os_version
    }

    #[must_use]
    pub const fn libc(&self) -> LinuxLibc {
        self.libc
    }

    #[must_use]
    pub const fn libc_version(&self) -> Option<OsVersion> {
        self.libc_version
    }

    #[must_use]
    pub fn window_systems(&self) -> &[WindowSystem] {
        &self.window_systems
    }

    #[must_use]
    pub const fn headless(&self) -> bool {
        self.headless
    }

    #[must_use]
    pub fn application_build(&self) -> &str {
        &self.application_build
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformIdentityError {
    InvalidBuildIdentity,
    HeadlessHasWindowSystem,
    NonLinuxLibc,
    MissingLibcVersion,
}

impl fmt::Display for PlatformIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidBuildIdentity => "application build identity is invalid",
            Self::HeadlessHasWindowSystem => "headless identity cannot list a window system",
            Self::NonLinuxLibc => "non-Linux identity cannot list a Linux libc",
            Self::MissingLibcVersion => "GNU/Linux identity requires a libc version",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PlatformIdentityError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendPreference(Vec<GraphicsBackend>);

impl BackendPreference {
    /// # Errors
    ///
    /// Returns an error when the preference is empty, too long, or contains duplicates.
    pub fn new(
        backends: impl IntoIterator<Item = GraphicsBackend>,
    ) -> Result<Self, BackendPreferenceError> {
        let backends = backends.into_iter().collect::<Vec<_>>();
        if backends.is_empty() || backends.len() > 3 {
            return Err(BackendPreferenceError);
        }
        let unique = backends.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != backends.len() {
            return Err(BackendPreferenceError);
        }
        Ok(Self(backends))
    }

    #[must_use]
    pub fn ordered(&self) -> &[GraphicsBackend] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendPreferenceError;

impl fmt::Display for BackendPreferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("backend preference must contain one to three unique backends")
    }
}

impl std::error::Error for BackendPreferenceError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreComputeRequirements {
    pub compute_dispatch: bool,
    pub float32_storage_buffers: bool,
    pub checked_buffer_alignment: u32,
    pub checked_buffer_size: u64,
    pub buffer_upload_copy_readback: bool,
    pub optional_r32float_storage_texture: bool,
    pub optional_rgba16float_attachment: bool,
}

impl CoreComputeRequirements {
    #[must_use]
    pub const fn initial() -> Self {
        Self {
            compute_dispatch: true,
            float32_storage_buffers: true,
            checked_buffer_alignment: 16,
            checked_buffer_size: 65_536,
            buffer_upload_copy_readback: true,
            optional_r32float_storage_texture: true,
            optional_rgba16float_attachment: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRequirement {
    target: TargetTriple,
    operating_system: OperatingSystem,
    architecture: CpuArchitecture,
    minimum_os: Option<OsVersion>,
    libc: LinuxLibc,
    minimum_libc: Option<OsVersion>,
    desktop: bool,
    headless: bool,
    window_systems: Vec<WindowSystem>,
    backends: BackendPreference,
    cpu_fallback: CpuFallbackPolicy,
    runner: String,
    package_target: TargetTriple,
}

impl PlatformRequirement {
    /// # Errors
    ///
    /// Returns an error when the requirement has contradictory execution or libc policy.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: TargetTriple,
        operating_system: OperatingSystem,
        architecture: CpuArchitecture,
        minimum_os: Option<OsVersion>,
        libc: LinuxLibc,
        minimum_libc: Option<OsVersion>,
        desktop: bool,
        headless: bool,
        window_systems: Vec<WindowSystem>,
        backends: BackendPreference,
        cpu_fallback: CpuFallbackPolicy,
        runner: String,
        package_target: TargetTriple,
    ) -> Result<Self, PlatformRequirementError> {
        if runner.is_empty() || runner.chars().any(char::is_control) {
            return Err(PlatformRequirementError::InvalidRunner);
        }
        if !desktop && !headless {
            return Err(PlatformRequirementError::NoExecutionMode);
        }
        if operating_system == OperatingSystem::Linux && desktop && window_systems.is_empty() {
            return Err(PlatformRequirementError::DesktopWithoutWindowSystem);
        }
        if operating_system == OperatingSystem::Linux
            && (libc == LinuxLibc::NotApplicable || minimum_libc.is_none())
        {
            return Err(PlatformRequirementError::LinuxWithoutLibcPolicy);
        }
        if operating_system != OperatingSystem::Linux
            && (libc != LinuxLibc::NotApplicable || minimum_libc.is_some())
        {
            return Err(PlatformRequirementError::NonLinuxLibcPolicy);
        }
        Ok(Self {
            target,
            operating_system,
            architecture,
            minimum_os,
            libc,
            minimum_libc,
            desktop,
            headless,
            window_systems,
            backends,
            cpu_fallback,
            runner,
            package_target,
        })
    }

    #[must_use]
    pub fn target(&self) -> &TargetTriple {
        &self.target
    }
    #[must_use]
    pub const fn operating_system(&self) -> OperatingSystem {
        self.operating_system
    }
    #[must_use]
    pub const fn architecture(&self) -> CpuArchitecture {
        self.architecture
    }
    #[must_use]
    pub const fn minimum_os(&self) -> Option<OsVersion> {
        self.minimum_os
    }
    #[must_use]
    pub const fn libc(&self) -> LinuxLibc {
        self.libc
    }
    #[must_use]
    pub const fn minimum_libc(&self) -> Option<OsVersion> {
        self.minimum_libc
    }
    #[must_use]
    pub const fn desktop(&self) -> bool {
        self.desktop
    }
    #[must_use]
    pub const fn headless(&self) -> bool {
        self.headless
    }
    #[must_use]
    pub fn window_systems(&self) -> &[WindowSystem] {
        &self.window_systems
    }
    #[must_use]
    pub fn backends(&self) -> &BackendPreference {
        &self.backends
    }
    #[must_use]
    pub const fn cpu_fallback(&self) -> CpuFallbackPolicy {
        self.cpu_fallback
    }
    #[must_use]
    pub fn runner(&self) -> &str {
        &self.runner
    }
    #[must_use]
    pub fn package_target(&self) -> &TargetTriple {
        &self.package_target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformRequirementError {
    InvalidRunner,
    NoExecutionMode,
    DesktopWithoutWindowSystem,
    LinuxWithoutLibcPolicy,
    NonLinuxLibcPolicy,
}

impl fmt::Display for PlatformRequirementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidRunner => "runner is empty or contains a control character",
            Self::NoExecutionMode => "platform must allow desktop or headless execution",
            Self::DesktopWithoutWindowSystem => "desktop support requires a window system",
            Self::LinuxWithoutLibcPolicy => "Linux support requires a libc and minimum version",
            Self::NonLinuxLibcPolicy => "non-Linux support cannot declare a libc policy",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PlatformRequirementError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformFinding {
    UnsupportedTarget,
    OperatingSystemMismatch,
    ArchitectureMismatch,
    MissingOsVersion,
    OsVersionBelowMinimum {
        required: OsVersion,
        actual: OsVersion,
    },
    UnknownLibc,
    LibcMismatch,
    MissingLibcVersion,
    LibcVersionBelowMinimum {
        required: OsVersion,
        actual: OsVersion,
    },
    MissingDesktopWindowSystem,
    GpuProbeUnavailable,
    NoPermittedGpuBackend,
    IdentityUnavailable,
}

impl fmt::Display for PlatformFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTarget => formatter.write_str("target is not in the supported matrix"),
            Self::OperatingSystemMismatch => {
                formatter.write_str("operating system does not match target policy")
            }
            Self::ArchitectureMismatch => {
                formatter.write_str("CPU architecture does not match target policy")
            }
            Self::MissingOsVersion => {
                formatter.write_str("required operating-system version is unavailable")
            }
            Self::OsVersionBelowMinimum { required, actual } => {
                write!(formatter, "OS version {actual} is below minimum {required}")
            }
            Self::UnknownLibc => formatter.write_str("Linux libc could not be identified"),
            Self::LibcMismatch => formatter.write_str("Linux libc does not match target policy"),
            Self::MissingLibcVersion => {
                formatter.write_str("required Linux libc version is unavailable")
            }
            Self::LibcVersionBelowMinimum { required, actual } => write!(
                formatter,
                "libc version {actual} is below minimum {required}"
            ),
            Self::MissingDesktopWindowSystem => {
                formatter.write_str("desktop mode has no supported window system")
            }
            Self::GpuProbeUnavailable => formatter.write_str("GPU capability probe is unavailable"),
            Self::NoPermittedGpuBackend => {
                formatter.write_str("no permitted GPU backend was qualified")
            }
            Self::IdentityUnavailable => {
                formatter.write_str("platform identity could not be collected")
            }
        }
    }
}

impl std::error::Error for PlatformFinding {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDecision {
    level: SupportLevel,
    identity: PlatformIdentity,
    requirement: Option<PlatformRequirement>,
    findings: Vec<PlatformFinding>,
    compute: CoreComputeRequirements,
}

impl PlatformDecision {
    #[must_use]
    pub const fn level(&self) -> SupportLevel {
        self.level
    }
    #[must_use]
    pub fn identity(&self) -> &PlatformIdentity {
        &self.identity
    }
    #[must_use]
    pub fn requirement(&self) -> Option<&PlatformRequirement> {
        self.requirement.as_ref()
    }
    #[must_use]
    pub fn findings(&self) -> &[PlatformFinding] {
        &self.findings
    }
    #[must_use]
    pub const fn compute_requirements(&self) -> CoreComputeRequirements {
        self.compute
    }
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        !matches!(self.level, SupportLevel::UnsupportedPlatform)
    }
}

pub trait GpuCapabilityProbePort {
    /// Returns whether at least one preferred backend satisfies the semantic requirements.
    ///
    /// # Errors
    ///
    /// Returns a bounded platform finding when probing is unavailable or cannot qualify a backend.
    fn has_qualified_backend(
        &self,
        preference: &BackendPreference,
        requirements: CoreComputeRequirements,
    ) -> Result<bool, PlatformFinding>;
}

pub trait PlatformCapabilityPort {
    /// # Errors
    ///
    /// Returns `IdentityUnavailable` when the native platform identity cannot be collected.
    fn identity(&self) -> Result<PlatformIdentity, PlatformFinding>;
    fn evaluate(&self, mode: ApplicationMode) -> PlatformDecision;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRegistry {
    requirements: Vec<PlatformRequirement>,
    compute: CoreComputeRequirements,
}

impl PlatformRegistry {
    #[must_use]
    pub fn new(requirements: Vec<PlatformRequirement>) -> Self {
        Self {
            requirements,
            compute: CoreComputeRequirements::initial(),
        }
    }

    #[must_use]
    pub fn requirements(&self) -> &[PlatformRequirement] {
        &self.requirements
    }

    #[must_use]
    pub fn evaluate(&self, identity: PlatformIdentity, mode: ApplicationMode) -> PlatformDecision {
        self.evaluate_with_probe(identity, mode, None)
    }

    #[must_use]
    pub fn evaluate_with_probe(
        &self,
        identity: PlatformIdentity,
        mode: ApplicationMode,
        probe: Option<&dyn GpuCapabilityProbePort>,
    ) -> PlatformDecision {
        let Some(requirement) = self
            .requirements
            .iter()
            .find(|candidate| candidate.target == identity.target)
        else {
            return self.unsupported(identity, None, PlatformFinding::UnsupportedTarget);
        };
        let mut findings = Vec::new();
        if requirement.operating_system != identity.operating_system {
            findings.push(PlatformFinding::OperatingSystemMismatch);
        }
        if requirement.architecture != identity.architecture {
            findings.push(PlatformFinding::ArchitectureMismatch);
        }
        match (requirement.minimum_os, identity.os_version) {
            (Some(required), Some(actual)) if actual < required => {
                findings.push(PlatformFinding::OsVersionBelowMinimum { required, actual });
            }
            (Some(_), None) => findings.push(PlatformFinding::MissingOsVersion),
            _ => {}
        }
        if requirement.libc != LinuxLibc::NotApplicable {
            if identity.libc == LinuxLibc::Unknown {
                findings.push(PlatformFinding::UnknownLibc);
            } else if identity.libc != requirement.libc {
                findings.push(PlatformFinding::LibcMismatch);
            }
            match (requirement.minimum_libc, identity.libc_version) {
                (Some(required), Some(actual)) if actual < required => {
                    findings.push(PlatformFinding::LibcVersionBelowMinimum { required, actual });
                }
                (Some(_), None) => findings.push(PlatformFinding::MissingLibcVersion),
                _ => {}
            }
        }
        if matches!(mode, ApplicationMode::Desktop)
            && (identity.headless
                || (!requirement.window_systems.is_empty()
                    && !requirement
                        .window_systems
                        .iter()
                        .any(|system| identity.window_systems.contains(system))))
        {
            findings.push(PlatformFinding::MissingDesktopWindowSystem);
        }
        if !findings.is_empty() {
            return PlatformDecision {
                level: SupportLevel::UnsupportedPlatform,
                identity,
                requirement: Some(requirement.clone()),
                findings,
                compute: self.compute,
            };
        }
        let level = match probe {
            None => SupportLevel::SupportedGpuCandidate,
            Some(probe) => match probe.has_qualified_backend(&requirement.backends, self.compute) {
                Ok(true) => SupportLevel::SupportedGpuCandidate,
                Ok(false) => SupportLevel::SupportedCpuOnly,
                Err(finding) => {
                    findings.push(finding);
                    SupportLevel::SupportedCpuOnly
                }
            },
        };
        PlatformDecision {
            level,
            identity,
            requirement: Some(requirement.clone()),
            findings,
            compute: self.compute,
        }
    }

    fn unsupported(
        &self,
        identity: PlatformIdentity,
        requirement: Option<PlatformRequirement>,
        finding: PlatformFinding,
    ) -> PlatformDecision {
        PlatformDecision {
            level: SupportLevel::UnsupportedPlatform,
            identity,
            requirement,
            findings: vec![finding],
            compute: self.compute,
        }
    }
}

impl Default for PlatformRegistry {
    fn default() -> Self {
        let linux = PlatformRequirement::new(
            TargetTriple::new("x86_64-unknown-linux-gnu").expect("static target"),
            OperatingSystem::Linux,
            CpuArchitecture::X86_64,
            Some(OsVersion::new(2, 35, 0)),
            LinuxLibc::Gnu,
            Some(OsVersion::new(2, 35, 0)),
            true,
            true,
            vec![WindowSystem::Wayland, WindowSystem::X11],
            BackendPreference::new([GraphicsBackend::Vulkan, GraphicsBackend::OpenGl])
                .expect("static backends"),
            CpuFallbackPolicy::Required,
            "ubuntu-latest".to_owned(),
            TargetTriple::new("x86_64-unknown-linux-gnu").expect("static target"),
        )
        .expect("static requirement");
        let macos = PlatformRequirement::new(
            TargetTriple::new("aarch64-apple-darwin").expect("static target"),
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
            Some(OsVersion::new(13, 0, 0)),
            LinuxLibc::NotApplicable,
            None,
            true,
            true,
            Vec::new(),
            BackendPreference::new([GraphicsBackend::Metal]).expect("static backends"),
            CpuFallbackPolicy::Required,
            "macos-latest".to_owned(),
            TargetTriple::new("aarch64-apple-darwin").expect("static target"),
        )
        .expect("static requirement");
        let windows = PlatformRequirement::new(
            TargetTriple::new("x86_64-pc-windows-msvc").expect("static target"),
            OperatingSystem::Windows,
            CpuArchitecture::X86_64,
            Some(OsVersion::new(10, 0, 0).with_build(19045)),
            LinuxLibc::NotApplicable,
            None,
            true,
            true,
            Vec::new(),
            BackendPreference::new([GraphicsBackend::Direct3D12, GraphicsBackend::Vulkan])
                .expect("static backends"),
            CpuFallbackPolicy::Required,
            "windows-latest".to_owned(),
            TargetTriple::new("x86_64-pc-windows-msvc").expect("static target"),
        )
        .expect("static requirement");
        Self::new(vec![linux, macos, windows])
    }
}

impl PlatformCapabilityPort for PlatformRegistry {
    fn identity(&self) -> Result<PlatformIdentity, PlatformFinding> {
        current_platform_identity().ok_or(PlatformFinding::IdentityUnavailable)
    }

    fn evaluate(&self, mode: ApplicationMode) -> PlatformDecision {
        match self.identity() {
            Ok(identity) => self.evaluate_with_probe(identity, mode, None),
            Err(finding) => {
                let identity = current_platform_identity().unwrap_or_else(fallback_identity);
                self.unsupported(identity, None, finding)
            }
        }
    }
}

///
/// # Panics
///
/// This function panics only if the compile-time fallback constants violate their bounded
/// constructors.
#[must_use]
pub fn fallback_identity() -> PlatformIdentity {
    PlatformIdentity::new(
        OperatingSystem::Unknown,
        CpuArchitecture::Unknown,
        TargetTriple::new("unknown").expect("static target"),
        None,
        LinuxLibc::NotApplicable,
        None,
        Vec::new(),
        true,
        "unknown",
    )
    .expect("static fallback identity")
}

#[must_use]
pub fn current_platform_identity() -> Option<PlatformIdentity> {
    let (os, architecture, target) = target::current_target();
    let (window_systems, headless) = target::current_window_systems();
    let (os_version, libc, libc_version) = native_platform_versions(os).unwrap_or({
        if matches!(os, OperatingSystem::Linux) {
            (None, LinuxLibc::Unknown, None)
        } else {
            (None, LinuxLibc::NotApplicable, None)
        }
    });
    PlatformIdentity::new(
        os,
        architecture,
        TargetTriple::new(target).ok()?,
        os_version,
        libc,
        libc_version,
        window_systems,
        headless,
        env!("CARGO_PKG_VERSION"),
    )
    .ok()
}

#[cfg(target_os = "linux")]
fn native_platform_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()?;
    let native_text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    if !native_text.contains("glibc") && !native_text.contains("gnu libc") {
        return None;
    }
    let version = find_dotted_version(&native_text)?;
    Some((Some(version), LinuxLibc::Gnu, Some(version)))
}

#[cfg(target_os = "macos")]
fn native_platform_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    let version = OsVersion::parse(String::from_utf8_lossy(&output.stdout).trim()).ok()?;
    Some((Some(version), LinuxLibc::NotApplicable, None))
}

#[cfg(target_os = "windows")]
fn native_platform_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()?;
    let version = find_dotted_version(&format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))?;
    let version = OsVersion::new(version.major(), version.minor(), 0).with_build(version.patch());
    Some((Some(version), LinuxLibc::NotApplicable, None))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_platform_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    None
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn find_dotted_version(value: &str) -> Option<OsVersion> {
    value.split_whitespace().find_map(|token| {
        let start = token.find(|character: char| character.is_ascii_digit())?;
        let candidate = token[start..]
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect::<String>();
        let candidate = candidate.split('.').take(3).collect::<Vec<_>>().join(".");
        (candidate.contains('.') && candidate.len() <= 32)
            .then(|| OsVersion::parse(&candidate).ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests;
