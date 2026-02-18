use crate::models::{self, Risk};
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

mod execute;
mod schema;
mod tools;

struct SessionCounters {
    total: u64,
    reads: u64,
    writes: u64,
    execs: u64,
    errors: u64,
}

pub async fn run(ledger_path: String, session_id: Uuid) -> Result<()> {
    let (project_root, project_name, tag, timeout_secs) = init_session().await;

    if let Some(ref t) = tag {
        eprintln!("[vigilo] tag={t}");
    }
    eprintln!("[vigilo] timeout={timeout_secs}s");

    let mut counters = SessionCounters {
        total: 0,
        reads: 0,
        writes: 0,
        execs: 0,
        errors: 0,
    };
    let started = std::time::Instant::now();

    process_messages(
        &ledger_path,
        session_id,
        &project_root,
        &project_name,
        tag.as_deref(),
        timeout_secs,
        &mut counters,
    )
    .await?;

    print_session_summary(session_id, &counters, started.elapsed().as_secs());
    Ok(())
}

async fn init_session() -> (Option<String>, Option<String>, Option<String>, u64) {
    let project_root = crate::git::root().await;
    let project_name = crate::git::name().await;
    let config = load_config();

    let project_branch = match project_root.as_deref() {
        Some(root) => crate::git::branch_in(root).await,
        None => crate::git::branch().await,
    };
    let tag = std::env::var("VIGILO_TAG")
        .ok()
        .or_else(|| config.get("TAG").cloned())
        .or(project_branch);

    let timeout_secs: u64 = std::env::var("VIGILO_TIMEOUT_SECS")
        .ok()
        .or_else(|| config.get("TIMEOUT_SECS").cloned())
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    (project_root, project_name, tag, timeout_secs)
}

#[allow(clippy::too_many_arguments)]
async fn process_messages(
    ledger_path: &str,
    session_id: Uuid,
    project_root: &Option<String>,
    project_name: &Option<String>,
    tag: Option<&str>,
    timeout_secs: u64,
    counters: &mut SessionCounters,
) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let response = dispatch(
            &msg,
            ledger_path,
            session_id,
            project_root,
            project_name,
            tag,
            timeout_secs,
        )
        .await;
        if let Some(response) = response {
            update_counters(&msg, &response, counters);
            let json = serde_json::to_string(&response)?;
            stdout.write_all(json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}

fn update_counters(
    msg: &serde_json::Value,
    response: &serde_json::Value,
    counters: &mut SessionCounters,
) {
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return;
    }
    let tool = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");
    counters.total += 1;
    if response.get("error").is_some() {
        counters.errors += 1;
    }
    match Risk::classify(tool) {
        Risk::Read => counters.reads += 1,
        Risk::Write => counters.writes += 1,
        Risk::Exec => counters.execs += 1,
        Risk::Unknown => {}
    }
}

fn print_session_summary(session_id: Uuid, c: &SessionCounters, elapsed: u64) {
    let sid = &session_id.to_string()[..8];
    eprintln!(
        "[vigilo] session {sid} ended — {} calls  read:{} write:{} exec:{} errors:{}  {elapsed}s",
        c.total, c.reads, c.writes, c.execs, c.errors
    );
}

async fn dispatch(
    msg: &serde_json::Value,
    ledger_path: &str,
    session_id: Uuid,
    project_root: &Option<String>,
    project_name: &Option<String>,
    tag: Option<&str>,
    timeout_secs: u64,
) -> Option<serde_json::Value> {
    let method = msg.get("method")?.as_str()?;

    match method {
        "initialize" => Some(on_initialize(msg)),
        "ping" => Some(on_ping(msg)),
        "tools/list" => Some(schema::on_tools_list(msg)),
        "tools/call" => Some(
            execute::on_tool_call(
                msg,
                ledger_path,
                session_id,
                project_root,
                project_name,
                tag,
                timeout_secs,
            )
            .await,
        ),
        _ => None,
    }
}

fn on_initialize(msg: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": msg["id"],
        "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "vigilo", "version": "0.1.0" },
        },
    })
}

fn on_ping(msg: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": msg["id"], "result": {} })
}

fn log_event(tool: &str, risk: Risk, duration_us: u64, is_error: bool) {
    let label = match risk {
        Risk::Read => "READ   ",
        Risk::Write => "WRITE  ",
        Risk::Exec => "EXEC   ",
        Risk::Unknown => "UNKNOWN",
    };
    let status = if is_error { "ERR" } else { "OK " };
    let dur = models::fmt_duration(duration_us);
    if matches!(risk, Risk::Exec) {
        eprintln!("⚠  [{status}] {label}  {tool}  ({dur})  ← EXEC");
    } else {
        eprintln!("[{status}] {label}  {tool}  ({dur})");
    }
}

pub fn load_config() -> std::collections::HashMap<String, String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = format!("{home}/.vigilo/config");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return std::collections::HashMap::new();
    };
    content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}
