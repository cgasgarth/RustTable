use super::*;

fn target(value: &str) -> TargetTriple {
    TargetTriple::new(value).expect("target")
}

fn identity(target_name: &str, os: OperatingSystem, arch: CpuArchitecture) -> PlatformIdentity {
    PlatformIdentity::new(
        os,
        arch,
        target(target_name),
        Some(OsVersion::new(13, 0, 0)),
        LinuxLibc::NotApplicable,
        None,
        Vec::new(),
        true,
        "test-build",
    )
    .expect("identity")
}

fn requirement() -> PlatformRequirement {
    PlatformRequirement::new(
        target("aarch64-apple-darwin"),
        OperatingSystem::MacOs,
        CpuArchitecture::Aarch64,
        Some(OsVersion::new(13, 0, 0)),
        LinuxLibc::NotApplicable,
        None,
        true,
        true,
        Vec::new(),
        BackendPreference::new([GraphicsBackend::Metal]).expect("backend"),
        CpuFallbackPolicy::Required,
        "macos-latest".to_owned(),
        target("aarch64-apple-darwin"),
    )
    .expect("requirement")
}

#[test]
fn versions_compare_without_lexical_ordering() {
    assert!(OsVersion::new(13, 0, 0) < OsVersion::new(13, 1, 0));
    assert_eq!(OsVersion::parse("19045").expect("version").major(), 19045);
    assert_eq!(
        OsVersion::parse("1.2.3-4").expect("version").build(),
        Some(4)
    );
    assert!(OsVersion::parse("1.2.3.4").is_err());
}

#[test]
fn identity_rejects_private_and_ambiguous_values() {
    assert!(
        PlatformIdentity::new(
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
            target("x"),
            None,
            LinuxLibc::NotApplicable,
            None,
            vec![WindowSystem::X11],
            true,
            "build"
        )
        .is_err()
    );
    assert!(
        PlatformIdentity::new(
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
            target("x"),
            None,
            LinuxLibc::Gnu,
            Some(OsVersion::new(1, 0, 0)),
            Vec::new(),
            true,
            "build"
        )
        .is_err()
    );
}

#[test]
fn unsupported_target_fails_closed() {
    let registry = PlatformRegistry::new(vec![requirement()]);
    let decision = registry.evaluate(
        identity(
            "x86_64-unknown-linux-gnu",
            OperatingSystem::Linux,
            CpuArchitecture::X86_64,
        ),
        ApplicationMode::Headless,
    );
    assert_eq!(decision.level(), SupportLevel::UnsupportedPlatform);
    assert_eq!(decision.findings(), [PlatformFinding::UnsupportedTarget]);
}

#[test]
fn supported_identity_is_gpu_candidate_until_probe_qualifies_it() {
    let registry = PlatformRegistry::new(vec![requirement()]);
    let decision = registry.evaluate(
        identity(
            "aarch64-apple-darwin",
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
        ),
        ApplicationMode::Headless,
    );
    assert_eq!(decision.level(), SupportLevel::SupportedGpuCandidate);
}

struct NoGpu;
impl GpuCapabilityProbePort for NoGpu {
    fn has_qualified_backend(
        &self,
        _: &BackendPreference,
        _: CoreComputeRequirements,
    ) -> Result<bool, PlatformFinding> {
        Ok(false)
    }
}

#[test]
fn qualified_probe_failure_preserves_cpu_fallback() {
    let registry = PlatformRegistry::new(vec![requirement()]);
    let decision = registry.evaluate_with_probe(
        identity(
            "aarch64-apple-darwin",
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
        ),
        ApplicationMode::Headless,
        Some(&NoGpu),
    );
    assert_eq!(decision.level(), SupportLevel::SupportedCpuOnly);
    assert!(decision.is_usable());
}
