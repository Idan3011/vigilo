use crate::{
    crypto, cursor_usage,
    models::{self, McpEvent, Outcome, Risk},
};
use anyhow::Result;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};

// ── ANSI helpers ─────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BRIGHT_RED: &str = "\x1b[91m";
const WHITE: &str = "\x1b[97m";
const BG_BLUE: &str = "\x1b[44m";
const BG_MAGENTA: &str = "\x1b[45m";

/// Format a client badge: ` CLAUDE ` on blue bg or ` CURSOR ` on magenta bg.
fn client_badge(server: &str) -> String {
    match server {
        "cursor" => format!("{BG_MAGENTA}{BOLD}{WHITE} CURSOR {RESET}"),
        _ => format!("{BG_BLUE}{BOLD}{WHITE} CLAUDE {RESET}"),
    }
}

/// Strip the project root from a path to produce a short relative path.
fn short_path(full: &str, project_root: Option<&str>) -> String {
    if let Some(root) = project_root {
        let root_slash = if root.ends_with('/') {
            root.to_string()
        } else {
            format!("{root}/")
        };
        if full.starts_with(&root_slash) {
            return full[root_slash.len()..].to_string();
        }
    }
    // Fallback: just show the file name
    full.rsplit('/').next().unwrap_or(full).to_string()
}

/// Truncate a string to `max` chars, appending "…" if truncated.
fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max.saturating_sub(1))
            .map(|(i, _)| i)
            .unwrap_or(max.saturating_sub(1));
        format!("{}…", &s[..end])
    }
}

/// Format tokens with K suffix for readability.
fn fmt_tokens(n: u64) -> String {
    match n {
        n if n >= 1_000 => format!("{}K", n / 1_000),
        n => n.to_string(),
    }
}

/// Normalize model names across clients (e.g. Cursor reports "default" for Auto).
fn normalize_model(m: &str) -> &str {
    match m {
        "default" | "auto" => "Auto",
        other => other,
    }
}

/// Risk symbol + colored label for event lines.
fn risk_decorated(risk: Risk, is_error: bool) -> String {
    if is_error {
        return format!("{BRIGHT_RED}✖ ERR   {RESET}");
    }
    match risk {
        Risk::Read => format!("{CYAN}○ READ  {RESET}"),
        Risk::Write => format!("{YELLOW}◆ WRITE {RESET}"),
        Risk::Exec => format!("{RED}● EXEC  {RESET}"),
        Risk::Unknown => format!("{DIM}? ???   {RESET}"),
    }
}

// ── Pricing ──────────────────────────────────────────────────────────────────

/// Public list prices per million tokens (input, output, cache_read).
/// Sources: anthropic.com/pricing · cursor.com/docs/models · openai.com/api/pricing
/// These are NOT actual billing data — use only for rough orientation.
///
/// Ordered longest-match first so "gpt-5-mini" matches before "gpt-5".
/// Cursor model names are separate entries because Cursor charges different rates.
#[rustfmt::skip]
const PRICE_TABLE: &[(&str, f64, f64, f64)] = &[
    // ── Anthropic API (Claude Code) ─────────────── input   output  cache_read
    ("claude-opus-4",                                 15.00,  75.00,   1.50),
    ("claude-sonnet-4",                                3.00,  15.00,   0.30),
    ("claude-haiku-4",                                 1.00,   5.00,   0.10),
    ("claude-3-5-sonnet",                              3.00,  15.00,   0.30),
    ("claude-3.5-sonnet",                              3.00,  15.00,   0.30),
    ("claude-3-5-haiku",                               0.80,   4.00,   0.08),
    ("claude-3.5-haiku",                               0.80,   4.00,   0.08),
    ("claude-3-opus",                                 15.00,  75.00,   1.50),
    ("claude-3-sonnet",                                3.00,  15.00,   0.30),
    ("claude-3-haiku",                                 0.25,   1.25,   0.025),
    // ── Cursor model names (Cursor pricing) ─────────────────────────────────
    ("claude-4.5-sonnet-thinking",                     3.00,  15.00,   0.30),
    ("Auto",                                           1.25,   6.00,   0.25),
    ("composer-1.5",                                   3.50,  17.50,   0.35),
    ("composer-1",                                     1.25,  10.00,   0.125),
    ("sonnet",                                         3.00,  15.00,   0.30),
    // ── OpenAI ──────────────────────────────────────────────────────────────
    ("gpt-5-mini",                                     0.25,   2.00,   0.025),
    ("gpt-5",                                          1.25,  10.00,   0.125),
    ("gpt-4o-mini",                                    0.15,   0.60,   0.075),
    ("gpt-4o",                                         2.50,  10.00,   1.25),
    ("o3-mini",                                        1.10,   4.40,   0.55),
    ("o1-mini",                                        1.10,   4.40,   0.55),
    ("o3",                                            15.00,  60.00,   7.50),
    ("o1",                                            15.00,  60.00,   7.50),
    // ── Google ──────────────────────────────────────────────────────────────
    ("gemini-2.5-flash",                               0.30,   2.50,   0.03),
    ("gemini-3-pro",                                   2.00,  12.00,   0.20),
    ("gemini-3-flash",                                 0.50,   3.00,   0.05),
    // ── xAI ─────────────────────────────────────────────────────────────────
    ("grok",                                           0.20,   1.50,   0.02),
];

fn pricing_for(model: &str) -> Option<(f64, f64, f64)> {
    let m = model.to_lowercase();
    for (fragment, inp_m, out_m, cr_m) in PRICE_TABLE {
        if m.contains(fragment) {
            return Some((inp_m / 1_000_000.0, out_m / 1_000_000.0, cr_m / 1_000_000.0));
        }
    }
    None
}

fn event_cost_usd(e: &McpEvent) -> Option<f64> {
    let (ip, op, crp) = pricing_for(e.model.as_deref()?)?;
    let inp = e.input_tokens? as f64;
    let out = e.output_tokens.unwrap_or(0) as f64;
    let cr = e.cache_read_tokens.unwrap_or(0) as f64;
    let cw = e.cache_write_tokens.unwrap_or(0) as f64;
    Some(inp * ip + out * op + cr * crp + cw * ip * 1.25)
}

/// Estimate total cost for a session by summing per-event costs.
fn session_cost_usd(events: &[McpEvent]) -> f64 {
    events.iter().filter_map(event_cost_usd).sum()
}

fn fmt_cost(usd: f64) -> String {
    match usd {
        usd if usd < 0.001 => format!("${usd:.5}"),
        usd if usd < 1.0 => format!("${usd:.4}"),
        usd => format!("${usd:.2}"),
    }
}

// ── Diff helpers ─────────────────────────────────────────────────────────────

fn diff_summary(diff: &str) -> (usize, usize) {
    let added: usize = diff
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let removed: usize = diff
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();
    (added, removed)
}

fn diff_badge(diff: Option<&str>) -> String {
    match diff {
        Some(d) if !crypto::is_encrypted(d) && d != "new file" => {
            let (a, r) = diff_summary(d);
            if a == 0 && r == 0 {
                return String::new();
            }
            format!("  {GREEN}+{a}{RESET}{RED}-{r}{RESET}")
        }
        Some("new file") => format!("  {GREEN}new{RESET}"),
        _ => String::new(),
    }
}

// ── Event argument extraction ────────────────────────────────────────────────

fn primary_arg(args: &serde_json::Value) -> serde_json::Value {
    args.get("file_path")
        .or_else(|| args.get("path"))
        .or_else(|| args.get("command"))
        .or_else(|| args.get("pattern"))
        .or_else(|| args.get("from"))
        .cloned()
        .unwrap_or(serde_json::Value::String("—".to_string()))
}

fn maybe_decrypt(key: Option<&[u8; 32]>, value: &serde_json::Value) -> String {
    let binding = value.to_string();
    let s = value.as_str().unwrap_or(&binding);
    if let Some(k) = key {
        if crypto::is_encrypted(s) {
            return crypto::decrypt(k, s).unwrap_or_else(|| "[decrypt failed]".to_string());
        }
    }
    s.to_string()
}

/// Format the argument for display: short path or truncated command.
fn fmt_arg(e: &McpEvent, key: Option<&[u8; 32]>, project_root: Option<&str>) -> String {
    let raw = maybe_decrypt(key, &primary_arg(&e.arguments));
    // If it looks like a file path, shorten it
    if raw.starts_with('/') || raw.contains('/') {
        short_path(&raw, project_root)
    } else {
        trunc(&raw, 50)
    }
}

// ── Ledger file loading ──────────────────────────────────────────────────────

#[derive(Default)]
pub struct ViewArgs {
    pub last: Option<usize>,
    pub risk: Option<String>,
    pub tool: Option<String>,
    pub session: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub expand: bool,
}

/// Max events shown at the head/tail of a session before collapsing the middle.
const COLLAPSE_HEAD: usize = 5;
const COLLAPSE_TAIL: usize = 5;

fn all_ledger_files(ledger_path: &str) -> Vec<std::path::PathBuf> {
    let path = std::path::Path::new(ledger_path);
    let parent = path.parent().unwrap_or(std::path::Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("events");
    let active_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

    let mut files: Vec<(std::path::PathBuf, u128)> = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name == active_name {
                return None;
            }
            if name.starts_with(stem) && name.ends_with(".jsonl") {
                let ts: u128 = name
                    .strip_prefix(&format!("{stem}."))?
                    .strip_suffix(".jsonl")?
                    .parse()
                    .ok()?;
                Some((e.path(), ts))
            } else {
                None
            }
        })
        .collect();

    files.sort_by_key(|(_, ts)| *ts);
    let mut result: Vec<std::path::PathBuf> = files.into_iter().map(|(p, _)| p).collect();
    result.push(path.to_path_buf());
    result
}

fn load_sessions(ledger_path: &str) -> Result<Vec<(String, Vec<McpEvent>)>> {
    let files = all_ledger_files(ledger_path);
    let any_exists = files.iter().any(|f| f.exists());
    if !any_exists {
        return Err(anyhow::anyhow!(
            "no ledger found at {ledger_path}\nRun vigilo first to generate events."
        ));
    }

    let mut sessions: Vec<(String, Vec<McpEvent>)> = Vec::new();
    for file_path in &files {
        let Ok(file) = File::open(file_path) else {
            continue;
        };
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(mut event) = serde_json::from_str::<McpEvent>(&line) {
                // Re-classify old events that were stored as Unknown
                if event.risk == Risk::Unknown {
                    event.risk = Risk::classify(&event.tool);
                }
                let sid = event.session_id.to_string();
                match sessions.iter_mut().find(|(id, _)| id == &sid) {
                    Some((_, events)) => events.push(event),
                    None => sessions.push((sid, vec![event])),
                }
            }
        }
    }
    // Sort by last event timestamp so the most recently active session appears last.
    sessions.sort_by(|a, b| {
        let last_a = a.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        let last_b = b.1.last().map(|e| e.timestamp.as_str()).unwrap_or("");
        last_a.cmp(last_b)
    });
    Ok(sessions)
}

// ── risk_label (plain text, used for filtering) ──────────────────────────────

fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Read => "read",
        Risk::Write => "write",
        Risk::Exec => "exec",
        Risk::Unknown => "unknown",
    }
}

// ── Cursor token enrichment (from local cache) ──────────────────────────────

/// Look up cached Cursor token data for a session's time window.
fn cursor_session_tokens(events: &[McpEvent]) -> Option<cursor_usage::CachedSessionTokens> {
    let first = events.first()?;
    if first.server != "cursor" {
        return None;
    }
    // Already has token data from the ledger — no enrichment needed
    if events.iter().any(|e| e.input_tokens.is_some()) {
        return None;
    }

    let (start_ms, end_ms) = session_time_range_ms(events)?;
    let cached = cursor_usage::load_cached_tokens_for_range(start_ms, end_ms);
    cursor_usage::aggregate_cached_tokens(&cached)
}

/// Parse the session's first/last timestamps into epoch milliseconds.
fn session_time_range_ms(events: &[McpEvent]) -> Option<(i64, i64)> {
    let parse_ts = |ts: &str| -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(ts)
            .or_else(|_| chrono::DateTime::parse_from_rfc3339(&format!("{ts}Z")))
            .ok()
            .map(|dt| dt.timestamp_millis())
    };

    let first_ts = events.first().and_then(|e| parse_ts(&e.timestamp))?;
    let last_ts = events.last().and_then(|e| parse_ts(&e.timestamp))?;

    // Pad by 1 minute on each side to account for timing differences
    Some((first_ts - 60_000, last_ts + 60_000))
}

// ── view (run) ───────────────────────────────────────────────────────────────

pub fn run(ledger_path: &str, args: ViewArgs) -> Result<()> {
    let key = crypto::load_key();
    let mut sessions = load_sessions(ledger_path)?;

    if let Some(prefix) = &args.session {
        sessions.retain(|(sid, _)| sid.starts_with(prefix.as_str()));
    }
    if args.since.is_some() || args.until.is_some() {
        for (_, events) in &mut sessions {
            events.retain(|e| {
                let ts = e.timestamp.get(..10).unwrap_or("");
                if let Some(since) = args.since.as_deref() {
                    if ts < since {
                        return false;
                    }
                }
                if let Some(until) = args.until.as_deref() {
                    if ts > until {
                        return false;
                    }
                }
                true
            });
        }
        sessions.retain(|(_, events)| !events.is_empty());
    }
    if let Some(n) = args.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }

    if sessions.is_empty() {
        println!("no events recorded yet.");
        return Ok(());
    }

    let risk_filter = args.risk.as_deref();
    let tool_filter = args.tool.as_deref();

    for (sid, events) in &sessions {
        // ── Session header ───────────────────────────────────────────────
        let first = &events[0];
        let badge = client_badge(&first.server);
        let sid_short = &sid[..8];
        let ts_header = first
            .timestamp
            .get(5..16)
            .unwrap_or(&first.timestamp)
            .replace('T', " ");

        println!();
        println!(" {badge}  {BOLD}{sid_short}{RESET}  {DIM}{ts_header}{RESET}");

        let p = &first.project;
        let project_line = match (p.name.as_deref(), p.branch.as_deref(), p.commit.as_deref()) {
            (Some(name), Some(branch), Some(commit)) => {
                let commit_short = &commit[..7.min(commit.len())];
                format!(
                    " │  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}@{DIM}{commit_short}{RESET}"
                )
            }
            (Some(name), Some(branch), None) => {
                format!(" │  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}")
            }
            (Some(name), None, None) => {
                format!(" │  {CYAN}{name}{RESET}")
            }
            _ => p
                .root
                .as_deref()
                .map(|r| format!(" │  {CYAN}{r}{RESET}"))
                .unwrap_or_default(),
        };
        if !project_line.is_empty() {
            println!("{project_line}");
        }

        // Model on header line 3 (tokens go in the footer to avoid redundancy)
        let cursor_tokens = cursor_session_tokens(events);
        if let Some(last_tok) = events.iter().rev().find(|e| e.model.is_some()) {
            let model_str = normalize_model(last_tok.model.as_deref().unwrap_or("unknown"));
            println!(" │  {DIM}{model_str}{RESET}");
        } else if let Some(ct) = &cursor_tokens {
            println!(" │  {DIM}{}{RESET}", ct.model);
        }

        let project_root = first.project.root.as_deref();

        // ── Event lines ──────────────────────────────────────────────────
        // Filter events first (risk/tool), then collapse the middle for long sessions.
        let visible: Vec<&McpEvent> = events
            .iter()
            .filter(|e| risk_filter.is_none_or(|r| risk_label(e.risk) == r))
            .filter(|e| tool_filter.is_none_or(|t| e.tool == t))
            .collect();

        let collapse = !args.expand && visible.len() > COLLAPSE_HEAD + COLLAPSE_TAIL + 2;
        let total_visible = visible.len();

        for (i, e) in visible.iter().enumerate() {
            if collapse && i == COLLAPSE_HEAD {
                let hidden = total_visible - COLLAPSE_HEAD - COLLAPSE_TAIL;
                println!(" │  {DIM}··· {hidden} more calls ···{RESET}");
            }
            if collapse && i >= COLLAPSE_HEAD && i < total_visible - COLLAPSE_TAIL {
                continue;
            }

            let is_error = matches!(e.outcome, Outcome::Err { .. });
            let time = e.timestamp.get(11..19).unwrap_or("??:??:??");
            let risk_sym = risk_decorated(e.risk, is_error);
            let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
            let arg = fmt_arg(e, key.as_ref(), project_root);
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

            println!(
                " │  {DIM}{time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}"
            );
        }

        // ── Session footer ───────────────────────────────────────────────
        let total_us: u64 = events.iter().map(|e| e.duration_us).sum();
        let reads = events
            .iter()
            .filter(|e| matches!(e.risk, Risk::Read))
            .count();
        let writes = events
            .iter()
            .filter(|e| matches!(e.risk, Risk::Write))
            .count();
        let execs = events
            .iter()
            .filter(|e| matches!(e.risk, Risk::Exec))
            .count();
        let errors = events
            .iter()
            .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
            .count();

        let err_str = if errors > 0 {
            format!(" · {BRIGHT_RED}{errors} err{RESET}")
        } else {
            String::new()
        };

        let dur_str = if total_us > 0 {
            format!(" · {}", models::fmt_duration(total_us))
        } else {
            String::new()
        };
        println!(
            " {DIM}└─ {} calls · r:{reads} w:{writes} e:{execs}{}{}{dur_str}{RESET}",
            events.len(),
            err_str,
            DIM
        );

        // Sum tokens across all events in the session (each event has per-turn values)
        let sum_in: u64 = events.iter().filter_map(|e| e.input_tokens).sum();
        let sum_out: u64 = events.iter().filter_map(|e| e.output_tokens).sum();
        let sum_cr: u64 = events.iter().filter_map(|e| e.cache_read_tokens).sum();
        let has_tokens = sum_in > 0 || sum_out > 0 || sum_cr > 0;

        if has_tokens {
            let cache_str = if sum_cr > 0 {
                format!(" · cache: {} read", fmt_tokens(sum_cr))
            } else {
                String::new()
            };
            let cost = session_cost_usd(events);
            let cost_str = if cost > 0.0 {
                format!(" · ~{} (list pricing)", fmt_cost(cost))
            } else {
                String::new()
            };
            println!(
                "    {DIM}tokens: {} in · {} out{cache_str}{cost_str}{RESET}",
                fmt_tokens(sum_in),
                fmt_tokens(sum_out)
            );
        } else if let Some(ct) = &cursor_tokens {
            let inp = fmt_tokens(ct.input_tokens);
            let out = fmt_tokens(ct.output_tokens);
            let cr = ct.cache_read_tokens;
            let cache_str = if cr > 0 {
                format!(" · cache: {} read", fmt_tokens(cr))
            } else {
                String::new()
            };
            let cost_str = if ct.cost_usd > 0.0 {
                format!(" · ${:.2}", ct.cost_usd)
            } else {
                String::new()
            };
            println!(
                "    {DIM}tokens: {inp} in · {out} out{cache_str}{cost_str} ({} reqs){RESET}",
                ct.request_count
            );
        }
    }

    println!();
    Ok(())
}

// ── errors ───────────────────────────────────────────────────────────────────

pub fn errors(ledger_path: &str, since: Option<&str>, until: Option<&str>) -> Result<()> {
    let key = crypto::load_key();
    let mut sessions = load_sessions(ledger_path)?;

    if since.is_some() || until.is_some() {
        for (_, events) in &mut sessions {
            events.retain(|e| {
                let ts = e.timestamp.get(..10).unwrap_or("");
                if let Some(s) = since {
                    if ts < s {
                        return false;
                    }
                }
                if let Some(u) = until {
                    if ts > u {
                        return false;
                    }
                }
                true
            });
        }
        sessions.retain(|(_, events)| !events.is_empty());
    }

    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let err_events: Vec<&McpEvent> = all_events
        .iter()
        .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
        .copied()
        .collect();

    if err_events.is_empty() {
        println!("\n  {GREEN}No errors found.{RESET}\n");
        return Ok(());
    }

    // ── Summary ──────────────────────────────────────────────────────────
    let total = all_events.len();
    let err_count = err_events.len();
    let pct = if total > 0 {
        err_count * 100 / total
    } else {
        0
    };

    println!();
    println!("{DIM}── vigilo errors ───────────────────────────────{RESET}");
    println!();
    println!("  {BRIGHT_RED}{err_count}{RESET} errors out of {total} calls ({pct}%)");

    // ── By tool ──────────────────────────────────────────────────────────
    let mut by_tool: HashMap<&str, Vec<&McpEvent>> = HashMap::new();
    for e in &err_events {
        by_tool.entry(e.tool.as_str()).or_default().push(e);
    }
    let mut tool_list: Vec<(&str, Vec<&McpEvent>)> = by_tool.into_iter().collect();
    tool_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    println!();
    println!("  {BOLD}by tool{RESET}");
    println!("  {DIM}───────{RESET}");
    for (tool, count) in tool_list.iter().map(|(t, v)| (t, v.len())) {
        let bar_len = (count * 20) / err_count.max(1);
        let bar: String = "█".repeat(bar_len.max(1));
        println!("  {BOLD}{count:>4}×{RESET} {tool:<12} {RED}{bar}{RESET}");
    }

    // ── Recent errors ────────────────────────────────────────────────────
    let recent: Vec<&McpEvent> = err_events.iter().rev().take(10).copied().collect();
    println!();
    println!("  {BOLD}recent errors{RESET} (last {})", recent.len());
    println!("  {DIM}─────────────{RESET}");
    for e in &recent {
        let badge = client_badge(&e.server);
        let time = e
            .timestamp
            .get(5..19)
            .unwrap_or(&e.timestamp)
            .replace('T', " ");
        let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
        let project_root = e.project.root.as_deref();
        let arg = fmt_arg(e, key.as_ref(), project_root);
        let arg_display = trunc(&arg, 35);
        let err_msg = match &e.outcome {
            Outcome::Err { message, .. } => trunc(message.trim(), 50),
            _ => String::new(),
        };

        println!("  {badge} {DIM}{time}{RESET}  {BRIGHT_RED}✖{RESET} {tool_name} {arg_display}");
        if !err_msg.is_empty() {
            println!("    {DIM}{err_msg}{RESET}");
        }
    }

    println!();
    Ok(())
}

// ── stats ────────────────────────────────────────────────────────────────────

pub fn stats_filtered(ledger_path: &str, since: Option<&str>, until: Option<&str>) -> Result<()> {
    let mut sessions = load_sessions(ledger_path)?;
    if since.is_some() || until.is_some() {
        for (_, events) in &mut sessions {
            events.retain(|e| {
                let ts = e.timestamp.get(..10).unwrap_or("");
                if let Some(s) = since {
                    if ts < s {
                        return false;
                    }
                }
                if let Some(u) = until {
                    if ts > u {
                        return false;
                    }
                }
                true
            });
        }
        sessions.retain(|(_, events)| !events.is_empty());
    }
    let sessions = sessions;

    if sessions.is_empty() {
        println!("no events recorded yet.");
        return Ok(());
    }

    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let total_calls = all_events.len();
    let total_us: u64 = all_events.iter().map(|e| e.duration_us).sum();
    let errors = all_events
        .iter()
        .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
        .count();
    let reads = all_events
        .iter()
        .filter(|e| matches!(e.risk, Risk::Read))
        .count();
    let writes = all_events
        .iter()
        .filter(|e| matches!(e.risk, Risk::Write))
        .count();
    let execs = all_events
        .iter()
        .filter(|e| matches!(e.risk, Risk::Exec))
        .count();

    let mut tool_counts: HashMap<&str, usize> = HashMap::new();
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    let mut project_risk: HashMap<String, (usize, usize, usize)> = HashMap::new();

    for e in &all_events {
        *tool_counts.entry(&e.tool).or_default() += 1;

        if let Some(path) = e
            .arguments
            .get("file_path")
            .or_else(|| e.arguments.get("path"))
            .and_then(|v| v.as_str())
        {
            if !crypto::is_encrypted(path) {
                let display = path.rsplit('/').next().unwrap_or(path).to_string();
                *file_counts.entry(display).or_default() += 1;
            }
        }

        let project = e
            .project
            .name
            .as_deref()
            .or(e.project.root.as_deref())
            .unwrap_or("unknown")
            .to_string();
        *project_counts.entry(project.clone()).or_default() += 1;
        let pr = project_risk.entry(project).or_default();
        match e.risk {
            Risk::Read => pr.0 += 1,
            Risk::Write => pr.1 += 1,
            Risk::Exec => pr.2 += 1,
            Risk::Unknown => {}
        }
    }

    let mut tools: Vec<(&str, usize)> = tool_counts.into_iter().collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1));

    let mut files: Vec<(String, usize)> = file_counts.into_iter().collect();
    files.sort_by(|a, b| b.1.cmp(&a.1));

    let error_pct = if total_calls > 0 {
        errors * 100 / total_calls
    } else {
        0
    };

    // ── Header ───────────────────────────────────────────────────────────
    println!();
    println!("{DIM}── vigilo stats ────────────────────────────────{RESET}");
    println!();

    let err_display = if errors > 0 {
        format!(
            " · {BRIGHT_RED}{errors} error{} ({error_pct}%){RESET}",
            if errors != 1 { "s" } else { "" }
        )
    } else {
        " · 0 errors".to_string()
    };
    println!(
        "  {BOLD}{}{RESET} sessions · {BOLD}{total_calls}{RESET} calls{err_display} · {} total",
        sessions.len(),
        models::fmt_duration(total_us)
    );
    println!("  risk: {CYAN}{reads} read{RESET} · {YELLOW}{writes} write{RESET} · {RED}{execs} exec{RESET}");

    // ── Tools + Files side-by-side ───────────────────────────────────────
    println!();
    println!("  {BOLD}tools{RESET}                    {BOLD}files{RESET}");
    println!("  {DIM}─────                    ─────{RESET}");

    let max_rows = 8;
    for i in 0..max_rows {
        let tool_col = if i < tools.len() {
            format!("  {BOLD}{:>4}×{RESET} {:<20}", tools[i].1, tools[i].0)
        } else {
            "                           ".to_string()
        };
        let file_col = if i < files.len() {
            format!("{BOLD}{:>4}×{RESET} {}", files[i].1, files[i].0)
        } else {
            String::new()
        };
        if i < tools.len() || i < files.len() {
            println!("{tool_col}{file_col}");
        }
    }

    // ── Models ───────────────────────────────────────────────────────────
    // Tuple: (calls, input_tokens, output_tokens, cache_read_tokens, cost_usd)
    let mut model_counts: HashMap<&str, (usize, u64, u64, u64, f64)> = HashMap::new();
    for e in &all_events {
        if let Some(m) = e.model.as_deref() {
            let entry = model_counts.entry(normalize_model(m)).or_default();
            entry.0 += 1;
            entry.1 += e.input_tokens.unwrap_or(0);
            entry.2 += e.output_tokens.unwrap_or(0);
            entry.3 += e.cache_read_tokens.unwrap_or(0);
            if let Some(c) = event_cost_usd(e) {
                entry.4 += c;
            }
        }
    }
    if !model_counts.is_empty() {
        let mut models: Vec<_> = model_counts.into_iter().collect();
        models.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        println!();
        println!("  {BOLD}models{RESET}");
        println!("  {DIM}──────{RESET}");
        for (model, (calls, inp, out, cr, cost)) in &models {
            let tok_str = if *inp > 0 || *out > 0 || *cr > 0 {
                let cache_part = if *cr > 0 {
                    format!(" · cache:{}", fmt_tokens(*cr))
                } else {
                    String::new()
                };
                format!(
                    "     {DIM}{} in · {} out{cache_part}{RESET}",
                    fmt_tokens(*inp),
                    fmt_tokens(*out)
                )
            } else {
                String::new()
            };
            let cost_str = if *cost > 0.0 {
                format!(" · ~{}", fmt_cost(*cost))
            } else {
                String::new()
            };
            println!("  {BOLD}{calls:>4}×{RESET} {model}{tok_str}{cost_str}");
        }

        let total_cost: f64 = models.iter().map(|(_, (_, _, _, _, c))| c).sum();
        let total_in: u64 = models.iter().map(|(_, (_, i, _, _, _))| i).sum();
        let total_out: u64 = models.iter().map(|(_, (_, _, o, _, _))| o).sum();
        let total_cr: u64 = models.iter().map(|(_, (_, _, _, c, _))| c).sum();
        if total_cost > 0.0 || total_in > 0 || total_out > 0 {
            println!();
            let cache_part = if total_cr > 0 {
                format!(" · cache: {} read", fmt_tokens(total_cr))
            } else {
                String::new()
            };
            println!(
                "  {DIM}total: {} in · {} out{cache_part}{RESET}",
                fmt_tokens(total_in),
                fmt_tokens(total_out)
            );
            if total_cost > 0.0 {
                println!("  {DIM}~{} (list pricing){RESET}", fmt_cost(total_cost));
            }
        }
    }

    // ── Projects ─────────────────────────────────────────────────────────
    let mut projects: Vec<(String, usize)> = project_counts.into_iter().collect();
    projects.sort_by(|a, b| b.1.cmp(&a.1));
    println!();
    println!("  {BOLD}projects{RESET}");
    println!("  {DIM}────────{RESET}");
    for (name, count) in &projects {
        let (r, w, e) = project_risk
            .get(name.as_str())
            .copied()
            .unwrap_or((0, 0, 0));
        println!("  {BOLD}{count:>4}×{RESET} {name}  {DIM}r:{r} w:{w} e:{e}{RESET}");
    }

    println!();
    Ok(())
}

// ── query ────────────────────────────────────────────────────────────────────

pub fn query(
    ledger_path: &str,
    since: Option<&str>,
    until: Option<&str>,
    tool: Option<&str>,
    risk: Option<&str>,
    session: Option<&str>,
) -> Result<()> {
    let sessions = load_sessions(ledger_path)?;
    let key = crypto::load_key();

    let events: Vec<&McpEvent> = sessions
        .iter()
        .filter(|(sid, _)| session.is_none_or(|pfx| sid.starts_with(pfx)))
        .flat_map(|(_, events)| events)
        .filter(|e| {
            let ts = e.timestamp.get(..10).unwrap_or("");
            if let Some(s) = since {
                if ts < s {
                    return false;
                }
            }
            if let Some(u) = until {
                if ts > u {
                    return false;
                }
            }
            if let Some(t) = tool {
                if e.tool != t {
                    return false;
                }
            }
            if let Some(r) = risk {
                if risk_label(e.risk) != r {
                    return false;
                }
            }
            true
        })
        .collect();

    if events.is_empty() {
        println!("no matching events.");
        return Ok(());
    }

    println!();
    println!(
        "{DIM}── {} matching events ──────────────────────────{RESET}",
        events.len()
    );
    println!();
    for e in events {
        let is_error = matches!(e.outcome, Outcome::Err { .. });
        let badge = client_badge(&e.server);
        let time = e.timestamp.get(11..19).unwrap_or("??:??:??");
        let risk_sym = risk_decorated(e.risk, is_error);
        let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
        let project_root = e.project.root.as_deref();
        let arg = fmt_arg(e, key.as_ref(), project_root);
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
        let sid_short = &e.session_id.to_string()[..8];

        println!(" {badge}  {DIM}{time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}  {DIM}{sid_short}{RESET}");
    }
    println!();
    Ok(())
}

// ── diff ─────────────────────────────────────────────────────────────────────

pub fn diff(ledger_path: &str, args: &ViewArgs) -> Result<()> {
    let key = crypto::load_key();
    let mut sessions = load_sessions(ledger_path)?;

    if let Some(prefix) = &args.session {
        sessions.retain(|(sid, _)| sid.starts_with(prefix.as_str()));
    }
    if args.since.is_some() || args.until.is_some() {
        for (_, events) in &mut sessions {
            events.retain(|e| {
                let ts = e.timestamp.get(..10).unwrap_or("");
                if let Some(since) = args.since.as_deref() {
                    if ts < since {
                        return false;
                    }
                }
                if let Some(until) = args.until.as_deref() {
                    if ts > until {
                        return false;
                    }
                }
                true
            });
        }
        sessions.retain(|(_, events)| !events.is_empty());
    }
    if let Some(n) = args.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }

    if sessions.is_empty() {
        println!("no events with diffs found.");
        return Ok(());
    }

    for (sid, events) in &sessions {
        let edits: Vec<&McpEvent> = events.iter().filter(|e| e.diff.is_some()).collect();

        if edits.is_empty() {
            continue;
        }

        let first = &events[0];
        let badge = client_badge(&first.server);
        let sid_short = &sid[..8];
        let project_root = first.project.root.as_deref();

        // Group edits by file path
        let mut by_file: Vec<(String, Vec<&McpEvent>)> = Vec::new();
        for e in &edits {
            let path = extract_file_path(e, key.as_ref(), project_root);
            match by_file.iter_mut().find(|(p, _)| p == &path) {
                Some((_, list)) => list.push(e),
                None => by_file.push((path, vec![e])),
            }
        }

        // Session header
        println!();
        println!("{DIM}── vigilo diff ── {RESET}{badge} {BOLD}{sid_short}{RESET} {DIM}────────────────────────{RESET}");
        if let Some(name) = first.project.name.as_deref() {
            let branch = first.project.branch.as_deref().unwrap_or("");
            if !branch.is_empty() {
                println!("  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}");
            } else {
                println!("  {CYAN}{name}{RESET}");
            }
        }

        let mut total_added: usize = 0;
        let mut total_removed: usize = 0;

        for (path, file_edits) in &by_file {
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

            total_added += file_add;
            total_removed += file_rem;

            // File header
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
            println!("  {BOLD}{path}{RESET}  {DIM}({edit_count} {edit_word}){RESET}{new_badge}{change_str}");
            println!("  {DIM}{}─{RESET}", "─".repeat(path.len().max(10)));

            // Each edit's diff
            for e in file_edits {
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

                println!("  {DIM}{time}{RESET}  {BOLD}{tool}{RESET}{mini_badge}");

                if !crypto::is_encrypted(diff_text) && diff_text != "new file" {
                    print_colored_diff(diff_text);
                }
            }
        }

        // Session footer
        let file_count = by_file.len();
        let file_word = if file_count == 1 { "file" } else { "files" };
        println!();
        println!("  {DIM}── {}{RESET} {DIM}{edit_word} across{RESET} {BOLD}{file_count}{RESET} {DIM}{file_word}{RESET} · {GREEN}+{total_added}{RESET} {RED}-{total_removed}{RESET} {DIM}net ──{RESET}",
            edits.len(), edit_word = if edits.len() == 1 { "edit" } else { "edits" });
    }

    println!();
    Ok(())
}

/// Extract the file path from an event for diff grouping.
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

/// Print diff text with colored + / - lines.
fn print_colored_diff(diff_text: &str) {
    for line in diff_text.lines() {
        if line.starts_with('+') {
            println!("    {GREEN}{line}{RESET}");
        } else if line.starts_with('-') {
            println!("    {RED}{line}{RESET}");
        } else {
            println!("    {DIM}{line}{RESET}");
        }
    }
}

// ── export (unchanged — data format, not display) ────────────────────────────

pub fn export(ledger_path: &str, format: &str) -> Result<()> {
    let sessions = load_sessions(ledger_path)?;
    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();

    if all_events.is_empty() {
        println!("no events recorded yet.");
        return Ok(());
    }

    if format == "json" {
        let json = serde_json::to_string_pretty(&all_events.iter().collect::<Vec<_>>())
            .map_err(|e| anyhow::anyhow!(e))?;
        println!("{json}");
        return Ok(());
    }

    println!("timestamp,session_id,project,branch,commit,dirty,tool,risk,arg,duration_us,status");
    for e in all_events {
        let status = if matches!(e.outcome, Outcome::Err { .. }) {
            "error"
        } else {
            "ok"
        };
        let risk = format!("{:?}", e.risk).to_lowercase();
        let arg = e
            .arguments
            .get("file_path")
            .or_else(|| e.arguments.get("path"))
            .or_else(|| e.arguments.get("command"))
            .or_else(|| e.arguments.get("pattern"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('"', "\"\"");
        let ts = e
            .timestamp
            .get(..19)
            .unwrap_or(&e.timestamp)
            .replace('T', " ");
        let sid = &e.session_id.to_string()[..8];
        let project = e
            .project
            .name
            .as_deref()
            .or(e.project.root.as_deref())
            .unwrap_or("");
        let branch = e.project.branch.as_deref().unwrap_or("");
        let commit = e.project.commit.as_deref().unwrap_or("");
        let dirty = if e.project.dirty { "1" } else { "0" };

        println!(
            "{ts},{sid},{project},{branch},{commit},{dirty},{},{risk},\"{arg}\",{},{}",
            e.tool, e.duration_us, status
        );
    }
    Ok(())
}

// ── watch ────────────────────────────────────────────────────────────────────

pub async fn watch(ledger_path: &str) -> Result<()> {
    let mut file = loop {
        match File::open(ledger_path) {
            Ok(f) => break f,
            Err(_) => {
                eprintln!("{DIM}[vigilo] waiting for ledger at {ledger_path}...{RESET}");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    };

    file.seek(SeekFrom::End(0))?;

    let key = crypto::load_key();
    println!("{DIM}[vigilo]{RESET} watching — ctrl+c to stop");
    println!();

    loop {
        let mut line = String::new();
        let n = BufReader::new(&file).read_line(&mut line)?;
        if n == 0 {
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            let pos = file.stream_position()?;
            file = File::open(ledger_path).unwrap_or(file);
            file.seek(SeekFrom::Start(pos))?;
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
            let is_error = matches!(e.outcome, Outcome::Err { .. });
            let badge = client_badge(&e.server);
            let time = e.timestamp.get(11..19).unwrap_or(&e.timestamp);
            let risk_sym = risk_decorated(e.risk, is_error);
            let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
            let project_root = e.project.root.as_deref();
            let arg = fmt_arg(&e, key.as_ref(), project_root);
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

            println!(" {badge}  {DIM}{time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}");
        }
    }
}
