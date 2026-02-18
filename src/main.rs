mod crypto;
mod cursor_usage;
mod git;
mod hook;
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

    match args.first().map(|s| s.as_str()) {
        Some("view") => {
            let view_args = parse_view_args(&args[1..]);
            return view::run(&ledger_path, view_args);
        }
        Some("generate-key") => {
            use base64::{engine::general_purpose::STANDARD, Engine};
            use rand::RngCore;
            let mut key = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut key);
            println!("{}", STANDARD.encode(key));
            return Ok(());
        }
        Some("stats") => {
            let since = get_flag(&args[1..], "--since").map(|s| parse_date(&s));
            let until = get_flag(&args[1..], "--until").map(|s| parse_date(&s));
            return view::stats_filtered(&ledger_path, since.as_deref(), until.as_deref());
        }
        Some("errors") => {
            let since = get_flag(&args[1..], "--since").map(|s| parse_date(&s));
            let until = get_flag(&args[1..], "--until").map(|s| parse_date(&s));
            return view::errors(&ledger_path, since.as_deref(), until.as_deref());
        }
        Some("query") => {
            let since = get_flag(&args[1..], "--since").map(|s| parse_date(&s));
            let until = get_flag(&args[1..], "--until").map(|s| parse_date(&s));
            let tool = get_flag(&args[1..], "--tool");
            let risk = get_flag(&args[1..], "--risk");
            let session = get_flag(&args[1..], "--session");
            return view::query(
                &ledger_path,
                since.as_deref(),
                until.as_deref(),
                tool.as_deref(),
                risk.as_deref(),
                session.as_deref(),
            );
        }
        Some("diff") => {
            let view_args = parse_view_args(&args[1..]);
            return view::diff(&ledger_path, &view_args);
        }
        Some("cursor-usage") => {
            let since = get_flag(&args[1..], "--since-days")
                .and_then(|s| s.parse().ok())
                .unwrap_or(30);
            if args.iter().any(|a| a == "--sync") {
                return cursor_usage::sync(since).await;
            }
            return cursor_usage::run(since).await;
        }
        Some("hook") => return hook::run(&ledger_path).await,
        Some("setup") => return setup::run(),
        Some("watch") => return view::watch(&ledger_path).await,
        Some("export") => {
            let format = get_flag(&args[1..], "--format").unwrap_or_else(|| "csv".to_string());
            return view::export(&ledger_path, &format);
        }
        _ => {}
    }

    let session_id = Uuid::new_v4();
    eprintln!("[vigilo] session={session_id}");
    eprintln!("[vigilo] ledger={ledger_path}");

    server::run(ledger_path, session_id).await
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
                if let Some(r) = args.get(i + 1) {
                    out.risk = Some(r.clone());
                    i += 1;
                }
            }
            "--tool" => {
                if let Some(t) = args.get(i + 1) {
                    out.tool = Some(t.clone());
                    i += 1;
                }
            }
            "--session" => {
                if let Some(s) = args.get(i + 1) {
                    out.session = Some(s.clone());
                    i += 1;
                }
            }
            "--since" => {
                if let Some(s) = args.get(i + 1) {
                    out.since = Some(parse_date(s));
                    i += 1;
                }
            }
            "--until" => {
                if let Some(s) = args.get(i + 1) {
                    out.until = Some(parse_date(s));
                    i += 1;
                }
            }
            "--expand" => {
                out.expand = true;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn print_help() {
    println!("vigilo {}", env!("CARGO_PKG_VERSION"));
    println!("Observe what AI agents do — every tool call logged, nothing sent anywhere.\n");
    println!("USAGE:");
    println!("  vigilo                          MCP server mode (reads stdio)");
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

/// Resolve a human-readable date expression to a YYYY-MM-DD string.
/// Accepts: today, yesterday, Nd (days), Nw (weeks), Nm (months), YYYY-MM-DD.
fn parse_date(expr: &str) -> String {
    use chrono::{Duration, Local, Months};

    let today = Local::now().date_naive();

    match expr {
        "today" => today.format("%Y-%m-%d").to_string(),
        "yesterday" => (today - Duration::days(1)).format("%Y-%m-%d").to_string(),
        s if s.ends_with('d') => {
            if let Ok(n) = s.trim_end_matches('d').parse::<u64>() {
                (today - Duration::days(n as i64))
                    .format("%Y-%m-%d")
                    .to_string()
            } else {
                expr.to_string()
            }
        }
        s if s.ends_with('w') => {
            if let Ok(n) = s.trim_end_matches('w').parse::<u64>() {
                (today - Duration::weeks(n as i64))
                    .format("%Y-%m-%d")
                    .to_string()
            } else {
                expr.to_string()
            }
        }
        s if s.ends_with('m') => {
            if let Ok(n) = s.trim_end_matches('m').parse::<u32>() {
                today
                    .checked_sub_months(Months::new(n))
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| expr.to_string())
            } else {
                expr.to_string()
            }
        }
        _ => expr.to_string(), // pass through YYYY-MM-DD or anything else
    }
}
