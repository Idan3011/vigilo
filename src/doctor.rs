use crate::view::fmt::{cprintln, BOLD, CYAN, DIM, GREEN, RED, RESET};
use std::path::Path;

pub fn run(ledger_path: &str) {
    cprintln!();
    cprintln!("{DIM}── vigilo doctor ───────────────────────────────{RESET}");
    cprintln!();

    let mut pass = 0;
    let mut fail = 0;

    check_ledger(ledger_path, &mut pass, &mut fail);
    check_disk_space(ledger_path);
    check_encryption_key(&mut pass, &mut fail);
    check_config(&mut pass, &mut fail);
    check_claude_mcp(&mut pass, &mut fail);
    check_claude_hook(&mut pass, &mut fail);
    check_cursor_mcp(&mut pass, &mut fail);
    check_cursor_db(&mut pass, &mut fail);
    check_mcp_session(&mut pass);

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
        let display = format_size(size);
        ok(&format!("ledger exists ({display})"), pass);
        check_ledger_event_count(ledger_path);
    } else if let Some(parent) = path.parent() {
        if parent.exists() || std::fs::create_dir_all(parent).is_ok() {
            ok("ledger directory writable (no events yet)", pass);
        } else {
            err("ledger directory not writable", fail);
        }
    } else {
        err("ledger path invalid", fail);
    }

    let (rotated, rotated_size) = count_rotated_files(ledger_path);
    if rotated > 0 {
        let active_size = std::fs::metadata(Path::new(ledger_path))
            .map(|m| m.len())
            .unwrap_or(0);
        let total = format_size(active_size + rotated_size);
        cprintln!("  {CYAN}i{RESET}  {rotated} rotated ledger file(s) ({total} total)");
    }
}

fn format_size(size: u64) -> String {
    if size >= 1_048_576 {
        format!("{:.1}MB", size as f64 / 1_048_576.0)
    } else if size >= 1024 {
        format!("{}KB", size / 1024)
    } else {
        format!("{size}B")
    }
}

fn check_ledger_event_count(ledger_path: &str) {
    let content = match std::fs::read_to_string(ledger_path) {
        Ok(s) => s,
        Err(_) => return,
    };

    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let total = lines.len();

    if total == 0 {
        cprintln!("  {CYAN}i{RESET}  ledger has 0 events — is vigilo registered as an MCP server?");
        return;
    }

    let bad = lines
        .iter()
        .filter(|l| serde_json::from_str::<serde_json::Value>(l).is_err())
        .count();

    if bad == 0 {
        cprintln!("  {CYAN}i{RESET}  {total} events in active ledger (all valid JSON)");
    } else {
        cprintln!("  {RED}!{RESET}  {total} events in active ledger ({bad} malformed line(s))");
    }
}

fn count_rotated_files(ledger_path: &str) -> (usize, u64) {
    let path = Path::new(ledger_path);
    let Some(parent) = path.parent() else {
        return (0, 0);
    };
    let stem = crate::ledger::ledger_stem(path);
    let active_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut count = 0usize;
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(stem) && name.ends_with(".jsonl") && name != active_name {
                count += 1;
                size += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    (count, size)
}

fn check_disk_space(ledger_path: &str) {
    let dir = Path::new(ledger_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));

    let dir_cstr = match std::ffi::CString::new(dir.to_string_lossy().as_bytes().to_vec()) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::statvfs(dir_cstr.as_ptr(), &mut stat) };
    if ret == 0 {
        #[allow(clippy::unnecessary_cast)]
        let avail = stat.f_bavail as u64 * stat.f_frsize as u64;
        if avail < 100 * 1024 * 1024 {
            cprintln!(
                "  {RED}!{RESET}  low disk space: {} available on ledger filesystem",
                format_size(avail)
            );
        }
    }
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
    let home = crate::models::home();
    let config_path = format!("{home}/.vigilo/config");

    if !Path::new(&config_path).exists() {
        cprintln!("  {DIM}-{RESET}  no config file (~/.vigilo/config)");
        return;
    }

    let config = crate::models::load_config();
    if config.is_empty() {
        cprintln!("  {DIM}-{RESET}  config file empty");
    } else {
        let keys: Vec<&str> = config.keys().map(|k| k.as_str()).collect();
        ok(&format!("config loaded ({})", keys.join(", ")), pass);

        for key in config.keys() {
            if !matches!(
                key.as_str(),
                "TAG"
                    | "TIMEOUT_SECS"
                    | "CURSOR_DB"
                    | "STORE_RESPONSE"
                    | "HOOK_STORE_RESPONSE"
                    | "LEDGER"
            ) {
                cprintln!("  {CYAN}i{RESET}  unknown config key: {key}");
            }
        }
    }
}

fn check_claude_mcp(pass: &mut u32, fail: &mut u32) {
    let path = format!("{}/.claude.json", crate::models::home());
    let config = read_json(&path);
    match config {
        None => {
            cprintln!("  {DIM}-{RESET}  ~/.claude.json not found");
        }
        Some(val) => {
            if val["mcpServers"]["vigilo"].is_object() {
                ok("Claude Code MCP server registered", pass);
            } else {
                err("vigilo not in ~/.claude.json mcpServers", fail);
            }
        }
    }
}

fn check_claude_hook(pass: &mut u32, fail: &mut u32) {
    let path = format!("{}/.claude/settings.json", crate::models::home());
    let config = read_json(&path);
    match config {
        None => {
            cprintln!("  {DIM}-{RESET}  ~/.claude/settings.json not found");
        }
        Some(val) => {
            if crate::setup::is_vigilo_hook_present(&val["hooks"]["PostToolUse"]) {
                ok("Claude Code PostToolUse hook installed", pass);
            } else {
                err(
                    "vigilo hook not in ~/.claude/settings.json — run 'vigilo setup'",
                    fail,
                );
            }
        }
    }
}

fn check_cursor_mcp(pass: &mut u32, fail: &mut u32) {
    let path = format!("{}/.cursor/mcp.json", crate::models::home());
    let config = read_json(&path);
    match config {
        None => {
            cprintln!("  {DIM}-{RESET}  ~/.cursor/mcp.json not found (optional)");
        }
        Some(val) => {
            if val["mcpServers"]["vigilo"].is_object() {
                ok("Cursor MCP server registered", pass);
            } else {
                err("vigilo not in ~/.cursor/mcp.json mcpServers", fail);
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

fn check_mcp_session(pass: &mut u32) {
    let path = crate::models::mcp_session_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        cprintln!("  {DIM}-{RESET}  no active MCP session");
        return;
    };
    let alive = content
        .lines()
        .nth(1)
        .and_then(|p| p.parse::<u32>().ok())
        .map(|pid| Path::new(&format!("/proc/{pid}")).exists())
        .unwrap_or(false);
    if alive {
        ok("MCP server running (session sync active)", pass);
    } else {
        cprintln!("  {DIM}-{RESET}  MCP session file stale (server not running)");
    }
}

fn read_json(path: &str) -> Option<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

fn short_path(path: &str) -> String {
    crate::models::shorten_home(path)
}

fn ok(msg: &str, pass: &mut u32) {
    cprintln!("  {GREEN}✓{RESET}  {msg}");
    *pass += 1;
}

fn err(msg: &str, fail: &mut u32) {
    cprintln!("  {RED}✗{RESET}  {msg}");
    *fail += 1;
}
