use crate::{
    crypto, git, ledger,
    models::{self, McpEvent, Outcome, ProjectContext, Risk},
};
use chrono::Utc;
use std::time::Instant;
use uuid::Uuid;

pub(super) async fn on_tool_call(
    msg: &serde_json::Value,
    ledger_path: &str,
    session_id: Uuid,
    project_root: &Option<String>,
    project_name: &Option<String>,
    tag: Option<&str>,
    timeout_secs: u64,
) -> serde_json::Value {
    let (tool, arguments) = parse_tool_call(msg);
    let before_content = capture_before_content(&tool, &arguments).await;

    let (exec, timed_out) = execute_with_timeout(&tool, &arguments, timeout_secs).await;
    let duration_us = exec.1;
    let is_error = exec.0.is_err();
    let risk = Risk::classify(&tool);
    let diff = compute_write_diff(&tool, &arguments, &before_content, exec.0.is_ok());

    let (outcome, response) = build_response(msg, exec.0);
    super::log_event(&tool, risk, duration_us, is_error);

    let (ledger_arguments, ledger_outcome, ledger_diff) =
        encrypt_for_ledger(&arguments, &outcome, &diff);
    let project = resolve_project(&arguments, project_root, project_name).await;

    let event = build_event(
        session_id,
        &tool,
        ledger_arguments,
        ledger_outcome,
        duration_us,
        risk,
        project,
        tag,
        ledger_diff,
        timed_out,
    );

    if let Err(e) = ledger::append_event(&event, ledger_path) {
        eprintln!("[vigilo] ledger error: {e}");
    }

    response
}

fn parse_tool_call(msg: &serde_json::Value) -> (String, serde_json::Value) {
    let tool = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    let arguments = msg
        .get("params")
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    (tool, arguments)
}

async fn execute_with_timeout(
    tool: &str,
    arguments: &serde_json::Value,
    timeout_secs: u64,
) -> ((Result<String, String>, u64), bool) {
    let started = Instant::now();
    let timeout_dur = std::time::Duration::from_secs(timeout_secs);
    let (exec, timed_out) =
        match tokio::time::timeout(timeout_dur, super::tools::execute(tool, arguments)).await {
            Ok(result) => (result, false),
            Err(_) => (Err(format!("timed out after {timeout_secs}s")), true),
        };
    let duration_us = started.elapsed().as_micros() as u64;
    ((exec, duration_us), timed_out)
}

async fn capture_before_content(tool: &str, arguments: &serde_json::Value) -> Option<String> {
    if tool != "write_file" {
        return None;
    }
    let path = arguments.get("path").and_then(|v| v.as_str())?;
    tokio::fs::read_to_string(path).await.ok()
}

fn compute_write_diff(
    tool: &str,
    arguments: &serde_json::Value,
    before_content: &Option<String>,
    success: bool,
) -> Option<String> {
    if tool != "write_file" || !success {
        return None;
    }
    let after = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match before_content {
        Some(before) => models::compute_unified_diff(before, after),
        None => Some("new file".to_string()),
    }
}

fn build_response(
    msg: &serde_json::Value,
    exec: Result<String, String>,
) -> (Outcome, serde_json::Value) {
    match exec {
        Ok(text) => (
            Outcome::Ok {
                result: serde_json::json!(text),
            },
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": msg["id"],
                "result": { "content": [{ "type": "text", "text": text }] },
            }),
        ),
        Err(e) => (
            Outcome::Err {
                code: -32603,
                message: e.clone(),
            },
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": msg["id"],
                "error": { "code": -32603, "message": e },
            }),
        ),
    }
}

fn encrypt_for_ledger(
    arguments: &serde_json::Value,
    outcome: &Outcome,
    diff: &Option<String>,
) -> (serde_json::Value, Outcome, Option<String>) {
    if let Some(key) = crypto::load_key() {
        let enc_args = serde_json::json!(crypto::encrypt(&key, &arguments.to_string()));
        let enc_outcome = match outcome {
            Outcome::Ok { result } => Outcome::Ok {
                result: serde_json::json!(crypto::encrypt(&key, &result.to_string())),
            },
            Outcome::Err { .. } => outcome.clone(),
        };
        let enc_diff = diff.as_deref().map(|d| crypto::encrypt(&key, d));
        (enc_args, enc_outcome, enc_diff)
    } else {
        (arguments.clone(), outcome.clone(), diff.clone())
    }
}

async fn resolve_project(
    arguments: &serde_json::Value,
    project_root: &Option<String>,
    project_name: &Option<String>,
) -> ProjectContext {
    let tool_dir: Option<String> = arguments
        .get("path")
        .or_else(|| arguments.get("cwd"))
        .and_then(|v| v.as_str())
        .map(|p| {
            let path = std::path::Path::new(p);
            if path.is_dir() {
                p.to_string()
            } else {
                path.parent()
                    .and_then(|d| d.to_str())
                    .unwrap_or(p)
                    .to_string()
            }
        });
    let git_dir = tool_dir.as_deref();
    let (branch, commit, dirty) = match git_dir {
        Some(d) => (
            git::branch_in(d).await,
            git::commit_in(d).await,
            git::dirty_in(d).await,
        ),
        None => (git::branch().await, git::commit().await, git::dirty().await),
    };
    let (root, name) = match (project_root, git_dir) {
        (Some(r), _) => (Some(r.clone()), project_name.clone()),
        (None, Some(d)) => (git::root_in(d).await, git::name_in(Some(d)).await),
        (None, None) => (None, None),
    };
    ProjectContext {
        root,
        name,
        branch,
        commit,
        dirty,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_event(
    session_id: Uuid,
    tool: &str,
    arguments: serde_json::Value,
    outcome: Outcome,
    duration_us: u64,
    risk: Risk,
    project: ProjectContext,
    tag: Option<&str>,
    diff: Option<String>,
    timed_out: bool,
) -> McpEvent {
    McpEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now().to_rfc3339(),
        session_id,
        server: "vigilo".to_string(),
        tool: tool.to_string(),
        arguments,
        outcome,
        duration_us,
        risk,
        project,
        tag: tag.map(|t| t.to_string()),
        diff,
        timed_out,
        model: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        stop_reason: None,
        service_tier: None,
        permission_mode: None,
        tool_use_id: None,
        cursor_version: None,
        generation_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tools::{arg_str, execute};
    use crate::models::Risk;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn arg_str_returns_value() {
        let args = json!({ "path": "/tmp/foo" });
        assert_eq!(arg_str(&args, "path").unwrap(), "/tmp/foo");
    }

    #[test]
    fn arg_str_missing_key_returns_err() {
        assert!(arg_str(&json!({}), "path").is_err());
    }

    #[test]
    fn risk_classify_read_tools() {
        assert_eq!(Risk::classify("read_file"), Risk::Read);
        assert_eq!(Risk::classify("list_directory"), Risk::Read);
        assert_eq!(Risk::classify("search_files"), Risk::Read);
        assert_eq!(Risk::classify("get_file_info"), Risk::Read);
        assert_eq!(Risk::classify("git_status"), Risk::Read);
        assert_eq!(Risk::classify("git_diff"), Risk::Read);
        assert_eq!(Risk::classify("git_log"), Risk::Read);
    }

    #[test]
    fn risk_classify_write_tools() {
        assert_eq!(Risk::classify("write_file"), Risk::Write);
        assert_eq!(Risk::classify("create_directory"), Risk::Write);
        assert_eq!(Risk::classify("delete_file"), Risk::Write);
        assert_eq!(Risk::classify("move_file"), Risk::Write);
        assert_eq!(Risk::classify("git_commit"), Risk::Write);
        assert_eq!(Risk::classify("patch_file"), Risk::Write);
    }

    #[test]
    fn risk_classify_unknown_returns_unknown() {
        assert_eq!(Risk::classify("foo"), Risk::Unknown);
    }

    #[tokio::test]
    async fn execute_read_file_returns_contents() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "hello").await.unwrap();

        let result = execute("read_file", &json!({ "path": path.to_str().unwrap() })).await;
        assert_eq!(result.unwrap(), "hello");
    }

    #[tokio::test]
    async fn execute_write_file_creates_and_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sub/out.txt");

        execute(
            "write_file",
            &json!({ "path": path.to_str().unwrap(), "content": "world" }),
        )
        .await
        .unwrap();

        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "world");
    }

    #[tokio::test]
    async fn execute_list_directory_returns_sorted_names() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "")
            .await
            .unwrap();

        let result = execute(
            "list_directory",
            &json!({ "path": dir.path().to_str().unwrap() }),
        )
        .await
        .unwrap();
        assert_eq!(result, "a.txt\nb.txt");
    }

    #[tokio::test]
    async fn execute_create_directory_makes_nested_dirs() {
        let dir = tempdir().unwrap();
        let new_dir = dir.path().join("sub/nested");

        execute(
            "create_directory",
            &json!({ "path": new_dir.to_str().unwrap() }),
        )
        .await
        .unwrap();

        assert!(new_dir.exists());
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_err() {
        let result = execute("unknown_tool", &json!({})).await;
        assert!(result.unwrap_err().contains("unknown tool"));
    }

    #[tokio::test]
    async fn execute_read_file_missing_path_arg_returns_err() {
        let result = execute("read_file", &json!({})).await;
        assert!(result.unwrap_err().contains("missing 'path'"));
    }

    #[tokio::test]
    async fn execute_delete_file_removes_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("to_delete.txt");
        tokio::fs::write(&path, "bye").await.unwrap();

        execute("delete_file", &json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap();

        assert!(!path.exists());
    }

    #[tokio::test]
    async fn execute_delete_file_missing_returns_err() {
        let result = execute(
            "delete_file",
            &json!({ "path": "/tmp/vigilo_no_such_file_xyz" }),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_move_file_renames_file() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("old.txt");
        let to = dir.path().join("new.txt");
        tokio::fs::write(&from, "content").await.unwrap();

        execute(
            "move_file",
            &json!({ "from": from.to_str().unwrap(), "to": to.to_str().unwrap() }),
        )
        .await
        .unwrap();

        assert!(!from.exists());
        assert!(to.exists());
    }

    #[tokio::test]
    async fn execute_search_files_finds_matches() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "hello world\nfoo bar")
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.txt"), "no match here")
            .await
            .unwrap();

        let result = execute(
            "search_files",
            &json!({ "path": dir.path().to_str().unwrap(), "pattern": "hello" }),
        )
        .await
        .unwrap();

        assert!(result.contains("a.txt"));
        assert!(result.contains("hello world"));
        assert!(!result.contains("b.txt"));
    }

    #[tokio::test]
    async fn execute_search_files_no_matches() {
        let dir = tempdir().unwrap();
        tokio::fs::write(dir.path().join("a.txt"), "nothing relevant")
            .await
            .unwrap();

        let result = execute(
            "search_files",
            &json!({ "path": dir.path().to_str().unwrap(), "pattern": "zzznomatch" }),
        )
        .await
        .unwrap();

        assert!(result.contains("no matches"));
    }

    #[test]
    fn risk_classify_exec_tool() {
        assert_eq!(Risk::classify("run_command"), Risk::Exec);
    }

    #[tokio::test]
    async fn execute_run_command_returns_stdout() {
        let result = execute("run_command", &json!({ "command": "echo hello" }))
            .await
            .unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn execute_run_command_nonzero_exit_returns_err() {
        let result = execute("run_command", &json!({ "command": "exit 1" })).await;
        assert!(result.unwrap_err().contains("exit 1"));
    }

    #[tokio::test]
    async fn execute_get_file_info_returns_metadata() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("info.txt");
        tokio::fs::write(&path, "hello").await.unwrap();

        let result = execute("get_file_info", &json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap();

        assert!(result.contains("file"));
        assert!(result.contains("5 bytes"));
    }

    #[tokio::test]
    async fn execute_get_file_info_on_directory() {
        let dir = tempdir().unwrap();
        let result = execute(
            "get_file_info",
            &json!({ "path": dir.path().to_str().unwrap() }),
        )
        .await
        .unwrap();
        assert!(result.contains("directory"));
    }

    #[tokio::test]
    async fn execute_run_command_respects_cwd() {
        let dir = tempdir().unwrap();
        let result = execute(
            "run_command",
            &json!({ "command": "pwd", "cwd": dir.path().to_str().unwrap() }),
        )
        .await
        .unwrap();
        assert!(result
            .trim()
            .ends_with(dir.path().file_name().unwrap().to_str().unwrap()));
    }
}
