use super::data::{cursor_session_tokens, load_sessions, LoadFilter};
use super::fmt::{
    client_badge, diff_badge, fmt_arg, fmt_cost, fmt_tokens, normalize_model, risk_decorated,
    risk_label, short_id, trunc, BOLD, BRIGHT_RED, CYAN, DIM, RESET,
};
use super::{ViewArgs, COLLAPSE_HEAD, COLLAPSE_TAIL};
use crate::{
    crypto,
    models::{self, McpEvent, Outcome, Risk},
};
use anyhow::Result;

pub fn run(ledger_path: &str, args: ViewArgs) -> Result<()> {
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
        println!("no events recorded yet.");
        return Ok(());
    }

    for (sid, events) in &sessions {
        let Some(first) = events.first() else {
            continue;
        };
        let cursor_tokens = cursor_session_tokens(events);
        print_session_header(sid, first);
        print_session_events(
            events,
            key.as_ref(),
            first.project.root.as_deref(),
            args.risk.as_deref(),
            args.tool.as_deref(),
            args.expand,
        );
        print_session_footer(events, &cursor_tokens);
    }

    println!();
    Ok(())
}

fn print_session_header(sid: &str, first: &McpEvent) {
    let badge = client_badge(&first.server);
    let sid_short = short_id(sid);
    let ts_header = first
        .timestamp
        .get(5..16)
        .unwrap_or(&first.timestamp)
        .replace('T', " ");

    println!();
    println!(" {badge}  {BOLD}{sid_short}{RESET}  {DIM}{ts_header}{RESET}");

    let project_line = format_project_line(&first.project);
    if !project_line.is_empty() {
        println!("{project_line}");
    }
}

fn format_project_line(p: &models::ProjectContext) -> String {
    match (p.name.as_deref(), p.branch.as_deref(), p.commit.as_deref()) {
        (Some(name), Some(branch), Some(commit)) => {
            let commit_short = &commit[..7.min(commit.len())];
            format!(" │  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}@{DIM}{commit_short}{RESET}")
        }
        (Some(name), Some(branch), None) => {
            format!(" │  {CYAN}{name}{RESET} · {CYAN}{branch}{RESET}")
        }
        (Some(name), None, None) => format!(" │  {CYAN}{name}{RESET}"),
        _ => p
            .root
            .as_deref()
            .map(|r| format!(" │  {CYAN}{r}{RESET}"))
            .unwrap_or_default(),
    }
}

fn print_session_events(
    events: &[McpEvent],
    key: Option<&[u8; 32]>,
    project_root: Option<&str>,
    risk_filter: Option<&str>,
    tool_filter: Option<&str>,
    expand: bool,
) {
    if let Some(last_tok) = events.iter().rev().find(|e| e.model.is_some()) {
        let model_str = normalize_model(last_tok.model.as_deref().unwrap_or("unknown"));
        println!(" │  {DIM}{model_str}{RESET}");
    }

    let visible: Vec<&McpEvent> = events
        .iter()
        .filter(|e| risk_filter.is_none_or(|r| risk_label(e.risk) == r))
        .filter(|e| tool_filter.is_none_or(|t| e.tool == t))
        .collect();

    let collapse = !expand && visible.len() > COLLAPSE_HEAD + COLLAPSE_TAIL + 2;
    let total_visible = visible.len();

    for (i, e) in visible.iter().enumerate() {
        if collapse && i == COLLAPSE_HEAD {
            let hidden = total_visible - COLLAPSE_HEAD - COLLAPSE_TAIL;
            println!(" │  {DIM}··· {hidden} more calls ···{RESET}");
        }
        if collapse && i >= COLLAPSE_HEAD && i < total_visible - COLLAPSE_TAIL {
            continue;
        }
        print_event_row(e, key, project_root);
    }
}

fn print_event_row(e: &McpEvent, key: Option<&[u8; 32]>, project_root: Option<&str>) {
    let is_error = matches!(e.outcome, Outcome::Err { .. });
    let time = e.timestamp.get(11..19).unwrap_or("??:??:??");
    let risk_sym = risk_decorated(e.risk, is_error);
    let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
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
    println!(" │  {DIM}{time}{RESET}  {risk_sym} {tool_name} {arg_display}{diff}{dur}{timeout}");
}

fn print_session_footer(
    events: &[McpEvent],
    cursor_tokens: &Option<crate::cursor_usage::CachedSessionTokens>,
) {
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

    print_footer_tokens(events, cursor_tokens);
}

fn print_footer_tokens(
    events: &[McpEvent],
    cursor_tokens: &Option<crate::cursor_usage::CachedSessionTokens>,
) {
    let sum_in: u64 = events.iter().filter_map(|e| e.input_tokens).sum();
    let sum_out: u64 = events.iter().filter_map(|e| e.output_tokens).sum();
    let sum_cr: u64 = events.iter().filter_map(|e| e.cache_read_tokens).sum();

    if sum_in > 0 || sum_out > 0 || sum_cr > 0 {
        let cache_str = if sum_cr > 0 {
            format!(" · cache: {} read", fmt_tokens(sum_cr))
        } else {
            String::new()
        };
        let cost = super::fmt::session_cost_usd(events);
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
    } else if let Some(ct) = cursor_tokens {
        let cache_str = if ct.cache_read_tokens > 0 {
            format!(" · cache: {} read", fmt_tokens(ct.cache_read_tokens))
        } else {
            String::new()
        };
        let cost_str = if ct.cost_usd > 0.0 {
            format!(" · ${:.2}", ct.cost_usd)
        } else {
            String::new()
        };
        println!(
            "    {DIM}tokens: {} in · {} out{cache_str}{cost_str} ({} reqs){RESET}",
            fmt_tokens(ct.input_tokens),
            fmt_tokens(ct.output_tokens),
            ct.request_count
        );
    }
}

pub fn sessions(ledger_path: &str, args: ViewArgs) -> Result<()> {
    let filter = LoadFilter {
        since: args.since.as_deref(),
        until: args.until.as_deref(),
        session: None,
    };
    let mut sessions = load_sessions(ledger_path, &filter)?;

    if let Some(n) = args.last {
        let skip = sessions.len().saturating_sub(n);
        sessions.drain(..skip);
    }

    if sessions.is_empty() {
        println!("\n  {DIM}no sessions found.{RESET}\n");
        return Ok(());
    }

    println!();
    println!(
        "{DIM}── {} sessions ─────────────────────────────────{RESET}",
        sessions.len()
    );
    println!();

    for (sid, events) in &sessions {
        print_session_list_row(sid, events);
    }

    println!();
    Ok(())
}

fn print_session_list_row(sid: &str, events: &[McpEvent]) {
    let Some(first) = events.first() else {
        return;
    };
    let badge = client_badge(&first.server);
    let sid_short = short_id(sid);
    let date = first
        .timestamp
        .get(5..16)
        .unwrap_or(&first.timestamp)
        .replace('T', " ");
    let project = first
        .project
        .name
        .as_deref()
        .or(first.project.root.as_deref())
        .unwrap_or("—");
    let project_display = trunc(project, 20);
    let total_us: u64 = events.iter().map(|e| e.duration_us).sum();

    println!(
        "  {badge}  {DIM}{sid_short}{RESET}  {DIM}{date}{RESET}  {CYAN}{project_display:<20}{RESET}  {BOLD}{:>4}{RESET} calls  {}",
        events.len(),
        models::fmt_duration(total_us)
    );
}

pub fn tail(ledger_path: &str, n: usize) -> Result<()> {
    let sessions = load_sessions(ledger_path, &LoadFilter::default())?;
    let key = crypto::load_key();

    let mut all: Vec<(&McpEvent, &str)> = sessions
        .iter()
        .flat_map(|(sid, events)| events.iter().map(move |e| (e, sid.as_str())))
        .collect();

    all.sort_by_key(|(e, _)| e.timestamp.as_str());

    let skip = all.len().saturating_sub(n);
    let tail_events = &all[skip..];

    if tail_events.is_empty() {
        println!("no events recorded yet.");
        return Ok(());
    }

    println!();
    for (e, sid) in tail_events {
        print_tail_row(e, sid, key.as_ref());
    }

    println!();
    Ok(())
}

fn print_tail_row(e: &McpEvent, sid: &str, key: Option<&[u8; 32]>) {
    let is_error = matches!(e.outcome, Outcome::Err { .. });
    let badge = client_badge(&e.server);
    let date_time = e
        .timestamp
        .get(5..19)
        .unwrap_or(&e.timestamp)
        .replace('T', " ");
    let risk_sym = risk_decorated(e.risk, is_error);
    let tool_name = format!("{BOLD}{:<8}{RESET}", trunc(&e.tool, 8));
    let project_root = e.project.root.as_deref();
    let arg = fmt_arg(e, key, project_root);
    let arg_display = trunc(&arg, 30);
    let diff = diff_badge(e.diff.as_deref());
    let sid_short = &sid[..8.min(sid.len())];

    println!(
        "  {DIM}{date_time}{RESET}  {risk_sym} {tool_name} {arg_display:<30}{diff}    {badge}  {DIM}{sid_short}{RESET}"
    );
}
