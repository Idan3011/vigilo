pub(super) fn arg_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing '{key}'"))
}

pub(super) async fn execute(tool: &str, args: &serde_json::Value) -> Result<String, String> {
    match tool {
        "read_file" => execute_read_file(args).await,
        "write_file" => execute_write_file(args).await,
        "list_directory" => execute_list_directory(args).await,
        "create_directory" => execute_create_directory(args).await,
        "delete_file" => execute_delete_file(args).await,
        "move_file" => execute_move_file(args).await,
        "search_files" => execute_search_files(args).await,
        "run_command" => execute_run_command(args).await,
        "get_file_info" => execute_get_file_info(args).await,
        "git_status" => execute_git_status(args).await,
        "git_diff" => execute_git_diff(args).await,
        "git_log" => execute_git_log(args).await,
        "git_commit" => execute_git_commit(args).await,
        "patch_file" => execute_patch_file(args).await,
        _ => Err(format!("unknown tool: {tool}")),
    }
}

async fn execute_read_file(args: &serde_json::Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| e.to_string())?;
    let start = args.get("start_line").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
    let end = args.get("end_line").and_then(|v| v.as_u64());
    if start == 1 && end.is_none() {
        return Ok(content);
    }
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

async fn execute_write_file(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_list_directory(args: &serde_json::Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    let mut entries = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    Ok(names.join("\n"))
}

async fn execute_create_directory(args: &serde_json::Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("created {path}"))
}

async fn execute_delete_file(args: &serde_json::Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    tokio::fs::remove_file(path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("deleted {path}"))
}

async fn execute_move_file(args: &serde_json::Value) -> Result<String, String> {
    let from = arg_str(args, "from")?;
    let to = arg_str(args, "to")?;
    tokio::fs::rename(from, to)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!("moved {from} → {to}"))
}

async fn execute_search_files(args: &serde_json::Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    let pattern = arg_str(args, "pattern")?;
    let use_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
    search(path, pattern, use_regex).await
}

const MAX_OUTPUT_BYTES: usize = 1_048_576;

async fn execute_run_command(args: &serde_json::Value) -> Result<String, String> {
    let command = arg_str(args, "command")?;
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", command]);
    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
        cmd.current_dir(cwd);
    }
    let output = cmd.output().await.map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = output.status.code().unwrap_or(-1);
    if output.status.success() {
        Ok(cap_output(&output.stdout))
    } else {
        Err(format!("exit {exit_code}\n{stderr}"))
    }
}

fn cap_output(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_OUTPUT_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let truncated = String::from_utf8_lossy(&bytes[..MAX_OUTPUT_BYTES]).into_owned();
    let omitted = bytes.len() - MAX_OUTPUT_BYTES;
    format!("{truncated}\n\n[output truncated — {omitted} bytes omitted]")
}

async fn execute_get_file_info(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_git_status(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_git_diff(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_git_log(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_git_commit(args: &serde_json::Value) -> Result<String, String> {
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

async fn execute_patch_file(args: &serde_json::Value) -> Result<String, String> {
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

const MAX_SEARCH_DEPTH: u32 = 12;

async fn search(root: &str, pattern: &str, use_regex: bool) -> Result<String, String> {
    let re = if use_regex {
        Some(regex::Regex::new(pattern).map_err(|e| format!("invalid regex: {e}"))?)
    } else {
        None
    };
    let mut matches = Vec::new();
    search_dir(root, pattern, &re, &mut matches, 0).await?;
    if matches.is_empty() {
        Ok(format!("no matches for '{pattern}'"))
    } else {
        Ok(matches.join("\n"))
    }
}

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    ".next",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    "dist",
    "build",
    ".cache",
];

async fn search_dir(
    dir: &str,
    pattern: &str,
    re: &Option<regex::Regex>,
    matches: &mut Vec<String>,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_SEARCH_DEPTH {
        return Ok(());
    }
    let mut entries = tokio::fs::read_dir(dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        let meta = entry.metadata().await.map_err(|e| e.to_string())?;
        if meta.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            Box::pin(search_dir(
                path.to_str().unwrap_or(""),
                pattern,
                re,
                matches,
                depth + 1,
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
