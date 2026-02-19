use crate::models::Risk;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

mod execute;
mod schema;
mod tools;

pub(crate) struct ServerContext {
    pub ledger_path: String,
    pub session_id: Uuid,
    pub project_root: Option<String>,
    pub project_name: Option<String>,
    pub tag: Option<String>,
    pub timeout_secs: u64,
    pub encryption_key: Option<[u8; 32]>,
}

struct SessionCounters {
    total: u64,
    reads: u64,
    writes: u64,
    execs: u64,
    errors: u64,
}

pub async fn run(ledger_path: String, session_id: Uuid) -> Result<()> {
    let (project_root, project_name, tag, timeout_secs) = init_session().await;
    let encryption_key = crate::crypto::load_or_create_key();

    if let Some(ref t) = tag {
        eprintln!("[vigilo] tag={t}");
    }
    eprintln!("[vigilo] timeout={timeout_secs}s");

    let ctx = ServerContext {
        ledger_path,
        session_id,
        project_root,
        project_name,
        tag,
        timeout_secs,
        encryption_key,
    };

    let mut counters = SessionCounters {
        total: 0,
        reads: 0,
        writes: 0,
        execs: 0,
        errors: 0,
    };
    let started = std::time::Instant::now();

    process_messages(&ctx, &mut counters).await?;

    cleanup_mcp_session_file();
    print_session_summary(ctx.session_id, &counters, started.elapsed().as_secs());
    Ok(())
}

async fn init_session() -> (Option<String>, Option<String>, Option<String>, u64) {
    let project_root = crate::git::root().await;
    let project_name = crate::git::name().await;
    let config = crate::models::load_config();

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

async fn process_messages(ctx: &ServerContext, counters: &mut SessionCounters) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        let line = tokio::select! {
            result = lines.next_line() => match result? {
                Some(line) => line,
                None => break,
            },
            _ = shutdown.recv() => {
                eprintln!("[vigilo] interrupted");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let response = dispatch(&msg, ctx).await;
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
    let full = session_id.to_string();
    let sid = &full[..8];
    eprintln!(
        "[vigilo] session {sid} ended — {} calls  read:{} write:{} exec:{} errors:{}  {elapsed}s",
        c.total, c.reads, c.writes, c.execs, c.errors
    );
}

async fn dispatch(msg: &serde_json::Value, ctx: &ServerContext) -> Option<serde_json::Value> {
    let method = msg.get("method")?.as_str()?;

    match method {
        "initialize" => Some(on_initialize(msg)),
        "ping" => Some(on_ping(msg)),
        "tools/list" => Some(schema::on_tools_list(msg)),
        "tools/call" => Some(execute::on_tool_call(msg, ctx).await),
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
            "serverInfo": { "name": "vigilo", "version": env!("CARGO_PKG_VERSION") },
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
    let dur = crate::view::fmt::fmt_duration(duration_us);
    if matches!(risk, Risk::Exec) {
        eprintln!("⚠  [{status}] {label}  {tool}  ({dur})  ← EXEC");
    } else {
        eprintln!("[{status}] {label}  {tool}  ({dur})");
    }
}

fn cleanup_mcp_session_file() {
    let _ = std::fs::remove_file(crate::models::mcp_session_path());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_ctx(ledger_path: &str) -> ServerContext {
        ServerContext {
            ledger_path: ledger_path.to_string(),
            session_id: uuid::Uuid::new_v4(),
            project_root: None,
            project_name: None,
            tag: None,
            timeout_secs: 5,
            encryption_key: None,
        }
    }

    #[tokio::test]
    async fn dispatch_initialize_returns_protocol_version() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let ctx = test_ctx("/tmp/test.jsonl");
        let resp = dispatch(&msg, &ctx).await.unwrap();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(resp["result"]["serverInfo"]["name"], "vigilo");
    }

    #[tokio::test]
    async fn dispatch_ping_returns_empty_result() {
        let msg = json!({ "jsonrpc": "2.0", "id": 42, "method": "ping" });
        let ctx = test_ctx("/tmp/test.jsonl");
        let resp = dispatch(&msg, &ctx).await.unwrap();
        assert_eq!(resp["id"], 42);
        assert!(resp["result"].is_object());
    }

    #[tokio::test]
    async fn dispatch_tools_list_returns_14_tools() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let ctx = test_ctx("/tmp/test.jsonl");
        let resp = dispatch(&msg, &ctx).await.unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 14);
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"run_command"));
        assert!(names.contains(&"git_commit"));
    }

    #[tokio::test]
    async fn dispatch_tools_call_read_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let test_file = dir.path().join("hello.txt");
        std::fs::write(&test_file, "world").unwrap();
        let ledger = dir.path().join("events.jsonl");
        let ctx = test_ctx(ledger.to_str().unwrap());

        let msg = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "read_file",
                "arguments": { "path": test_file.to_str().unwrap() }
            }
        });
        let resp = dispatch(&msg, &ctx).await.unwrap();
        assert_eq!(resp["id"], 7);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "world");

        // Verify event was written to ledger
        let ledger_content = std::fs::read_to_string(&ledger).unwrap();
        let event: serde_json::Value = serde_json::from_str(ledger_content.trim()).unwrap();
        assert_eq!(event["tool"], "read_file");
        assert_eq!(event["risk"], "read");
        assert_eq!(event["server"], "vigilo");
    }

    #[tokio::test]
    async fn dispatch_tools_call_error_returns_jsonrpc_error() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("events.jsonl");
        let ctx = test_ctx(ledger.to_str().unwrap());

        let msg = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "read_file",
                "arguments": { "path": "/nonexistent/file.txt" }
            }
        });
        let resp = dispatch(&msg, &ctx).await.unwrap();
        assert_eq!(resp["id"], 3);
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32603);
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_none() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "unknown/method" });
        let ctx = test_ctx("/tmp/test.jsonl");
        assert!(dispatch(&msg, &ctx).await.is_none());
    }

    #[tokio::test]
    async fn dispatch_missing_method_returns_none() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1 });
        let ctx = test_ctx("/tmp/test.jsonl");
        assert!(dispatch(&msg, &ctx).await.is_none());
    }
}
