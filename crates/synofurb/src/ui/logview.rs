//! Log buffer behind the install log viewer: bounded storage, a "minimum level" filter with live counts, an
//! optional text search, and plain-text export. Pure (no GPUI), unit-tested.

use synofurb_core::logs::{Level, LogLine};

/// Lines kept in memory; when exceeded the oldest [`DROP_CHUNK`] are dropped (the full log is on disk anyway).
pub const MAX_LINES: usize = 50_000;
pub const DROP_CHUNK: usize = 10_000;

/// Glyph + three-letter word for a level: status is never colour alone.
pub fn level_tag(level: Level) -> (&'static str, &'static str) {
    match level {
        Level::Debug => ("\u{00b7}", "DBG"),
        Level::Info => ("\u{25cf}", "INF"),
        Level::Warn => ("\u{25b2}", "WRN"),
        Level::Error => ("\u{2716}", "ERR"),
    }
}

pub const LEVELS: [Level; 4] = [Level::Debug, Level::Info, Level::Warn, Level::Error];

pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::Debug => "Debug",
        Level::Info => "Info",
        Level::Warn => "Warn",
        Level::Error => "Error",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFilter {
    /// Show this level and everything more severe.
    pub min_level: Level,
    /// Case-insensitive substring; empty = no search.
    pub text: String,
}

impl Default for LogFilter {
    fn default() -> Self {
        LogFilter { min_level: Level::Debug, text: String::new() }
    }
}

impl LogFilter {
    pub fn matches(&self, l: &LogLine) -> bool {
        l.level >= self.min_level && (self.text.is_empty() || l.msg.to_lowercase().contains(&self.text.to_lowercase()))
    }
}

#[derive(Default)]
pub struct LogBuffer {
    lines: Vec<LogLine>,
    filter: LogFilter,
    /// Indices into `lines` that pass `filter`, kept in step incrementally.
    visible: Vec<usize>,
}

impl LogBuffer {
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn push(&mut self, l: LogLine) {
        if self.lines.len() >= MAX_LINES {
            self.lines.drain(..DROP_CHUNK);
            self.rebuild();
        }
        if self.filter.matches(&l) {
            self.visible.push(self.lines.len());
        }
        self.lines.push(l);
    }

    fn rebuild(&mut self) {
        self.visible = self.lines.iter().enumerate().filter(|(_, l)| self.filter.matches(l)).map(|(i, _)| i).collect();
    }

    pub fn filter(&self) -> &LogFilter {
        &self.filter
    }

    pub fn set_min_level(&mut self, level: Level) {
        if self.filter.min_level != level {
            self.filter.min_level = level;
            self.rebuild();
        }
    }

    pub fn set_text(&mut self, text: &str) {
        if self.filter.text != text {
            self.filter.text = text.to_string();
            self.rebuild();
        }
    }

    /// Number of lines currently shown.
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// The `i`-th visible line.
    pub fn visible_line(&self, i: usize) -> Option<&LogLine> {
        self.visible.get(i).and_then(|&ix| self.lines.get(ix))
    }

    /// Lines at `level` or above, ignoring the text search (the number shown on the segmented control).
    pub fn count_at_least(&self, level: Level) -> usize {
        self.lines.iter().filter(|l| l.level >= level).count()
    }

    #[cfg(test)]
    pub fn count_exact(&self, level: Level) -> usize {
        self.lines.iter().filter(|l| l.level == level).count()
    }

    /// Index (in the visible list) of the first warning or error, to jump to it.
    pub fn first_problem(&self) -> Option<usize> {
        (0..self.visible.len()).find(|&i| self.visible_line(i).is_some_and(|l| l.level >= Level::Warn))
    }

    /// Plain text of the visible lines (what "Copy" puts on the clipboard).
    pub fn visible_text(&self) -> String {
        let mut s = String::new();
        for i in 0..self.visible.len() {
            if let Some(l) = self.visible_line(i) {
                s.push_str(&format_line(l));
                s.push('\n');
            }
        }
        s
    }

    /// The last `n` lines at Info or above (failure screen summary).
    pub fn tail_info(&self, n: usize) -> Vec<&LogLine> {
        let mut v: Vec<&LogLine> = self.lines.iter().rev().filter(|l| l.level >= Level::Info).take(n).collect();
        v.reverse();
        v
    }
}

/// `12:00:01 INF [inst] message`, for copy/export.
pub fn format_line(l: &LogLine) -> String {
    format!("{} {} [{}] {}", l.ts, level_tag(l.level).1, l.source.tag(), l.msg)
}

/// U-Boot lines the installer printed (ARM boards): `uboot-env| setenv ...` -> `setenv ...`.
pub fn uboot_command(msg: &str) -> Option<String> {
    msg.split_once("uboot-env| ").map(|(_, c)| c.trim_end().to_string()).filter(|c| !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::logs::Source;

    fn line(level: Level, msg: &str) -> LogLine {
        LogLine { ts: "12:00:00".into(), level, source: Source::Installer, msg: msg.into() }
    }

    fn sample() -> LogBuffer {
        let mut b = LogBuffer::default();
        b.push(line(Level::Debug, "CMD mdadm --create"));
        b.push(line(Level::Info, "creating RAID arrays"));
        b.push(line(Level::Warn, "slow disk sdc"));
        b.push(line(Level::Error, "cannot write GRUB to sdc"));
        b.push(line(Level::Info, "retrying"));
        b
    }

    #[test]
    fn min_level_filter_is_cumulative_and_counts_are_live() {
        let mut b = sample();
        assert_eq!(b.visible_len(), 5);
        b.set_min_level(Level::Info);
        assert_eq!(b.visible_len(), 4);
        b.set_min_level(Level::Warn);
        assert_eq!(b.visible_len(), 2);
        b.set_min_level(Level::Error);
        assert_eq!(b.visible_len(), 1);
        assert_eq!(b.count_at_least(Level::Debug), 5);
        assert_eq!(b.count_at_least(Level::Warn), 2);
        assert_eq!(b.count_exact(Level::Info), 2);
        b.push(line(Level::Error, "another"));
        assert_eq!(b.visible_len(), 2, "new lines honour the active filter");
        b.push(line(Level::Info, "ignored while filtering errors"));
        assert_eq!(b.visible_len(), 2);
    }

    #[test]
    fn text_search_is_case_insensitive_and_combines_with_level() {
        let mut b = sample();
        b.set_text("SDC");
        assert_eq!(b.visible_len(), 2);
        b.set_min_level(Level::Error);
        assert_eq!(b.visible_len(), 1);
        assert_eq!(b.visible_line(0).unwrap().msg, "cannot write GRUB to sdc");
        b.set_text("");
        b.set_min_level(Level::Debug);
        assert_eq!(b.visible_len(), 5);
    }

    #[test]
    fn first_problem_and_export() {
        let mut b = sample();
        assert_eq!(b.first_problem(), Some(2));
        b.set_min_level(Level::Info);
        assert_eq!(b.first_problem(), Some(1));
        let t = b.visible_text();
        assert!(t.contains("12:00:00 WRN [inst] slow disk sdc\n"));
        assert!(!t.contains("DBG"));
        assert_eq!(b.tail_info(2).len(), 2);
        assert_eq!(b.tail_info(2)[1].msg, "retrying");
    }

    #[test]
    fn buffer_is_bounded_and_keeps_filter_consistent() {
        let mut b = LogBuffer::default();
        b.set_min_level(Level::Warn);
        for i in 0..MAX_LINES + 5 {
            b.push(line(if i % 100 == 0 { Level::Warn } else { Level::Info }, &format!("n{i}")));
        }
        assert!(b.len() <= MAX_LINES, "drops the oldest chunk");
        assert!(b.len() > MAX_LINES - DROP_CHUNK);
        assert!(b.visible_len() > 0);
        for i in 0..b.visible_len() {
            assert!(b.visible_line(i).unwrap().level >= Level::Warn);
        }
    }

    #[test]
    fn tags_are_glyph_plus_word() {
        assert_eq!(level_tag(Level::Warn), ("\u{25b2}", "WRN"));
        assert_eq!(level_tag(Level::Error).1, "ERR");
    }

    #[test]
    fn uboot_lines() {
        assert_eq!(uboot_command("12:00 uboot-env| setenv bootcmd 'x'  ").as_deref(), Some("setenv bootcmd 'x'"));
        assert_eq!(uboot_command("nothing"), None);
        assert_eq!(uboot_command("uboot-env| "), None);
    }
}
