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
}

/// The Recovery rows as a function of the facts: a row that cannot work says why.
pub fn recovery_rows(f: &UnitFacts) -> Vec<RecRow> {
    let dsm_default = !f.default_not_dsm && !f.agent_unknown;
    vec![
        RecRow {
            kind: RecKind::BootDsm,
            label: "Boot DSM",
            button: "Boot",
            desc: if f.disks {
                "Makes DSM the default. The loader may show Synology's install page. Reboot afterwards."
            } else {
                "Makes DSM the default boot entry. Reboot the unit afterwards."
            },
            off: dsm_default.then_some("DSM is already the default."),
        },
        RecRow {
            kind: RecKind::Agent,
            label: "Open agent",
            button: "Show command",
            desc: "Shows the command that boots the SSH agent once.",
            off: (!f.flash).then_some("No agent on the flash. Set it up under Boot manager."),
        },
        RecRow {
            kind: RecKind::RestoreFlash,
            label: "Restore flash",
            button: "Show command",
            desc: "Shows the command that writes the saved image back to the boot flash.",
            off: (!f.flash).then_some("Nothing of ours is on the flash."),
        },
        RecRow {
            kind: RecKind::RemoveEntries,
            label: "Remove entries",
            button: "Show command",
            desc: "Shows the command that leaves only Synology's own files on the flash.",
            off: if !f.flash {
                Some("Nothing of ours is on the flash.")
            } else if f.default_not_dsm && f.debian {
                Some("Debian starts through these entries. Boot DSM first.")
            } else {
                None
            },
        },
        RecRow {
            kind: RecKind::ReinstallDsm,
            label: "Reinstall DSM",
            button: "Open guide",
            desc: "Use Synology's page; no data promise.",
            off: None,
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

/// The engine's agent-timeout text ends with "The NAS defaults to DSM: ... it will boot DSM normally." That is true only
/// if the agent ran; when it never answered the unit may keep booting the agent (`boot/dom/README.md`, "Fail-safe
/// design"). The UI never shows that promise: this replaces the sentence by what is known. Other text is unchanged.
pub fn scrub_agent_promise(text: &str) -> String {
    let Some(i) = text.find("The NAS defaults to DSM") else { return text.to_string() };
    if !is_agent_timeout(text) {
        return text.to_string();
    }
    let head = text[..i].trim_end().trim_end_matches('.');
    format!("{head}. Nothing was reset after the wait.")
}

/// The two commands of the manual recovery (`boot/dom/README.md`, "Manual recovery"): 1. on DSM after choosing
/// `SYNOLOGY_2` in the boot menu, 2. on another Linux computer with the boot flash attached. `/dev/sdX` is the one
/// thing only the user can fill in (the text says so); the image path is the newest saved backup when there is one.
pub fn manual_recovery_commands(backup: Option<&str>) -> [String; 2] {
    let image = backup.map(shell_quote).unwrap_or_else(|| "<saved boot-flash image>".to_string());
    ["sudo synofurb-dom default dsm --yes".to_string(), format!("sudo SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes {image}")]
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+=:,@%".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The command that boots the agent once (`boot-manager enter`), with the host and user already filled in.
pub fn agent_command(host: &str, user: &str) -> String {
    format!("synofurb-cli boot-manager enter --host {} --user {}", shell_quote(host), shell_quote(user))
}

/// The newest `synofurb-dom-backup-*.img` in `dir` (the names carry a sortable time stamp), if any.
pub fn latest_backup(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut found: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("synofurb-dom-backup-") && n.ends_with(".img")))
        .collect();
    found.sort();
    found.pop()
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

/// One line of the destroyed-disk list on Confirm: `sda · WDC WD40EFRX · …K0000 · 3.64 TiB` (name, model, serial tail,
/// size). Names and sizes only: nothing here is a claim about the data on the disk.
pub fn destroyed_disk_line(name: &str, model: &str, serial: &str, size: &str) -> String {
    let tail: String = serial.chars().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect();
    let mut parts: Vec<String> = vec![name.to_string()];
    if !model.trim().is_empty() {
        parts.push(model.trim().to_string());
    }
    if !tail.is_empty() {
        parts.push(format!("\u{2026}{tail}"));
    }
    parts.push(size.to_string());
    parts.join(" \u{00b7} ")
}

/// How many disk lines Confirm shows before "and N more" (a wide chassis has 12 or more bays).
pub const DESTROYED_LINES_MAX: usize = 8;

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
        assert!(debian[1..4].iter().all(|r| r.button == "Show command"), "rows that only show a command say so");
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

    #[test]
    fn destroyed_disk_line_names_size_and_serial_tail() {
        assert_eq!(
            destroyed_disk_line("sda", "WDC WD40EFRX", "WD-WCC7K0000", "3.64 TiB"),
            "sda \u{00b7} WDC WD40EFRX \u{00b7} \u{2026}K0000 \u{00b7} 3.64 TiB"
        );
        assert_eq!(destroyed_disk_line("nvme0n1", " ", "", "1.00 TiB"), "nvme0n1 \u{00b7} 1.00 TiB");
    }

    #[test]
    fn the_engine_promise_about_dsm_is_never_shown() {
        let raw = "the boot agent did not appear within 10 minutes. The NAS defaults to DSM: power-cycle it if it is stuck and it will boot DSM normally.";
        let t = scrub_agent_promise(raw);
        assert_eq!(t, "the boot agent did not appear within 10 minutes. Nothing was reset after the wait.");
        assert!(!t.contains("normally") && !t.contains("defaults to DSM"));
        assert_eq!(scrub_agent_promise("grub-install failed"), "grub-install failed");
        // the engine's newer, honest text is left alone
        let honest = "The boot agent did not answer within 10 minutes of the reboot request, and nothing was reset after the wait.";
        assert_eq!(scrub_agent_promise(honest), honest);
    }

    #[test]
    fn manual_recovery_commands_follow_the_readme() {
        let c = manual_recovery_commands(Some("/home/u/.local/share/synofurb/dom-backups/synofurb-dom-backup-ds918-20260101.img"));
        assert_eq!(c[0], "sudo synofurb-dom default dsm --yes");
        assert!(c[1].starts_with("sudo SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes /home/u/"));
        let q = manual_recovery_commands(Some("/tmp/my dir/x.img"));
        assert!(q[1].ends_with("'/tmp/my dir/x.img'"));
        assert!(manual_recovery_commands(None)[1].contains("<saved boot-flash image>"));
    }

    #[test]
    fn latest_backup_is_the_newest_stamp() {
        let dir = std::env::temp_dir().join(format!("fv-latest-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["synofurb-dom-backup-a-20260101.img", "synofurb-dom-backup-a-20260301.img", "other.img"] {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
        assert_eq!(latest_backup(&dir).unwrap().file_name().unwrap(), "synofurb-dom-backup-a-20260301.img");
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(latest_backup(&dir).is_none());
    }

    #[test]
    fn agent_command_is_filled_in() {
        assert_eq!(agent_command("192.168.1.20", "admin"), "synofurb-cli boot-manager enter --host 192.168.1.20 --user admin");
        assert!(agent_command("a b", "x").contains("'a b'"));
    }
}
