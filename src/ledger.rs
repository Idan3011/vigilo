use anyhow::{Context, Result};
use fs2::FileExt;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SIZE: u64 = 10 * 1024 * 1024;
const MAX_ROTATED: usize = 5;

pub fn append_event(event: &impl Serialize, ledger_path: &str) -> Result<()> {
    let path = Path::new(ledger_path);

    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).context("creating ledger directory")?;
        }
    }

    let line = {
        let mut s = serde_json::to_string(event).context("serializing event")?;
        s.push('\n');
        s
    };

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("opening ledger file")?;

    file.lock_exclusive().context("locking ledger file")?;

    file.write_all(line.as_bytes())?;
    file.flush()?;

    if let Ok(meta) = file.metadata() {
        if meta.len() > MAX_SIZE {
            // still holding lock — safe to rotate
            drop(file); // releases lock + handle
            if let Err(e) = rotate_and_cleanup(&PathBuf::from(ledger_path), MAX_ROTATED) {
                eprintln!("[vigilo] ledger rotation failed: {e}");
            }
        } else {
            file.unlock().ok();
        }
    } else {
        file.unlock().ok();
    }

    Ok(())
}

fn rotate_and_cleanup(ledger_path: &PathBuf, keep: usize) -> std::io::Result<()> {
    let parent = ledger_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = ledger_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("events");

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();

    let rotated_name = format!("{stem}.{ts}.jsonl");
    fs::rename(ledger_path, parent.join(rotated_name))?;

    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(ledger_path)?;

    let mut rotated: Vec<(PathBuf, SystemTime)> = fs::read_dir(parent)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let matches = name.starts_with(stem)
                && name.ends_with(".jsonl")
                && name != ledger_path.file_name()?.to_str()?;
            if !matches {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), modified))
        })
        .collect();

    rotated.sort_by(|a, b| b.1.cmp(&a.1));

    for (path, _) in rotated.into_iter().skip(keep) {
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("[vigilo] failed to remove rotated ledger {path:?}: {e}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestEvent {
        id: String,
        data: String,
    }

    #[test]
    fn append_event_writes_valid_json_line() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("test.jsonl");
        let event = TestEvent {
            id: "1".into(),
            data: "hello".into(),
        };

        append_event(&event, path.to_str().unwrap()).expect("append should succeed");

        let contents = fs::read_to_string(&path).expect("read file");
        let lines: Vec<&str> = contents.lines().collect();

        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
        assert_eq!(parsed["id"], "1");
        assert_eq!(parsed["data"], "hello");
    }

    #[test]
    fn append_event_returns_error_for_directory_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let event = TestEvent {
            id: "1".into(),
            data: "hello".into(),
        };
        let result = append_event(&event, dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn append_event_triggers_rotation_over_10mb() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("events.jsonl");
        let path_str = path.to_str().unwrap();

        let big_data = "x".repeat(8192);
        let count = (10 * 1024 * 1024) / 8300 + 100;
        for i in 0..count {
            let event = TestEvent {
                id: i.to_string(),
                data: big_data.clone(),
            };
            append_event(&event, path_str).expect("append should succeed");
        }

        let active_size = fs::metadata(&path).expect("active file").len();
        assert!(
            active_size < 1024 * 1024,
            "active ledger should be small after rotation, got {active_size}"
        );

        let rotated: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("events.") && name.ends_with(".jsonl") && name != "events.jsonl"
            })
            .collect();
        assert!(!rotated.is_empty(), "expected at least 1 rotated file");
    }
}
