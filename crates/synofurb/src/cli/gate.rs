//! The three-part data-loss gate: a complete backup, acceptance that data can be destroyed, and the typed phrase `ERASE <MODEL>`.
//! All three are needed every time. Flags can supply them for scripts; on a terminal they can be answered instead. Nothing else
//! (`--yes`, `--non-interactive`, an environment variable) satisfies any part.

use super::args::GateOpts;
use super::exit::{CliError, CliResult};
use super::out::Out;
use super::prompt::Prompter;
use synofurb_core::model::DeviceFacts;
use synofurb_core::session::confirm_phrase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    Backup,
    Accept,
    Phrase,
}

/// What of the gate the flags do not provide yet. `phrase` is `None` before the unit has been inspected (any `--confirm` counts as
/// present, to be compared later).
pub fn missing(g: &GateOpts, phrase: Option<&str>) -> Vec<Missing> {
    let mut m = vec![];
    if !g.have_backup {
        m.push(Missing::Backup);
    }
    if !g.accept_data_loss {
        m.push(Missing::Accept);
    }
    let typed_ok = match (g.confirm.as_deref().map(str::trim), phrase) {
        (Some(t), Some(p)) => t == p,
        (Some(t), None) => !t.is_empty(),
        (None, _) => false,
    };
    if !typed_ok {
        m.push(Missing::Phrase);
    }
    m
}

pub fn explain(m: &[Missing], phrase: Option<&str>) -> String {
    let mut parts = vec![];
    for x in m {
        parts.push(match x {
            Missing::Backup => "--have-backup (a complete, verified backup exists)".to_string(),
            Missing::Accept => "--accept-data-loss (you understand data can be destroyed, even data marked as preserved)".to_string(),
            Missing::Phrase => match phrase {
                Some(p) => format!("--confirm \"{p}\" (the phrase must match exactly)"),
                None => "--confirm \"ERASE <MODEL>\" (run `plan` or `inspect` to see the exact phrase for the unit)".to_string(),
            },
        });
    }
    format!(
        "refusing to continue: still missing {}. There is NO guarantee any data on this NAS survives, including volumes marked as preserved. Back up first.",
        parts.join(", ")
    )
}

/// Before anything is contacted: when nobody can be asked, flags that are plainly absent refuse right away.
pub fn precheck(g: &GateOpts, out: &Out) -> CliResult<()> {
    if out.interactive {
        return Ok(());
    }
    let m = missing(g, None);
    if m.is_empty() { Ok(()) } else { Err(CliError::refused(explain(&m, None))) }
}

/// After the plan was shown: complete the gate with the flags, or by asking on a terminal.
pub fn enforce(g: &GateOpts, facts: &DeviceFacts, address: &str, out: &Out, prompt: &mut dyn Prompter) -> CliResult<()> {
    let phrase = confirm_phrase(facts);
    let m = missing(g, Some(&phrase));
    if m.is_empty() {
        return Ok(());
    }
    if !out.interactive {
        return Err(CliError::refused(explain(&m, Some(&phrase))));
    }
    prompt.note("");
    prompt.note("=== CONFIRMATION: this will repartition the disks of ===");
    prompt.note(&format!(
        "    {}   address {}   serial {}",
        facts.model,
        address,
        if facts.serial.is_empty() { "unknown" } else { &facts.serial }
    ));
    prompt.note("");
    if m.contains(&Missing::Backup)
        && !prompt.confirm("Do you have a complete, verified backup of everything on this NAS, somewhere else?", false)?
    {
        return Err(CliError::refused("declined: make a complete, verified backup first. Nothing was changed."));
    }
    if m.contains(&Missing::Accept)
        && !prompt.confirm("Do you understand that data on this NAS can be destroyed, even data marked as preserved, and that there is no guarantee anything survives?", false)?
    {
        return Err(CliError::refused("declined: nothing was changed."));
    }
    if m.contains(&Missing::Phrase) {
        let mut ok = false;
        for attempt in 1..=3 {
            let typed = prompt.ask(&format!("Type {phrase} exactly to start (empty cancels)"), None)?;
            if typed.trim().is_empty() {
                break;
            }
            if typed.trim() == phrase {
                ok = true;
                break;
            }
            if attempt < 3 {
                prompt.note("  that is not the phrase; try again");
            }
        }
        if !ok {
            return Err(CliError::refused("the confirmation phrase was not typed correctly. Nothing was changed."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::facts::demo_facts;
    use super::super::prompt::Scripted;
    use super::*;

    fn g(b: bool, a: bool, c: Option<&str>) -> GateOpts {
        GateOpts { have_backup: b, accept_data_loss: a, confirm: c.map(String::from) }
    }
    fn tty() -> Out {
        let mut o = Out::plain();
        o.interactive = true;
        o
    }

    #[test]
    fn all_three_parts_are_required() {
        assert_eq!(missing(&g(false, false, None), Some("ERASE DS918+")), vec![Missing::Backup, Missing::Accept, Missing::Phrase]);
        assert_eq!(missing(&g(true, false, Some("ERASE DS918+")), Some("ERASE DS918+")), vec![Missing::Accept]);
        assert_eq!(missing(&g(false, true, Some("ERASE DS918+")), Some("ERASE DS918+")), vec![Missing::Backup]);
        assert_eq!(missing(&g(true, true, Some("erase ds918+")), Some("ERASE DS918+")), vec![Missing::Phrase], "case matters");
        assert_eq!(missing(&g(true, true, Some("ERASE DS923+")), Some("ERASE DS918+")), vec![Missing::Phrase], "wrong model");
        assert!(missing(&g(true, true, Some("  ERASE DS918+\n")), Some("ERASE DS918+")).is_empty(), "pasted whitespace is tolerated");
        assert_eq!(missing(&g(true, true, Some("")), None), vec![Missing::Phrase]);
    }

    #[test]
    fn nothing_but_the_three_parts_opens_the_gate_without_a_terminal() {
        let (facts, _) = demo_facts();
        let mut out = Out::plain();
        out.yes = true; // --yes must not matter
        let mut p = Scripted::new(Vec::<String>::new());
        let e = enforce(&g(false, false, None), &facts, "10.0.0.2", &out, &mut p).err().unwrap();
        assert_eq!(e.code(), 3);
        assert!(
            e.to_string().contains("--have-backup")
                && e.to_string().contains("--accept-data-loss")
                && e.to_string().contains("ERASE DS918+")
        );
        assert!(precheck(&g(true, true, None), &out).is_err());
        assert!(precheck(&g(true, true, Some("x")), &out).is_ok());
        assert!(enforce(&g(true, true, Some("ERASE DS918+")), &facts, "10.0.0.2", &out, &mut p).is_ok());
        assert_eq!(enforce(&g(true, true, Some("ERASE DS918")), &facts, "10.0.0.2", &out, &mut p).err().unwrap().code(), 3);
    }

    #[test]
    fn a_terminal_can_answer_instead_of_the_flags() {
        let (facts, _) = demo_facts();
        let mut p = Scripted::new(["y", "yes", "ERASE DS918+"]);
        enforce(&g(false, false, None), &facts, "10.0.0.2", &tty(), &mut p).unwrap();
        assert!(p.said("serial 1920PDN123456") && p.said("10.0.0.2"), "the unit's address and serial are shown: {:?}", p.transcript);
    }

    #[test]
    fn declining_or_mistyping_refuses() {
        let (facts, _) = demo_facts();
        let mut p = Scripted::new(["n"]);
        assert_eq!(enforce(&g(false, false, None), &facts, "h", &tty(), &mut p).err().unwrap().code(), 3);
        let mut p = Scripted::new(["y", "n"]);
        assert_eq!(enforce(&g(false, false, None), &facts, "h", &tty(), &mut p).err().unwrap().code(), 3);
        let mut p = Scripted::new(["y", "y", "ERASE", "erase ds918+", "nope"]);
        assert_eq!(enforce(&g(false, false, None), &facts, "h", &tty(), &mut p).err().unwrap().code(), 3, "three wrong tries");
        let mut p = Scripted::new(["y", "y", ""]);
        assert_eq!(enforce(&g(false, false, None), &facts, "h", &tty(), &mut p).err().unwrap().code(), 3, "empty cancels");
        let mut p = Scripted::new(["ERASE nothing", "ERASE DS918+"]);
        enforce(&g(true, true, None), &facts, "h", &tty(), &mut p).expect("a second try may succeed");
    }

    #[test]
    fn flags_already_given_are_not_asked_again() {
        let (facts, _) = demo_facts();
        let mut p = Scripted::new(["ERASE DS918+"]);
        enforce(&g(true, true, None), &facts, "h", &tty(), &mut p).unwrap();
        assert_eq!(p.transcript.iter().filter(|l| l.starts_with("? ")).count(), 1);
    }
}
