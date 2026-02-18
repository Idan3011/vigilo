use std::fs;
use std::io::{BufRead, BufReader};

mod common {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Serialize, Deserialize, Clone)]
    pub struct TestEvent {
        pub id: Uuid,
        pub timestamp: String,
        pub session_id: Uuid,
        pub server: String,
        pub tool: String,
        pub arguments: serde_json::Value,
        pub outcome: TestOutcome,
        pub duration_us: u64,
        pub risk: String,
        #[serde(default)]
        pub project: TestProject,
    }

    #[derive(Serialize, Deserialize, Clone)]
    #[serde(tag = "status", rename_all = "snake_case")]
    pub enum TestOutcome {
        Ok { result: serde_json::Value },
        Err { code: i32, message: String },
    }

    #[derive(Serialize, Deserialize, Clone, Default)]
    pub struct TestProject {
        pub root: Option<String>,
        pub name: Option<String>,
        pub branch: Option<String>,
        pub commit: Option<String>,
        #[serde(default)]
        pub dirty: bool,
    }

    pub fn make_event(session_id: Uuid, tool: &str, risk: &str) -> TestEvent {
        TestEvent {
            id: Uuid::new_v4(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            session_id,
            server: "vigilo".to_string(),
            tool: tool.to_string(),
            arguments: serde_json::json!({ "path": "/tmp/test" }),
            outcome: TestOutcome::Ok {
                result: serde_json::json!("ok"),
            },
            duration_us: 1234,
            risk: risk.to_string(),
            project: TestProject {
                root: Some("/projects/test".to_string()),
                name: Some("test".to_string()),
                branch: Some("main".to_string()),
                commit: Some("abc1234".to_string()),
                dirty: false,
            },
        }
    }
}

fn append_event(event: &common::TestEvent, path: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    if let Some(parent) = std::path::Path::new(path).parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut line = serde_json::to_string(event).unwrap();
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(line.as_bytes()).unwrap();
}

fn count_events(path: &str) -> usize {
    let file = fs::File::open(path).unwrap();
    BufReader::new(file)
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.trim().is_empty())
        .count()
}

fn read_events(path: &str) -> Vec<common::TestEvent> {
    let file = fs::File::open(path).unwrap();
    BufReader::new(file)
        .lines()
        .filter_map(|l| l.ok())
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(&l).ok())
        .collect()
}

#[test]
fn ledger_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("events.jsonl");
    let path_str = ledger_path.to_str().unwrap();

    let session_id = uuid::Uuid::new_v4();
    let e1 = common::make_event(session_id, "read_file", "read");
    let e2 = common::make_event(session_id, "write_file", "write");
    let e3 = common::make_event(session_id, "run_command", "exec");

    append_event(&e1, path_str);
    append_event(&e2, path_str);
    append_event(&e3, path_str);

    assert_eq!(count_events(path_str), 3);

    let events = read_events(path_str);
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].tool, "read_file");
    assert_eq!(events[0].risk, "read");
    assert_eq!(events[1].tool, "write_file");
    assert_eq!(events[1].risk, "write");
    assert_eq!(events[2].tool, "run_command");
    assert_eq!(events[2].risk, "exec");

    assert_eq!(events[0].session_id, session_id);
    assert_eq!(events[1].session_id, session_id);
    assert_eq!(events[2].session_id, session_id);

    assert_eq!(events[0].project.name.as_deref(), Some("test"));
}

#[test]
fn ledger_multiple_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("events.jsonl");
    let path_str = ledger_path.to_str().unwrap();

    let s1 = uuid::Uuid::new_v4();
    let s2 = uuid::Uuid::new_v4();

    append_event(&common::make_event(s1, "read_file", "read"), path_str);
    append_event(&common::make_event(s1, "write_file", "write"), path_str);
    append_event(&common::make_event(s2, "run_command", "exec"), path_str);

    let events = read_events(path_str);
    let session_ids: std::collections::HashSet<uuid::Uuid> =
        events.iter().map(|e| e.session_id).collect();
    assert_eq!(session_ids.len(), 2);
}

#[test]
fn ledger_error_outcome_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("events.jsonl");
    let path_str = ledger_path.to_str().unwrap();

    let session_id = uuid::Uuid::new_v4();
    let mut event = common::make_event(session_id, "delete_file", "write");
    event.outcome = common::TestOutcome::Err {
        code: -1,
        message: "file not found".to_string(),
    };

    append_event(&event, path_str);

    let events = read_events(path_str);
    assert_eq!(events.len(), 1);
    match &events[0].outcome {
        common::TestOutcome::Err { code, message } => {
            assert_eq!(*code, -1);
            assert_eq!(message, "file not found");
        }
        common::TestOutcome::Ok { .. } => panic!("expected error outcome"),
    }
}

#[test]
fn ledger_large_file_contains_valid_events() {
    let dir = tempfile::tempdir().unwrap();
    let ledger_path = dir.path().join("events.jsonl");
    let path_str = ledger_path.to_str().unwrap();

    let big_content = "x".repeat(4096);
    let session_id = uuid::Uuid::new_v4();

    let count = 500;
    for _ in 0..count {
        let mut event = common::make_event(session_id, "read_file", "read");
        event.arguments = serde_json::json!({ "path": big_content });
        append_event(&event, path_str);
    }

    let events = read_events(path_str);
    assert_eq!(events.len(), count);
    assert!(events.iter().all(|e| e.tool == "read_file"));
}
