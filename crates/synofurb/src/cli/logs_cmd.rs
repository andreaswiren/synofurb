//! `logs list|show|path`: the log files written by earlier runs (`<data dir>/logs`).

use super::args::LogsCmd;
use super::exit::{CliError, CliResult};
use super::facts::{SCHEMA, config_dir};
use super::out::Out;
use anyhow::Context;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn logs_dir() -> PathBuf {
    config_dir().join("logs")
}

#[derive(Debug, Clone)]
pub struct LogFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
}

/// Files in `dir`, newest first. `all`: also the generated stage-2 scripts and plans; otherwise only `*.log`.
pub fn list_files(dir: &Path, all: bool) -> Vec<LogFile> {
    let mut v: Vec<LogFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().to_string();
            if !md.is_file() || (!all && !name.ends_with(".log")) {
                return None;
            }
            Some(LogFile { path: p, bytes: md.len(), modified: md.modified().unwrap_or(SystemTime::UNIX_EPOCH) })
        })
        .collect();
    // names carry a timestamp, so sorting by name breaks ties between files written within the same second
    v.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.path.cmp(&a.path)));
    v
}

pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn stamp(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn logs(out: &Out, action: LogsCmd) -> CliResult<()> {
    let dir = logs_dir();
    match action {
        LogsCmd::Path { latest } => {
            if latest {
                let f = list_files(&dir, false).into_iter().next().ok_or_else(|| anyhow::anyhow!("no log files in {}", dir.display()))?;
                out.data(&f.path.display().to_string());
            } else {
                out.data(&dir.display().to_string());
            }
            Ok(())
        }
        LogsCmd::List { json, all } => {
            let files = list_files(&dir, all);
            if json {
                let rows: Vec<_> = files
                    .iter()
                    .enumerate()
                    .map(
                        |(i, f)| json!({"n": i + 1, "path": f.path.display().to_string(), "bytes": f.bytes, "modified": stamp(f.modified)}),
                    )
                    .collect();
                out.json_doc(&json!({"schema": SCHEMA, "dir": dir.display().to_string(), "files": rows}));
            } else if files.is_empty() {
                out.data(&format!("no log files in {}", dir.display()));
            } else {
                out.data(&format!("{} file(s) in {}", files.len(), dir.display()));
                for (i, f) in files.iter().enumerate() {
                    out.data(&format!(
                        "{:3}  {}  {:>9}  {}",
                        i + 1,
                        stamp(f.modified),
                        f.bytes,
                        f.path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
            Ok(())
        }
        LogsCmd::Show { n, tail } => {
            let files = list_files(&dir, false);
            if n == 0 || n > files.len() {
                return Err(CliError::usage(if files.is_empty() {
                    format!("no log files in {}", dir.display())
                } else {
                    format!("there are {} log file(s); choose 1..={}", files.len(), files.len())
                }));
            }
            let f = &files[n - 1];
            let text = std::fs::read_to_string(&f.path).with_context(|| format!("reading {}", f.path.display()))?;
            out.info(&format!("--- {} ---", f.path.display()));
            match tail {
                Some(k) => out.data(&last_lines(&text, k)),
                None => {
                    for l in text.lines() {
                        out.data(l);
                    }
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_newest_first_and_filters_by_extension() {
        let dir = std::env::temp_dir().join(format!("sf-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("synofurb-20260101-000001.log"), "a\nb\nc\n").unwrap();
        std::fs::write(dir.join("synofurb-20260101-000002.log"), "x\n").unwrap();
        std::fs::write(dir.join("stage2-abcd.sh"), "#!/bin/sh\n").unwrap();
        let only_logs = list_files(&dir, false);
        assert_eq!(only_logs.len(), 2);
        assert!(only_logs[0].path.to_string_lossy().ends_with("000002.log"), "newest first: {only_logs:?}");
        assert_eq!(list_files(&dir, true).len(), 3);
        assert!(list_files(&dir.join("missing"), true).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail() {
        assert_eq!(last_lines("a\nb\nc\nd\n", 2), "c\nd");
        assert_eq!(last_lines("a\nb\n", 10), "a\nb");
    }
}
