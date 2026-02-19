use super::data::{load_sessions, LoadFilter};
use super::fmt::{
    ceprintln, client_badge, cprintln, diff_badge, diff_summary, fmt_arg, maybe_decrypt,
    print_colored_diff, risk_decorated, risk_label, short_id, short_path, trunc, BOLD, BRIGHT_RED,
    CYAN, DIM, GREEN, RED, RESET,
};
use super::ViewArgs;
use crate::{
    crypto,
    models::{self, McpEvent, Outcome, Risk},
};
use anyhow::Result;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};

pub fn query(
    ledger_path: &str,
    since: Option<&str>,
    until: Option<&str>,
    tool: Option<&str>,
    risk: Option<&str>,
    session: Option<&str>,
) -> Result<()> {
    let filter = LoadFilter {
        since,
        until,
        session,
    };
    let sessions = load_sessions(ledger_path, &filter)?;
    let key = crypto::load_key();

    let events: Vec<&McpEvent> = sessions
        .iter()
        .flat_map(|(_, events)| events)
        .filter(|e| tool.is_none_or(|t| e.tool == t))
        .filter(|e| risk.is_none_or(|r| risk_label(e.risk) == r))
        .collect();

    if events.is_empty() {
        println!("no matching events.");
        return Ok(());
    }

    println!();
    cprintln!(
        "{DIM}── {} matching events ──────────────────────────{RESET}",
        events.len()
    );
    println!();
    for e in events {
        print_query_row(e, key.as_ref());
    }
    println!();
    Ok(())
}

fn print_query_row(e: &McpEvent, key: Option<&[u8; 32]>) {
    let is_error = matches!(e.outcome, Outcome::Err { .. });
    let badge = client_badge(&e.server);
    let date_time = e.timestamp.get(5..19).unwrap_or("??-?? ??:??:??");
    let risk_sym = risk_decorated(e.risk, is_error);
    let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
    let project_root = e.project.root.as_deref();
    let arg = fmt_arg(e, key, project_root);
    let arg_display = trunc(&arg, 40);
    let dur = if e.duration_us > 0 {
        format!("  {DIM}{}{RESET}", models::fmt_duration(e.duration_us))
    } else {
        String::new()
    };
    let diff = diff_badge(e.diff.as_deref());
    let timeout = if e.timed_out {
        format!("  {BRIGHT_RED}TIMEOUT{RESET}")
    } else {
        String::new()
    };
    let sid_str = e.session_id.to_string();
    let sid_short = short_id(&sid_str);

    cprintln!(" {badge}  {DIM}{date_time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}  {DIM}{sid_short}{RESET}");
}

pub fn diff(ledger_path: &str, args: &ViewArgs) -> Result<()> {
    let key = crypto::load_key();
    let filter = LoadFilter {
        since: args.since.as_deref(),
        until: args.until.as_deref(),
        session: args.session.as_deref(),
    };
    let mut sessions = load_sessions(ledger_path, &filter)?;

    if let Some(n) = args.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }

    if sessions.is_empty() {
        println!("no events with diffs found.");
        return Ok(());
    }

    for (sid, events) in &sessions {
        print_diff_session(sid, events, key.as_ref());
    }

    println!();
    Ok(())
}

fn print_diff_session(sid: &str, events: &[McpEvent], key: Option<&[u8; 32]>) {
    let edits: Vec<&McpEvent> = events.iter().filter(|e| e.diff.is_some()).collect();
    if edits.is_empty() {
        return;
    }

    let Some(first) = events.first() else {
        return;
    };
    let badge = client_badge(&first.server);
    let sid_short = short_id(sid);
    let project_root = first.project.root.as_deref();

    let mut by_file: Vec<(String, Vec<&McpEvent>)> = Vec::new();
    for e in &edits {
        let path = extract_file_path(e, key, project_root);
        match by_file.iter_mut().find(|(p, _)| p == &path) {
            Some((_, list)) => list.push(e),
            None => by_file.push((path, vec![e])),
        }
    }

    println!();
    cprintln!("{DIM}── vigilo diff ── {RESET}{badge} {BOLD}{sid_short}{RESET} {DIM}────────────────────────{RESET}");
    if let Some(name) = first.project.name.as_deref() {
        let branch = first.project.branch.as_deref().unwrap_or("");
        if !branch.is_empty() {
            cprintln!("  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}");
        } else {
            cprintln!("  {CYAN}{name}{RESET}");
        }
    }

    let mut total_added: usize = 0;
    let mut total_removed: usize = 0;

    for (path, file_edits) in &by_file {
        let (a, r) = print_diff_file(path, file_edits);
        total_added += a;
        total_removed += r;
    }

    let file_count = by_file.len();
    let file_word = if file_count == 1 { "file" } else { "files" };
    let edit_word = if edits.len() == 1 { "edit" } else { "edits" };
    println!();
    cprintln!(
        "  {DIM}── {}{RESET} {DIM}{edit_word} across{RESET} {BOLD}{file_count}{RESET} {DIM}{file_word}{RESET} · {GREEN}+{total_added}{RESET} {RED}-{total_removed}{RESET} {DIM}net ──{RESET}",
        edits.len()
    );
}

fn print_diff_file(path: &str, file_edits: &[&McpEvent]) -> (usize, usize) {
    let (file_add, file_rem) = file_edits
        .iter()
        .filter_map(|e| e.diff.as_deref())
        .filter(|d| !crypto::is_encrypted(d) && *d != "new file")
        .fold((0usize, 0usize), |(a, r), d| {
            let (da, dr) = diff_summary(d);
            (a + da, r + dr)
        });

    let has_new = file_edits
        .iter()
        .any(|e| e.diff.as_deref() == Some("new file"));
    let edit_count = file_edits.len();
    let edit_word = if edit_count == 1 { "edit" } else { "edits" };
    let new_badge = if has_new {
        format!(" {GREEN}new{RESET}")
    } else {
        String::new()
    };
    let change_str = if file_add > 0 || file_rem > 0 {
        format!("  {GREEN}+{file_add}{RESET} {RED}-{file_rem}{RESET}")
    } else {
        String::new()
    };

    println!();
    cprintln!(
        "  {BOLD}{path}{RESET}  {DIM}({edit_count} {edit_word}){RESET}{new_badge}{change_str}"
    );
    cprintln!("  {DIM}{}─{RESET}", "─".repeat(path.len().max(10)));

    for e in file_edits {
        print_diff_edit(e);
    }

    (file_add, file_rem)
}

fn print_diff_edit(e: &McpEvent) {
    let time = e.timestamp.get(11..19).unwrap_or("??:??:??");
    let tool = &e.tool;
    let diff_text = e.diff.as_deref().unwrap_or("");

    let (a, r) = if !crypto::is_encrypted(diff_text) && diff_text != "new file" {
        diff_summary(diff_text)
    } else {
        (0, 0)
    };
    let mini_badge = match () {
        _ if a > 0 || r > 0 => format!("  {GREEN}+{a}{RESET} {RED}-{r}{RESET}"),
        _ if diff_text == "new file" => format!("  {GREEN}new file{RESET}"),
        _ => String::new(),
    };

    cprintln!("  {DIM}{time}{RESET}  {BOLD}{tool}{RESET}{mini_badge}");

    if !crypto::is_encrypted(diff_text) && diff_text != "new file" {
        print_colored_diff(diff_text);
    }
}

fn extract_file_path(e: &McpEvent, key: Option<&[u8; 32]>, project_root: Option<&str>) -> String {
    let raw = e
        .arguments
        .get("file_path")
        .or_else(|| e.arguments.get("path"))
        .or_else(|| e.arguments.get("from"));
    match raw {
        Some(v) => {
            let decrypted = maybe_decrypt(key, v);
            short_path(&decrypted, project_root)
        }
        None => "unknown".to_string(),
    }
}

pub fn export(
    ledger_path: &str,
    format: &str,
    args: &ViewArgs,
    output: Option<&str>,
) -> Result<()> {
    let filter = LoadFilter {
        since: args.since.as_deref(),
        until: args.until.as_deref(),
        session: args.session.as_deref(),
    };
    let mut sessions = load_sessions(ledger_path, &filter)?;
    if let Some(n) = args.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }
    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();

    if all_events.is_empty() {
        eprintln!("no events to export.");
        return Ok(());
    }

    let ext = if format == "json" { "json" } else { "csv" };
    let default_path = default_export_path(ext);
    let dest = output.unwrap_or(&default_path);

    if let Some(parent) = std::path::Path::new(dest).parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut file = std::fs::File::create(dest)?;

    if format == "json" {
        let json = serde_json::to_string_pretty(&all_events.iter().collect::<Vec<_>>())
            .map_err(|e| anyhow::anyhow!(e))?;
        writeln!(file, "{json}")?;
    } else {
        write_csv(&mut file, &all_events)?;
    }

    let display_path = shorten_home(dest);
    println!("exported {} events to {display_path}", all_events.len());
    Ok(())
}

fn default_export_path(ext: &str) -> String {
    format!("{}/.vigilo/export.{ext}", crate::models::home())
}

fn shorten_home(path: &str) -> String {
    let home = crate::models::home();
    if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

fn write_csv(w: &mut impl Write, all_events: &[&McpEvent]) -> Result<()> {
    writeln!(
        w,
        "timestamp,session,server,project,branch,tool,risk,arg,duration,status,error,model,input_tokens,output_tokens"
    )?;
    for e in all_events {
        let (status, error_msg) = match &e.outcome {
            Outcome::Err { message, .. } => ("error", message.replace('"', "\"\"").clone()),
            _ => ("ok", String::new()),
        };
        let risk = format!("{:?}", e.risk).to_lowercase();
        let project_root = e.project.root.as_deref();
        let raw_arg = e
            .arguments
            .get("file_path")
            .or_else(|| e.arguments.get("path"))
            .or_else(|| e.arguments.get("command"))
            .or_else(|| e.arguments.get("pattern"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let arg = short_path(raw_arg, project_root).replace('"', "\"\"");
        let arg = trunc(&arg, 80);
        let ts = e
            .timestamp
            .get(..19)
            .unwrap_or(&e.timestamp)
            .replace('T', " ");
        let sid_str = e.session_id.to_string();
        let sid = short_id(&sid_str);
        let server = &e.server;
        let project = e.project.name.as_deref().unwrap_or("");
        let branch = e.project.branch.as_deref().unwrap_or("");
        let dur = if e.duration_us > 0 {
            models::fmt_duration(e.duration_us)
        } else {
            String::new()
        };
        let model = e.model.as_deref().unwrap_or("");
        let in_tok = e.input_tokens.map(|t| t.to_string()).unwrap_or_default();
        let out_tok = e.output_tokens.map(|t| t.to_string()).unwrap_or_default();
        let err_trunc = trunc(&error_msg, 80);
        writeln!(
            w,
            "\"{ts}\",\"{sid}\",\"{server}\",\"{project}\",\"{branch}\",\"{}\",\"{risk}\",\"{arg}\",\"{dur}\",\"{status}\",\"{err_trunc}\",\"{model}\",\"{in_tok}\",\"{out_tok}\"",
            e.tool
        )?;
    }
    Ok(())
}

pub async fn watch(ledger_path: &str) -> Result<()> {
    let mut file = wait_for_ledger(ledger_path).await;
    file.seek(SeekFrom::End(0))?;

    let key = crypto::load_key();
    cprintln!("{DIM}[vigilo]{RESET} watching — ctrl+c to stop");
    println!();

    loop {
        let mut line = String::new();
        let n = BufReader::new(&file).read_line(&mut line)?;
        if n == 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let pos = file.stream_position()?;
            file = File::open(ledger_path).unwrap_or(file);
            let new_len = file.metadata().map(|m| m.len()).unwrap_or(pos);
            if new_len < pos {
                // file was rotated — start from beginning of new file
                file.seek(SeekFrom::Start(0))?;
            } else {
                file.seek(SeekFrom::Start(pos))?;
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(mut e) = serde_json::from_str::<McpEvent>(trimmed) {
            if e.risk == Risk::Unknown {
                e.risk = Risk::classify(&e.tool);
            }
            print_watch_event(&e, key.as_ref());
        }
    }
}

async fn wait_for_ledger(ledger_path: &str) -> File {
    loop {
        match File::open(ledger_path) {
            Ok(f) => break f,
            Err(_) => {
                ceprintln!("{DIM}[vigilo] waiting for ledger at {ledger_path}...{RESET}");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}

fn print_watch_event(e: &McpEvent, key: Option<&[u8; 32]>) {
    let is_error = matches!(e.outcome, Outcome::Err { .. });
    let badge = client_badge(&e.server);
    let time = e.timestamp.get(11..19).unwrap_or(&e.timestamp);
    let risk_sym = risk_decorated(e.risk, is_error);
    let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
    let project_root = e.project.root.as_deref();
    let arg = fmt_arg(e, key, project_root);
    let arg_display = trunc(&arg, 40);
    let dur = if e.duration_us > 0 {
        format!("  {DIM}{}{RESET}", models::fmt_duration(e.duration_us))
    } else {
        String::new()
    };
    let diff = diff_badge(e.diff.as_deref());
    let timeout = if e.timed_out {
        format!("  {BRIGHT_RED}TIMEOUT{RESET}")
    } else {
        String::new()
    };

    cprintln!(
        " {badge}  {DIM}{time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}"
    );
}
