use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::model::ReferenceIdentity;

pub(crate) fn reference_commit(source: &Path) -> String {
    if let Ok(commit) = fs::read_to_string(source.join(".rusttable-reference-commit")) {
        let commit = commit.trim();
        if !commit.is_empty() {
            return commit.to_owned();
        }
    }
    std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args([
            "-C",
            &source.display().to_string(),
            "rev-parse",
            "--verify",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "fixture".to_owned(), |value| value.trim().to_owned())
}

pub(crate) fn reference_identity(source: &Path) -> ReferenceIdentity {
    let commit = reference_commit(source);
    let canonical = commit == "cfe57f3bbf5269bfacf31e832267279caa6938ad";
    ReferenceIdentity {
        source_commit: commit,
        build_version: if canonical {
            "5.7.0".to_owned()
        } else {
            "darktable-reference".to_owned()
        },
        executable_hash: if canonical {
            "23de77c31d57acf7d2270cbe26485e8d568f541b34852b795b2cd22098a694ef".to_owned()
        } else {
            "not-built".to_owned()
        },
        data_bundle_hash: if canonical {
            "9a9f5dbb9a05fcdb3e1b66a350eb44d6173c38fd85a041e43ce48bac11199b8b".to_owned()
        } else {
            "not-built".to_owned()
        },
        target_triple: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        c_abi_model: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        build_option_hash: if canonical {
            "2d1acec2a7a2cedec88d7ae509f7d52c2f703be04076a7063db46e0744d0f5f4".to_owned()
        } else {
            "not-built".to_owned()
        },
        canonical_identity: if canonical {
            "fixtures/reference/darktable.toml".to_owned()
        } else {
            "fixture".to_owned()
        },
        identity_hash: if canonical {
            "4a4f64adf4c57bb63e7ee3d7f8f4d91f8fba2a0a3c6c42c6f24bc1d6748eaf45".to_owned()
        } else {
            String::new()
        },
        version: "5.7.0".to_owned(),
        executable_sha256: if canonical {
            "23de77c31d57acf7d2270cbe26485e8d568f541b34852b795b2cd22098a694ef".to_owned()
        } else {
            "not-built".to_owned()
        },
        data_dir_sha256: if canonical {
            "9a9f5dbb9a05fcdb3e1b66a350eb44d6173c38fd85a041e43ce48bac11199b8b".to_owned()
        } else {
            "not-built".to_owned()
        },
        opencl_bundle_sha256: if canonical {
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned()
        } else {
            "not-built".to_owned()
        },
        target: if canonical {
            "x86_64-unknown-linux-gnu".to_owned()
        } else {
            "aarch64-apple-darwin".to_owned()
        },
        architecture: if canonical {
            "x86_64".to_owned()
        } else {
            "aarch64".to_owned()
        },
        build_options_hash: if canonical {
            "2d1acec2a7a2cedec88d7ae509f7d52c2f703be04076a7063db46e0744d0f5f4".to_owned()
        } else {
            "not-built".to_owned()
        },
        compiler: if canonical {
            "gcc-darktable-5.7.0".to_owned()
        } else {
            "not-built".to_owned()
        },
        native_library_identity: if canonical {
            "darktable-native-5.7.0".to_owned()
        } else {
            "not-built".to_owned()
        },
        cli_reference_hash: "darktable-cli-man-v1".to_owned(),
    }
}

pub(crate) fn manifest_reference(
    identity: &rusttable_testkit::reference::ReferenceIdentity,
) -> ReferenceIdentity {
    let receipt = identity.receipt();
    let canonical = serde_json::to_vec(&receipt).unwrap_or_default();
    let mut identity_hash = String::with_capacity(64);
    for byte in Sha256::digest(canonical) {
        write!(&mut identity_hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    ReferenceIdentity {
        source_commit: identity.commit.clone(),
        build_version: identity.version.clone(),
        executable_hash: identity.executable_sha256.clone(),
        data_bundle_hash: identity.data_dir_sha256.clone(),
        target_triple: identity.target.clone(),
        c_abi_model: identity.c_abi_model.clone(),
        build_option_hash: identity.build_options_hash.clone(),
        canonical_identity: "fixtures/reference/darktable.toml".to_owned(),
        identity_hash,
        version: identity.version.clone(),
        executable_sha256: identity.executable_sha256.clone(),
        data_dir_sha256: identity.data_dir_sha256.clone(),
        opencl_bundle_sha256: identity.opencl_bundle_sha256.clone(),
        target: identity.target.clone(),
        architecture: identity.architecture.clone(),
        build_options_hash: identity.build_options_hash.clone(),
        compiler: identity.compiler.clone(),
        native_library_identity: identity.native_library_identity.clone(),
        cli_reference_hash: identity.cli.reference_hash.clone(),
    }
}
