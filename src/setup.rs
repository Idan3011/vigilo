use anyhow::Result;
use std::io::{self, Write};

pub async fn run() -> Result<()> {
    println!("\nvigilo setup\n");

    let has_claude = detect_claude();
    let has_cursor = detect_cursor();

    print_detection(has_claude, has_cursor);

    let default_ledger = crate::models::vigilo_path("events.jsonl").to_string_lossy().into_owned();
    let ledger = prompt(
        &format!("[1/4] Ledger path [{}]", default_ledger),
        &default_ledger,
    )?;

    if let Some(parent) = std::path::Path::new(&ledger).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let encryption_key = setup_encryption()?;
    setup_claude_if_detected(has_claude, &ledger)?;
    let cursor_db = setup_cursor_if_detected(has_cursor, &ledger)?;

    write_config(&ledger, encryption_key.as_deref(), cursor_db.as_deref())?;

    if cursor_db.is_some() {
        sync_cursor_usage().await;
    }

    print_completion(encryption_key.as_deref());
    Ok(())
}

async fn sync_cursor_usage() {
    println!("\n      Syncing Cursor token usage...");
    match crate::cursor::sync(30).await {
        Ok(()) => {}
        Err(e) => eprintln!("      {e}\n      You can retry later with: vigilo cursor-usage"),
    }
}

fn print_detection(has_claude: bool, has_cursor: bool) {
    if has_claude {
        println!("  Claude Code detected ✓");
    }
    if has_cursor {
        println!("  Cursor detected       ✓");
    }
    if !has_claude && !has_cursor {
        println!("  Neither Claude Code nor Cursor detected.");
        println!("  You can still set up vigilo manually — see README.");
    }
    println!();
}

fn setup_encryption() -> Result<Option<String>> {
    println!("\n[2/4] Encryption");

    let existing = crate::crypto::load_key_from_file().is_some()
        || std::env::var("VIGILO_ENCRYPTION_KEY").is_ok();

    if existing {
        println!("      Encryption key already configured ✓");
        return Ok(None);
    }

    println!("      Encrypt file paths and arguments at rest? (AES-256-GCM)");
    println!("      Note: The MCP server auto-generates a key on first run if none exists.");
    if !prompt_yn("      Generate encryption key now?", true)? {
        return Ok(None);
    }

    match crate::crypto::generate_and_save_key() {
        Ok(_) => {
            let path = crate::crypto::key_file_path();
            println!("      ✓ Key saved to {}", path.display());
            let b64 = std::fs::read_to_string(&path)
                .unwrap_or_default()
                .trim()
                .to_string();
            Ok(Some(b64))
        }
        Err(e) => {
            eprintln!("      ! Could not save key file: {e}");
            let key = crate::crypto::generate_key_b64();
            println!("      Generated key: {key}");
            println!("      ⚠  Save this key manually — add to shell profile:");
            println!("         export VIGILO_ENCRYPTION_KEY={key}");
            Ok(Some(key))
        }
    }
}

fn setup_claude_if_detected(has_claude: bool, ledger: &str) -> Result<()> {
    if has_claude {
        println!("\n[3/4] Claude Code integration");
        println!("      Sets up MCP server in ~/.claude.json");
        println!("      Sets up PostToolUse hook in ~/.claude/settings.json");
        if prompt_yn("      Configure Claude Code?", true)? {
            if let Err(e) = setup_claude(ledger) {
                eprintln!("      ! Error: {e}");
            }
        }
    } else {
        println!("\n[3/4] Claude Code — not detected, skipping");
    }
    Ok(())
}

fn setup_cursor_if_detected(has_cursor: bool, ledger: &str) -> Result<Option<String>> {
    if !has_cursor {
        println!("\n[4/4] Cursor — not detected, skipping");
        return Ok(None);
    }
    println!("\n[4/4] Cursor integration");
    println!("      Sets up MCP server in ~/.cursor/mcp.json");
    println!("      Sets up lifecycle hooks in ~/.cursor/hooks.json");
    if !prompt_yn("      Configure Cursor?", true)? {
        return Ok(None);
    }
    if let Err(e) = setup_cursor(ledger) {
        eprintln!("      ! Error: {e}");
    }
    Ok(discover_cursor_db())
}

fn print_completion(encryption_key: Option<&str>) {
    println!("\n  Done.\n");
    println!("  Use your AI editor to make some tool calls, then run:");
    println!("    vigilo view\n");
    println!("  To verify your setup:");
    println!("    vigilo doctor");
    if encryption_key.is_some() {
        let path = crate::crypto::key_file_path();
        println!("\n  Encryption key saved to: {}", path.display());
        println!("  Back up this file — losing it means losing access to encrypted events.");
    }
    println!();
}

fn setup_claude(ledger: &str) -> Result<()> {
    setup_claude_mcp(ledger)?;
    setup_claude_hook()?;
    Ok(())
}

fn setup_claude_mcp(ledger: &str) -> Result<()> {
    let path = crate::models::home_dir().join(".claude.json").to_string_lossy().into_owned();
    let mut config: serde_json::Value = read_json_or_empty(&path);

    if config["mcpServers"].is_null() {
        config["mcpServers"] = serde_json::json!({});
    }
    config["mcpServers"]["vigilo"] = serde_json::json!({
        "command": binary_path(),
        "type": "stdio",
        "env": { "VIGILO_LEDGER": ledger }
    });

    write_json(&path, &config)?;
    println!("      ✓ ~/.claude.json");
    Ok(())
}

fn setup_claude_hook() -> Result<()> {
    let path = crate::models::home_dir().join(".claude/settings.json").to_string_lossy().into_owned();
    let mut config: serde_json::Value = read_json_or_empty(&path);

    let hooks = config["hooks"].as_object_mut().cloned().unwrap_or_default();
    let mut hooks_val = serde_json::Value::Object(hooks.into_iter().collect());

    let already = is_vigilo_hook_present(&hooks_val["PostToolUse"]);
    if !already {
        hooks_val["PostToolUse"] = serde_json::json!([{
            "matcher": ".*",
            "hooks": [{ "type": "command", "command": "vigilo hook" }]
        }]);
    }

    config["hooks"] = hooks_val;
    write_json(&path, &config)?;
    println!("      ✓ ~/.claude/settings.json");
    Ok(())
}

pub(crate) fn is_vigilo_hook_present(post_tool_use: &serde_json::Value) -> bool {
    post_tool_use
        .as_array()
        .map(|arr| {
            arr.iter().any(|h| {
                h["hooks"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|x| x["command"].as_str())
                    == Some("vigilo hook")
            })
        })
        .unwrap_or(false)
}

fn setup_cursor(ledger: &str) -> Result<()> {
    setup_cursor_mcp(ledger)?;
    setup_cursor_hooks()?;
    Ok(())
}

fn setup_cursor_mcp(ledger: &str) -> Result<()> {
    let path = crate::models::home_dir().join(".cursor/mcp.json").to_string_lossy().into_owned();
    let mut config: serde_json::Value = read_json_or_empty(&path);

    if config["mcpServers"].is_null() {
        config["mcpServers"] = serde_json::json!({});
    }
    config["mcpServers"]["vigilo"] = serde_json::json!({
        "command": binary_path(),
        "env": { "VIGILO_LEDGER": ledger }
    });

    write_json(&path, &config)?;
    println!("      ✓ ~/.cursor/mcp.json");
    Ok(())
}

fn setup_cursor_hooks() -> Result<()> {
    let path = crate::models::home_dir().join(".cursor/hooks.json").to_string_lossy().into_owned();
    let mut config = read_json_or_empty(&path);

    if config["version"].is_null() {
        config["version"] = serde_json::json!(1);
    }
    if config["hooks"].is_null() {
        config["hooks"] = serde_json::json!({});
    }

    let our_command = "vigilo hook";
    for hook_type in ["beforeShellExecution", "afterFileEdit"] {
        ensure_hook_entry(&mut config["hooks"], hook_type, our_command);
    }

    write_json(&path, &config)?;
    println!("      ✓ ~/.cursor/hooks.json");
    Ok(())
}

fn ensure_hook_entry(hooks: &mut serde_json::Value, hook_type: &str, command: &str) {
    let existing = hooks[hook_type].as_array().cloned().unwrap_or_default();
    let already = existing
        .iter()
        .any(|h| h["command"].as_str() == Some(command));
    if already {
        return;
    }
    let mut arr = existing;
    arr.push(serde_json::json!({ "command": command }));
    hooks[hook_type] = serde_json::Value::Array(arr);
}

fn discover_cursor_db() -> Option<String> {
    println!("\n      Locating Cursor database...");
    match crate::cursor::discover_db() {
        Ok(path) => {
            println!("      ✓ {path}");
            println!("      (saved to config — `vigilo cursor-usage` will use this)");
            Some(path)
        }
        Err(e) => {
            eprintln!("      ! Could not find Cursor DB: {e}");
            eprintln!("      You can set CURSOR_DATA_DIR later to enable `vigilo cursor-usage`.");
            None
        }
    }
}

const MANAGED_KEYS: &[&str] = &["LEDGER", "CURSOR_DB"];

fn write_config(ledger: &str, encryption_key: Option<&str>, cursor_db: Option<&str>) -> Result<()> {
    let dir = crate::models::vigilo_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config");

    let _ = encryption_key; // key is now stored in ~/.vigilo/encryption.key
    let mut lines = vec![format!("LEDGER={ledger}")];
    if let Some(db) = cursor_db {
        lines.push(format!("CURSOR_DB={db}"));
    }

    if let Ok(existing) = std::fs::read_to_string(&path) {
        for line in existing.lines() {
            let key = line.split('=').next().unwrap_or("").trim();
            if !MANAGED_KEYS.contains(&key) && !line.trim().is_empty() && !line.starts_with('#') {
                lines.push(line.to_string());
            }
        }
    }

    std::fs::write(&path, lines.join("\n") + "\n")?;
    Ok(())
}

fn detect_claude() -> bool {
    crate::models::home_dir().join(".claude").exists() || which("claude").is_some()
}

fn detect_cursor() -> bool {
    crate::models::home_dir().join(".cursor").exists()
        || which("cursor").is_some()
        || crate::cursor::discover_db().is_ok()
}

fn which(cmd: &str) -> Option<String> {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

fn binary_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "vigilo".to_string())
}

fn read_json_or_empty(path: &str) -> serde_json::Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

fn write_json(path: &str, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)? + "\n")?;
    Ok(())
}

fn prompt(question: &str, default: &str) -> Result<String> {
    print!("  {question}: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

fn prompt_yn(question: &str, default_yes: bool) -> Result<bool> {
    let hint = if default_yes { "Y/n" } else { "y/N" };
    print!("  {question} [{hint}]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(match input.trim().to_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default_yes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_vigilo_hook_present_true_when_configured() {
        let val = serde_json::json!([{
            "matcher": ".*",
            "hooks": [{ "type": "command", "command": "vigilo hook" }]
        }]);
        assert!(is_vigilo_hook_present(&val));
    }

    #[test]
    fn is_vigilo_hook_present_false_when_empty() {
        assert!(!is_vigilo_hook_present(&serde_json::json!([])));
    }

    #[test]
    fn is_vigilo_hook_present_false_for_null() {
        assert!(!is_vigilo_hook_present(&serde_json::Value::Null));
    }

    #[test]
    fn is_vigilo_hook_present_false_for_other_hook() {
        let val = serde_json::json!([{
            "matcher": ".*",
            "hooks": [{ "type": "command", "command": "other-tool hook" }]
        }]);
        assert!(!is_vigilo_hook_present(&val));
    }

    #[test]
    fn ensure_hook_entry_adds_new_entry() {
        let mut hooks = serde_json::json!({});
        ensure_hook_entry(&mut hooks, "afterFileEdit", "vigilo hook");
        let arr = hooks["afterFileEdit"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["command"], "vigilo hook");
    }

    #[test]
    fn ensure_hook_entry_is_idempotent() {
        let mut hooks = serde_json::json!({
            "afterFileEdit": [{ "command": "vigilo hook" }]
        });
        ensure_hook_entry(&mut hooks, "afterFileEdit", "vigilo hook");
        assert_eq!(hooks["afterFileEdit"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn ensure_hook_entry_preserves_existing() {
        let mut hooks = serde_json::json!({
            "afterFileEdit": [{ "command": "other-tool" }]
        });
        ensure_hook_entry(&mut hooks, "afterFileEdit", "vigilo hook");
        let arr = hooks["afterFileEdit"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["command"], "other-tool");
        assert_eq!(arr[1]["command"], "vigilo hook");
    }

    #[test]
    fn read_json_or_empty_returns_empty_for_missing_file() {
        let val = read_json_or_empty("/nonexistent/path.json");
        assert_eq!(val, serde_json::json!({}));
    }

    #[test]
    fn read_json_or_empty_parses_valid_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.json");
        std::fs::write(&path, r#"{"key": "value"}"#).expect("write");
        let val = read_json_or_empty(path.to_str().unwrap());
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn read_json_or_empty_returns_empty_for_invalid_json() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json at all").expect("write");
        let val = read_json_or_empty(path.to_str().unwrap());
        assert_eq!(val, serde_json::json!({}));
    }

    #[test]
    fn write_json_creates_parent_dirs() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("sub/dir/file.json");
        let val = serde_json::json!({"hello": "world"});
        write_json(path.to_str().unwrap(), &val).expect("write_json");
        let contents = std::fs::read_to_string(&path).expect("read");
        let parsed: serde_json::Value = serde_json::from_str(contents.trim()).expect("parse");
        assert_eq!(parsed["hello"], "world");
    }

    #[test]
    fn write_config_all_scenarios() {
        let dir = tempfile::tempdir().expect("temp dir");
        let home_str = dir.path().to_str().unwrap();
        std::env::set_var("HOME", home_str);

        let config_path = dir.path().join(".vigilo/config");
        let ledger = dir.path().join("events.jsonl");
        let ledger_str = ledger.to_str().unwrap();

        write_config(ledger_str, None, None).expect("write_config basic");
        let contents = std::fs::read_to_string(&config_path).expect("read");
        assert!(contents.starts_with("LEDGER="));

        write_config(ledger_str, None, Some("/some/cursor.db")).expect("write_config cursor_db");
        let contents = std::fs::read_to_string(&config_path).expect("read");
        assert!(contents.contains("CURSOR_DB=/some/cursor.db"));

        std::fs::write(&config_path, "LEDGER=old\nMY_CUSTOM=keep\n").expect("seed");
        write_config(ledger_str, None, None).expect("write_config preserve");
        let contents = std::fs::read_to_string(&config_path).expect("read");
        assert!(contents.contains("MY_CUSTOM=keep"));
        assert!(!contents.contains("LEDGER=old"));

        std::env::remove_var("HOME");
    }

    #[test]
    fn binary_path_returns_something() {
        let path = binary_path();
        assert!(!path.is_empty());
    }

    #[test]
    fn home_returns_home_dir() {
        let h = crate::models::home_dir();
        assert!(!h.as_os_str().is_empty());
    }
}
