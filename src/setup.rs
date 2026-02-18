use anyhow::Result;
use std::io::{self, Write};

pub async fn run() -> Result<()> {
    println!("\nvigilo setup\n");

    let has_claude = detect_claude();
    let has_cursor = detect_cursor();

    print_detection(has_claude, has_cursor);

    let default_ledger = format!("{}/.vigilo/events.jsonl", home());
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
    match crate::cursor_usage::sync(30).await {
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
    println!("      Encrypt file paths and arguments at rest? (AES-256-GCM)");
    println!("      Recommended for sensitive codebases. You can enable it later.");
    if !prompt_yn("      Enable encryption?", false)? {
        return Ok(None);
    }
    let key = crate::crypto::generate_key_b64();
    println!("      Generated key: {key}");
    println!("      ⚠  Save this key — losing it means losing access to encrypted events.");
    Ok(Some(key))
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
    println!("  Run:  vigilo view");
    if let Some(key) = encryption_key {
        println!("  Add to your shell profile:");
        println!("  export VIGILO_ENCRYPTION_KEY={key}");
    }
    println!();
}

fn setup_claude(ledger: &str) -> Result<()> {
    setup_claude_mcp(ledger)?;
    setup_claude_hook()?;
    Ok(())
}

fn setup_claude_mcp(ledger: &str) -> Result<()> {
    let path = format!("{}/.claude.json", home());
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
    let path = format!("{}/.claude/settings.json", home());
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

fn is_vigilo_hook_present(post_tool_use: &serde_json::Value) -> bool {
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
    let path = format!("{}/.cursor/mcp.json", home());
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
    let path = format!("{}/.cursor/hooks.json", home());
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
    match crate::cursor_usage::discover_db() {
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
    let dir = format!("{}/.vigilo", home());
    std::fs::create_dir_all(&dir)?;
    let path = format!("{dir}/config");

    let mut lines = vec![format!("LEDGER={ledger}")];
    if let Some(db) = cursor_db {
        lines.push(format!("CURSOR_DB={db}"));
    }
    if let Some(key) = encryption_key {
        lines.push(format!(
            "# Add to shell profile: export VIGILO_ENCRYPTION_KEY={key}"
        ));
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
    std::path::Path::new(&format!("{}/.claude", home())).exists() || which("claude").is_some()
}

fn detect_cursor() -> bool {
    std::path::Path::new(&format!("{}/.cursor", home())).exists()
        || which("cursor").is_some()
        || crate::cursor_usage::discover_db().is_ok()
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

fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| ".".into())
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
