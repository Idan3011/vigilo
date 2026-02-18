mod crypto;
mod cursor_usage;
mod git;
mod hook;
mod hook_helpers;
mod ledger;
mod models;
mod server;
mod setup;
mod view;

use anyhow::Result;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let ledger_path =
        std::env::var("VIGILO_LEDGER").unwrap_or_else(|_| format!("{home}/.vigilo/events.jsonl"));

    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h")
        || args.first().map(|s| s.as_str()) == Some("help")
    {
        print_help();
        return Ok(());
    }

    if let Some(result) = dispatch_subcommand(&args, &ledger_path).await {
        return result;
    }

    let session_id = Uuid::new_v4();
    eprintln!("[vigilo] session={session_id}");
    eprintln!("[vigilo] ledger={ledger_path}");

    server::run(ledger_path, session_id).await
}

async fn dispatch_subcommand(args: &[String], ledger_path: &str) -> Option<Result<()>> {
    match args.first().map(|s| s.as_str()) {
        Some("view") => Some(view::run(ledger_path, parse_view_args(&args[1..]))),
        Some("generate-key") => Some(generate_key()),
        Some("stats") => Some(dispatch_stats(&args[1..], ledger_path)),
        Some("errors") => Some(dispatch_errors(&args[1..], ledger_path)),
        Some("query") => Some(dispatch_query(&args[1..], ledger_path)),
        Some("diff") => Some(view::diff(ledger_path, &parse_view_args(&args[1..]))),
        Some("cursor-usage") => Some(dispatch_cursor_usage(&args[1..]).await),
        Some("hook") => Some(hook::run(ledger_path).await),
        Some("setup") => Some(setup::run()),
        Some("watch") => Some(view::watch(ledger_path).await),
        Some("summary") => Some(view::summary(ledger_path)),
        Some("sessions") => Some(view::sessions(ledger_path, parse_view_args(&args[1..]))),
        Some("tail") => Some(dispatch_tail(&args[1..], ledger_path)),
        Some("export") => Some(dispatch_export(&args[1..], ledger_path)),
        _ => None,
    }
}

fn generate_key() -> Result<()> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use rand::RngCore;
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    println!("{}", STANDARD.encode(key));
    Ok(())
}

fn dispatch_stats(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    view::stats_filtered(ledger_path, since.as_deref(), until.as_deref())
}

fn dispatch_errors(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    view::errors(ledger_path, since.as_deref(), until.as_deref())
}

fn dispatch_query(args: &[String], ledger_path: &str) -> Result<()> {
    let since = get_flag(args, "--since").map(|s| parse_date(&s));
    let until = get_flag(args, "--until").map(|s| parse_date(&s));
    let tool = get_flag(args, "--tool");
    let risk = get_flag(args, "--risk");
    let session = get_flag(args, "--session");
    view::query(
        ledger_path,
        since.as_deref(),
        until.as_deref(),
        tool.as_deref(),
        risk.as_deref(),
        session.as_deref(),
    )
}

async fn dispatch_cursor_usage(args: &[String]) -> Result<()> {
    let since = get_flag(args, "--since-days")
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    if args.iter().any(|a| a == "--sync") {
        cursor_usage::sync(since).await
    } else {
        cursor_usage::run(since).await
    }
}

fn dispatch_tail(args: &[String], ledger_path: &str) -> Result<()> {
    let n = get_flag(args, "-n")
        .or_else(|| get_flag(args, "--last"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    view::tail(ledger_path, n)
}

fn dispatch_export(args: &[String], ledger_path: &str) -> Result<()> {
    let format = get_flag(args, "--format").unwrap_or_else(|| "csv".to_string());
    view::export(ledger_path, &format)
}

fn parse_view_args(args: &[String]) -> view::ViewArgs {
    let mut out = view::ViewArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--last" => {
                if let Some(n) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    out.last = Some(n);
                    i += 1;
                }
            }
            "--risk" => {
                out.risk = args.get(i + 1).cloned();
                i += 1;
            }
            "--tool" => {
                out.tool = args.get(i + 1).cloned();
                i += 1;
            }
            "--session" => {
                out.session = args.get(i + 1).cloned();
                i += 1;
            }
            "--since" => {
                out.since = args.get(i + 1).map(|s| parse_date(s));
                i += 1;
            }
            "--until" => {
                out.until = args.get(i + 1).map(|s| parse_date(s));
                i += 1;
            }
            "--expand" => out.expand = true,
            _ => {}
        }
        i += 1;
    }
    out
}

fn print_help() {
    println!("vigilo {}", env!("CARGO_PKG_VERSION"));
    println!("Observe what AI agents do — every tool call logged, nothing sent anywhere.\n");
    print_help_usage();
    print_help_options();
}

fn print_help_usage() {
    println!("USAGE:");
    println!("  vigilo                          MCP server mode (reads stdio)");
    println!("  vigilo summary                  Today at a glance");
    println!("  vigilo sessions [OPTIONS]       List all sessions (one line each)");
    println!("  vigilo tail     [-n N]          Last N events flat (default: 20)");
    println!("  vigilo view     [OPTIONS]       View ledger grouped by session");
    println!("  vigilo watch                    Live tail of incoming events");
    println!("  vigilo stats    [OPTIONS]       Aggregate stats across all sessions");
    println!("  vigilo errors   [OPTIONS]       Show errors grouped by tool");
    println!("  vigilo diff     [OPTIONS]       Show file diffs grouped by session");
    println!("  vigilo query    [OPTIONS]       Filter events across all sessions");
    println!("  vigilo cursor-usage [OPTIONS]   Fetch real token usage from cursor.com");
    println!("  vigilo export [--format json]   Dump all events as CSV or JSON to stdout");
    println!("  vigilo hook                     Process a Claude Code PostToolUse hook event (reads stdin)");
    println!("  vigilo generate-key             Generate a base64 AES-256 encryption key");
    println!("  vigilo help                     Show this message\n");
}

fn print_help_options() {
    println!("VIEW / STATS / QUERY OPTIONS:");
    println!("  --since <expr>    From date  (today, yesterday, 7d, 2w, 1m, YYYY-MM-DD)");
    println!("  --until <expr>    To date    (same formats as --since)");
    println!("  --risk <level>    Filter by risk level: read | write | exec");
    println!("  --tool <name>     Filter by tool name (view and query)");
    println!("  --session <pfx>   Filter by session UUID prefix");
    println!("  --last <n>        Show only the last N sessions");
    println!("  --expand          Show all events (default: first 5 + last 5 per session)\n");
    println!("CURSOR-USAGE OPTIONS:");
    println!("  --since-days <n>  Number of days to look back (default: 30)");
    println!("  --sync            Fetch and cache token data without printing\n");
    println!("ENVIRONMENT:");
    println!("  VIGILO_LEDGER           Path to ledger file (default: ~/.vigilo/events.jsonl)");
    println!("  VIGILO_ENCRYPTION_KEY   Base64 AES-256 key — encrypts arguments and results\n");
    println!("TOOLS (Risk level):");
    println!("  read    read_file, list_directory, search_files, get_file_info, git_status, git_diff, git_log");
    println!(
        "  write   write_file, create_directory, delete_file, move_file, patch_file, git_commit"
    );
    println!("  exec    run_command");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    #[test]
    fn parse_date_today() {
        let expected = Local::now().date_naive().format("%Y-%m-%d").to_string();
        assert_eq!(parse_date("today"), expected);
    }

    #[test]
    fn parse_date_days() {
        let result = parse_date("7d");
        assert!(result.len() == 10 && result.contains('-'));
    }

    #[test]
    fn parse_date_weeks() {
        let result = parse_date("2w");
        assert!(result.len() == 10 && result.contains('-'));
    }

    #[test]
    fn parse_date_months() {
        let result = parse_date("1m");
        assert!(result.len() == 10 && result.contains('-'));
    }

    #[test]
    fn parse_date_passthrough() {
        assert_eq!(parse_date("2026-02-01"), "2026-02-01");
    }
}

fn get_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn parse_date(expr: &str) -> String {
    use chrono::{Duration, Local};

    let today = Local::now().date_naive();

    match expr {
        "today" => today.format("%Y-%m-%d").to_string(),
        "yesterday" => (today - Duration::days(1)).format("%Y-%m-%d").to_string(),
        s if s.ends_with('d') => parse_duration_days(s, today),
        s if s.ends_with('w') => parse_duration_weeks(s, today),
        s if s.ends_with('m') => parse_duration_months(s, today),
        _ => expr.to_string(),
    }
}

fn parse_duration_days(s: &str, today: chrono::NaiveDate) -> String {
    use chrono::Duration;
    s.trim_end_matches('d')
        .parse::<u64>()
        .ok()
        .map(|n| {
            (today - Duration::days(n as i64))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| s.to_string())
}

fn parse_duration_weeks(s: &str, today: chrono::NaiveDate) -> String {
    use chrono::Duration;
    s.trim_end_matches('w')
        .parse::<u64>()
        .ok()
        .map(|n| {
            (today - Duration::weeks(n as i64))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| s.to_string())
}

fn parse_duration_months(s: &str, today: chrono::NaiveDate) -> String {
    use chrono::Months;
    s.trim_end_matches('m')
        .parse::<u32>()
        .ok()
        .and_then(|n| {
            today
                .checked_sub_months(Months::new(n))
                .map(|d| d.format("%Y-%m-%d").to_string())
        })
        .unwrap_or_else(|| s.to_string())
}
