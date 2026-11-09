//! Factory-reset help for Synology units: how to get back to a login (forgotten admin password, wrong IP) or to a DSM
//! reinstall, and a per-model entry for the models in `docs/RESET.md`.
//!
//! Where the facts come from (and what is NOT verified) is written in `docs/RESET.md`. The short version:
//!
//! * The button sequence (about 4 s until a beep; for the reinstall release and press again within 10 s) is Synology's own wording
//!   as quoted by its product manuals and the knowledge-base article "How do I reset my Synology NAS? (For DSM 6.2.3 or earlier)".
//!   The knowledge-base pages only render with JavaScript, so the wording was not read from the article text itself.
//! * The button's **location** is model specific. It is recorded here only where Synology's own guide was read (`RS812+`: back panel).
//!   Every other model points to its own page in Synology's Download Center, where the Hardware Installation Guide is.
//! * Nothing here was tried on a unit. Synology describes the button resets as keeping the data on the volumes, but that text was not
//!   read from the article itself; read the current article before relying on it, and make a backup first.

use crate::compat::normalize_model;

/// The three ways back, from least to most destructive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResetMode {
    /// "Mode 1": RESET button, one press. Admin password, IP address and DNS go back to their defaults.
    PasswordAndNetwork,
    /// "Mode 2": RESET button, two presses. DSM is reinstalled and its settings are cleared.
    ReinstallDsm,
    /// DSM 7 "System Reset" in Control Panel. Needs a login. Erases the data on the drives.
    EraseAll,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ResetMethod {
    pub mode: ResetMode,
    pub title: &'static str,
    /// What you want it for, in one line.
    pub use_when: &'static str,
    pub steps: &'static [&'static str],
    /// What it resets or deletes. Plain words.
    pub effect: &'static str,
    /// Whether it can erase the data volumes.
    pub erases_data: bool,
}

const METHODS: [ResetMethod; 3] = [
    ResetMethod {
        mode: ResetMode::PasswordAndNetwork,
        title: "Mode 1: forgot the admin password or the IP address",
        use_when: "You cannot log in to DSM, or the unit is on an address you do not know.",
        steps: &[
            "Leave the NAS switched on and running.",
            "Straighten a paper clip. Find the RESET button on the unit (see \"Where is the RESET button\" above).",
            "Press and hold RESET for about 4 seconds, until you hear one beep. Release it at once.",
            "Wait two minutes, then look for the unit with find.synology.com or Synology Assistant, or run a scan in this app.",
            "Log in as admin. The admin password is cleared (blank on DSM 6 and earlier); newer DSM may ask you to set a new one: follow the page.",
        ],
        effect: "Resets the IP address, the DNS server and the password of the admin account to their defaults. Synology's guide for the button lists nothing else.",
        erases_data: false,
    },
    ResetMethod {
        mode: ResetMode::ReinstallDsm,
        title: "Mode 2: reinstall DSM",
        use_when: "DSM does not start, or you want DSM's own settings cleared without logging in.",
        steps: &[
            "Leave the NAS switched on and running.",
            "Press and hold RESET for about 4 seconds, until you hear a beep. Release it.",
            "Within 10 seconds press and hold RESET again for about 4 seconds, until you hear several beeps (Synology's older text says three). Release it.",
            "Wait for the unit to restart. It appears as \"Not installed\" in find.synology.com or Synology Assistant.",
            "Install DSM with the .pat file for your model from the Download Center (the page linked above), then migrate or keep your volumes when the installer offers it.",
        ],
        effect: "Reinstalls DSM and clears its system settings, including users and shares. Synology describes it as keeping the data on the volumes, but this project could not read that article's text: confirm it in the current article and back up first. Works only with a disk installed and a DSM that was set up before.",
        erases_data: false,
    },
    ResetMethod {
        mode: ResetMode::EraseAll,
        title: "System Reset in DSM 7: erase everything",
        use_when: "You can log in and want the unit empty before it is sold, given away or reinstalled.",
        steps: &[
            "Log in to DSM as an administrator.",
            "Open Control Panel, then Update & Restore, then System Reset.",
            "Choose Erase all data and follow the confirmation steps.",
        ],
        effect: "Erases all data on the drives and resets DSM to its defaults. Not available on a unit in a High Availability cluster.",
        erases_data: true,
    },
];

pub fn methods() -> &'static [ResetMethod] {
    &METHODS
}

/// Product family, from the model name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// `DS...`
    DiskStation,
    /// `RS...`
    RackStation,
}

/// One model with the facts that are verified. Not every field is known for every model: `None` is "not verified".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct KnownModel {
    pub model: &'static str,
    pub family: Family,
    /// Year of release as Synology lists it (the owner's list); informational.
    pub year: u16,
    /// Where the RESET button is, only when Synology's guide for this model was read.
    pub button: Option<&'static str>,
}

macro_rules! m {
    ($model:expr, $family:ident, $year:expr) => {
        KnownModel { model: $model, family: Family::$family, year: $year, button: None }
    };
    ($model:expr, $family:ident, $year:expr, $button:expr) => {
        KnownModel { model: $model, family: Family::$family, year: $year, button: Some($button) }
    };
}

/// Every model whose Download Center page resolved when checked (2026-11-09: 77 of the 80 names in the owner's list, plus `RS812+`).
/// `DS122j`, `DS222+` and `DS2414+` were asked for but do not exist on Synology's site and are not listed.
const MODELS: &[KnownModel] = &[
    // 2025 / 2026
    m!("DS225+", DiskStation, 2025),
    m!("DS223j", DiskStation, 2023),
    // 2024
    m!("DS124", DiskStation, 2024),
    m!("DS224+", DiskStation, 2024),
    m!("DS423+", DiskStation, 2023),
    m!("DS423", DiskStation, 2023),
    // 2023
    m!("DS223", DiskStation, 2023),
    m!("DS723+", DiskStation, 2023),
    m!("DS923+", DiskStation, 2023),
    m!("DS1823xs+", DiskStation, 2023),
    // 2022
    m!("DS1522+", DiskStation, 2022),
    m!("DS2422+", DiskStation, 2022),
    m!("DS3622xs+", DiskStation, 2022),
    // 2021
    m!("DS1621+", DiskStation, 2021),
    m!("DS1621xs+", DiskStation, 2021),
    m!("DS1821+", DiskStation, 2021),
    // 2020
    m!("DS120j", DiskStation, 2020),
    m!("DS220j", DiskStation, 2020),
    m!("DS220+", DiskStation, 2020),
    m!("DS420j", DiskStation, 2020),
    m!("DS420+", DiskStation, 2020),
    m!("DS720+", DiskStation, 2020),
    m!("DS920+", DiskStation, 2020),
    m!("DS1520+", DiskStation, 2020),
    // 2019
    m!("DS419slim", DiskStation, 2019),
    m!("DS1019+", DiskStation, 2019),
    // 2018
    m!("DS118", DiskStation, 2018),
    m!("DS218j", DiskStation, 2018),
    m!("DS218", DiskStation, 2018),
    m!("DS218play", DiskStation, 2018),
    m!("DS218+", DiskStation, 2018),
    m!("DS418j", DiskStation, 2018),
    m!("DS418", DiskStation, 2018),
    m!("DS418play", DiskStation, 2018),
    m!("DS718+", DiskStation, 2018),
    m!("DS918+", DiskStation, 2018),
    m!("DS1618+", DiskStation, 2018),
    m!("DS3018xs", DiskStation, 2018),
    // 2017
    m!("DS1517", DiskStation, 2017),
    m!("DS1517+", DiskStation, 2017),
    m!("DS1817", DiskStation, 2017),
    m!("DS1817+", DiskStation, 2017),
    m!("DS3617xs", DiskStation, 2017),
    // 2016
    m!("DS116", DiskStation, 2016),
    m!("DS216se", DiskStation, 2016),
    m!("DS216j", DiskStation, 2016),
    m!("DS216", DiskStation, 2016),
    m!("DS216play", DiskStation, 2016),
    m!("DS216+", DiskStation, 2016),
    m!("DS216+II", DiskStation, 2016),
    m!("DS416j", DiskStation, 2016),
    m!("DS416slim", DiskStation, 2016),
    m!("DS416", DiskStation, 2016),
    m!("DS416play", DiskStation, 2016),
    m!("DS716+", DiskStation, 2016),
    m!("DS716+II", DiskStation, 2016),
    m!("DS916+", DiskStation, 2016),
    // 2015
    m!("DS115j", DiskStation, 2015),
    m!("DS115", DiskStation, 2015),
    m!("DS215j", DiskStation, 2015),
    m!("DS215+", DiskStation, 2015),
    m!("DS415play", DiskStation, 2015),
    m!("DS415+", DiskStation, 2015),
    m!("DS715", DiskStation, 2015),
    m!("DS1515", DiskStation, 2015),
    m!("DS1515+", DiskStation, 2015),
    m!("DS1815+", DiskStation, 2015),
    m!("DS2015xs", DiskStation, 2015),
    m!("DS3615xs", DiskStation, 2015),
    // 2014
    m!("DS114", DiskStation, 2014),
    m!("DS214se", DiskStation, 2014),
    m!("DS214", DiskStation, 2014),
    m!("DS214play", DiskStation, 2014),
    m!("DS214+", DiskStation, 2014),
    m!("DS414slim", DiskStation, 2014),
    m!("DS414j", DiskStation, 2014),
    m!("DS414", DiskStation, 2014),
    // 2012: read in Synology's own quick installation guide (v9, 23 pages): "RESET Button ... Back Panel"
    m!("RS812+", RackStation, 2012, "Back panel"),
];

pub fn models() -> &'static [KnownModel] {
    MODELS
}

/// Synology's Download Center page of a model: the Hardware Installation Guide (where the button is) and the DSM `.pat` files are there.
/// Checked per model: a name Synology does not know is redirected to the generic page, so only listed models get their own link.
pub fn download_url(model: &str) -> String {
    format!("https://www.synology.com/en-global/support/download/{model}")
}

/// Where to look when the model is not in the list.
pub const GENERIC_DOWNLOAD_URL: &str = "https://www.synology.com/en-global/support/download";
/// The knowledge-base article for the button procedure (DSM 6.2.3 and earlier; DSM 7 has its own "System Reset" page).
pub const KB_URL: &str = "https://kb.synology.com/en-global/knowledgebase/tutorials/493";
pub const FIND_URL: &str = "https://find.synology.com";

#[derive(Debug, Clone, serde::Serialize)]
pub struct ResetGuide {
    /// The model as asked, or as listed when it was found.
    pub model: String,
    /// `true` when the model is in the list; `false` gives the generic procedure and the generic download page.
    pub known: bool,
    pub year: Option<u16>,
    /// Where the RESET button is when Synology's guide for the model was read; `None` otherwise.
    pub button_location: Option<&'static str>,
    /// Always set: tells where to find the location when it is not recorded.
    pub button_note: String,
    pub download_url: String,
    pub kb_url: &'static str,
    pub methods: Vec<ResetMethod>,
}

/// The model in the list for a name typed any way (`ds918plus`, `DS 918 +`, `rs812+`).
pub fn find(model: &str) -> Option<&'static KnownModel> {
    let want = normalize_model(model);
    if want.is_empty() {
        return None;
    }
    MODELS.iter().find(|m| normalize_model(m.model) == want)
}

pub fn guide(model: &str) -> ResetGuide {
    let found = find(model);
    let shown = found.map_or_else(|| model.trim().to_string(), |m| m.model.to_string());
    let button_note = match found.and_then(|m| m.button) {
        Some(loc) => format!("{loc}. From Synology's own guide for this model."),
        None => "Not recorded for this model. It is a small hole or button on the unit, usually on the back; the exact place is in the Hardware Installation Guide of your model (Download Center page below).".into(),
    };
    ResetGuide {
        known: found.is_some(),
        year: found.map(|m| m.year),
        button_location: found.and_then(|m| m.button),
        button_note,
        download_url: if found.is_some() { download_url(&shown) } else { GENERIC_DOWNLOAD_URL.to_string() },
        kb_url: KB_URL,
        methods: METHODS.to_vec(),
        model: shown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_listed_once_and_has_a_plausible_name() {
        let mut seen = std::collections::HashSet::new();
        for m in models() {
            assert!(seen.insert(normalize_model(m.model)), "{} twice", m.model);
            assert!(m.model.starts_with("DS") || m.model.starts_with("RS"), "{}", m.model);
            assert!((2010..=2026).contains(&m.year), "{} {}", m.model, m.year);
            assert_eq!(m.family == Family::RackStation, m.model.starts_with("RS"), "{}", m.model);
        }
        assert_eq!(models().len(), 78);
    }

    #[test]
    fn the_three_names_that_do_not_exist_at_synology_are_not_listed() {
        for bad in ["DS122j", "DS222+", "DS2414+"] {
            assert!(find(bad).is_none(), "{bad}");
            assert!(!guide(bad).known);
        }
    }

    #[test]
    fn names_are_found_however_they_are_typed() {
        for q in ["DS918+", "ds918plus", "DS 918 +", "ds-918+", " ds918+ "] {
            assert_eq!(find(q).map(|m| m.model), Some("DS918+"), "{q}");
        }
        assert!(find("").is_none());
        assert_eq!(find("rs812plus").map(|m| m.model), Some("RS812+"));
        // a name that is a prefix of another is not a match
        assert_eq!(find("DS216").map(|m| m.model), Some("DS216"));
        assert_eq!(find("DS216+").map(|m| m.model), Some("DS216+"));
        assert_eq!(find("DS216+II").map(|m| m.model), Some("DS216+II"));
        assert!(find("DS21").is_none());
    }

    #[test]
    fn the_button_location_is_given_only_where_a_guide_was_read() {
        let rs = guide("RS812+");
        assert_eq!(rs.button_location, Some("Back panel"));
        assert!(rs.button_note.contains("Back panel"));
        let ds = guide("DS918+");
        assert_eq!(ds.button_location, None);
        assert!(ds.button_note.contains("Hardware Installation Guide"));
        assert_eq!(models().iter().filter(|m| m.button.is_some()).count(), 1);
    }

    #[test]
    fn an_unknown_model_gets_the_procedure_and_the_generic_page() {
        let g = guide("DS999zz");
        assert!(!g.known);
        assert_eq!(g.model, "DS999zz");
        assert_eq!(g.download_url, GENERIC_DOWNLOAD_URL);
        assert_eq!(g.methods.len(), 3);
    }

    #[test]
    fn a_known_model_links_to_its_own_page() {
        assert_eq!(guide("ds918plus").download_url, "https://www.synology.com/en-global/support/download/DS918+");
    }

    #[test]
    fn only_the_software_reset_is_marked_as_erasing_data() {
        let erasing: Vec<ResetMode> = methods().iter().filter(|m| m.erases_data).map(|m| m.mode).collect();
        assert_eq!(erasing, vec![ResetMode::EraseAll]);
        for m in methods() {
            assert!(!m.steps.is_empty() && !m.effect.is_empty(), "{:?}", m.mode);
        }
    }

    #[test]
    fn the_button_sequence_matches_the_documented_one() {
        let m1 = &methods()[0];
        assert!(m1.steps.iter().any(|s| s.contains("about 4 seconds") && s.contains("one beep")));
        let m2 = &methods()[1];
        assert!(m2.steps.iter().any(|s| s.contains("Within 10 seconds")));
    }
}
