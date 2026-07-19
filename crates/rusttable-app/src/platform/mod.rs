//! Startup platform preflight and narrow native identity adapters.

use rusttable_core::platform::{
    ApplicationMode, LinuxLibc, OperatingSystem, OsVersion, PlatformDecision, PlatformIdentity,
    PlatformRegistry, SupportLevel, current_platform_identity, fallback_identity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StartupPreflight {
    Supported(PlatformDecision),
    Unsupported(PlatformDecision),
}

impl StartupPreflight {
    pub(crate) const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported(_))
    }
}

pub(crate) fn startup_preflight() -> StartupPreflight {
    let registry = PlatformRegistry::default();
    let identity = collect_identity().unwrap_or_else(fallback_identity);
    let decision = registry.evaluate(identity, ApplicationMode::Desktop);
    if decision.level() == SupportLevel::UnsupportedPlatform {
        StartupPreflight::Unsupported(decision)
    } else {
        StartupPreflight::Supported(decision)
    }
}

fn collect_identity() -> Option<PlatformIdentity> {
    let base = current_platform_identity()?;
    let (os_version, libc, libc_version) = native_versions(base.operating_system())?;
    PlatformIdentity::new(
        base.operating_system(),
        base.architecture(),
        base.target().clone(),
        os_version,
        libc,
        libc_version,
        base.window_systems().to_vec(),
        base.headless(),
        base.application_build(),
    )
    .ok()
}

#[cfg(target_os = "linux")]
fn native_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("ldd")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let native_text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let native_text = if native_text.contains("glibc") || native_text.contains("gnu libc") {
        native_text
    } else {
        String::from_utf8_lossy(&output.stderr).to_ascii_lowercase()
    };
    if !native_text.contains("glibc") && !native_text.contains("gnu libc") {
        return None;
    }
    let version = find_dotted_version(&native_text)?;
    Some((Some(version), LinuxLibc::Gnu, Some(version)))
}

#[cfg(target_os = "macos")]
fn native_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = OsVersion::parse(String::from_utf8_lossy(&output.stdout).trim()).ok()?;
    Some((Some(version), LinuxLibc::NotApplicable, None))
}

#[cfg(target_os = "windows")]
fn native_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    let output = std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = find_dotted_version(&String::from_utf8_lossy(&output.stdout))?;
    let version = OsVersion::new(version.major(), version.minor(), 0).with_build(version.patch());
    Some((Some(version), LinuxLibc::NotApplicable, None))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn native_versions(
    _: OperatingSystem,
) -> Option<(Option<OsVersion>, LinuxLibc, Option<OsVersion>)> {
    None
}

#[allow(dead_code)]
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
mod tests {
    use super::StartupPreflight;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use super::find_dotted_version;
    use rusttable_core::platform::SupportLevel;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn native_version_parsing_discards_unbounded_suffixes() {
        assert_eq!(
            find_dotted_version("GNU libc 2.35-0ubuntu3.8"),
            Some(rusttable_core::platform::OsVersion::new(2, 35, 0))
        );
        assert_eq!(
            find_dotted_version("Microsoft Windows [Version 10.0.19045.6093]"),
            Some(rusttable_core::platform::OsVersion::new(10, 0, 19045))
        );
    }

    #[test]
    fn fallback_preflight_fails_closed() {
        let decision = rusttable_core::platform::PlatformRegistry::default().evaluate(
            rusttable_core::platform::fallback_identity(),
            rusttable_core::platform::ApplicationMode::Desktop,
        );
        let result = if decision.level() == SupportLevel::UnsupportedPlatform {
            StartupPreflight::Unsupported(decision)
        } else {
            StartupPreflight::Supported(decision)
        };
        assert!(!result.is_supported());
        assert_eq!(
            match result {
                StartupPreflight::Unsupported(decision) | StartupPreflight::Supported(decision) => {
                    decision.level()
                }
            },
            SupportLevel::UnsupportedPlatform
        );
    }
}
