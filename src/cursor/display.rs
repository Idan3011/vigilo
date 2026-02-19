use std::collections::HashMap;

use crate::view::fmt::{
    cprintln, fmt_tokens, normalize_model, BOLD, CYAN, DIM, GREEN, RED, RESET, YELLOW,
};

#[derive(Default)]
pub(super) struct TokenTotals {
    pub count: usize,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_cents: f64,
}

impl TokenTotals {
    pub fn from_event(ev: &serde_json::Value) -> Self {
        let tok = &ev["tokenUsage"];
        Self {
            count: 1,
            input: tok["inputTokens"].as_u64().unwrap_or(0),
            output: tok["outputTokens"].as_u64().unwrap_or(0),
            cache_read: tok["cacheReadTokens"].as_u64().unwrap_or(0),
            cache_write: tok["cacheWriteTokens"].as_u64().unwrap_or(0),
            cost_cents: tok["totalCents"].as_f64().unwrap_or(0.0),
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.count += other.count;
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cost_cents += other.cost_cents;
    }
}

pub(super) fn fmt_cost_cents(cents: f64) -> String {
    let usd = cents / 100.0;
    match usd {
        usd if usd < 0.01 => format!("${usd:.4}"),
        usd if usd < 1.0 => format!("${usd:.3}"),
        usd => format!("${usd:.2}"),
    }
}

pub(super) fn print_summary(summary: &serde_json::Value) {
    let start = summary["billingCycleStart"].as_str().unwrap_or("?");
    let end = summary["billingCycleEnd"].as_str().unwrap_or("?");
    let kind = summary["limitType"].as_str().unwrap_or("unknown");

    let s = start.get(5..10).unwrap_or(start);
    let e = end.get(5..10).unwrap_or(end);
    cprintln!("  {DIM}billing: {s}{RESET} → {DIM}{e}{RESET}  {DIM}({kind}){RESET}");

    let plan = &summary["individualUsage"]["plan"];
    if plan.is_object() {
        let used = plan["used"].as_u64().unwrap_or(0);
        let limit = plan["limit"].as_u64().unwrap_or(0);
        let remaining = plan["remaining"].as_u64().unwrap_or(0);
        let pct = plan["totalPercentUsed"].as_f64().unwrap_or(0.0);
        let color = if pct > 80.0 {
            RED
        } else if pct > 50.0 {
            YELLOW
        } else {
            GREEN
        };
        cprintln!("  {DIM}plan:{RESET} {BOLD}{used}{RESET}/{limit} requests  {DIM}({remaining} remaining){RESET}  {color}{pct:.0}%{RESET}");
    }

    let od = &summary["individualUsage"]["onDemand"];
    if od["enabled"].as_bool() == Some(true) {
        let used = od["used"].as_u64().unwrap_or(0);
        let limit = od["limit"]
            .as_u64()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unlimited".to_string());
        cprintln!(
            "  {DIM}on-demand:{RESET} {BOLD}{used}{RESET} used  {DIM}(limit: {limit}){RESET}"
        );
    }
}

pub(super) fn print_events(events: &[serde_json::Value], since_days: u32) {
    let mut totals = TokenTotals::default();
    let mut by_model: HashMap<String, TokenTotals> = HashMap::new();

    for ev in events {
        let t = TokenTotals::from_event(ev);
        totals.merge(&t);
        by_model
            .entry(normalize_model(ev["model"].as_str().unwrap_or("unknown")).to_string())
            .or_default()
            .merge(&t);
    }

    println!();
    cprintln!("{DIM}── token usage ({since_days}d) ─────────────────────────────{RESET}");
    println!();
    cprintln!("  {BOLD}{}{RESET} requests", totals.count);
    cprintln!("  {CYAN}{}{RESET} input · {CYAN}{}{RESET} output · {DIM}{} cache read · {} cache write{RESET}",
        fmt_tokens(totals.input), fmt_tokens(totals.output),
        fmt_tokens(totals.cache_read), fmt_tokens(totals.cache_write));

    if totals.cost_cents > 0.0 {
        cprintln!(
            "  {YELLOW}{}{RESET} total cost",
            fmt_cost_cents(totals.cost_cents)
        );
    }

    print_model_breakdown(by_model);
}

fn print_model_breakdown(by_model: HashMap<String, TokenTotals>) {
    let mut models: Vec<(String, TokenTotals)> = by_model.into_iter().collect();
    models.sort_by(|a, b| b.1.count.cmp(&a.1.count));

    println!();
    cprintln!("  {BOLD}by model{RESET}");
    cprintln!("  {DIM}────────{RESET}");
    for (model, t) in &models {
        let cost = if t.cost_cents > 0.0 {
            format!(" · {}", fmt_cost_cents(t.cost_cents))
        } else {
            String::new()
        };
        let cache = if t.cache_read > 0 {
            format!(" · cache:{}", fmt_tokens(t.cache_read))
        } else {
            String::new()
        };
        cprintln!(
            "  {BOLD}{:>4}×{RESET} {model}  {DIM}{} in · {} out{cache}{cost}{RESET}",
            t.count,
            fmt_tokens(t.input),
            fmt_tokens(t.output)
        );
    }
}
