//! Terminal output policy: colour, verbosity, NDJSON events, and the install progress display.
//!
//! Rules (documented in docs/CLI.md):
//! * stdout carries DATA only: tables you asked for, `--json` documents, the final result, and with `--log-format json` the
//!   NDJSON event stream. stderr carries everything meant for a human: banners, warnings, prompts, progress.
//! * Colour only on a terminal, never with `NO_COLOR` set, `--no-color` or `TERM=dumb`.
//! * The progress bar is drawn only when stderr is a terminal; otherwise one plain line per step.

use super::args::{Global, LogFormat};
use serde_json::{Value, json};
use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use synofurb_core::logs::{Event, Level, LogLine, Phase, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Red,
    Yellow,
    Green,
    Bold,
    Dim,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Style::Red => "1;31",
            Style::Yellow => "33",
            Style::Green => "32",
            Style::Bold => "1",
            Style::Dim => "2",
        }
    }
}

/// Should output to a stream get colour? (pure, so it can be tested)
pub fn color_allowed(no_color_flag: bool, no_color_env: Option<&str>, term: Option<&str>, is_tty: bool) -> bool {
    !no_color_flag && no_color_env.is_none_or(str::is_empty) && term != Some("dumb") && is_tty
}

pub struct Out {
    pub format: LogFormat,
    pub quiet: bool,
    pub verbose: u8,
    pub color_err: bool,
    pub color_out: bool,
    pub tty_err: bool,
    pub tty_out: bool,
    pub dumb: bool,
    /// May we ask the user questions? (stdin and stderr are terminals and `--non-interactive`/`CI` are not in effect)
    pub interactive: bool,
    pub yes: bool,
    lock: Mutex<()>,
}

impl Out {
    pub fn from_global(g: &Global) -> Out {
        let term = std::env::var("TERM").ok();
        let no_color_env = std::env::var("NO_COLOR").ok();
        let tty_err = std::io::stderr().is_terminal();
        let tty_out = std::io::stdout().is_terminal();
        let ci = std::env::var("CI").is_ok_and(|v| !v.is_empty() && v != "0" && v != "false");
        let format = g.log_format.unwrap_or(LogFormat::Text);
        Out {
            format,
            quiet: g.quiet,
            verbose: g.verbose,
            color_err: color_allowed(g.no_color, no_color_env.as_deref(), term.as_deref(), tty_err),
            color_out: color_allowed(g.no_color, no_color_env.as_deref(), term.as_deref(), tty_out),
            tty_err,
            tty_out,
            dumb: term.as_deref() == Some("dumb"),
            interactive: std::io::stdin().is_terminal() && tty_err && !g.non_interactive && !ci,
            yes: g.yes,
            lock: Mutex::new(()),
        }
    }

    /// Plain, non-interactive output (tests, library use).
    pub fn plain() -> Out {
        Out {
            format: LogFormat::Text,
            quiet: false,
            verbose: 0,
            color_err: false,
            color_out: false,
            tty_err: false,
            tty_out: false,
            dumb: true,
            interactive: false,
            yes: false,
            lock: Mutex::new(()),
        }
    }

    pub fn json_events(&self) -> bool {
        self.format == LogFormat::Json
    }

    pub fn paint_err(&self, s: Style, text: &str) -> String {
        paint(self.color_err, s, text)
    }
    pub fn paint_out(&self, s: Style, text: &str) -> String {
        paint(self.color_out, s, text)
    }

    // ------------------------------------------------------------------------------------------ stdout (data)

    /// One line of data on stdout. A closed pipe (`| head`) ends the program quietly instead of panicking.
    pub fn data(&self, line: &str) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut so = std::io::stdout().lock();
        if writeln!(so, "{line}").is_err() {
            std::process::exit(0);
        }
        let _ = so.flush();
    }

    /// Several lines of data.
    pub fn data_lines<I: IntoIterator<Item = String>>(&self, lines: I) {
        for l in lines {
            self.data(&l);
        }
    }

    /// Pretty JSON document on stdout.
    pub fn json_doc(&self, v: &Value) {
        self.data(&serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".into()));
    }

    /// One NDJSON event on stdout (only when `--log-format json`).
    pub fn event(&self, mut v: Value) {
        if !self.json_events() {
            return;
        }
        if let Value::Object(m) = &mut v {
            m.entry("ts").or_insert_with(|| Value::String(chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)));
        }
        self.data(&serde_json::to_string(&v).unwrap_or_default());
    }

    // ------------------------------------------------------------------------------------------ stderr (humans)

    /// Always printed (prompts, essential results for humans).
    pub fn say(&self, line: &str) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(std::io::stderr().lock(), "{line}");
    }

    /// Informational line, hidden by `--quiet`.
    pub fn info(&self, line: &str) {
        if !self.quiet {
            self.say(line);
        }
    }

    pub fn detail(&self, line: &str) {
        if !self.quiet && self.verbose >= 1 {
            self.say(line);
        }
    }

    pub fn warn(&self, line: &str) {
        self.say(&format!("{} {line}", self.paint_err(Style::Yellow, "warning:")));
    }

    pub fn error(&self, line: &str) {
        self.say(&format!("{} {line}", self.paint_err(Style::Red, "error:")));
    }

    /// A command's human-readable result: stdout in text mode (it IS the data), stderr when stdout carries NDJSON.
    pub fn result_text(&self, lines: &[String]) {
        for l in lines {
            if self.json_events() {
                self.info(l);
            } else {
                self.data(l);
            }
        }
    }

    /// Final machine-readable result (NDJSON mode only; text mode prints through `result_text`).
    pub fn result_event(&self, command: &str, ok: bool, data: Value) {
        self.event(json!({"event": "result", "command": command, "ok": ok, "data": data}));
    }
}

/// Write to stderr without ever panicking (a vanished terminal, e.g. a dropped SSH session, must not kill a running install).
pub fn err_raw(text: &str) {
    let mut e = std::io::stderr().lock();
    let _ = e.write_all(text.as_bytes());
    let _ = e.flush();
}

pub fn paint(enabled: bool, s: Style, text: &str) -> String {
    if enabled { format!("\x1b[{}m{text}\x1b[0m", s.code()) } else { text.to_string() }
}

// ---------------------------------------------------------------------------------------------- event mapping

pub fn phase_name(p: &Phase) -> &'static str {
    match p {
        Phase::Connecting => "connecting",
        Phase::Inspecting => "inspecting",
        Phase::Staging => "staging",
        Phase::Booting => "booting",
        Phase::Installing => "installing",
        Phase::WaitingForReboot => "waiting_for_reboot",
        Phase::Verifying => "verifying",
        Phase::Finished => "finished",
        Phase::Failed => "failed",
    }
}

pub fn phase_label(p: &Phase) -> &'static str {
    match p {
        Phase::Connecting => "Connecting to the NAS",
        Phase::Inspecting => "Re-checking the disks",
        Phase::Staging => "Preparing and uploading the installer",
        Phase::Booting => "Booting the installer",
        Phase::Installing => "Installing Debian",
        Phase::WaitingForReboot => "Waiting for the NAS to reboot",
        Phase::Verifying => "Verifying the new system",
        Phase::Finished => "Finished",
        Phase::Failed => "Failed",
    }
}

/// Position in the seven-phase pipeline (same numbering as the GUI); 0 for `Failed`.
pub fn phase_index(p: &Phase) -> u32 {
    match p {
        Phase::Connecting => 1,
        Phase::Inspecting => 2,
        Phase::Staging => 3,
        Phase::Booting => 4,
        Phase::Installing => 5,
        Phase::WaitingForReboot => 6,
        Phase::Verifying | Phase::Finished => 7,
        Phase::Failed => 0,
    }
}

pub const PHASE_COUNT: u32 = 7;

pub fn level_name(l: Level) -> &'static str {
    match l {
        Level::Debug => "debug",
        Level::Info => "info",
        Level::Warn => "warn",
        Level::Error => "error",
    }
}

pub fn source_name(s: Source) -> &'static str {
    match s {
        Source::App => "app",
        Source::Remote => "remote",
        Source::Installer => "installer",
        Source::Syslog => "syslog",
    }
}

/// The NDJSON form of a core event. `None` for events with no machine representation.
pub fn event_json(e: &Event) -> Value {
    match e {
        Event::Log(l) => json!({"event": "log", "time": l.ts, "level": level_name(l.level), "source": source_name(l.source), "msg": l.msg}),
        Event::Phase(p) => {
            json!({"event": "phase", "phase": phase_name(p), "label": phase_label(p), "index": phase_index(p), "of": PHASE_COUNT})
        }
        Event::Step { n, total, name } => json!({"event": "step", "n": n, "total": total, "name": name}),
        Event::Progress { what, fraction } => {
            json!({"event": "progress", "what": what, "fraction": (*fraction as f64 * 1000.0).round() / 1000.0})
        }
        Event::PointOfNoReturn => json!({"event": "point_of_no_return"}),
        Event::InstallerFailed(m) => json!({"event": "installer_failed", "message": m}),
        Event::InstallerDone => json!({"event": "done"}),
    }
}

/// Which log lines does the verbosity level show on a terminal / in the NDJSON stream?
///  default: app info and above, installer/syslog warnings and above, ssh nothing below error
///  -v:      installer info, ssh warnings        -vv: everything including debug (every remote command)
pub fn log_visible(l: &LogLine, verbose: u8) -> bool {
    if verbose >= 2 {
        return true;
    }
    match l.source {
        Source::App => l.level >= Level::Info || verbose >= 1,
        Source::Installer | Source::Syslog => {
            if verbose >= 1 {
                l.level >= Level::Info
            } else {
                l.level >= Level::Warn
            }
        }
        Source::Remote => {
            if verbose >= 1 {
                l.level >= Level::Warn
            } else {
                l.level >= Level::Error
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- progress display

/// Renders the event stream of a running job. One instance per job; not shared between threads.
pub struct Reporter<'a> {
    out: &'a Out,
    step: Option<(u32, u32, String)>,
    phase: Option<Phase>,
    progress: Option<(String, f32)>,
    last_bucket: Option<(String, i32)>,
    bar_visible: bool,
    pub past_ponr: bool,
    pub installer_failed: Option<String>,
    pub installer_done: bool,
}

impl<'a> Reporter<'a> {
    pub fn new(out: &'a Out) -> Reporter<'a> {
        Reporter {
            out,
            step: None,
            phase: None,
            progress: None,
            last_bucket: None,
            bar_visible: false,
            past_ponr: false,
            installer_failed: None,
            installer_done: false,
        }
    }

    fn live_bar(&self) -> bool {
        self.out.tty_err && !self.out.dumb && !self.out.quiet
    }

    fn clear_bar(&mut self) {
        if self.bar_visible {
            err_raw("\r\x1b[K");
            self.bar_visible = false;
        }
    }

    fn draw_bar(&mut self) {
        if !self.live_bar() {
            return;
        }
        let (frac, label) = match (&self.step, &self.progress) {
            (Some((n, total, name)), _) if *total > 0 => (((*n as f32 - 0.5).max(0.0)) / *total as f32, format!("step {n}/{total} {name}")),
            (_, Some((what, f))) => (*f, what.clone()),
            _ => (0.0, self.phase.as_ref().map(|p| phase_label(p).to_string()).unwrap_or_default()),
        };
        let width = 24usize;
        let filled = ((frac.clamp(0.0, 1.0)) * width as f32).round() as usize;
        let mut line = format!("[{}{}] {label}", "#".repeat(filled), "-".repeat(width - filled));
        if let Some((what, f)) = &self.progress
            && self.step.is_some()
        {
            line.push_str(&format!("  ({what} {:.0}%)", f * 100.0));
        }
        let cols = 100;
        if line.chars().count() > cols {
            line = line.chars().take(cols).collect();
        }
        err_raw(&format!("\r\x1b[K{line}"));
        self.bar_visible = true;
    }

    /// Feed one core event.
    pub fn handle(&mut self, e: &Event) {
        // machine stream first: every non-log event, log lines from Info up (Debug too with -v)
        if self.out.json_events() {
            let emit = match e {
                Event::Log(l) => l.level >= Level::Info || self.out.verbose >= 1,
                _ => true,
            };
            if emit {
                self.out.event(event_json(e));
            }
        }
        match e {
            Event::Log(l) => {
                let show = if self.out.quiet { l.level >= Level::Error } else { log_visible(l, self.out.verbose) };
                // with NDJSON on stdout, stderr only repeats warnings and errors (everything with -v)
                let show = show && (!self.out.json_events() || l.level >= Level::Warn || self.out.verbose >= 1);
                if show {
                    self.clear_bar();
                    let lvl = format!("{:5}", level_name(l.level).to_uppercase());
                    let lvl = match l.level {
                        Level::Error => self.out.paint_err(Style::Red, &lvl),
                        Level::Warn => self.out.paint_err(Style::Yellow, &lvl),
                        Level::Debug => self.out.paint_err(Style::Dim, &lvl),
                        Level::Info => lvl,
                    };
                    self.out.say(&format!("{} {} [{}] {}", l.ts, lvl, l.source.tag(), l.msg));
                    self.draw_bar();
                }
            }
            Event::Phase(p) => {
                self.phase = Some(p.clone());
                if !self.out.quiet {
                    self.clear_bar();
                    let idx = phase_index(p);
                    let head = if idx > 0 { format!("[{idx}/{PHASE_COUNT}] {}", phase_label(p)) } else { phase_label(p).to_string() };
                    let styled =
                        if *p == Phase::Failed { self.out.paint_err(Style::Red, &head) } else { self.out.paint_err(Style::Bold, &head) };
                    self.out.say(&styled);
                    self.draw_bar();
                }
            }
            Event::Step { n, total, name } => {
                let changed = self.step.as_ref().is_none_or(|(pn, _, _)| pn != n);
                self.step = Some((*n, *total, name.clone()));
                if !self.out.quiet && changed {
                    if self.live_bar() {
                        self.clear_bar();
                        self.out.say(&format!("  step {n}/{total}: {name}"));
                        self.draw_bar();
                    } else {
                        self.out.say(&format!("  step {n}/{total}: {name}"));
                    }
                } else if self.live_bar() {
                    self.draw_bar();
                }
            }
            Event::Progress { what, fraction } => {
                self.progress = Some((what.clone(), *fraction));
                if self.live_bar() {
                    self.draw_bar();
                } else if !self.out.quiet && (!self.out.json_events() || self.out.verbose >= 1) {
                    let bucket = (fraction * 10.0).floor() as i32;
                    let seen = matches!(&self.last_bucket, Some((w, b)) if w == what && *b == bucket);
                    if !seen {
                        self.last_bucket = Some((what.clone(), bucket));
                        self.out.say(&format!("  {what}: {:.0}%", fraction * 100.0));
                    }
                }
            }
            Event::PointOfNoReturn => {
                self.past_ponr = true;
                self.clear_bar();
                self.out.say(
                    &self.out.paint_err(
                        Style::Red,
                        "POINT OF NO RETURN passed: the disks are being changed now. This can no longer be cancelled.",
                    ),
                );
                self.draw_bar();
            }
            Event::InstallerFailed(m) => {
                self.installer_failed = Some(m.clone());
                self.clear_bar();
                self.out.say(&self.out.paint_err(Style::Red, &format!("installer reported a failure: {m}")));
            }
            Event::InstallerDone => {
                self.installer_done = true;
                self.clear_bar();
                if !self.out.quiet {
                    self.out.say(&self.out.paint_err(Style::Green, "the installer finished; waiting for the new system"));
                }
            }
        }
    }

    pub fn finish(&mut self) {
        self.clear_bar();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: Level, source: Source) -> LogLine {
        LogLine { ts: "t".into(), level, source, msg: "m".into() }
    }

    #[test]
    fn colour_policy() {
        assert!(color_allowed(false, None, Some("xterm-256color"), true));
        assert!(!color_allowed(true, None, Some("xterm"), true), "--no-color");
        assert!(!color_allowed(false, Some("1"), Some("xterm"), true), "NO_COLOR set");
        assert!(color_allowed(false, Some(""), Some("xterm"), true), "an empty NO_COLOR does not count (no-color.org)");
        assert!(!color_allowed(false, None, Some("dumb"), true), "TERM=dumb");
        assert!(!color_allowed(false, None, Some("xterm"), false), "not a terminal");
        assert_eq!(paint(false, Style::Red, "x"), "x");
        assert_eq!(paint(true, Style::Red, "x"), "\x1b[1;31mx\x1b[0m");
    }

    #[test]
    fn verbosity_filters() {
        assert!(log_visible(&line(Level::Info, Source::App), 0));
        assert!(!log_visible(&line(Level::Debug, Source::App), 0));
        assert!(!log_visible(&line(Level::Info, Source::Installer), 0));
        assert!(log_visible(&line(Level::Warn, Source::Installer), 0));
        assert!(log_visible(&line(Level::Info, Source::Installer), 1));
        assert!(!log_visible(&line(Level::Debug, Source::Remote), 1));
        assert!(log_visible(&line(Level::Debug, Source::Remote), 2));
        assert!(log_visible(&line(Level::Error, Source::Remote), 0));
    }

    #[test]
    fn events_have_a_stable_json_shape() {
        let v = event_json(&Event::Step { n: 4, total: 12, name: "Partitioning".into() });
        assert_eq!(v["event"], "step");
        assert_eq!((v["n"].as_u64(), v["total"].as_u64()), (Some(4), Some(12)));
        let v = event_json(&Event::Phase(Phase::WaitingForReboot));
        assert_eq!((v["phase"].as_str(), v["index"].as_u64(), v["of"].as_u64()), (Some("waiting_for_reboot"), Some(6), Some(7)));
        let v =
            event_json(&Event::Log(LogLine { ts: "12:00:01.000".into(), level: Level::Warn, source: Source::Installer, msg: "x".into() }));
        assert_eq!(v["level"], "warn");
        assert_eq!(v["source"], "installer");
        assert_eq!(v["time"], "12:00:01.000");
        assert_eq!(event_json(&Event::PointOfNoReturn)["event"], "point_of_no_return");
        assert_eq!(event_json(&Event::InstallerDone)["event"], "done");
        assert_eq!(event_json(&Event::InstallerFailed("boom".into()))["message"], "boom");
        let p = event_json(&Event::Progress { what: "payload".into(), fraction: 0.5 });
        assert_eq!(p["fraction"], 0.5);
    }
}
