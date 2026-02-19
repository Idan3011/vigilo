use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, PartialEq)]
pub(super) enum Platform {
    Linux,
    MacOs,
    Windows,
    Wsl,
}

pub(super) fn detect_platform() -> Platform {
    if cfg!(target_os = "windows") {
        return Platform::Windows;
    }
    if cfg!(target_os = "macos") {
        return Platform::MacOs;
    }
    if is_wsl() {
        return Platform::Wsl;
    }
    Platform::Linux
}

pub(super) fn platform_name() -> &'static str {
    match detect_platform() {
        Platform::Linux => "Linux",
        Platform::MacOs => "macOS",
        Platform::Windows => "Windows",
        Platform::Wsl => "WSL",
    }
}

fn is_wsl() -> bool {
    std::fs::read_to_string("/proc/version")
        .map(|v| v.to_lowercase().contains("microsoft"))
        .unwrap_or(false)
}

pub(super) const DB_SUFFIX: &str = "User/globalStorage/state.vscdb";

pub fn resolve_db_path() -> Result<String> {
    if let Ok(dir) = std::env::var("CURSOR_DATA_DIR") {
        let path = format!("{dir}/{DB_SUFFIX}");
        return require_exists(&path, "CURSOR_DATA_DIR points to a missing DB");
    }

    if let Some(path) = crate::models::load_config().get("CURSOR_DB").cloned() {
        if Path::new(&path).exists() {
            return Ok(path);
        }
    }

    discover_db()
}

pub fn discover_db() -> Result<String> {
    let candidates = candidate_paths();

    candidates
        .iter()
        .find(|p| Path::new(p).exists())
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cursor database not found.\n\
             Platform: {}\n\
             Searched:\n  {}\n\n\
             Run `vigilo setup` to configure, or set CURSOR_DATA_DIR.",
                platform_name(),
                candidates.join("\n  ")
            )
        })
}

fn candidate_paths() -> Vec<String> {
    let home = crate::models::home_dir();
    let home_str = home.to_string_lossy();
    match detect_platform() {
        Platform::Wsl => wsl_candidates(&home_str),
        Platform::MacOs => vec![home
            .join("Library/Application Support/Cursor")
            .join(DB_SUFFIX)
            .to_string_lossy()
            .into_owned()],
        Platform::Windows => windows_candidates(),
        Platform::Linux => vec![home
            .join(".config/Cursor")
            .join(DB_SUFFIX)
            .to_string_lossy()
            .into_owned()],
    }
}

fn windows_candidates() -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        paths.push(format!("{appdata}/Cursor/{DB_SUFFIX}"));
    }
    paths
}

fn wsl_candidates(home: &str) -> Vec<String> {
    let mount = wsl_mount_root();
    let mut paths = Vec::new();

    if let Some(user) = wsl_windows_username() {
        paths.push(format!(
            "{mount}/Users/{user}/AppData/Roaming/Cursor/{DB_SUFFIX}"
        ));
    }

    let users_dir = format!("{mount}/Users");
    if let Ok(entries) = std::fs::read_dir(&users_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_system_user(&name) {
                continue;
            }
            let candidate = format!(
                "{}/AppData/Roaming/Cursor/{DB_SUFFIX}",
                entry.path().display()
            );
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }

    paths.push(format!("{home}/.config/Cursor/{DB_SUFFIX}"));

    paths
}

pub(super) fn is_system_user(name: &str) -> bool {
    matches!(name, "Default" | "Public" | "Default User" | "All Users")
}

fn wsl_mount_root() -> String {
    if let Some(path) = run_command("wslpath", &["-u", "C:\\"]) {
        return path.trim_end_matches('/').to_string();
    }

    if let Ok(conf) = std::fs::read_to_string("/etc/wsl.conf") {
        for line in conf.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("root") {
                if let Some(val) = trimmed
                    .split_once('=')
                    .map(|(_, v)| v.trim().trim_matches('/'))
                {
                    if !val.is_empty() {
                        return format!("/{val}/c");
                    }
                }
            }
        }
    }

    "/mnt/c".to_string()
}

fn wsl_windows_username() -> Option<String> {
    run_command("wslvar", &["USERNAME"]).or_else(|| {
        run_command("cmd.exe", &["/c", "echo", "%USERNAME%"]).filter(|s| !s.contains('%'))
    })
}

fn run_command(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn require_exists(path: &str, hint: &str) -> Result<String> {
    if Path::new(path).exists() {
        Ok(path.to_string())
    } else {
        Err(anyhow::anyhow!("{hint}: {path}"))
    }
}

pub(super) fn open_db(path: &str) -> Result<rusqlite::Connection> {
    let effective = if needs_local_copy(path) {
        copy_to_local(path)?
    } else {
        path.to_string()
    };

    rusqlite::Connection::open_with_flags(&effective, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("cannot open Cursor DB at {path}"))
}

pub(super) fn needs_local_copy(path: &str) -> bool {
    path.starts_with("/mnt/")
}

fn copy_to_local(src: &str) -> Result<String> {
    let dest = crate::models::vigilo_path("cursor-state.vscdb")
        .to_string_lossy()
        .into_owned();
    if let Some(parent) = std::path::Path::new(&dest).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, &dest).with_context(|| format!("failed to copy {src} → {dest}"))?;
    Ok(dest)
}
