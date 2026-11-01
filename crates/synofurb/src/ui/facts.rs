//! Pure decisions that the approved mockup derives from facts about the unit (never from the screen name):
//! the risk Tag and its "Way back" sentence, the Recovery rows and their primary, the Quick setup defaults and
//! the twin-aware confirmation phrase. No GPUI types, so every rule is unit-tested.

use synofurb_core::plan::DataRaid;

/// What the app knows has been written to (or is wrong with) the unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitFacts {
    /// Our entries or the agent are on the boot flash.
    pub flash: bool,
    /// The installer passed step 3: disks were repartitioned.
    pub disks: bool,
    /// Debian is installed and answered.
    pub debian: bool,
    /// An install or agent run stopped with an error.
    pub fail: bool,
    /// The boot agent was armed but never answered: the default boot entry is NOT confirmed to be DSM.
    pub agent_unknown: bool,
    /// The default boot entry is Debian (or our agent), not DSM.
    pub default_not_dsm: bool,
}

/// Health of a running Debian unit (Manage).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    Rebuild,
    FailedDisk,
    ServiceDown,
    Unreachable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Risk {
    ReadOnly,
    PlanOnly,
    FlashChanged,
    DisksChanged,
    FailedSafely,
    BootUnconfirmed,
    DebianRunning,
    NeedsAttention,
    Critical,
    Unreachable,
}

/// Where the user is, as far as the Tag cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Where {
    /// Settings and About are not about a unit: no Tag.
    Neutral,
    /// Setup (storage or system) or Confirm: a plan exists and nothing is applied.
    Planning,
    /// Anywhere else before something is written.
    Wizard,
    /// The Manage page, with the unit's health.
    Manage(Health, /* services fixed */ bool),
}

/// The risk Tag, a pure function of the facts. `None` where there is no unit (Settings, About).
pub fn risk(f: &UnitFacts, at: Where) -> Option<Risk> {
    match at {
        Where::Neutral => None,
        Where::Manage(h, fixed) => Some(match h {
            Health::Unreachable => Risk::Unreachable,
            Health::FailedDisk => Risk::Critical,
            Health::Rebuild => Risk::NeedsAttention,
            Health::ServiceDown if !fixed => Risk::NeedsAttention,
            _ => Risk::DebianRunning,
        }),
        _ => Some(if f.debian && !f.fail {
            Risk::DebianRunning
        } else if f.disks {
            Risk::DisksChanged
        } else if f.agent_unknown {
            Risk::BootUnconfirmed
        } else if f.fail {
            Risk::FailedSafely
        } else if f.flash {
            Risk::FlashChanged
        } else if at == Where::Planning {
            Risk::PlanOnly
        } else {
            Risk::ReadOnly
        }),
    }
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::ReadOnly => "Read-only",
            Risk::PlanOnly => "Plan only",
            Risk::FlashChanged => "Flash changed",
            Risk::DisksChanged => "Disks changed",
            Risk::FailedSafely => "Failed safely",
            Risk::BootUnconfirmed => "Boot unconfirmed",
            Risk::DebianRunning => "Debian running",
            Risk::NeedsAttention => "Needs attention",
            Risk::Critical => "Critical",
            Risk::Unreachable => "Unreachable",
        }
    }

    /// The "Way back" sentence (UX spec 1.1; the mockup's `wayBack()`).
    pub fn way_back(self, at: Where) -> &'static str {
        match self {
            Risk::ReadOnly => "Nothing to undo.",
            Risk::PlanOnly if at == Where::Planning => {
                "Nothing is written yet. From installer step 3 there is no undo: a failure opens a 15-minute rescue shell, then the DSM loader. No data promise."
            }
            Risk::PlanOnly => "Nothing to undo.",
            Risk::FlashChanged => "Power-cycle or Cancel: DSM boots. Our entries stay until removed.",
            Risk::DisksChanged => "Rescue shell 15 min, then the DSM loader; no data promise.",
            Risk::FailedSafely => "DSM boots again by itself.",
            Risk::BootUnconfirmed => "DSM may not return by itself: see Help, Recovery.",
            Risk::DebianRunning => "Recovery stays under Manage.",
            Risk::NeedsAttention => "Recovery stays under Manage. Repair the flagged item first.",
            Risk::Critical => "Recovery stays under Manage. Replace the failed disk soon.",
            Risk::Unreachable => "Check power and network; Recovery stays under Manage.",
        }
    }
}

// ------------------------------------------------------------------------------------------------ recovery rows

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecKind {
    BootDsm,
    Agent,
    RestoreFlash,
    RemoveEntries,
    ReinstallDsm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecRow {
    pub kind: RecKind,
    pub label: &'static str,
    pub button: &'static str,
    /// The tip behind the row's info icon.
    pub desc: &'static str,
    /// `Some(reason)` when the row is unavailable.
    pub off: Option<&'static str>,
    /// Typed confirmation phrase (destructive rows), `None` otherwise.
    pub phrase: Option<&'static str>,
}

pub const RESTORE_PHRASE: &str = "RESTORE FLASH";
pub const REMOVE_PHRASE: &str = "REMOVE ENTRIES";

/// The Recovery rows as a function of the facts: a row that cannot work says why.
pub fn recovery_rows(f: &UnitFacts) -> Vec<RecRow> {
    let dsm_default = !f.default_not_dsm && !f.agent_unknown;
    vec![
        RecRow {
            kind: RecKind::BootDsm,
            label: "Boot DSM",
            button: "Boot",
            desc: if f.disks {
                "Reboots into the DSM loader; it may show Synology's install page."
            } else {
                "Makes the unit boot DSM next time."
            },
            off: dsm_default.then_some("DSM is already the default."),
            phrase: None,
        },
        RecRow {
            kind: RecKind::Agent,
            label: "Open agent",
            button: "Start",
            desc: "Boots the SSH agent; returns to DSM by itself.",
            off: (!f.flash).then_some("No agent on the flash. Set it up under Boot manager."),
            phrase: None,
        },
        RecRow {
            kind: RecKind::RestoreFlash,
            label: "Restore flash",
            button: "Restore",
            desc: "Writes the saved image back to the boot flash.",
            off: (!f.flash).then_some("Nothing of ours is on the flash."),
            phrase: Some(RESTORE_PHRASE),
        },
        RecRow {
            kind: RecKind::RemoveEntries,
            label: "Remove entries",
            button: "Remove",
            desc: "Leaves only Synology's own files on the flash.",
            off: if !f.flash {
                Some("Nothing of ours is on the flash.")
            } else if f.default_not_dsm && f.debian {
                Some("Debian starts through these entries. Boot DSM first.")
            } else {
                None
            },
            phrase: Some(REMOVE_PHRASE),
        },
        RecRow {
            kind: RecKind::ReinstallDsm,
            label: "Reinstall DSM",
            button: "Open guide",
            desc: "Use Synology's page; no data promise.",
            off: None,
            phrase: None,
        },
    ]
}

/// What the page highlights: facts decide (a running Debian unit is not broken: nothing is highlighted).
pub fn recovery_primary(f: &UnitFacts) -> Option<RecKind> {
    if f.debian {
        return None;
    }
    if f.agent_unknown {
        // Boot DSM needs the unit to answer; the manual recovery dialog is the primary (handled by the screen).
        return None;
    }
    if f.default_not_dsm && (f.flash || f.disks) {
        return Some(RecKind::BootDsm);
    }
    if f.disks {
        return Some(RecKind::ReinstallDsm);
    }
    None
}

/// True when the page should offer "Manual recovery" as its primary action.
pub fn recovery_manual_primary(f: &UnitFacts) -> bool {
    !f.debian && f.agent_unknown
}

/// The engine's error when the boot agent never answered (`domboot::wait_for_agent`): the default boot entry is then not
/// confirmed to be DSM, so the UI offers the manual recovery instead of promising that DSM returns.
pub fn is_agent_timeout(error: &str) -> bool {
    error.contains("boot agent did not appear") || error.contains("boot agent did not answer")
}

// ------------------------------------------------------------------------------------------------ quick setup

/// What "Quick setup" chooses for the data layout (everything else is the planner's default).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickDefaults {
    pub raid: DataRaid,
    pub system_gib: u64,
    pub copies: usize,
    pub free_percent: u8,
    /// The disks differ enough that Hybrid beats the plain level, so the dialog asks.
    pub mixed: bool,
}

/// Disk sizes in TiB. Mixed sizes (largest/smallest >= 1.1) with at least 2 disks pick Hybrid 1-disk when it gives at
/// least 20 % more usable space than Auto; the dialog still shows both so the choice is explicit.
pub fn quick_defaults(disks_tib: &[f64], hybrid_usable: f64, auto_usable: f64) -> QuickDefaults {
    let mx = disks_tib.iter().cloned().fold(0.0, f64::max);
    let mn = disks_tib.iter().cloned().fold(f64::MAX, f64::min);
    let mixed = disks_tib.len() >= 2 && mn > 0.0 && mx / mn >= 1.1;
    let raid = if mixed && hybrid_usable > auto_usable * 1.2 { DataRaid::Shr1 } else { DataRaid::Auto };
    QuickDefaults { raid, system_gib: 32, copies: disks_tib.len().clamp(1, 3), free_percent: 20, mixed }
}

// ------------------------------------------------------------------------------------------------ phrase

/// `ERASE <MODEL>`, plus the serial's last four characters when another unit of the same model is on the network.
pub fn confirm_phrase_for(base: &str, serial: &str, twin: bool) -> String {
    if twin {
        let tail: String = serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        format!("{base} {tail}")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> UnitFacts {
        UnitFacts::default()
    }

    #[test]
    fn risk_tag_is_a_function_of_the_facts() {
        assert_eq!(risk(&f(), Where::Wizard), Some(Risk::ReadOnly));
        assert_eq!(risk(&f(), Where::Planning), Some(Risk::PlanOnly));
        assert_eq!(risk(&f(), Where::Neutral), None);
        assert_eq!(risk(&UnitFacts { flash: true, ..f() }, Where::Planning), Some(Risk::FlashChanged));
        assert_eq!(risk(&UnitFacts { flash: true, disks: true, ..f() }, Where::Wizard), Some(Risk::DisksChanged));
        assert_eq!(risk(&UnitFacts { flash: true, fail: true, ..f() }, Where::Wizard), Some(Risk::FailedSafely));
        assert_eq!(
            risk(&UnitFacts { flash: true, fail: true, disks: true, ..f() }, Where::Wizard),
            Some(Risk::DisksChanged),
            "a failure after step 3 is not safe"
        );
        assert_eq!(risk(&UnitFacts { flash: true, fail: true, agent_unknown: true, ..f() }, Where::Wizard), Some(Risk::BootUnconfirmed));
        assert_eq!(risk(&UnitFacts { flash: true, disks: true, debian: true, ..f() }, Where::Wizard), Some(Risk::DebianRunning));
        // Debian that ended in a failure is not "running"
        assert_eq!(risk(&UnitFacts { debian: true, fail: true, flash: true, ..f() }, Where::Wizard), Some(Risk::FailedSafely));
    }

    #[test]
    fn copy_module_and_facts_agree_on_the_tag_texts() {
        use super::super::copy::*;
        for (r, it) in [
            (Risk::ReadOnly, &RISK_READ_ONLY),
            (Risk::PlanOnly, &RISK_PLAN_ONLY),
            (Risk::FlashChanged, &RISK_FLASH_CHANGED),
            (Risk::DisksChanged, &RISK_DISKS_CHANGED),
            (Risk::FailedSafely, &RISK_FAILED_SAFELY),
            (Risk::BootUnconfirmed, &RISK_BOOT_UNCONFIRMED),
            (Risk::DebianRunning, &RISK_DEBIAN_RUNNING),
            (Risk::NeedsAttention, &RISK_NEEDS_ATTENTION),
            (Risk::Critical, &RISK_CRITICAL),
            (Risk::Unreachable, &RISK_UNREACHABLE),
        ] {
            assert_eq!(r.label(), it.short);
            assert_eq!(r.way_back(Where::Wizard), it.body, "{r:?}");
        }
    }

    #[test]
    fn manage_tag_follows_health() {
        let d = UnitFacts { debian: true, ..f() };
        assert_eq!(risk(&d, Where::Manage(Health::Ok, false)), Some(Risk::DebianRunning));
        assert_eq!(risk(&d, Where::Manage(Health::Rebuild, false)), Some(Risk::NeedsAttention));
        assert_eq!(risk(&d, Where::Manage(Health::ServiceDown, false)), Some(Risk::NeedsAttention));
        assert_eq!(risk(&d, Where::Manage(Health::ServiceDown, true)), Some(Risk::DebianRunning));
        assert_eq!(risk(&d, Where::Manage(Health::FailedDisk, false)), Some(Risk::Critical));
        assert_eq!(risk(&d, Where::Manage(Health::Unreachable, false)), Some(Risk::Unreachable));
    }

    #[test]
    fn way_back_never_promises_data() {
        for r in [
            Risk::ReadOnly,
            Risk::PlanOnly,
            Risk::FlashChanged,
            Risk::DisksChanged,
            Risk::FailedSafely,
            Risk::BootUnconfirmed,
            Risk::DebianRunning,
            Risk::NeedsAttention,
            Risk::Critical,
            Risk::Unreachable,
        ] {
            let t = r.way_back(Where::Planning);
            assert!(!t.is_empty());
            assert!(!t.to_lowercase().contains("will be restored"), "{t}");
        }
        assert!(Risk::PlanOnly.way_back(Where::Planning).contains("step 3"));
        assert_eq!(Risk::PlanOnly.way_back(Where::Wizard), "Nothing to undo.");
        assert!(Risk::DisksChanged.way_back(Where::Wizard).contains("no data promise"));
        assert!(Risk::BootUnconfirmed.way_back(Where::Wizard).contains("may not return"));
    }

    #[test]
    fn recovery_rows_say_why_not() {
        let fresh = recovery_rows(&f());
        assert_eq!(fresh.len(), 5);
        assert!(fresh[0].off.is_some(), "DSM is already the default");
        assert!(fresh[1].off.unwrap().contains("Boot manager"));
        assert!(fresh[2].off.is_some() && fresh[3].off.is_some());
        assert!(fresh[4].off.is_none(), "the guide always works");
        let flashed = recovery_rows(&UnitFacts { flash: true, default_not_dsm: true, ..f() });
        assert!(flashed[0].off.is_none() && flashed[1].off.is_none() && flashed[2].off.is_none() && flashed[3].off.is_none());
        let debian = recovery_rows(&UnitFacts { flash: true, debian: true, default_not_dsm: true, disks: true, ..f() });
        assert!(debian[3].off.unwrap().contains("Boot DSM first"), "Debian starts through the entries");
        assert!(debian[2].phrase == Some(RESTORE_PHRASE) && debian[3].phrase == Some(REMOVE_PHRASE));
        assert!(debian[0].desc.contains("loader"));
    }

    #[test]
    fn recovery_primary_follows_the_facts() {
        assert_eq!(recovery_primary(&f()), None);
        assert_eq!(
            recovery_primary(&UnitFacts { debian: true, disks: true, default_not_dsm: true, ..f() }),
            None,
            "a running Debian unit is not broken"
        );
        assert_eq!(recovery_primary(&UnitFacts { flash: true, default_not_dsm: true, ..f() }), Some(RecKind::BootDsm));
        assert_eq!(recovery_primary(&UnitFacts { disks: true, ..f() }), Some(RecKind::ReinstallDsm));
        let unknown = UnitFacts { flash: true, agent_unknown: true, ..f() };
        assert_eq!(recovery_primary(&unknown), None);
        assert!(recovery_manual_primary(&unknown));
        assert!(!recovery_manual_primary(&UnitFacts { debian: true, agent_unknown: true, ..f() }));
        // with an unconfirmed default, Boot DSM is not disabled by "already the default"
        assert!(recovery_rows(&unknown)[0].off.is_none());
    }

    #[test]
    fn quick_setup_defaults() {
        let same = quick_defaults(&[3.6, 3.6, 3.6, 3.6], 10.8, 10.8);
        assert_eq!((same.raid, same.mixed, same.system_gib, same.copies, same.free_percent), (DataRaid::Auto, false, 32, 3, 20));
        let mixed_small_gain = quick_defaults(&[3.6, 3.6, 4.0], 7.2, 7.0);
        assert_eq!(mixed_small_gain.raid, DataRaid::Auto);
        let mixed = quick_defaults(&[3.6, 3.6, 7.3], 7.2, 3.6);
        assert_eq!((mixed.raid, mixed.mixed), (DataRaid::Shr1, true));
        assert_eq!(quick_defaults(&[3.6], 0.0, 3.6).copies, 1);
        assert_eq!(quick_defaults(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0], 5.0, 5.0).copies, 3);
    }

    #[test]
    fn agent_timeout_is_recognised() {
        assert!(is_agent_timeout("the boot agent did not appear within 10 minutes. The NAS defaults to DSM: power-cycle it"));
        assert!(!is_agent_timeout("grub-install failed on sdc"));
    }

    #[test]
    fn twin_phrase_gets_the_serial_tail() {
        assert_eq!(confirm_phrase_for("ERASE DS918+", "1930PDN123456", false), "ERASE DS918+");
        assert_eq!(confirm_phrase_for("ERASE DS918+", "1930PDN123456", true), "ERASE DS918+ 3456");
        assert_eq!(confirm_phrase_for("ERASE X", "12", true), "ERASE X 12");
    }
}
