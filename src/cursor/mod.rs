mod api;
mod cache;
mod credentials;
mod display;
mod platform;

pub use cache::{
    aggregate_cached_tokens, is_cache_stale, load_cached_tokens_for_range, CachedSessionTokens,
};
pub use platform::{discover_db, resolve_db_path};

use anyhow::Result;

use crate::view::fmt::{ceprintln, cprintln, BG_MAGENTA, BOLD, DIM, RESET, WHITE};

const MS_PER_DAY: i64 = 86_400_000;

pub fn has_cursor_db() -> bool {
    resolve_db_path().is_ok()
}

pub async fn sync(since_days: u32) -> Result<()> {
    let db_path = resolve_db_path()?;
    let creds = credentials::read_credentials(&db_path)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (since_days as i64 * MS_PER_DAY);
    let events = api::fetch_all_events(&client, &creds, start_ms, now_ms).await?;

    cache::write_cache(&events)?;
    cprintln!(
        "  {DIM}synced {} events to {}{RESET}",
        events.len(),
        crate::models::vigilo_path("cursor-tokens.jsonl").display()
    );
    Ok(())
}

pub async fn run(since_days: u32) -> Result<()> {
    let db_path = resolve_db_path()?;
    let creds = credentials::read_credentials(&db_path)?;

    let badge = format!("{BG_MAGENTA}{BOLD}{WHITE} CURSOR {RESET}");
    let email = creds.email.as_deref().unwrap_or("unknown");
    let membership = creds.membership.as_deref().unwrap_or("unknown");

    println!();
    cprintln!(" {badge}  {BOLD}{email}{RESET}  {DIM}({membership}){RESET}");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    crate::view::fmt::ceprint!("  {DIM}⠋ connecting to cursor.com...{RESET}");
    match api::fetch_summary(&client, &creds).await {
        Ok(s) => {
            eprint!("\r                                    \r");
            display::print_summary(&s);
        }
        Err(e) => {
            eprint!("\r                                    \r");
            ceprintln!("  {DIM}usage-summary: {e}{RESET}");
        }
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let start_ms = now_ms - (since_days as i64 * MS_PER_DAY);
    let events = api::fetch_all_events(&client, &creds, start_ms, now_ms).await?;

    if events.is_empty() {
        cprintln!("  {DIM}no usage events in the last {since_days} days{RESET}");
    } else {
        display::print_events(&events, since_days);
        cache::write_cache(&events)?;
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::cache::CachedTokenEvent;
    use super::credentials::{auth_cookie, percent_encode, Credentials};
    use super::display::{fmt_cost_cents, TokenTotals};
    use super::platform::{is_system_user, needs_local_copy};

    #[test]
    fn is_system_user_filters_known() {
        assert!(is_system_user("Default"));
        assert!(is_system_user("Public"));
        assert!(is_system_user("Default User"));
        assert!(is_system_user("All Users"));
    }

    #[test]
    fn is_system_user_allows_normal() {
        assert!(!is_system_user("john"));
        assert!(!is_system_user("admin"));
    }

    #[test]
    fn needs_local_copy_mnt_paths() {
        assert!(needs_local_copy("/mnt/c/Users/foo/state.vscdb"));
        assert!(!needs_local_copy("/home/user/.config/Cursor/state.vscdb"));
    }

    #[test]
    fn percent_encode_leaves_unreserved() {
        assert_eq!(
            percent_encode("hello-world_v1.0~test"),
            "hello-world_v1.0~test"
        );
    }

    #[test]
    fn percent_encode_encodes_special_chars() {
        assert_eq!(percent_encode("a::b"), "a%3A%3Ab");
        assert_eq!(percent_encode("foo bar"), "foo%20bar");
    }

    #[test]
    fn auth_cookie_format() {
        let creds = Credentials {
            user_id: "user123".to_string(),
            access_token: "tok456".to_string(),
            email: None,
            membership: None,
        };
        let cookie = auth_cookie(&creds);
        assert!(cookie.starts_with("WorkosCursorSessionToken="));
        assert!(cookie.contains("user123"));
        assert!(cookie.contains("tok456"));
    }

    #[test]
    fn auth_cookie_encodes_colons() {
        let creds = Credentials {
            user_id: "u".to_string(),
            access_token: "t".to_string(),
            email: None,
            membership: None,
        };
        let cookie = auth_cookie(&creds);
        assert!(cookie.contains("%3A%3A"));
    }

    #[test]
    fn fmt_cost_cents_small() {
        assert_eq!(fmt_cost_cents(0.5), "$0.0050");
    }

    #[test]
    fn fmt_cost_cents_medium() {
        assert_eq!(fmt_cost_cents(50.0), "$0.500");
    }

    #[test]
    fn fmt_cost_cents_large() {
        assert_eq!(fmt_cost_cents(1234.0), "$12.34");
    }

    #[test]
    fn token_totals_from_event() {
        let ev = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "tokenUsage": {
                "inputTokens": 1000,
                "outputTokens": 500,
                "cacheReadTokens": 200,
                "cacheWriteTokens": 50,
                "totalCents": 3.5
            }
        });
        let t = TokenTotals::from_event(&ev);
        assert_eq!(t.count, 1);
        assert_eq!(t.input, 1000);
        assert_eq!(t.output, 500);
        assert_eq!(t.cache_read, 200);
        assert_eq!(t.cache_write, 50);
        assert!((t.cost_cents - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn token_totals_from_event_missing_fields() {
        let ev = serde_json::json!({ "model": "unknown" });
        let t = TokenTotals::from_event(&ev);
        assert_eq!(t.input, 0);
        assert_eq!(t.output, 0);
    }

    #[test]
    fn token_totals_merge() {
        let mut a = TokenTotals {
            count: 2,
            input: 100,
            output: 50,
            cache_read: 10,
            cache_write: 5,
            cost_cents: 1.0,
        };
        let b = TokenTotals {
            count: 1,
            input: 200,
            output: 100,
            cache_read: 20,
            cache_write: 10,
            cost_cents: 2.0,
        };
        a.merge(&b);
        assert_eq!(a.count, 3);
        assert_eq!(a.input, 300);
        assert_eq!(a.output, 150);
        assert_eq!(a.cache_read, 30);
        assert_eq!(a.cache_write, 15);
        assert!((a.cost_cents - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cached_token_event_from_api() {
        let ev = serde_json::json!({
            "timestamp": "1708300000000",
            "model": "claude-sonnet-4-20250514",
            "tokenUsage": {
                "inputTokens": 800,
                "outputTokens": 400,
                "cacheReadTokens": 100,
                "cacheWriteTokens": 25,
                "totalCents": 2.0
            }
        });
        let cached = CachedTokenEvent::from_api(&ev).unwrap();
        assert_eq!(cached.timestamp_ms, 1708300000000);
        assert_eq!(cached.input_tokens, 800);
        assert_eq!(cached.output_tokens, 400);
        assert_eq!(cached.cache_read_tokens, 100);
        assert_eq!(cached.cache_write_tokens, 25);
    }

    #[test]
    fn cached_token_event_from_api_missing_timestamp() {
        let ev = serde_json::json!({ "model": "test" });
        assert!(CachedTokenEvent::from_api(&ev).is_none());
    }

    #[test]
    fn cached_token_event_from_api_invalid_timestamp() {
        let ev = serde_json::json!({ "timestamp": "not-a-number" });
        assert!(CachedTokenEvent::from_api(&ev).is_none());
    }

    #[test]
    fn aggregate_cached_tokens_empty() {
        assert!(aggregate_cached_tokens(&[]).is_none());
    }

    #[test]
    fn aggregate_cached_tokens_sums_correctly() {
        let events = vec![
            CachedTokenEvent {
                timestamp_ms: 1000,
                model: "sonnet".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
                cache_write_tokens: 5,
                cost_cents: 1.0,
            },
            CachedTokenEvent {
                timestamp_ms: 2000,
                model: "sonnet".to_string(),
                input_tokens: 200,
                output_tokens: 100,
                cache_read_tokens: 20,
                cache_write_tokens: 10,
                cost_cents: 2.0,
            },
            CachedTokenEvent {
                timestamp_ms: 3000,
                model: "opus".to_string(),
                input_tokens: 50,
                output_tokens: 25,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_cents: 3.0,
            },
        ];
        let agg = aggregate_cached_tokens(&events).unwrap();
        assert_eq!(agg.input_tokens, 350);
        assert_eq!(agg.output_tokens, 175);
        assert_eq!(agg.cache_read_tokens, 30);
        assert_eq!(agg.request_count, 3);
        assert!((agg.cost_usd - 0.06).abs() < 0.001);
        assert_eq!(agg.model, "sonnet");
    }

    #[test]
    fn cached_token_event_round_trips_through_json() {
        let event = CachedTokenEvent {
            timestamp_ms: 1708300000000,
            model: "claude-sonnet-4-20250514".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 200,
            cache_write_tokens: 50,
            cost_cents: 3.5,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: CachedTokenEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.timestamp_ms, event.timestamp_ms);
        assert_eq!(parsed.model, event.model);
        assert_eq!(parsed.input_tokens, event.input_tokens);
        assert_eq!(parsed.output_tokens, event.output_tokens);
    }
}
