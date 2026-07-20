use std::path::Path;
use std::process::Command;

use crate::{Result, run_process};

pub(crate) fn run(root: &Path) -> Result {
    match std::env::consts::OS {
        "macos" => run_process(
            "macOS distribution",
            Command::new("bash")
                .current_dir(root)
                .arg("scripts/macos-distribution-smoke.sh"),
        ),
        "linux" => run_process(
            "Linux distribution",
            Command::new("bash")
                .current_dir(root)
                .arg("scripts/linux-distribution-smoke.sh"),
        ),
        platform => Err(format!(
            "distribution is not implemented for host platform {platform}"
        )),
    }
}
