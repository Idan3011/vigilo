use crate::{
    crypto,
    models::{McpEvent, Risk},
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static FORCE_NO_COLOR: AtomicBool = AtomicBool::new(false);
static COLOR: OnceLock<bool> = OnceLock::new();

pub(crate) fn disable_color() {
    FORCE_NO_COLOR.store(true, Ordering::Relaxed);
}

pub(crate) fn use_color() -> bool {
    if FORCE_NO_COLOR.load(Ordering::Relaxed) {
        return false;
    }
    *COLOR.get_or_init(|| std::env::var("NO_COLOR").is_err() && atty::is(atty::Stream::Stdout))
}

pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_esc = false;
    for ch in s.chars() {
        if in_esc {
            if ch == 'm' {
                in_esc = false;
            }
        } else if ch == '\x1b' {
            in_esc = true;
        } else {
            out.push(ch);
        }
    }
    out
}

macro_rules! cprintln {
    () => { println!() };
    ($($arg:tt)*) => {{
        let s = format!($($arg)*);
        if $crate::view::fmt::use_color() {
            println!("{s}");
        } else {
            println!("{}", $crate::view::fmt::strip_ansi(&s));
        }
    }};
}
pub(crate) use cprintln;

macro_rules! ceprintln {
    () => { eprintln!() };
    ($($arg:tt)*) => {{
        let s = format!($($arg)*);
        if $crate::view::fmt::use_color() {
            eprintln!("{s}");
        } else {
            eprintln!("{}", $crate::view::fmt::strip_ansi(&s));
        }
    }};
}
pub(crate) use ceprintln;

macro_rules! ceprint {
    ($($arg:tt)*) => {{
        let s = format!($($arg)*);
        if $crate::view::fmt::use_color() {
            eprint!("{s}");
        } else {
            eprint!("{}", $crate::view::fmt::strip_ansi(&s));
        }
    }};
}
pub(crate) use ceprint;

pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const BOLD: &str = "\x1b[1m";
pub(crate) const DIM: &str = "\x1b[2m";
pub(crate) const CYAN: &str = "\x1b[36m";
pub(crate) const GREEN: &str = "\x1b[32m";
pub(crate) const RED: &str = "\x1b[31m";
pub(crate) const YELLOW: &str = "\x1b[33m";
pub(crate) const BRIGHT_RED: &str = "\x1b[91m";
pub(crate) const WHITE: &str = "\x1b[97m";
pub(crate) const BG_BLUE: &str = "\x1b[44m";
pub(crate) const BG_MAGENTA: &str = "\x1b[45m";

pub(crate) fn client_badge(server: &str) -> String {
    match server {
        "cursor" => format!("{BG_MAGENTA}{BOLD}{WHITE} CURSOR {RESET}"),
        _ => format!("{BG_BLUE}{BOLD}{WHITE} CLAUDE {RESET}"),
    }
}

pub(crate) fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

pub(crate) fn short_path(full: &str, project_root: Option<&str>) -> String {
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
    full.rsplit('/').next().unwrap_or(full).to_string()
}

pub(crate) fn trunc(s: &str, max: usize) -> String {
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

pub(crate) fn fmt_tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1_000_000.0),
        n if n >= 1_000 => format!("{}K", n / 1_000),
        n => n.to_string(),
    }
}

pub(crate) fn normalize_model(m: &str) -> &str {
    match m {
        "default" | "auto" => "Auto",
        other => other,
    }
}

pub(crate) fn risk_decorated(risk: Risk, is_error: bool) -> String {
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

pub(crate) fn risk_label(risk: Risk) -> &'static str {
    match risk {
        Risk::Read => "read",
        Risk::Write => "write",
        Risk::Exec => "exec",
        Risk::Unknown => "unknown",
    }
}

#[rustfmt::skip]
const PRICE_TABLE: &[(&str, f64, f64, f64)] = &[
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
    ("claude-4.5-sonnet-thinking",                     3.00,  15.00,   0.30),
    ("Auto",                                           1.25,   6.00,   0.25),
    ("composer-1.5",                                   3.50,  17.50,   0.35),
    ("composer-1",                                     1.25,  10.00,   0.125),
    ("sonnet",                                         3.00,  15.00,   0.30),
    ("gpt-5-mini",                                     0.25,   2.00,   0.025),
    ("gpt-5",                                          1.25,  10.00,   0.125),
    ("gpt-4o-mini",                                    0.15,   0.60,   0.075),
    ("gpt-4o",                                         2.50,  10.00,   1.25),
    ("o3-mini",                                        1.10,   4.40,   0.55),
    ("o1-mini",                                        1.10,   4.40,   0.55),
    ("o3",                                            15.00,  60.00,   7.50),
    ("o1",                                            15.00,  60.00,   7.50),
    ("gemini-2.5-flash",                               0.30,   2.50,   0.03),
    ("gemini-3-pro",                                   2.00,  12.00,   0.20),
    ("gemini-3-flash",                                 0.50,   3.00,   0.05),
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

pub(crate) fn event_cost_usd(e: &McpEvent) -> Option<f64> {
    let (ip, op, crp) = pricing_for(e.model.as_deref()?)?;
    let inp = e.input_tokens? as f64;
    let out = e.output_tokens.unwrap_or(0) as f64;
    let cr = e.cache_read_tokens.unwrap_or(0) as f64;
    let cw = e.cache_write_tokens.unwrap_or(0) as f64;
    Some(inp * ip + out * op + cr * crp + cw * ip * 1.25)
}

pub(crate) fn session_cost_usd(events: &[McpEvent]) -> f64 {
    events.iter().filter_map(event_cost_usd).sum()
}

pub(crate) fn fmt_cost(usd: f64) -> String {
    match usd {
        usd if usd < 0.001 => format!("${usd:.5}"),
        usd if usd < 1.0 => format!("${usd:.4}"),
        usd => format!("${usd:.2}"),
    }
}

pub(crate) fn diff_summary(diff: &str) -> (usize, usize) {
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

pub(crate) fn diff_badge(diff: Option<&str>) -> String {
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

pub(crate) fn print_colored_diff(diff_text: &str) {
    for line in diff_text.lines() {
        if line.starts_with('+') {
            cprintln!("    {GREEN}{line}{RESET}");
        } else if line.starts_with('-') {
            cprintln!("    {RED}{line}{RESET}");
        } else {
            cprintln!("    {DIM}{line}{RESET}");
        }
    }
}

pub(crate) fn primary_arg(args: &serde_json::Value) -> serde_json::Value {
    args.get("file_path")
        .or_else(|| args.get("path"))
        .or_else(|| args.get("command"))
        .or_else(|| args.get("pattern"))
        .or_else(|| args.get("from"))
        .cloned()
        .unwrap_or(serde_json::Value::String("—".to_string()))
}

pub(crate) fn maybe_decrypt(key: Option<&[u8; 32]>, value: &serde_json::Value) -> String {
    let binding = value.to_string();
    let s = value.as_str().unwrap_or(&binding);
    if let Some(k) = key {
        if crypto::is_encrypted(s) {
            return crypto::decrypt(k, s).unwrap_or_else(|| "[decrypt failed]".to_string());
        }
    }
    s.to_string()
}

pub(crate) fn fmt_arg(e: &McpEvent, key: Option<&[u8; 32]>, project_root: Option<&str>) -> String {
    let raw = maybe_decrypt(key, &primary_arg(&e.arguments));
    if raw.starts_with('/') || raw.contains('/') {
        short_path(&raw, project_root)
    } else {
        trunc(&raw, 50)
    }
}
