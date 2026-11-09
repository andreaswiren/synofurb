//! `reset-help`: factory-reset help for a Synology model. Reads nothing from the network and touches no unit.

use super::args::ResetHelpArgs;
use super::exit::CliResult;
use super::facts::SCHEMA;
use super::out::Out;
use serde_json::json;
use synofurb_core::reset::{self, ResetGuide};

/// The guide as plain text lines (also used by the tests).
pub fn guide_lines(g: &ResetGuide) -> Vec<String> {
    let mut v = Vec::new();
    if g.known {
        v.push(format!("{} ({}): ways back to a login or a reinstall", g.model, g.year.map_or(String::new(), |y| y.to_string())));
    } else if g.model.is_empty() {
        v.push("Ways back to a login or a reinstall (any Synology model)".into());
    } else {
        v.push(format!("{}: not in the list. The procedure below is the general one; check it against your model's guide.", g.model));
    }
    v.push(String::new());
    v.push(format!("Where is the RESET button: {}", g.button_note));
    v.push(format!("Hardware Installation Guide and DSM files: {}", g.download_url));
    v.push(format!("Synology's article on the button (DSM 6.2.3 and earlier): {}", g.kb_url));
    for m in &g.methods {
        v.push(String::new());
        v.push(m.title.to_string());
        v.push(format!("  Use it when: {}", m.use_when));
        for (i, s) in m.steps.iter().enumerate() {
            v.push(format!("  {}. {s}", i + 1));
        }
        v.push(format!("  Effect: {}", m.effect));
        if m.erases_data {
            v.push("  WARNING: this erases data. Make a verified backup first.".into());
        }
    }
    v.push(String::new());
    v.push(
        "None of this was tried by synofurb on a unit; see docs/RESET.md for what is verified. Back up before you press anything.".into(),
    );
    v
}

pub fn reset_help(out: &Out, a: ResetHelpArgs) -> CliResult<()> {
    if a.list {
        if a.json {
            out.json_doc(&json!({"schema": SCHEMA, "models": reset::models()}));
        } else {
            for m in reset::models() {
                out.data(&format!(
                    "{:12} {:12} {}  {}",
                    m.model,
                    format!("{:?}", m.family),
                    m.year,
                    m.button.unwrap_or("button location: see the guide")
                ));
            }
        }
        return Ok(());
    }
    let g = reset::guide(a.model.as_deref().unwrap_or(""));
    if a.json {
        out.json_doc(&json!({"schema": SCHEMA, "guide": g}));
    } else {
        out.data_lines(guide_lines(&g));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_for_a_known_model_names_the_page_the_button_and_all_three_ways() {
        let t = guide_lines(&reset::guide("RS812+")).join("\n");
        assert!(t.contains("RS812+ (2012)"));
        assert!(t.contains("Back panel"));
        assert!(t.contains("support/download/RS812+"));
        assert!(t.contains("Mode 1") && t.contains("Mode 2") && t.contains("Erase all data"));
        assert!(t.contains("WARNING: this erases data"));
    }

    #[test]
    fn an_unknown_model_is_said_to_be_unknown_and_the_generic_page_is_given() {
        let t = guide_lines(&reset::guide("DS999zz")).join("\n");
        assert!(t.contains("DS999zz: not in the list"));
        assert!(t.contains("support/download\n") || t.contains("support/download "));
        assert!(!t.contains("support/download/DS999zz"));
    }

    #[test]
    fn no_model_gives_the_general_text() {
        let t = guide_lines(&reset::guide("")).join("\n");
        assert!(t.starts_with("Ways back to a login"));
    }
}
