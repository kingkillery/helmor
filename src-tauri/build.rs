use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const GITHUB_CLIENT_ID_KEY: &str = "HELMOR_GITHUB_CLIENT_ID";
const UPDATER_ENDPOINTS_KEY: &str = "HELMOR_UPDATER_ENDPOINTS";
const UPDATER_PUBKEY_KEY: &str = "HELMOR_UPDATER_PUBKEY";

fn main() {
    ensure_external_bin_placeholders();

    println!("cargo:rerun-if-changed=build.rs");
    for key in [
        GITHUB_CLIENT_ID_KEY,
        UPDATER_ENDPOINTS_KEY,
        UPDATER_PUBKEY_KEY,
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }

    for env_path in candidate_env_paths() {
        // Only watch files that exist. Watching a missing file makes Cargo
        // treat the fingerprint as permanently stale, which forces a full
        // recompile of the crate on every single `cargo build` invocation.
        if env_path.exists() {
            println!("cargo:rerun-if-changed={}", env_path.display());
        }
        load_env_var(&env_path, GITHUB_CLIENT_ID_KEY);
        load_env_var(&env_path, UPDATER_ENDPOINTS_KEY);
        load_env_var(&env_path, UPDATER_PUBKEY_KEY);
    }

    tauri_build::build();
}

fn ensure_external_bin_placeholders() {
    let Ok(target) = env::var("TARGET") else {
        return;
    };

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"));
    ensure_executable_placeholder(external_bin_path(
        manifest_dir.join("target").join("bundled"),
        "helmor-cli",
        &target,
    ));

    if let Some(repo_root) = manifest_dir.parent() {
        ensure_executable_placeholder(external_bin_path(
            repo_root.join("sidecar").join("dist"),
            "helmor-sidecar",
            &target,
        ));
    }
}

fn external_bin_path(dir: PathBuf, base_name: &str, target: &str) -> PathBuf {
    let file_name = if cfg!(windows) {
        format!("{base_name}-{target}.exe")
    } else {
        format!("{base_name}-{target}")
    };
    dir.join(file_name)
}

fn ensure_executable_placeholder(path: PathBuf) {
    if path.exists() {
        return;
    }

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    #[cfg(windows)]
    let placeholder = "@echo off\r\nexit /b 0\r\n";
    #[cfg(not(windows))]
    let placeholder = "#!/bin/sh\nexit 0\n";

    let _ = fs::write(&path, placeholder);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o755));
    }
}

fn candidate_env_paths() -> Vec<PathBuf> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"));
    let mut paths = vec![manifest_dir.join(".env.local")];

    if let Some(repo_root) = manifest_dir.parent() {
        paths.push(repo_root.join(".env.local"));
        // Lowest-priority fallback: committed `.env.example` provides defaults
        // for public values (e.g. GitHub Device Flow client ID) so a fresh
        // `cargo build` works without any manual `cp .env.example .env.local`.
        paths.push(repo_root.join(".env.example"));
    }

    paths
}

fn load_env_var(path: &Path, key: &str) {
    if env::var_os(key).is_some() || !path.exists() {
        return;
    }

    let Ok(iter) = dotenvy::from_path_iter(path) else {
        return;
    };

    for item in iter.flatten() {
        if item.0 == key {
            println!("cargo:rustc-env={}={}", item.0, item.1);
            break;
        }
    }
}
