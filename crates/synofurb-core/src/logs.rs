//! Debug logging and the event stream the GUI/CLI consume.
//!
//! Everything that happens during an install - app actions, remote commands and their output, and
//! the installer's own log stream - goes through [`Logger`]. It writes a timestamped file (the log
//! you attach to a bug report), redacts secrets, and forwards events to whoever is listening.

use chrono::Local;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The desktop app itself
    App,
    /// A command run on the NAS over SSH (and its output)
    Remote,
    /// The stage-2 installer log stream
    Installer,
    /// Raw syslog from the Debian installer
    Syslog,
}

impl Source {
    pub fn tag(self) -> &'static str {
        match self {
            Source::App => "app",
            Source::Remote => "ssh",
            Source::Installer => "inst",
            Source::Syslog => "syslog",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub ts: String,
    pub level: Level,
    pub source: Source,
    pub msg: String,
}

/// High-level progress of an install job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    Inspecting,
    Staging,
    Booting,
    Installing,
    WaitingForReboot,
    Verifying,
    Finished,
    Failed,
}

#[derive(Debug, Clone)]
pub enum Event {
    Log(LogLine),
    Phase(Phase),
    /// Installer step `n` of `total`.
    Step { n: u32, total: u32, name: String },
    /// Upload/download progress 0.0..=1.0
    Progress { what: String, fraction: f32 },
    PointOfNoReturn,
    InstallerFailed(String),
    InstallerDone,
}

#[derive(Clone)]
pub struct Logger {
    inner: Arc<Inner>,
}

struct Inner {
    file: Mutex<Option<File>>,
    path: Option<PathBuf>,
    tx: Mutex<Option<Sender<Event>>>,
    secrets: Mutex<Vec<String>>,
}

impl Logger {
    /// A logger that writes `dir/synofurb-<timestamp>.log`.
    pub fn to_dir(dir: &Path, tx: Option<Sender<Event>>) -> std::io::Result<Logger> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("synofurb-{}.log", Local::now().format("%Y%m%d-%H%M%S")));
        let file = File::create(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Logger { inner: Arc::new(Inner { file: Mutex::new(Some(file)), path: Some(path), tx: Mutex::new(tx), secrets: Mutex::new(vec![]) }) })
    }

    /// A logger that only forwards events (tests, CLI dry-runs).
    pub fn memory(tx: Option<Sender<Event>>) -> Logger {
        Logger { inner: Arc::new(Inner { file: Mutex::new(None), path: None, tx: Mutex::new(tx), secrets: Mutex::new(vec![]) }) }
    }

    pub fn path(&self) -> Option<&Path> {
        self.inner.path.as_deref()
    }

    pub fn set_sender(&self, tx: Option<Sender<Event>>) {
        *self.inner.tx.lock().unwrap() = tx;
    }

    /// Anything registered here is replaced with `***` in every log line.
    pub fn add_secret(&self, s: &str) {
        if s.len() >= 4 {
            self.inner.secrets.lock().unwrap().push(s.to_string());
        }
    }

    fn redact(&self, msg: &str) -> String {
        let mut out = msg.to_string();
        for s in self.inner.secrets.lock().unwrap().iter() {
            out = out.replace(s.as_str(), "***");
        }
        out
    }

    pub fn emit(&self, e: Event) {
        if let Some(tx) = self.inner.tx.lock().unwrap().as_ref() {
            let _ = tx.send(e);
        }
    }

    pub fn log(&self, level: Level, source: Source, msg: &str) {
        for line in msg.lines() {
            let l = LogLine { ts: Local::now().format("%H:%M:%S%.3f").to_string(), level, source, msg: self.redact(line) };
            if let Some(f) = self.inner.file.lock().unwrap().as_mut() {
                let _ = writeln!(f, "{} {:5} [{}] {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), format!("{level:?}").to_uppercase(), source.tag(), l.msg);
                let _ = f.flush();
            }
            self.emit(Event::Log(l));
        }
    }
    pub fn debug(&self, m: impl AsRef<str>) {
        self.log(Level::Debug, Source::App, m.as_ref())
    }
    pub fn info(&self, m: impl AsRef<str>) {
        self.log(Level::Info, Source::App, m.as_ref())
    }
    pub fn warn(&self, m: impl AsRef<str>) {
        self.log(Level::Warn, Source::App, m.as_ref())
    }
    pub fn error(&self, m: impl AsRef<str>) {
        self.log(Level::Error, Source::App, m.as_ref())
    }
    pub fn phase(&self, p: Phase) {
        self.info(format!("phase: {p:?}"));
        self.emit(Event::Phase(p));
    }

    /// Feed one line from the installer (TCP stream or syslog). Parses `@@` markers.
    pub fn installer_line(&self, source: Source, raw: &str) {
        let line = raw.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return;
        }
        let level = if line.contains("ERROR") || line.contains("@@FAIL") {
            Level::Error
        } else if line.contains("WARN") {
            Level::Warn
        } else if line.contains(" CMD ") || line.contains("CMD?") {
            Level::Debug
        } else {
            Level::Info
        };
        self.log(level, source, line);
        if let Some(i) = line.find("@@") {
            match parse_marker(&line[i..]) {
                Some(Event::InstallerFailed(m)) => {
                    self.emit(Event::InstallerFailed(m));
                }
                Some(e) => self.emit(e),
                None => {}
            }
        }
    }
}

/// Parse `@@STEP 4/12 Partitioning`, `@@FAIL msg`, `@@DONE ...`, `@@POINT_OF_NO_RETURN`.
pub fn parse_marker(s: &str) -> Option<Event> {
    let s = s.strip_prefix("@@")?;
    let (kind, rest) = s.split_once(' ').unwrap_or((s, ""));
    match kind {
        "STEP" => {
            let (frac, name) = rest.split_once(' ').unwrap_or((rest, ""));
            let (n, total) = frac.split_once('/')?;
            Some(Event::Step { n: n.parse().ok()?, total: total.parse().ok()?, name: name.to_string() })
        }
        "FAIL" => Some(Event::InstallerFailed(rest.to_string())),
        "DONE" => Some(Event::InstallerDone),
        "POINT_OF_NO_RETURN" => Some(Event::PointOfNoReturn),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn markers_are_parsed() {
        assert!(matches!(parse_marker("@@STEP 4/12 Partitioning"), Some(Event::Step { n: 4, total: 12, .. })));
        assert!(matches!(parse_marker("@@FAIL boom"), Some(Event::InstallerFailed(m)) if m == "boom"));
        assert!(matches!(parse_marker("@@DONE install completed"), Some(Event::InstallerDone)));
        assert!(matches!(parse_marker("@@POINT_OF_NO_RETURN"), Some(Event::PointOfNoReturn)));
        assert!(parse_marker("@@STEP x").is_none());
        assert!(parse_marker("@@WHAT").is_none());
    }

    #[test]
    fn secrets_are_redacted_everywhere() {
        let (tx, rx) = channel();
        let dir = std::env::temp_dir().join(format!("sf-log-{}", std::process::id()));
        let l = Logger::to_dir(&dir, Some(tx)).unwrap();
        l.add_secret("hunter2hunter2");
        l.info("login with hunter2hunter2 ok");
        let ev = rx.try_recv().unwrap();
        match ev {
            Event::Log(l) => assert_eq!(l.msg, "login with *** ok"),
            _ => panic!(),
        }
        let text = std::fs::read_to_string(l.path().unwrap()).unwrap();
        assert!(!text.contains("hunter2"));
        assert!(text.contains("login with *** ok"));
    }

    #[test]
    fn installer_lines_drive_events() {
        let (tx, rx) = channel();
        let l = Logger::memory(Some(tx));
        l.installer_line(Source::Installer, "12:00:01 #7 @@STEP 5/12 Creating RAID arrays");
        l.installer_line(Source::Installer, "12:00:02 #8 ERROR something broke");
        let evs: Vec<Event> = rx.try_iter().collect();
        assert!(evs.iter().any(|e| matches!(e, Event::Step { n: 5, .. })));
        assert!(evs.iter().any(|e| matches!(e, Event::Log(l) if l.level == Level::Error)));
    }
}
