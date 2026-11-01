//! UI state machine pieces that do not need GPUI: screens and the step rail, the risk level, the data-loss gate,
//! boot-method choice, serial-console input parsing and the install progress model. Pure, unit-tested.

use super::logview::{LogBuffer, uboot_command};
use synofurb_core::logs::{Event, Phase};
use synofurb_core::model::{DeviceFacts, Firmware, SerialConsole};

// ------------------------------------------------------------------------------------------------ screens

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    Inventory,
    Connect,
    Inspect,
    Goal,
    Plan,
    System,
    Confirm,
    Install,
    Failed,
    Done,
    Settings,
    BootMgr,
    Recover,
}

/// Names accepted by `synofurb gui --demo --screen <name>`.
pub const DEMO_SCREEN_NAMES: [&str; 14] = [
    "inventory",
    "connect",
    "inspect",
    "goal",
    "plan",
    "system",
    "confirm",
    "install",
    "failed",
    "done",
    "update",
    "settings",
    "bootmgr",
    "recover",
];

/// Where a demo run starts: a screen, optionally with the "update available" dialog on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoTarget {
    pub screen: Screen,
    pub update_dialog: bool,
}

impl DemoTarget {
    /// Parses a `--screen` value. The old egui names `layout` and `account` keep working.
    pub fn parse(name: &str) -> Option<DemoTarget> {
        let n = name.trim().to_ascii_lowercase();
        let (screen, update_dialog) = match n.as_str() {
            "inventory" | "find" => (Screen::Inventory, false),
            "connect" => (Screen::Connect, false),
            "inspect" => (Screen::Inspect, false),
            "goal" => (Screen::Goal, false),
            "plan" | "layout" | "storage" => (Screen::Plan, false),
            "system" | "account" => (Screen::System, false),
            "confirm" => (Screen::Confirm, false),
            "install" => (Screen::Install, false),
            "failed" | "failure" => (Screen::Failed, false),
            "done" => (Screen::Done, false),
            "update" => (Screen::Inventory, true),
            "settings" | "about" => (Screen::Settings, false),
            "bootmgr" | "boot-manager" => (Screen::BootMgr, false),
            "recover" | "restore" => (Screen::Recover, false),
            _ => return None,
        };
        Some(DemoTarget { screen, update_dialog })
    }
}

/// The nine steps of the guided flow (concept A: one spine, persistent rail).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    Find,
    Connect,
    Inspect,
    Goal,
    Storage,
    System,
    Confirm,
    Install,
    Done,
}

impl Step {
    pub const ALL: [Step; 9] =
        [Step::Find, Step::Connect, Step::Inspect, Step::Goal, Step::Storage, Step::System, Step::Confirm, Step::Install, Step::Done];

    pub fn label(self) -> &'static str {
        match self {
            Step::Find => "Find NAS",
            Step::Connect => "Connect",
            Step::Inspect => "Inspect",
            Step::Goal => "Goal",
            Step::Storage => "Storage",
            Step::System => "System",
            Step::Confirm => "Confirm",
            Step::Install => "Install",
            Step::Done => "Done",
        }
    }

    pub fn number(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0) + 1
    }

    /// The screen a rail click goes to.
    pub fn screen(self) -> Screen {
        match self {
            Step::Find => Screen::Inventory,
            Step::Connect => Screen::Connect,
            Step::Inspect => Screen::Inspect,
            Step::Goal => Screen::Goal,
            Step::Storage => Screen::Plan,
            Step::System => Screen::System,
            Step::Confirm => Screen::Confirm,
            Step::Install => Screen::Install,
            Step::Done => Screen::Done,
        }
    }
}

impl Screen {
    /// Which rail step a screen belongs to (`None`: Settings, which keeps the rail where it was).
    pub fn step(self) -> Option<Step> {
        Some(match self {
            Screen::Inventory => Step::Find,
            Screen::Connect => Step::Connect,
            Screen::Inspect => Step::Inspect,
            Screen::Goal | Screen::BootMgr | Screen::Recover => Step::Goal,
            Screen::Plan => Step::Storage,
            Screen::System => Step::System,
            Screen::Confirm => Step::Confirm,
            Screen::Install | Screen::Failed => Step::Install,
            Screen::Done => Step::Done,
            Screen::Settings => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Upcoming,
}

pub fn step_state(current: Step, s: Step) -> StepState {
    use std::cmp::Ordering::*;
    match s.cmp(&current) {
        Less => StepState::Done,
        Equal => StepState::Current,
        Greater => StepState::Upcoming,
    }
}

/// Rail clicks only go back through completed steps, and never while an install is running or finished.
pub fn can_jump(current: Step, target: Step, install_started: bool) -> bool {
    !install_started && target < current
}

// ------------------------------------------------------------------------------------------------ risk

/// The four-level risk state that is always visible (UX safety pattern 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    ReadingOnly,
    PlanPrepared,
    Confirmed,
    DisksModified,
}

impl Risk {
    pub const ALL: [Risk; 4] = [Risk::ReadingOnly, Risk::PlanPrepared, Risk::Confirmed, Risk::DisksModified];

    pub fn label(self) -> &'static str {
        match self {
            Risk::ReadingOnly => "Reading only",
            Risk::PlanPrepared => "Plan prepared",
            Risk::Confirmed => "You confirmed",
            Risk::DisksModified => "Disks modified",
        }
    }

    pub fn sentence(self) -> &'static str {
        match self {
            Risk::ReadingOnly => "Nothing on the NAS has been changed. synofurb only reads.",
            Risk::PlanPrepared => "A plan exists on this computer. Nothing on the NAS has been changed.",
            Risk::Confirmed => "You confirmed. The installer is starting; the disks are still untouched before the point of no return.",
            Risk::DisksModified => "The point of no return has passed. The disks have been changed.",
        }
    }
}

/// Risk level for a screen. `past_no_return` comes from the installer's marker event.
pub fn risk_for(screen: Screen, install_started: bool, past_no_return: bool) -> Risk {
    if past_no_return {
        return Risk::DisksModified;
    }
    match screen {
        Screen::Inventory | Screen::Connect | Screen::Inspect | Screen::Goal | Screen::Settings | Screen::Recover => Risk::ReadingOnly,
        Screen::BootMgr => Risk::ReadingOnly,
        Screen::Plan | Screen::System | Screen::Confirm => Risk::PlanPrepared,
        Screen::Install => Risk::Confirmed,
        Screen::Failed | Screen::Done => {
            if install_started {
                Risk::Confirmed
            } else {
                Risk::ReadingOnly
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------ data-loss gate

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateItem {
    PlanOk,
    Backup,
    Accept,
    Phrase,
}

#[derive(Clone, Debug)]
pub struct ConfirmGate<'a> {
    pub phrase: &'a str,
    pub typed: &'a str,
    pub ack_backup: bool,
    pub ack_loss: bool,
    pub plan_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhraseMatch {
    Empty,
    /// Typed so far is a correct prefix: `n` of `total` characters.
    Partial {
        n: usize,
        total: usize,
    },
    Match,
    /// Differs from the phrase at character index `at`.
    Mismatch {
        at: usize,
    },
}

impl ConfirmGate<'_> {
    /// Surrounding whitespace is ignored (a pasted phrase often carries a newline); case is not.
    pub fn phrase_state(&self) -> PhraseMatch {
        let typed = self.typed.trim();
        if typed.is_empty() {
            return PhraseMatch::Empty;
        }
        if typed == self.phrase {
            return PhraseMatch::Match;
        }
        let total = self.phrase.chars().count();
        let n = typed.chars().zip(self.phrase.chars()).take_while(|(a, b)| a == b).count();
        if n == typed.chars().count() && n < total { PhraseMatch::Partial { n, total } } else { PhraseMatch::Mismatch { at: n } }
    }

    /// Everything still missing, in the order the screen shows it.
    pub fn missing(&self) -> Vec<GateItem> {
        let mut m = vec![];
        if !self.plan_ok {
            m.push(GateItem::PlanOk);
        }
        if !self.ack_backup {
            m.push(GateItem::Backup);
        }
        if !self.ack_loss {
            m.push(GateItem::Accept);
        }
        if self.phrase_state() != PhraseMatch::Match {
            m.push(GateItem::Phrase);
        }
        m
    }

    pub fn is_open(&self) -> bool {
        self.missing().is_empty()
    }

    /// Why the red button is disabled (shown beside it, never silent).
    pub fn reason(&self) -> Option<String> {
        let m = self.missing();
        if m.is_empty() {
            return None;
        }
        if m == [GateItem::PlanOk] {
            return Some("The plan has errors. Go back and fix them first.".into());
        }
        if m.contains(&GateItem::PlanOk) {
            return Some("The plan has errors. Go back and fix them, then complete the confirmation.".into());
        }
        let boxes = m.iter().filter(|i| matches!(i, GateItem::Backup | GateItem::Accept)).count();
        let phrase = m.contains(&GateItem::Phrase);
        Some(match (boxes, phrase) {
            (0, true) => format!("Type {} exactly to enable this button.", self.phrase),
            (1, false) => "Tick the remaining box to enable this button.".into(),
            (_, false) => "Tick both boxes to enable this button.".into(),
            (1, true) => format!("Tick the remaining box and type {} to enable this button.", self.phrase),
            (_, true) => format!("Tick both boxes and type {} to enable this button.", self.phrase),
        })
    }
}

// ------------------------------------------------------------------------------------------------ boot method

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BootChoice {
    #[default]
    Auto,
    Dom,
    Kexec,
    Kit,
}

impl BootChoice {
    pub const ALL: [BootChoice; 4] = [BootChoice::Auto, BootChoice::Dom, BootChoice::Kexec, BootChoice::Kit];

    pub fn label(self) -> &'static str {
        match self {
            BootChoice::Auto => "Automatic (recommended)",
            BootChoice::Dom => "Through the internal boot flash",
            BootChoice::Kexec => "kexec over SSH",
            BootChoice::Kit => "Installer kit (USB / TFTP / U-Boot)",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            BootChoice::Auto => "synofurb picks the method that suits this unit: boot flash if it has one, else kexec, else the kit.",
            BootChoice::Dom => {
                "x86 units with an internal boot flash (DOM): back it up to this computer, add a small boot agent, reboot into it and let it start the installer. SSH only; DSM stays the fail-safe default."
            }
            BootChoice::Kexec => {
                "Uploads a static kexec and jumps from the running DSM straight into the installer. Needs kexec support in DSM's kernel."
            }
            BootChoice::Kit => {
                "Writes kernel, initrd and instructions to a folder; you boot the NAS from them (USB, TFTP, U-Boot). Needed for ARM U-Boot boards."
            }
        }
    }

    /// `Err(reason)` when this unit cannot use the method.
    pub fn available(self, facts: &DeviceFacts) -> Result<(), &'static str> {
        match self {
            BootChoice::Auto | BootChoice::Kit => Ok(()),
            BootChoice::Dom => {
                if facts.firmware == Firmware::Dom && facts.dom.as_ref().is_some_and(|d| d.is_editable()) {
                    Ok(())
                } else {
                    Err("This unit has no editable internal boot flash (DOM).")
                }
            }
            BootChoice::Kexec => {
                if facts.kexec {
                    Ok(())
                } else {
                    Err("The DSM kernel on this unit has no kexec support.")
                }
            }
        }
    }

    /// What `Auto` means for this unit (same rule as the command line).
    pub fn resolve(self, facts: &DeviceFacts) -> BootChoice {
        match self {
            BootChoice::Auto if facts.firmware == Firmware::Dom => BootChoice::Dom,
            BootChoice::Auto if facts.kexec => BootChoice::Kexec,
            BootChoice::Auto => BootChoice::Kit,
            other => other,
        }
    }
}

/// The steps of the boot-flash (DOM) flow, as described in `synofurb_core::domboot`.
pub const DOM_FLOW: [&str; 4] = [
    "Back up the whole boot flash to this computer (streamed, hashed, verified against the NAS).",
    "Upload the boot agent (kernel, initrd, per-unit settings) and add it as a boot entry, armed for the next boot only.",
    "Reboot DSM. The agent resets the default boot entry to DSM first (fail-safe) and brings up SSH.",
    "Connect to the agent and start the Debian installer with kexec from this computer.",
];

// ------------------------------------------------------------------------------------------------ SSH key / kernel

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMode {
    Generate,
    Paste,
    None,
}

/// Serial console from the two form fields: a plain tty name (`ttyS0`), or a raw console such as
/// `uart8250,mmio32,0xfedc9000,115200n8` (AMD boards have no legacy UART). Returns `None` for input the
/// core would reject.
pub fn serial_from_input(text: &str, speed: &str) -> Option<SerialConsole> {
    let t = text.trim();
    let speed_n: u32 = speed.trim().parse().ok().filter(|s| (1200..=4_000_000).contains(s)).or_else(|| {
        // a raw console carries its own speed as the last field
        t.rsplit(',').next().map(|f| f.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).and_then(|d| d.parse().ok())
    })?;
    if t.is_empty() {
        return None;
    }
    if t.contains(',') || t.contains("0x") {
        let c = SerialConsole::from_cmdline(&format!("console={t}"))?;
        return Some(SerialConsole { speed: speed_n, ..c });
    }
    if t.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Some(SerialConsole { tty: t.to_string(), speed: speed_n, raw: None });
    }
    None
}

/// Text for the serial field of detected facts: the raw console if there is one, else the tty name.
pub fn serial_field_text(c: &SerialConsole) -> String {
    c.raw.clone().unwrap_or_else(|| c.tty.clone())
}

// ------------------------------------------------------------------------------------------------ recovery copy

/// The three-line fallback plan shown on the confirm screen, worded after `docs/RECOVERY.md`: what is saved before the
/// first write, what happens if it fails before the point of no return, and what is (not) possible after it. No
/// promise of data recovery, ever.
pub fn fallback_lines(fw: Firmware) -> [&'static str; 3] {
    const SAVED: &str = "Saved on this computer before the first write: the first and last MiB of every disk the plan changes (partition tables only, never your files). That is not a backup of your data.";
    match fw {
        Firmware::Dom => [
            "Saved on this computer before anything else: a full image of the unit's boot flash (checked against the NAS), then the first and last MiB of every disk the plan changes (partition tables only, never your files). Neither is a backup of your data.",
            "Before the point of no return the disks are untouched and DSM stays the default boot entry. If a step fails, the installer sets the default to DSM and reboots the unit after 60 s; a power cycle does the same.",
            "After the point of no return the default is set back to DSM and the unit reboots after 15 min (time to read the log). Only the DSM loader runs: DSM's operating system on the reclaimed partitions is gone. Reinstall DSM from find.synology.com with the .pat file for your model. Synology documents keeping data volumes on a reinstall, but synofurb cannot promise that any data survives.",
        ],
        Firmware::UBoot => [
            SAVED,
            "Before the point of no return the disks are untouched. Cancelling or power-cycling boots DSM as before: synofurb never writes the unit's U-Boot flash.",
            "After it, DSM's system partition is gone. The stock U-Boot still boots DSM from flash unless you saved a changed environment over the serial console; the install log prints the original bootcmd. Reinstall DSM from find.synology.com. No data recovery is promised.",
        ],
        _ => [
            SAVED,
            "Before the point of no return the disks are untouched. Cancelling or power-cycling boots DSM as before.",
            "After it, the installer stops instead of guessing and keeps its log and a rescue shell; DSM's operating system is gone. Reinstall DSM from find.synology.com with the .pat file for your model. No data recovery is promised.",
        ],
    }
}

// ------------------------------------------------------------------------------------------------ install progress

/// Short names of the 12 installer steps (the live step name from the installer is shown beside the bar).
pub const INSTALL_STEPS: [&str; 12] = [
    "Load installer components",
    "Preflight: match disks",
    "Release RAID/LVM, wipe reclaimed areas",
    "Partition the disks",
    "Create RAID arrays",
    "Create LVM volumes",
    "Create filesystems",
    "Mount the new system",
    "Install the Debian base system",
    "Write fstab and unpack the payload",
    "Configure packages, users, boot",
    "Finish: save logs, unmount, reboot",
];

/// The installer marks the point of no return at the start of this step.
pub const PONR_STEP: u32 = 3;
const _: () = assert!(PONR_STEP > 1 && PONR_STEP <= 12);

pub const STEP_GROUPS: [(&str, std::ops::RangeInclusive<u32>); 4] =
    [("Prepare", 1..=2), ("Install", 3..=9), ("Configure", 10..=11), ("Verify", 12..=12)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepProgress {
    Done,
    Running,
    Pending,
    Failed,
}

/// Everything the install screens show, fed by the logger's [`Event`]s.
#[derive(Default)]
pub struct InstallView {
    pub phase: Option<Phase>,
    pub step_n: u32,
    pub step_total: u32,
    pub step_name: String,
    /// Latest upload/download progress `(what, fraction)`.
    pub progress: Option<(String, f32)>,
    pub past_no_return: bool,
    pub installer_failed: Option<String>,
    pub installer_done: bool,
    /// Step the installer was in when it reported the failure.
    pub failed_at: Option<u32>,
    pub uboot_lines: Vec<String>,
    pub log: LogBuffer,
}

impl InstallView {
    pub fn reset(&mut self) {
        *self = InstallView::default();
    }

    pub fn apply(&mut self, ev: Event) {
        match ev {
            Event::Log(l) => {
                if let Some(cmd) = uboot_command(&l.msg) {
                    self.uboot_lines.push(cmd);
                }
                self.log.push(l);
            }
            Event::Phase(p) => self.phase = Some(p),
            Event::Step { n, total, name } => {
                self.step_n = n;
                self.step_total = total;
                self.step_name = name;
            }
            Event::Progress { what, fraction } => self.progress = Some((what, fraction)),
            Event::PointOfNoReturn => self.past_no_return = true,
            Event::InstallerFailed(m) => {
                self.failed_at = Some(self.step_n);
                self.installer_failed = Some(m);
            }
            Event::InstallerDone => self.installer_done = true,
        }
    }

    /// Fraction of the 12-step bar that is complete (the running step counts as half).
    pub fn bar_fraction(&self) -> f32 {
        if self.step_total == 0 {
            return 0.0;
        }
        if self.installer_done {
            return 1.0;
        }
        ((self.step_n.saturating_sub(1)) as f32 + 0.5) / self.step_total as f32
    }

    pub fn step_progress(&self, n: u32, failed: bool) -> StepProgress {
        if failed && self.failed_at == Some(n) {
            return StepProgress::Failed;
        }
        if self.installer_done || n < self.step_n {
            StepProgress::Done
        } else if n == self.step_n && self.step_n > 0 {
            StepProgress::Running
        } else {
            StepProgress::Pending
        }
    }

    /// Cancelling is free until the point of no return.
    pub fn can_cancel(&self) -> bool {
        !self.past_no_return
    }

    pub fn phase_label(&self) -> &'static str {
        match &self.phase {
            None => "Starting",
            Some(Phase::Connecting) => "Connecting to the NAS",
            Some(Phase::Inspecting) => "Re-checking the disks",
            Some(Phase::Staging) => "Preparing and uploading the installer",
            Some(Phase::Booting) => "Booting the installer",
            Some(Phase::Installing) => "Installing Debian",
            Some(Phase::WaitingForReboot) => "Waiting for the NAS to reboot",
            Some(Phase::Verifying) => "Verifying the new system",
            Some(Phase::Finished) => "Finished",
            Some(Phase::Failed) => "Failed",
        }
    }

    /// 1..=7 position of the phase in the install pipeline (UX: "seven phases").
    pub fn phase_index(&self) -> usize {
        match &self.phase {
            None | Some(Phase::Connecting) => 1,
            Some(Phase::Inspecting) => 2,
            Some(Phase::Staging) => 3,
            Some(Phase::Booting) => 4,
            Some(Phase::Installing) => 5,
            Some(Phase::WaitingForReboot) => 6,
            Some(Phase::Verifying) | Some(Phase::Finished) => 7,
            Some(Phase::Failed) => 0,
        }
    }
}

pub const PHASE_COUNT: usize = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::inspect::{fixtures, parse_inspect};
    use synofurb_core::logs::{Level, LogLine, Source};

    fn facts() -> DeviceFacts {
        parse_inspect(&fixtures::ds_4bay()).unwrap()
    }

    #[test]
    fn demo_names_all_parse_and_legacy_names_work() {
        for n in DEMO_SCREEN_NAMES {
            assert!(DemoTarget::parse(n).is_some(), "{n}");
        }
        assert_eq!(DemoTarget::parse("layout").unwrap().screen, Screen::Plan);
        assert_eq!(DemoTarget::parse("account").unwrap().screen, Screen::System);
        let u = DemoTarget::parse("update").unwrap();
        assert!(u.update_dialog);
        assert_eq!(u.screen, Screen::Inventory);
        assert!(DemoTarget::parse("nope").is_none());
        assert_eq!(DemoTarget::parse("  Confirm ").unwrap().screen, Screen::Confirm);
    }

    #[test]
    fn rail_states_and_jumping() {
        assert_eq!(Step::ALL.len(), 9);
        assert_eq!(step_state(Step::Storage, Step::Connect), StepState::Done);
        assert_eq!(step_state(Step::Storage, Step::Storage), StepState::Current);
        assert_eq!(step_state(Step::Storage, Step::Confirm), StepState::Upcoming);
        assert!(can_jump(Step::Storage, Step::Connect, false));
        assert!(!can_jump(Step::Storage, Step::Confirm, false), "never forward through the rail");
        assert!(!can_jump(Step::Install, Step::Goal, true), "no jumping while installing");
        assert_eq!(Screen::Failed.step(), Some(Step::Install));
        assert_eq!(Screen::BootMgr.step(), Some(Step::Goal));
        assert_eq!(Screen::Settings.step(), None);
        for s in Step::ALL {
            assert_eq!(s.screen().step(), Some(s), "rail click lands on its own step");
        }
        assert_eq!(Step::Find.number(), 1);
        assert_eq!(Step::Done.number(), 9);
    }

    #[test]
    fn risk_rises_monotonically_through_the_flow() {
        let flow = [
            Screen::Inventory,
            Screen::Connect,
            Screen::Inspect,
            Screen::Goal,
            Screen::Plan,
            Screen::System,
            Screen::Confirm,
            Screen::Install,
        ];
        let mut last = Risk::ReadingOnly;
        for s in flow {
            let r = risk_for(s, s == Screen::Install, false);
            assert!(r >= last, "{s:?}");
            last = r;
        }
        assert_eq!(risk_for(Screen::Install, true, true), Risk::DisksModified);
        assert_eq!(risk_for(Screen::Failed, true, false), Risk::Confirmed, "failure before the point of no return: disks untouched");
        assert_eq!(risk_for(Screen::Failed, true, true), Risk::DisksModified);
        assert!(Risk::PlanPrepared.sentence().contains("Nothing on the NAS has been changed"));
    }

    fn gate<'a>(typed: &'a str, b: bool, l: bool, ok: bool) -> ConfirmGate<'a> {
        ConfirmGate { phrase: "ERASE DS918+", typed, ack_backup: b, ack_loss: l, plan_ok: ok }
    }

    #[test]
    fn the_gate_opens_only_when_everything_is_satisfied() {
        assert!(!gate("", false, false, true).is_open());
        assert!(!gate("ERASE DS918+", false, true, true).is_open());
        assert!(!gate("ERASE DS918+", true, false, true).is_open());
        assert!(!gate("erase ds918+", true, true, true).is_open(), "case matters");
        assert!(!gate("ERASE DS918+", true, true, false).is_open(), "a plan with errors never opens it");
        assert!(gate("ERASE DS918+", true, true, true).is_open());
        assert!(gate("  ERASE DS918+\n", true, true, true).is_open(), "pasted whitespace is tolerated");
        assert!(gate("ERASE DS918+", true, true, true).reason().is_none());
    }

    #[test]
    fn the_gate_explains_what_is_missing() {
        let r = gate("", false, false, true).reason().unwrap();
        assert!(r.contains("both boxes") && r.contains("ERASE DS918+"));
        assert!(gate("ERASE DS918+", true, false, true).reason().unwrap().contains("remaining box"));
        assert!(gate("", true, true, true).reason().unwrap().starts_with("Type ERASE DS918+"));
        assert!(gate("ERASE DS918+", true, true, false).reason().unwrap().contains("plan has errors"));
    }

    #[test]
    fn phrase_progress() {
        assert_eq!(gate("", false, false, true).phrase_state(), PhraseMatch::Empty);
        assert_eq!(gate("ERASE D", false, false, true).phrase_state(), PhraseMatch::Partial { n: 7, total: 12 });
        assert_eq!(gate("ERASE X", false, false, true).phrase_state(), PhraseMatch::Mismatch { at: 6 });
        assert_eq!(gate("ERASE DS918+ x", false, false, true).phrase_state(), PhraseMatch::Mismatch { at: 12 });
        assert_eq!(gate("ERASE DS918+", false, false, true).phrase_state(), PhraseMatch::Match);
    }

    #[test]
    fn boot_choice_follows_the_hardware() {
        let f = facts();
        assert_eq!(BootChoice::Auto.resolve(&f), BootChoice::Kexec);
        assert!(BootChoice::Dom.available(&f).is_err());
        assert!(BootChoice::Kexec.available(&f).is_ok());
        let mut nokexec = f.clone();
        nokexec.kexec = false;
        assert_eq!(BootChoice::Auto.resolve(&nokexec), BootChoice::Kit);
        assert!(BootChoice::Kexec.available(&nokexec).is_err());
        let mut dom = f.clone();
        dom.firmware = Firmware::Dom;
        dom.dom = Some(synofurb_core::model::DomFacts {
            sectors: 262_144,
            p1_sectors: 65_536,
            p2_sectors: 196_608,
            p1_magic: true,
            p2_magic: true,
        });
        assert_eq!(BootChoice::Auto.resolve(&dom), BootChoice::Dom);
        assert!(BootChoice::Dom.available(&dom).is_ok());
        assert_eq!(DOM_FLOW.len(), 4);
    }

    #[test]
    fn serial_input_parsing() {
        let c = serial_from_input("ttyS0", "115200").unwrap();
        assert_eq!((c.tty.as_str(), c.speed, c.raw.is_none()), ("ttyS0", 115_200, true));
        let amd = serial_from_input("uart8250,mmio32,0xfedc9000,115200n8", "115200").unwrap();
        assert_eq!(amd.raw.as_deref(), Some("uart8250,mmio32,0xfedc9000,115200n8"));
        assert_eq!(amd.tty, "");
        assert_eq!(amd.kernel_arg(), "console=uart8250,mmio32,0xfedc9000,115200n8");
        assert!(serial_from_input("ttyS0;rm -rf /", "115200").is_none());
        assert!(serial_from_input("", "115200").is_none());
        assert!(serial_from_input("ttyS0", "fast").is_none());
        assert_eq!(serial_from_input("ttyS1", "9600").unwrap().speed, 9600);
        assert_eq!(serial_field_text(&amd), "uart8250,mmio32,0xfedc9000,115200n8");
        assert_eq!(serial_field_text(&SerialConsole::default()), "ttyS0");
    }

    fn log(level: Level, msg: &str) -> Event {
        Event::Log(LogLine { ts: "t".into(), level, source: Source::Installer, msg: msg.into() })
    }

    #[test]
    fn install_view_follows_events() {
        let mut v = InstallView::default();
        assert_eq!(v.phase_label(), "Starting");
        v.apply(Event::Phase(Phase::Staging));
        v.apply(Event::Progress { what: "payload".into(), fraction: 0.4 });
        assert_eq!(v.phase_index(), 3);
        assert!(v.can_cancel());
        v.apply(Event::Step { n: 2, total: 12, name: "Preflight".into() });
        assert_eq!(v.step_progress(1, false), StepProgress::Done);
        assert_eq!(v.step_progress(2, false), StepProgress::Running);
        assert_eq!(v.step_progress(3, false), StepProgress::Pending);
        v.apply(Event::PointOfNoReturn);
        assert!(!v.can_cancel(), "cancel is unavailable after the point of no return");
        v.apply(Event::Step { n: 5, total: 12, name: "RAID".into() });
        assert!((v.bar_fraction() - 4.5 / 12.0).abs() < 1e-6);
        v.apply(log(Level::Info, "uboot-env| setenv foo bar"));
        assert_eq!(v.uboot_lines, vec!["setenv foo bar".to_string()]);
        assert_eq!(v.log.len(), 1);
        v.apply(Event::InstallerFailed("boom".into()));
        assert_eq!(v.failed_at, Some(5));
        assert_eq!(v.step_progress(5, true), StepProgress::Failed);
        v.apply(Event::InstallerDone);
        assert_eq!(v.bar_fraction(), 1.0);
        v.reset();
        assert_eq!(v.step_n, 0);
        assert!(v.log.is_empty());
    }

    #[test]
    fn fallback_copy_follows_recovery_doc() {
        let dom = fallback_lines(Firmware::Dom);
        assert!(dom[1].contains("60 s") && dom[2].contains("15 min"));
        for fw in [Firmware::Dom, Firmware::Uefi, Firmware::Bios, Firmware::UBoot] {
            let all = fallback_lines(fw).join(" ");
            assert!(all.contains("never your files"), "{fw:?}");
            assert!(all.contains("not a backup") || all.contains("Neither is a backup"), "{fw:?}");
            assert!(
                !all.to_lowercase().contains("will be restored") && !all.contains("guarantee that data"),
                "no promise of data recovery"
            );
        }
        assert!(fallback_lines(Firmware::UBoot)[2].contains("U-Boot"));
    }

    #[test]
    fn step_groups_cover_all_twelve_steps_once() {
        let mut seen = [0; 13];
        for (_, r) in STEP_GROUPS.iter() {
            for n in r.clone() {
                seen[n as usize] += 1;
            }
        }
        assert!(seen[1..].iter().all(|c| *c == 1));
        assert_eq!(INSTALL_STEPS.len(), 12);
    }
}
