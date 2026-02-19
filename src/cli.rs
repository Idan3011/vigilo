use crate::view;

pub fn print_help() {
    println!("vigilo {}", env!("CARGO_PKG_VERSION"));
    println!("Observe what AI agents do — every tool call logged, nothing sent anywhere.\n");
    print_usage();
    print_options();
}

fn print_usage() {
    println!("USAGE:");
    println!("  vigilo                          MCP server mode (reads stdio)");
    println!("  vigilo summary                  Today at a glance");
    println!("  vigilo sessions [OPTIONS]       List all sessions (one line each)");
    println!("  vigilo tail     [-n N | --last N]  Last N events flat (default: 20)");
    println!("  vigilo view     [OPTIONS]       View ledger grouped by session");
    println!("  vigilo watch                    Live tail of incoming events");
    println!("  vigilo stats    [OPTIONS]       Aggregate stats across all sessions");
    println!("  vigilo errors   [OPTIONS]       Show errors (--expand for full details)");
    println!("  vigilo diff     [OPTIONS]       Show file diffs grouped by session");
    println!("  vigilo query    [OPTIONS]       Filter events across all sessions");
    println!("  vigilo export   [OPTIONS]       Export events as CSV or JSON");
    println!("  vigilo cursor-usage [OPTIONS]   Fetch real token usage from cursor.com");
    println!("  vigilo prune    [OPTIONS]       Delete old rotated ledger files");
    println!("  vigilo doctor                   Check configuration and dependencies");
    println!("  vigilo setup                    Interactive setup wizard");
    println!("  vigilo generate-key             Generate a base64 AES-256 encryption key");
    println!("  vigilo help | --help | -h       Show this message");
    println!("  vigilo --version | -V           Show version\n");
    println!("INTERNAL:");
    println!(
        "  vigilo hook                     Process a hook event from stdin (used by editors)\n"
    );
}

fn print_options() {
    println!("VIEW / STATS / QUERY OPTIONS:");
    println!("  --since <expr>    From date  (today, yesterday, 7d, 2w, 1m, YYYY-MM-DD)");
    println!("  --until <expr>    To date    (same formats as --since)");
    println!("  --risk <level>    Filter by risk level: read | write | exec");
    println!("  --tool <name>     Filter by tool name (view and query)");
    println!("  --session <pfx>   Filter by session UUID prefix");
    println!("  --last <n>        Show only the last N sessions");
    println!("  --expand          Show all events / full error details");
    println!("  --no-color        Disable colored output (also respects NO_COLOR env)\n");
    println!("EXPORT OPTIONS:");
    println!("  --format <fmt>    Output format: csv (default) | json");
    println!("  --output <path>   Write to file (default: ~/.vigilo/export.<ext>)\n");
    println!("PRUNE OPTIONS:");
    println!("  --older-than <n>  Days threshold (default: 30)\n");
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

pub fn parse_view_args(args: &[String]) -> view::ViewArgs {
    let mut out = view::ViewArgs::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--last" => match args.get(i + 1) {
                Some(s) => match s.parse() {
                    Ok(n) => {
                        out.last = Some(n);
                        i += 1;
                    }
                    Err(_) => eprintln!("vigilo: --last requires a number, got '{s}'"),
                },
                None => eprintln!("vigilo: --last requires a value"),
            },
            "--risk" | "--tool" | "--session" | "--since" | "--until" => {
                let flag = args[i].as_str();
                match args.get(i + 1) {
                    Some(val) => {
                        match flag {
                            "--risk" => out.risk = Some(val.clone()),
                            "--tool" => out.tool = Some(val.clone()),
                            "--session" => out.session = Some(val.clone()),
                            "--since" => out.since = Some(parse_date(val)),
                            "--until" => out.until = Some(parse_date(val)),
                            _ => {}
                        }
                        i += 1;
                    }
                    None => eprintln!("vigilo: {flag} requires a value"),
                }
            }
            "--expand" => out.expand = true,
            other if other.starts_with("--") => {
                eprintln!("vigilo: unknown option '{other}'");
            }
            _ => {}
        }
        i += 1;
    }
    out
}

pub fn get_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

pub fn filter_flags(args: &[String], flags: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if flags.contains(&arg.as_str()) {
            skip_next = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

pub fn parse_date(expr: &str) -> String {
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
