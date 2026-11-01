//! Questions on a terminal. Everything the wizard and the safety gate ask goes through [`Prompter`], so the same flows can be driven by
//! a script in tests. Prompts are written to stderr (stdout stays clean for data) and answers are read line by line, so a dumb terminal,
//! a serial console or an SSH session without a pty-aware client all work.

use anyhow::{Result, bail};
use std::collections::VecDeque;
use std::io::{BufRead, Write};

pub trait Prompter {
    /// Free text. Empty input returns `default` (or an empty string).
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String>;
    /// Text that must not be echoed (passwords).
    fn secret(&mut self, question: &str) -> Result<String>;
    /// Yes/no. Empty input returns `default`.
    fn confirm(&mut self, question: &str, default: bool) -> Result<bool>;
    /// Numbered choice; returns the index. Empty input returns `default`.
    fn choose(&mut self, question: &str, items: &[String], default: usize) -> Result<usize>;
    /// Informational text between questions.
    fn note(&mut self, text: &str);
}

pub fn parse_yes_no(s: &str, default: bool) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" => Some(default),
        "y" | "yes" | "j" | "ja" | "1" | "true" => Some(true),
        "n" | "no" | "nej" | "0" | "false" => Some(false),
        _ => None,
    }
}

/// Resolve what was typed at a numbered menu: a number, or the start of an item's text.
pub fn parse_choice(s: &str, items: &[String], default: usize) -> Option<usize> {
    let t = s.trim();
    if t.is_empty() {
        return Some(default);
    }
    if let Ok(n) = t.parse::<usize>() {
        return (1..=items.len()).contains(&n).then(|| n - 1);
    }
    let lower = t.to_ascii_lowercase();
    let hits: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.to_ascii_lowercase().starts_with(&lower)).map(|(i, _)| i).collect();
    (hits.len() == 1).then(|| hits[0])
}

/// Real terminal: prompts on stderr, answers from stdin, secrets from the controlling terminal without echo.
pub struct StdPrompter;

impl StdPrompter {
    fn read_line(&self) -> Result<String> {
        let mut s = String::new();
        let n = std::io::stdin().lock().read_line(&mut s)?;
        if n == 0 {
            bail!("input ended (no more answers on stdin)");
        }
        Ok(s.trim_end_matches(['\r', '\n']).to_string())
    }
    fn prompt(&self, text: &str) {
        let mut e = std::io::stderr().lock();
        let _ = write!(e, "{text}");
        let _ = e.flush();
    }
}

impl Prompter for StdPrompter {
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String> {
        match default {
            Some(d) if !d.is_empty() => self.prompt(&format!("{question} [{d}]: ")),
            _ => self.prompt(&format!("{question}: ")),
        }
        let a = self.read_line()?;
        Ok(if a.trim().is_empty() { default.unwrap_or("").to_string() } else { a.trim().to_string() })
    }

    fn secret(&mut self, question: &str) -> Result<String> {
        self.prompt(&format!("{question}: "));
        let v = rpassword::read_password()?;
        self.prompt("\n");
        Ok(v)
    }

    fn confirm(&mut self, question: &str, default: bool) -> Result<bool> {
        loop {
            self.prompt(&format!("{question} [{}]: ", if default { "Y/n" } else { "y/N" }));
            match parse_yes_no(&self.read_line()?, default) {
                Some(v) => return Ok(v),
                None => self.prompt("  please answer y or n\n"),
            }
        }
    }

    fn choose(&mut self, question: &str, items: &[String], default: usize) -> Result<usize> {
        loop {
            self.note(question);
            for (i, it) in items.iter().enumerate() {
                self.note(&format!("  {}) {it}{}", i + 1, if i == default { "   (default)" } else { "" }));
            }
            self.prompt(&format!("choice [{}]: ", default + 1));
            match parse_choice(&self.read_line()?, items, default) {
                Some(i) => return Ok(i),
                None => self.prompt("  not a valid choice\n"),
            }
        }
    }

    fn note(&mut self, text: &str) {
        let _ = writeln!(std::io::stderr().lock(), "{text}");
    }
}

/// Answers from a list; everything printed is kept. For tests.
#[derive(Default)]
pub struct Scripted {
    pub answers: VecDeque<String>,
    pub transcript: Vec<String>,
}

impl Scripted {
    pub fn new<I: IntoIterator<Item = S>, S: Into<String>>(answers: I) -> Scripted {
        Scripted { answers: answers.into_iter().map(Into::into).collect(), transcript: vec![] }
    }
    fn next(&mut self, q: &str) -> Result<String> {
        self.transcript.push(format!("? {q}"));
        match self.answers.pop_front() {
            Some(a) => Ok(a),
            None => bail!("input ended (scripted answers ran out at: {q})"),
        }
    }
    pub fn said(&self, needle: &str) -> bool {
        self.transcript.iter().any(|l| l.contains(needle))
    }
}

impl Prompter for Scripted {
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String> {
        let a = self.next(question)?;
        Ok(if a.trim().is_empty() { default.unwrap_or("").to_string() } else { a.trim().to_string() })
    }
    fn secret(&mut self, question: &str) -> Result<String> {
        self.next(question)
    }
    fn confirm(&mut self, question: &str, default: bool) -> Result<bool> {
        let a = self.next(question)?;
        match parse_yes_no(&a, default) {
            Some(v) => Ok(v),
            None => bail!("scripted answer {a:?} is not yes/no"),
        }
    }
    fn choose(&mut self, question: &str, items: &[String], default: usize) -> Result<usize> {
        let a = self.next(question)?;
        match parse_choice(&a, items, default) {
            Some(v) => Ok(v),
            None => bail!("scripted answer {a:?} is not a valid choice for {items:?}"),
        }
    }
    fn note(&mut self, text: &str) {
        self.transcript.push(text.to_string());
    }
}

/// Answers by matching the question text (first rule whose pattern occurs in it wins; no match = just press Enter). Unlike
/// [`Scripted`] it does not depend on the order or number of questions, so a wizard test only states what differs from the defaults.
#[derive(Default)]
pub struct Responder {
    pub rules: Vec<(String, String)>,
    pub transcript: Vec<String>,
    /// Questions that were asked, in order.
    pub asked: Vec<String>,
}

impl Responder {
    pub fn new<I: IntoIterator<Item = (&'static str, &'static str)>>(rules: I) -> Responder {
        Responder { rules: rules.into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), ..Default::default() }
    }
    fn answer(&mut self, q: &str) -> String {
        self.asked.push(q.to_string());
        self.transcript.push(format!("? {q}"));
        self.rules.iter().find(|(pat, _)| q.contains(pat.as_str())).map(|(_, a)| a.clone()).unwrap_or_default()
    }
    pub fn said(&self, needle: &str) -> bool {
        self.transcript.iter().any(|l| l.contains(needle))
    }
    pub fn asked_about(&self, needle: &str) -> bool {
        self.asked.iter().any(|l| l.contains(needle))
    }
}

impl Prompter for Responder {
    fn ask(&mut self, question: &str, default: Option<&str>) -> Result<String> {
        let a = self.answer(question);
        Ok(if a.trim().is_empty() { default.unwrap_or("").to_string() } else { a.trim().to_string() })
    }
    fn secret(&mut self, question: &str) -> Result<String> {
        Ok(self.answer(question))
    }
    fn confirm(&mut self, question: &str, default: bool) -> Result<bool> {
        let a = self.answer(question);
        parse_yes_no(&a, default).ok_or_else(|| anyhow::anyhow!("responder answer {a:?} is not yes/no for: {question}"))
    }
    fn choose(&mut self, question: &str, items: &[String], default: usize) -> Result<usize> {
        let a = self.answer(question);
        parse_choice(&a, items, default)
            .ok_or_else(|| anyhow::anyhow!("responder answer {a:?} is not a valid choice for: {question} {items:?}"))
    }
    fn note(&mut self, text: &str) {
        self.transcript.push(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_no() {
        assert_eq!(parse_yes_no("", true), Some(true));
        assert_eq!(parse_yes_no("", false), Some(false));
        assert_eq!(parse_yes_no(" Y ", false), Some(true));
        assert_eq!(parse_yes_no("no", true), Some(false));
        assert_eq!(parse_yes_no("maybe", true), None);
    }

    #[test]
    fn choices_by_number_or_prefix() {
        let items: Vec<String> = ["Install Debian", "Boot manager", "Recover"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_choice("", &items, 1), Some(1));
        assert_eq!(parse_choice("3", &items, 0), Some(2));
        assert_eq!(parse_choice("0", &items, 0), None);
        assert_eq!(parse_choice("4", &items, 0), None);
        assert_eq!(parse_choice("rec", &items, 0), Some(2));
        assert_eq!(parse_choice("b", &items, 0), Some(1));
        assert_eq!(parse_choice("x", &items, 0), None);
    }

    #[test]
    fn scripted_runs_out_with_a_message() {
        let mut p = Scripted::new(["a"]);
        assert_eq!(p.ask("one", None).unwrap(), "a");
        assert!(p.ask("two", None).unwrap_err().to_string().contains("ran out at: two"));
    }
}
