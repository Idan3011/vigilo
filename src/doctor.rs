use crate::view::fmt::{cprintln, BOLD, CYAN, DIM, GREEN, RED, RESET};
use std::path::Path;

pub fn run(ledger_path: &str) {
    cprintln!();
    cprintln!("{DIM}── vigilo doctor ───────────────────────────────{RESET}");
    cprintln!();

    let mut pass = 0;
    let mut fail = 0;

    check_ledger(ledger_path, &mut pass, &mut fail);
    check_encryption_key(&mut pass, &mut fail);
    check_config(&mut pass, &mut fail);
    check_cursor_db(&mut pass, &mut fail);

    cprintln!();
    cprintln!(
        "  {BOLD}{pass}{RESET} passed  {}{fail}{} failed",
        if fail > 0 { RED } else { DIM },
        RESET
    );
    cprintln!();
}

fn check_ledger(ledger_path: &str, pass: &mut u32, fail: &mut u32) {
    let path = Path::new(ledger_path);

    if path.exists() {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let display = if size >= 1_048_576 {
            format!("{:.1}MB", size as f64 / 1_048_576.0)
        } else if size >= 1024 {
            format!("{}KB", size / 1024)
        } else {
            format!("{size}B")
        };
        ok(&format!("ledger exists ({display})"), pass);
    } else if let Some(parent) = path.parent() {
        if parent.exists() || std::fs::create_dir_all(parent).is_ok() {
            ok("ledger directory writable", pass);
        } else {
            err("ledger directory not writable", fail);
        }
    } else {
        err("ledger path invalid", fail);
    }

    let rotated = count_rotated_files(ledger_path);
    if rotated > 0 {
        cprintln!("  {CYAN}i{RESET}  {rotated} rotated ledger file(s)");
    }
}

fn count_rotated_files(ledger_path: &str) -> usize {
    let path = Path::new(ledger_path);
    let Some(parent) = path.parent() else {
        return 0;
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("events");

    std::fs::read_dir(parent)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.starts_with(stem)
                        && name.ends_with(".jsonl")
                        && name
                            != path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .as_ref()
                })
                .count()
        })
        .unwrap_or(0)
}

fn check_encryption_key(pass: &mut u32, fail: &mut u32) {
    match std::env::var("VIGILO_ENCRYPTION_KEY") {
        Ok(val) => {
            use base64::{engine::general_purpose::STANDARD, Engine};
            match STANDARD.decode(&val) {
                Ok(bytes) if bytes.len() == 32 => ok("encryption key valid (AES-256)", pass),
                Ok(bytes) => err(
                    &format!("encryption key wrong size ({} bytes, need 32)", bytes.len()),
                    fail,
                ),
                Err(_) => err("encryption key is not valid base64", fail),
            }
        }
        Err(_) => {
            cprintln!("  {DIM}-{RESET}  encryption key not set (content stored in plaintext)");
        }
    }
}

fn check_config(pass: &mut u32, _fail: &mut u32) {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let config_path = format!("{home}/.vigilo/config");

    if !Path::new(&config_path).exists() {
        cprintln!("  {DIM}-{RESET}  no config file (~/.vigilo/config)");
        return;
    }

    let config = crate::server::load_config();
    if config.is_empty() {
        cprintln!("  {DIM}-{RESET}  config file empty");
    } else {
        let keys: Vec<&str> = config.keys().map(|k| k.as_str()).collect();
        ok(&format!("config loaded ({})", keys.join(", ")), pass);

        for key in config.keys() {
            if !matches!(
                key.as_str(),
                "TAG" | "TIMEOUT_SECS" | "CURSOR_DB" | "STORE_RESPONSE" | "LEDGER"
            ) {
                cprintln!("  {CYAN}i{RESET}  unknown config key: {key}");
            }
        }
    }
}

fn check_cursor_db(pass: &mut u32, _fail: &mut u32) {
    match crate::cursor_usage::resolve_db_path() {
        Ok(path) => ok(&format!("cursor DB found ({})", short_path(&path)), pass),
        Err(_) => {
            cprintln!("  {DIM}-{RESET}  cursor DB not found (optional — for cursor-usage)");
        }
    }
}

fn short_path(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

fn ok(msg: &str, pass: &mut u32) {
    cprintln!("  {GREEN}✓{RESET}  {msg}");
    *pass += 1;
}

fn err(msg: &str, fail: &mut u32) {
    cprintln!("  {RED}✗{RESET}  {msg}");
    *fail += 1;
}
