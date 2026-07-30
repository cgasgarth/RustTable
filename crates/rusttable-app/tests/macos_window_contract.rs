#[test]
fn desktop_window_action_maximizes_without_entering_native_fullscreen() {
    let composition = include_str!("../src/composition/mod.rs");

    assert!(composition.contains("cfg!(target_os = \"macos\")"));
    assert!(composition.contains("\"window/maximize\""));
    assert!(composition.contains("callback_window.maximize()"));
    assert!(composition.contains("callback_window.unmaximize()"));
    assert!(composition.contains("\"window/fullscreen\""));
    assert!(composition.contains("callback_window.fullscreen()"));
    assert!(composition.contains("callback_window.unfullscreen()"));
}
