use super::data::cursor_session_tokens;
use super::fmt::{
    client_badge, event_cost_usd, fmt_arg, fmt_cost, fmt_tokens, normalize_model, trunc, BOLD,
    BRIGHT_RED, DIM, RED, RESET,
};
use crate::{
    crypto,
    models::{McpEvent, Outcome, Risk},
};
use std::collections::HashMap;

pub(super) struct EventCounts {
    pub total: usize,
    pub reads: usize,
    pub writes: usize,
    pub execs: usize,
    pub errors: usize,
    pub total_us: u64,
    pub total_in: u64,
    pub total_out: u64,
    pub total_cr: u64,
    pub total_cost: f64,
}

impl EventCounts {
    pub fn from_events(events: &[&McpEvent]) -> Self {
        Self {
            total: events.len(),
            reads: events
                .iter()
                .filter(|e| matches!(e.risk, Risk::Read))
                .count(),
            writes: events
                .iter()
                .filter(|e| matches!(e.risk, Risk::Write))
                .count(),
            execs: events
                .iter()
                .filter(|e| matches!(e.risk, Risk::Exec))
                .count(),
            errors: events
                .iter()
                .filter(|e| matches!(e.outcome, Outcome::Err { .. }))
                .count(),
            total_us: events.iter().map(|e| e.duration_us).sum(),
            total_in: events.iter().filter_map(|e| e.input_tokens).sum(),
            total_out: events.iter().filter_map(|e| e.output_tokens).sum(),
            total_cr: events.iter().filter_map(|e| e.cache_read_tokens).sum(),
            total_cost: events.iter().filter_map(|e| event_cost_usd(e)).sum(),
        }
    }

    pub fn add_cursor_tokens(&mut self, sessions: &[(String, Vec<McpEvent>)]) {
        for (_, events) in sessions {
            if let Some(ct) = cursor_session_tokens(events) {
                self.total_in += ct.input_tokens;
                self.total_out += ct.output_tokens;
                self.total_cr += ct.cache_read_tokens;
                self.total_cost += ct.cost_usd;
            }
        }
    }
}

pub(super) fn print_tool_file_table(events: &[&McpEvent]) {
    let tools = count_tools(events);
    let files = count_files(events);
    print_two_column_table(&tools, &files);
}

fn count_tools(events: &[&McpEvent]) -> Vec<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for e in events {
        *counts.entry(&e.tool).or_default() += 1;
    }
    let mut sorted: Vec<(String, usize)> = counts
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}

fn count_files(events: &[&McpEvent]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for e in events {
        if let Some(path) = e
            .arguments
            .get("file_path")
            .or_else(|| e.arguments.get("path"))
            .and_then(|v| v.as_str())
        {
            if !crypto::is_encrypted(path) {
                let display = path.rsplit('/').next().unwrap_or(path).to_string();
                *counts.entry(display).or_default() += 1;
            }
        }
    }
    let mut sorted: Vec<(String, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));
    sorted
}

fn print_two_column_table(tools: &[(String, usize)], files: &[(String, usize)]) {
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
}

#[derive(Default)]
struct ModelStats {
    calls: usize,
    input: u64,
    output: u64,
    cache_read: u64,
    cost: f64,
}

pub(super) fn print_models_section(events: &[&McpEvent], sessions: &[(String, Vec<McpEvent>)]) {
    let mut model_counts: HashMap<String, ModelStats> = HashMap::new();
    for e in events {
        if let Some(m) = e.model.as_deref() {
            let entry = model_counts
                .entry(normalize_model(m).to_string())
                .or_default();
            entry.calls += 1;
            entry.input += e.input_tokens.unwrap_or(0);
            entry.output += e.output_tokens.unwrap_or(0);
            entry.cache_read += e.cache_read_tokens.unwrap_or(0);
            if let Some(c) = event_cost_usd(e) {
                entry.cost += c;
            }
        }
    }
    for (_, events) in sessions {
        if let Some(ct) = cursor_session_tokens(events) {
            let entry = model_counts.entry(ct.model.clone()).or_default();
            entry.input += ct.input_tokens;
            entry.output += ct.output_tokens;
            entry.cache_read += ct.cache_read_tokens;
            entry.cost += ct.cost_usd;
        }
    }
    if model_counts.is_empty() {
        return;
    }
    let mut models: Vec<_> = model_counts.into_iter().collect();
    models.sort_by(|a, b| b.1.calls.cmp(&a.1.calls));
    println!();
    println!("  {BOLD}models{RESET}");
    println!("  {DIM}──────{RESET}");
    for (model, s) in &models {
        let tok_str = format_model_tokens(s.input, s.output, s.cache_read);
        let cost_str = if s.cost > 0.0 {
            format!(" · ~{}", fmt_cost(s.cost))
        } else {
            String::new()
        };
        println!("  {BOLD}{:>4}×{RESET} {model}{tok_str}{cost_str}", s.calls);
    }
    print_model_totals(&models);
}

fn format_model_tokens(inp: u64, out: u64, cr: u64) -> String {
    if inp == 0 && out == 0 && cr == 0 {
        return String::new();
    }
    let cache_part = if cr > 0 {
        format!(" · cache:{}", fmt_tokens(cr))
    } else {
        String::new()
    };
    format!(
        "     {DIM}{} in · {} out{cache_part}{RESET}",
        fmt_tokens(inp),
        fmt_tokens(out)
    )
}

fn print_model_totals(models: &[(String, ModelStats)]) {
    let total_cost: f64 = models.iter().map(|(_, s)| s.cost).sum();
    let total_in: u64 = models.iter().map(|(_, s)| s.input).sum();
    let total_out: u64 = models.iter().map(|(_, s)| s.output).sum();
    let total_cr: u64 = models.iter().map(|(_, s)| s.cache_read).sum();
    if total_cost == 0.0 && total_in == 0 && total_out == 0 {
        return;
    }
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

pub(super) fn print_projects_section(events: &[&McpEvent]) {
    let mut project_counts: HashMap<String, usize> = HashMap::new();
    let mut project_risk: HashMap<String, (usize, usize, usize)> = HashMap::new();

    for e in events {
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
}

pub(super) fn print_error_chart(err_events: &[&McpEvent]) {
    let err_count = err_events.len();
    let mut by_tool: HashMap<&str, usize> = HashMap::new();
    for e in err_events {
        *by_tool.entry(e.tool.as_str()).or_default() += 1;
    }
    let mut tool_list: Vec<(&str, usize)> = by_tool.into_iter().collect();
    tool_list.sort_by(|a, b| b.1.cmp(&a.1));

    println!();
    println!("  {BOLD}by tool{RESET}");
    println!("  {DIM}───────{RESET}");
    for (tool, count) in &tool_list {
        let bar_len = (count * 20) / err_count.max(1);
        let bar: String = "█".repeat(bar_len.max(1));
        println!("  {BOLD}{count:>4}×{RESET} {tool:<12} {RED}{bar}{RESET}");
    }
}

pub(super) fn print_recent_errors(err_events: &[&McpEvent], key: Option<&[u8; 32]>) {
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
        let arg = fmt_arg(e, key, project_root);
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
}

pub(super) fn collect_active_projects(sessions: &[(String, Vec<McpEvent>)]) -> Vec<String> {
    let mut active: Vec<String> = Vec::new();
    for (_, events) in sessions {
        if let Some(last) = events.last() {
            let label = match (last.project.name.as_deref(), last.project.branch.as_deref()) {
                (Some(name), Some(branch)) => format!("{name}/{branch}"),
                (Some(name), None) => name.to_string(),
                _ => continue,
            };
            if !active.contains(&label) {
                active.push(label);
            }
        }
    }
    active
}
