use crate::{
    crypto, git, ledger,
    models::{self, McpEvent, Outcome, ProjectContext, Risk},
};
use anyhow::Result;
use chrono::Utc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

pub async fn run(ledger_path: String, session_id: Uuid) -> Result<()> {
    let project_root = git::root().await;
    let project_name = git::name().await;

    let config = load_config();

    // tag: VIGILO_TAG env > config TAG > current git branch (auto)
    let project_branch = match project_root.as_deref() {
        Some(root) => git::branch_in(root).await,
        None => git::branch().await,
    };
    let tag = std::env::var("VIGILO_TAG")
        .ok()
        .or_else(|| config.get("TAG").cloned())
        .or(project_branch);

    // timeout: VIGILO_TIMEOUT_SECS env > config TIMEOUT_SECS > 30
    let timeout_secs: u64 = std::env::var("VIGILO_TIMEOUT_SECS")
        .ok()
        .or_else(|| config.get("TIMEOUT_SECS").cloned())
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    if let Some(ref t) = tag {
        eprintln!("[vigilo] tag={t}");
    }
    eprintln!("[vigilo] timeout={timeout_secs}s");

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();

    let mut total: u64 = 0;
    let mut reads: u64 = 0;
    let mut writes: u64 = 0;
    let mut execs: u64 = 0;
    let mut errors: u64 = 0;
    let started = std::time::Instant::now();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let msg: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(response) = dispatch(
            &msg,
            &ledger_path,
            session_id,
            &project_root,
            &project_name,
            tag.as_deref(),
            timeout_secs,
        )
        .await
        {
            // tally tool calls for session summary
            if msg.get("method").and_then(|m| m.as_str()) == Some("tools/call") {
                let tool = msg
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let is_err = response.get("error").is_some();
                total += 1;
                if is_err {
                    errors += 1;
                }
                match Risk::classify(tool) {
                    Risk::Read => reads += 1,
                    Risk::Write => writes += 1,
                    Risk::Exec => execs += 1,
                    Risk::Unknown => {}
                }
            }

            let json = serde_json::to_string(&response)?;
            stdout.write_all(json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    let elapsed = started.elapsed().as_secs();
    let sid = &session_id.to_string()[..8];
    eprintln!(
        "[vigilo] session {sid} ended — {total} calls  read:{reads} write:{writes} exec:{execs} errors:{errors}  {elapsed}s"
    );

    Ok(())
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
        "tools/list" => Some(on_tools_list(msg)),
        "tools/call" => Some(
            on_tool_call(
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

fn on_tools_list(msg: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": msg["id"],
        "result": {
            "tools": [
                {
                    "name": "read_file",
                    "description": "Read the contents of a file, optionally limited to a line range",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "start_line": { "type": "number", "description": "First line to read (1-indexed, inclusive)" },
                            "end_line": { "type": "number", "description": "Last line to read (1-indexed, inclusive)" },
                        },
                        "required": ["path"],
                    },
                },
                {
                    "name": "write_file",
                    "description": "Write content to a file, creating it if it does not exist",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "content": { "type": "string" },
                        },
                        "required": ["path", "content"],
                    },
                },
                {
                    "name": "list_directory",
                    "description": "List entries inside a directory",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                },
                {
                    "name": "create_directory",
                    "description": "Create a directory and any missing parent directories",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                },
                {
                    "name": "delete_file",
                    "description": "Delete a file",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                },
                {
                    "name": "move_file",
                    "description": "Move or rename a file or directory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "from": { "type": "string" },
                            "to": { "type": "string" },
                        },
                        "required": ["from", "to"],
                    },
                },
                {
                    "name": "search_files",
                    "description": "Search for a text pattern across files in a directory",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "pattern": { "type": "string" },
                            "regex": { "type": "boolean", "description": "Treat pattern as a regular expression" },
                        },
                        "required": ["path", "pattern"],
                    },
                },
                {
                    "name": "run_command",
                    "description": "Run a shell command and return its stdout and stderr",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "command": { "type": "string" },
                            "cwd": { "type": "string" },
                        },
                        "required": ["command"],
                    },
                },
                {
                    "name": "get_file_info",
                    "description": "Get metadata for a file or directory (size, type, modified time)",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                },
                {
                    "name": "git_status",
                    "description": "Show the working tree status of a git repository",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "path": { "type": "string" } },
                        "required": ["path"],
                    },
                },
                {
                    "name": "git_diff",
                    "description": "Show unstaged changes in a git repository",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "staged": { "type": "boolean" },
                        },
                        "required": ["path"],
                    },
                },
                {
                    "name": "git_log",
                    "description": "Show recent commits in a git repository",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "count": { "type": "number" },
                        },
                        "required": ["path"],
                    },
                },
                {
                    "name": "git_commit",
                    "description": "Stage all changes and create a git commit with the given message",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "message": { "type": "string" },
                        },
                        "required": ["path", "message"],
                    },
                },
                {
                    "name": "patch_file",
                    "description": "Apply a unified diff patch to a file",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string" },
                            "patch": { "type": "string" },
                        },
                        "required": ["path", "patch"],
                    },
                },
            ],
        },
    })
}

async fn on_tool_call(
    msg: &serde_json::Value,
    ledger_path: &str,
    session_id: Uuid,
    project_root: &Option<String>,
    project_name: &Option<String>,
    tag: Option<&str>,
    timeout_secs: u64,
) -> serde_json::Value {
    let tool = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let arguments = msg
        .get("params")
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    // Capture existing content before write_file so we can diff later
    let before_content = if tool == "write_file" {
        if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
            tokio::fs::read_to_string(path).await.ok()
        } else {
            None
        }
    } else {
        None
    };

    let started = Instant::now();
    let timeout_dur = std::time::Duration::from_secs(timeout_secs);
    let (exec, timed_out) = match tokio::time::timeout(timeout_dur, execute(tool, &arguments)).await
    {
        Ok(result) => (result, false),
        Err(_) => (Err(format!("timed out after {timeout_secs}s")), true),
    };
    let duration_us = started.elapsed().as_micros() as u64;
    let is_error = exec.is_err();
    let risk = Risk::classify(tool);

    // Compute unified diff for write_file
    let diff = if tool == "write_file" && exec.is_ok() {
        let after = arguments
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match before_content {
            Some(ref before) => models::compute_unified_diff(before, after),
            None => Some("new file".to_string()),
        }
    } else {
        None
    };

    let (outcome, response) = match exec {
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
    };

    log_event(tool, risk, duration_us, is_error);

    let (ledger_arguments, ledger_outcome, ledger_diff) = if let Some(key) = crypto::load_key() {
        let enc_args = serde_json::json!(crypto::encrypt(&key, &arguments.to_string()));
        let enc_outcome = match &outcome {
            Outcome::Ok { result } => Outcome::Ok {
                result: serde_json::json!(crypto::encrypt(&key, &result.to_string())),
            },
            Outcome::Err { .. } => outcome.clone(),
        };
        let enc_diff = diff.as_deref().map(|d| crypto::encrypt(&key, d));
        (enc_args, enc_outcome, enc_diff)
    } else {
        (arguments.clone(), outcome.clone(), diff.clone())
    };

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
    let project = ProjectContext {
        root,
        name,
        branch,
        commit,
        dirty,
    };

    let event = McpEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now().to_rfc3339(),
        session_id,
        server: "vigilo".to_string(),
        tool: tool.to_string(),
        arguments: ledger_arguments,
        outcome: ledger_outcome,
        duration_us,
        risk,
        project,
        tag: tag.map(|t| t.to_string()),
        diff: ledger_diff,
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
    };

    if let Err(e) = ledger::append_event(&event, ledger_path) {
        eprintln!("[vigilo] ledger error: {e}");
    }

    response
}

fn arg_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing '{key}'"))
}

async fn execute(tool: &str, args: &serde_json::Value) -> Result<String, String> {
    match tool {
        "read_file" => {
            let path = arg_str(args, "path")?;
            let content = tokio::fs::read_to_string(path)
                .await
                .map_err(|e| e.to_string())?;
            let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let end = args.get("end_line").and_then(|v| v.as_u64());
            if start == 1 && end.is_none() {
                Ok(content)
            } else {
                let lines: Vec<&str> = content.lines().collect();
                let start_idx = start.saturating_sub(1).min(lines.len());
                let end_idx = end
                    .map(|e| (e as usize).min(lines.len()))
                    .unwrap_or(lines.len());
                let selected: Vec<String> = lines[start_idx..end_idx]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{}: {line}", start_idx + i + 1))
                    .collect();
                Ok(selected.join("\n"))
            }
        }

        "write_file" => {
            let path = arg_str(args, "path")?;
            let content = arg_str(args, "content")?;
            if let Some(parent) = std::path::Path::new(path).parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            tokio::fs::write(path, content)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("wrote {} bytes to {path}", content.len()))
        }

        "list_directory" => {
            let path = arg_str(args, "path")?;
            let mut entries = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
            let mut names = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
            names.sort();
            Ok(names.join("\n"))
        }

        "create_directory" => {
            let path = arg_str(args, "path")?;
            tokio::fs::create_dir_all(path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("created {path}"))
        }

        "delete_file" => {
            let path = arg_str(args, "path")?;
            tokio::fs::remove_file(path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("deleted {path}"))
        }

        "move_file" => {
            let from = arg_str(args, "from")?;
            let to = arg_str(args, "to")?;
            tokio::fs::rename(from, to)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!("moved {from} → {to}"))
        }

        "search_files" => {
            let path = arg_str(args, "path")?;
            let pattern = arg_str(args, "pattern")?;
            let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
            search(path, pattern, use_regex).await
        }

        "run_command" => {
            let command = arg_str(args, "command")?;
            let mut cmd = tokio::process::Command::new("sh");
            cmd.args(["-c", command]);
            if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
                cmd.current_dir(cwd);
            }
            let output = cmd.output().await.map_err(|e| e.to_string())?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);
            if output.status.success() {
                Ok(stdout.into_owned())
            } else {
                Err(format!("exit {exit_code}\n{stderr}"))
            }
        }

        "get_file_info" => {
            let path = arg_str(args, "path")?;
            let meta = tokio::fs::metadata(path).await.map_err(|e| e.to_string())?;
            let kind = if meta.is_dir() {
                "directory"
            } else if meta.is_file() {
                "file"
            } else {
                "other"
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            Ok(format!(
                "path: {path}\ntype: {kind}\nsize: {} bytes\nmodified: {modified}",
                meta.len()
            ))
        }

        "git_status" => {
            let path = arg_str(args, "path")?;
            let out = tokio::process::Command::new("git")
                .args(["status", "--short"])
                .current_dir(path)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            Ok(if text.trim().is_empty() {
                "nothing to commit, working tree clean".to_string()
            } else {
                text
            })
        }

        "git_diff" => {
            let path = arg_str(args, "path")?;
            let staged = args
                .get("staged")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut cmd = tokio::process::Command::new("git");
            cmd.arg("diff");
            if staged {
                cmd.arg("--staged");
            }
            let out = cmd
                .current_dir(path)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            Ok(if text.trim().is_empty() {
                "no changes".to_string()
            } else {
                text
            })
        }

        "git_log" => {
            let path = arg_str(args, "path")?;
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10);
            let out = tokio::process::Command::new("git")
                .args(["log", &format!("-{count}"), "--oneline", "--decorate"])
                .current_dir(path)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            Ok(if text.trim().is_empty() {
                "no commits".to_string()
            } else {
                text
            })
        }

        "git_commit" => {
            let path = arg_str(args, "path")?;
            let message = arg_str(args, "message")?;
            let add = tokio::process::Command::new("git")
                .args(["add", "-A"])
                .current_dir(path)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if !add.status.success() {
                return Err(String::from_utf8_lossy(&add.stderr).into_owned());
            }
            let out = tokio::process::Command::new("git")
                .args(["commit", "-m", message])
                .current_dir(path)
                .output()
                .await
                .map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).into_owned())
            }
        }

        "patch_file" => {
            let path = arg_str(args, "path")?;
            let patch = arg_str(args, "patch")?;
            let mut cmd = tokio::process::Command::new("patch");
            cmd.args(["-u", path]);
            cmd.stdin(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if let Some(stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                let mut stdin = stdin;
                stdin
                    .write_all(patch.as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
            if out.status.success() {
                Ok(format!("patched {path}"))
            } else {
                Err(String::from_utf8_lossy(&out.stderr).into_owned())
            }
        }

        _ => Err(format!("unknown tool: {tool}")),
    }
}

async fn search(root: &str, pattern: &str, use_regex: bool) -> Result<String, String> {
    let re = if use_regex {
        Some(regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {e}"))?)
    } else {
        None
    };
    let mut matches = Vec::new();
    search_dir(root, pattern, &re, &mut matches).await?;
    if matches.is_empty() {
        Ok(format!("no matches for '{pattern}'"))
    } else {
        Ok(matches.join("\n"))
    }
}

async fn search_dir(
    dir: &str,
    pattern: &str,
    re: &Option<regex::Regex>,
    matches: &mut Vec<String>,
) -> Result<(), String> {
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        let meta = entry.metadata().await.map_err(|e| e.to_string())?;
        if meta.is_dir() {
            Box::pin(search_dir(
                path.to_str().unwrap_or(""),
                pattern,
                re,
                matches,
            ))
            .await?;
        } else if meta.is_file() {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                for (i, line) in content.lines().enumerate() {
                    let hit = match re {
                        Some(r) => r.is_match(line),
                        None => line.contains(pattern),
                    };
                    if hit {
                        matches.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
                    }
                }
            }
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
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
