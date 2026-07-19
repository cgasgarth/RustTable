use super::{CpuArchitecture, OperatingSystem, WindowSystem};

#[cfg(target_os = "linux")]
pub(super) fn current_window_systems() -> (Vec<WindowSystem>, bool) {
    let mut systems = Vec::new();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        systems.push(WindowSystem::Wayland);
    }
    if std::env::var_os("DISPLAY").is_some() {
        systems.push(WindowSystem::X11);
    }
    (systems.clone(), systems.is_empty())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn current_window_systems() -> (Vec<WindowSystem>, bool) {
    (Vec::new(), false)
}

pub(super) fn current_target() -> (OperatingSystem, CpuArchitecture, &'static str) {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return (
            OperatingSystem::Linux,
            CpuArchitecture::X86_64,
            "x86_64-unknown-linux-gnu",
        );
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return (
            OperatingSystem::MacOs,
            CpuArchitecture::Aarch64,
            "aarch64-apple-darwin",
        );
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return (
            OperatingSystem::Windows,
            CpuArchitecture::X86_64,
            "x86_64-pc-windows-msvc",
        );
    }
    #[allow(unreachable_code)]
    (
        OperatingSystem::Unknown,
        CpuArchitecture::Unknown,
        "unknown",
    )
}
