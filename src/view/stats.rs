use super::counts::{
    collect_active_projects, print_error_chart, print_models_section, print_projects_section,
    print_recent_errors, print_tool_file_table, EventCounts,
};
use super::data::{load_sessions, LoadFilter};
use super::fmt::{fmt_cost, fmt_tokens, BOLD, BRIGHT_RED, CYAN, DIM, GREEN, RED, RESET, YELLOW};
use crate::{
    crypto,
    models::{self, McpEvent, Outcome},
};
use anyhow::Result;

pub fn stats_filtered(ledger_path: &str, since: Option<&str>, until: Option<&str>) -> Result<()> {
    let filter = LoadFilter {
        since,
        until,
        ..Default::default()
    };
    let sessions = load_sessions(ledger_path, &filter)?;

    if sessions.is_empty() {
        println!("no events recorded yet.");
        return Ok(());
    }

    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let c = EventCounts::from_events(&all_events);

    print_stats_header(sessions.len(), &c);
    print_tool_file_table(&all_events);
    print_models_section(&all_events, &sessions);
    print_projects_section(&all_events);

    println!();
    Ok(())
}

fn print_stats_header(session_count: usize, c: &EventCounts) {
    let error_pct = if c.total > 0 {
        c.errors * 100 / c.total
    } else {
        0
    };
    let err_display = if c.errors > 0 {
        format!(
            " · {BRIGHT_RED}{} error{} ({error_pct}%){RESET}",
            c.errors,
            if c.errors != 1 { "s" } else { "" }
        )
    } else {
        " · 0 errors".to_string()
    };
    println!();
    println!("{DIM}── vigilo stats ────────────────────────────────{RESET}");
    println!();
    println!(
        "  {BOLD}{session_count}{RESET} sessions · {BOLD}{}{RESET} calls{err_display} · {} total",
        c.total,
        models::fmt_duration(c.total_us)
    );
    println!(
        "  risk: {CYAN}{} read{RESET} · {YELLOW}{} write{RESET} · {RED}{} exec{RESET}",
        c.reads, c.writes, c.execs
    );
}

pub fn errors(ledger_path: &str, since: Option<&str>, until: Option<&str>) -> Result<()> {
    let key = crypto::load_key();
    let filter = LoadFilter {
        since,
        until,
        ..Default::default()
    };
    let sessions = load_sessions(ledger_path, &filter)?;

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

    print_error_chart(&err_events);
    print_recent_errors(&err_events, key.as_ref());

    println!();
    Ok(())
}

pub fn summary(ledger_path: &str) -> Result<()> {
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let filter = LoadFilter {
        since: Some(&today),
        until: Some(&today),
        ..Default::default()
    };
    let sessions = load_sessions(ledger_path, &filter)?;

    if sessions.is_empty() {
        println!("\n  {DIM}no sessions today.{RESET}\n");
        return Ok(());
    }

    let all_events: Vec<&McpEvent> = sessions.iter().flat_map(|(_, e)| e).collect();
    let mut c = EventCounts::from_events(&all_events);
    c.add_cursor_tokens(&sessions);

    print_summary_body(sessions.len(), &c);
    print_summary_tokens(&c);

    let active_projects = collect_active_projects(&sessions);
    if !active_projects.is_empty() {
        println!(
            "  active: {CYAN}{}{RESET}",
            active_projects.join(&format!("{RESET} · {CYAN}"))
        );
    }

    println!();
    Ok(())
}

fn print_summary_body(session_count: usize, c: &EventCounts) {
    let err_str = if c.errors > 0 {
        format!("{BRIGHT_RED}{} errors{RESET}", c.errors)
    } else {
        "0 errors".to_string()
    };
    println!();
    println!("{DIM}── today ───────────────────────────────────────{RESET}");
    println!();
    println!(
        "  {BOLD}{session_count}{RESET} sessions · {BOLD}{}{RESET} calls · {err_str} · {}",
        c.total,
        models::fmt_duration(c.total_us)
    );
    println!(
        "  risk: {CYAN}{} read{RESET} · {YELLOW}{} write{RESET} · {RED}{} exec{RESET}",
        c.reads, c.writes, c.execs
    );
}

fn print_summary_tokens(c: &EventCounts) {
    if c.total_in == 0 && c.total_out == 0 {
        return;
    }
    let cache_str = if c.total_cr > 0 {
        format!(" · cache: {} read", fmt_tokens(c.total_cr))
    } else {
        String::new()
    };
    let cost_str = if c.total_cost > 0.0 {
        format!(" · ~{}", fmt_cost(c.total_cost))
    } else {
        String::new()
    };
    println!(
        "  tokens: {} in · {} out{cache_str}{cost_str}",
        fmt_tokens(c.total_in),
        fmt_tokens(c.total_out)
    );
}
