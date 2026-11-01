//! UI state machine pieces that do not need GPUI: screens and the stepper, the risk tag, the data-loss gate,
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
    /// The page of an already converted unit (D15).
    Manage,
    About,
}

/// Names accepted by `synofurb gui --demo --screen <name>`. The Help centre is a sheet over the current screen, so
/// `help` and `help-topic` are demo targets with an overlay, not screens.
pub const DEMO_SCREEN_NAMES: [&str; 51] = [
    "failed-agent",
    "manage-rebuild",
    "manage-faildisk",
    "manage-services",
    "manage-unreachable",
    "manage-signin",
    "manage-update",
    "manage-storage",
    "manage-storage-degraded",
    "manage-storage-hybrid",
    "manage-health-ok",
    "recovery-fresh",
    "recovery-debian",
    "recovery-agent",
    "inventory",
    "inventory-offline",
    "inventory-empty",
    "inventory-scanning",
    "connect",
    "inspect",
    "goal",
    "plan",
    "plan-keep",
    "system",
    "system-offline",
    "confirm",
    "confirm-ready",
    "confirm-keep",
    "confirm-twin",
    "install",
    "install-after-ponr",
    "failed",
    "failed-pre",
    "failed-post",
    "done",
    "update",
    "settings",
    "bootmgr",
    "recover",
    "manage",
    "help",
    "help-topic",
    "about",
    "dlg-recovery",
    "dlg-keep",
    "dlg-quick",
    "dlg-hostkey",
    "dlg-hostchanged",
    "dlg-manual",
    "dlg-next",
    "first-launch",
];

/// What a demo run opens on top of its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    /// The "update available" dialog.
    Update,
    /// The Help sheet (`Some(topic id)` opens a topic).
    Help(Option<&'static str>),
    /// "How recovery works" dialog.
    RecoveryDialog,
    /// "Keep existing volumes" dialog.
    KeepDialog,
    /// "Trust this NAS?" (Connect).
    HostKey,
    /// "Host key changed" (Connect).
    HostKeyChanged,
    /// "Quick setup" dialog (Goal).
    Quick,
    /// "Manual recovery" dialog (Recovery).
    Manual,
    /// "What happens next" dialog of the failure screen.
    NextDialog,
    /// "Before you start" dialog of the first launch.
    FirstLaunch,
}

/// Where a demo run starts: a screen, optionally with something open on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DemoTarget {
    pub screen: Screen,
    pub overlay: Overlay,
    /// The failure screen shows the "after the point of no return" variant.
    pub past_no_return: bool,
    /// System and Inventory screens start in Offline mode.
    pub offline: bool,
    /// Plan screen starts in Keep mode.
    pub keep: bool,
    /// Unit facts the Recovery and Manage demos start from.
    pub fixture: Fixture,
    /// Manage: which sample health report, which tab, and the update row's state.
    pub health: super::mgmt::DemoHealth,
    pub tab: usize,
    pub hybrid_view: bool,
    pub updating: bool,
    /// Another unit of the same model is on the network (the twin erase phrase).
    pub twin: bool,
    /// The Confirm gate is satisfied (Erase enabled).
    pub ready: bool,
    /// Find with no units: `Some(false)` empty and done, `Some(true)` still scanning.
    pub no_units: Option<bool>,
}

/// What a demo run pretends to know about the unit (drives the risk Tag and the Recovery rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Fixture {
    #[default]
    None,
    /// Boot flash written and disks changed by a failed install; default is DSM.
    Failed,
    /// Debian running through our boot-flash entries.
    Debian,
    /// The agent was armed but never answered.
    Agent,
}

impl DemoTarget {
    fn at(screen: Screen) -> DemoTarget {
        DemoTarget {
            screen,
            overlay: Overlay::None,
            past_no_return: false,
            offline: false,
            keep: false,
            fixture: Fixture::None,
            health: super::mgmt::DemoHealth::Ok,
            tab: 0,
            hybrid_view: false,
            updating: false,
            twin: false,
            ready: false,
            no_units: None,
        }
    }

    /// Parses a `--screen` value. The old names (`layout`, `account`, `find`, ...) keep working.
    pub fn parse(name: &str) -> Option<DemoTarget> {
        let n = name.trim().to_ascii_lowercase();
        let t = match n.as_str() {
            "inventory" | "find" => DemoTarget::at(Screen::Inventory),
            "inventory-empty" => DemoTarget { no_units: Some(false), ..DemoTarget::at(Screen::Inventory) },
            "inventory-scanning" => DemoTarget { no_units: Some(true), ..DemoTarget::at(Screen::Inventory) },
            "inventory-offline" => DemoTarget { offline: true, ..DemoTarget::at(Screen::Inventory) },
            "connect" => DemoTarget::at(Screen::Connect),
            "inspect" => DemoTarget::at(Screen::Inspect),
            "goal" => DemoTarget::at(Screen::Goal),
            "plan" | "layout" | "storage" => DemoTarget::at(Screen::Plan),
            "plan-keep" => DemoTarget { keep: true, ..DemoTarget::at(Screen::Plan) },
            "system" | "account" => DemoTarget::at(Screen::System),
            "system-offline" => DemoTarget { offline: true, ..DemoTarget::at(Screen::System) },
            "confirm" => DemoTarget::at(Screen::Confirm),
            "confirm-ready" => DemoTarget { ready: true, ..DemoTarget::at(Screen::Confirm) },
            "confirm-keep" => DemoTarget { keep: true, ..DemoTarget::at(Screen::Confirm) },
            "confirm-twin" => DemoTarget { twin: true, ..DemoTarget::at(Screen::Confirm) },
            "install" => DemoTarget::at(Screen::Install),
            "install-after-ponr" => DemoTarget { past_no_return: true, ..DemoTarget::at(Screen::Install) },
            "failed" | "failure" | "failed-pre" => DemoTarget::at(Screen::Failed),
            "failed-agent" => DemoTarget { fixture: Fixture::Agent, ..DemoTarget::at(Screen::Failed) },
            "failed-post" | "failed-keep" => DemoTarget { past_no_return: true, ..DemoTarget::at(Screen::Failed) },
            "done" => DemoTarget::at(Screen::Done),
            "update" => DemoTarget { overlay: Overlay::Update, ..DemoTarget::at(Screen::Inventory) },
            "settings" => DemoTarget::at(Screen::Settings),
            "about" => DemoTarget::at(Screen::About),
            "bootmgr" | "boot-manager" => DemoTarget::at(Screen::BootMgr),
            "recovery-fresh" => DemoTarget::at(Screen::Recover),
            "recovery-debian" => DemoTarget { fixture: Fixture::Debian, ..DemoTarget::at(Screen::Recover) },
            "recovery-agent" => DemoTarget { fixture: Fixture::Agent, ..DemoTarget::at(Screen::Recover) },
            "recover" | "recovery" | "restore" => DemoTarget { fixture: Fixture::Failed, ..DemoTarget::at(Screen::Recover) },
            "manage" | "manage-health-ok" => DemoTarget::at(Screen::Manage),
            "manage-rebuild" => DemoTarget { health: super::mgmt::DemoHealth::Rebuild, ..DemoTarget::at(Screen::Manage) },
            "manage-faildisk" => DemoTarget { health: super::mgmt::DemoHealth::FailedDisk, ..DemoTarget::at(Screen::Manage) },
            "manage-services" => DemoTarget { health: super::mgmt::DemoHealth::ServicesDown, ..DemoTarget::at(Screen::Manage) },
            "manage-unreachable" => DemoTarget { health: super::mgmt::DemoHealth::Unreachable, ..DemoTarget::at(Screen::Manage) },
            "manage-signin" => DemoTarget { health: super::mgmt::DemoHealth::NeedsSignIn, ..DemoTarget::at(Screen::Manage) },
            "manage-update" => DemoTarget { updating: true, ..DemoTarget::at(Screen::Manage) },
            "manage-storage" => DemoTarget { tab: 1, ..DemoTarget::at(Screen::Manage) },
            "manage-storage-degraded" => {
                DemoTarget { tab: 1, health: super::mgmt::DemoHealth::FailedDisk, ..DemoTarget::at(Screen::Manage) }
            }
            "manage-storage-hybrid" => DemoTarget { tab: 1, hybrid_view: true, ..DemoTarget::at(Screen::Manage) },
            "help" => DemoTarget { overlay: Overlay::Help(None), ..DemoTarget::at(Screen::Inventory) },
            "help-topic" => DemoTarget { overlay: Overlay::Help(Some("recovery")), ..DemoTarget::at(Screen::Confirm) },
            "dlg-recovery" => DemoTarget { overlay: Overlay::RecoveryDialog, ..DemoTarget::at(Screen::Confirm) },
            "dlg-hostkey" => DemoTarget { overlay: Overlay::HostKey, ..DemoTarget::at(Screen::Connect) },
            "dlg-hostchanged" => DemoTarget { overlay: Overlay::HostKeyChanged, ..DemoTarget::at(Screen::Connect) },
            "dlg-quick" => DemoTarget { overlay: Overlay::Quick, ..DemoTarget::at(Screen::Goal) },
            "dlg-manual" => DemoTarget { overlay: Overlay::Manual, fixture: Fixture::Agent, ..DemoTarget::at(Screen::Recover) },
            "dlg-keep" => DemoTarget { overlay: Overlay::KeepDialog, keep: true, ..DemoTarget::at(Screen::Plan) },
            "dlg-next" => DemoTarget { overlay: Overlay::NextDialog, past_no_return: true, ..DemoTarget::at(Screen::Failed) },
            "first-launch" => DemoTarget { overlay: Overlay::FirstLaunch, ..DemoTarget::at(Screen::Inventory) },
            _ => return None,
        };
        Some(t)
    }
}

/// The six steps of the guided flow (approved mockup): Inspect belongs to Connect, Plan and System are the two tabs of
/// Setup, Done belongs to Install.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    Find,
    Connect,
    Goal,
    Setup,
    Confirm,
    Install,
}

impl Step {
    pub const ALL: [Step; 6] = [Step::Find, Step::Connect, Step::Goal, Step::Setup, Step::Confirm, Step::Install];

    pub fn number(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0) + 1
    }

    /// The screen a stepper click goes to.
    pub fn screen(self) -> Screen {
        match self {
            Step::Find => Screen::Inventory,
            Step::Connect => Screen::Inspect,
            Step::Goal => Screen::Goal,
            Step::Setup => Screen::Plan,
            Step::Confirm => Screen::Confirm,
            Step::Install => Screen::Install,
        }
    }
}

impl Screen {
    /// Which stepper step a screen belongs to. `None`: Settings, About and Manage hide the stepper.
    pub fn step(self) -> Option<Step> {
        Some(match self {
            Screen::Inventory => Step::Find,
            Screen::Connect | Screen::Inspect => Step::Connect,
            Screen::Goal | Screen::BootMgr | Screen::Recover => Step::Goal,
            Screen::Plan | Screen::System => Step::Setup,
            Screen::Confirm => Step::Confirm,
            Screen::Install | Screen::Failed | Screen::Done => Step::Install,
            Screen::Settings | Screen::About | Screen::Manage => return None,
        })
    }

    /// Settings, About and Manage replace the content area without the stepper.
    pub fn is_side_page(self) -> bool {
        matches!(self, Screen::Settings | Screen::About | Screen::Manage)
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

/// The stepper only goes back through completed steps, and never once an install has started.
pub fn can_jump(current: Step, target: Step, install_started: bool) -> bool {
    !install_started && target < current
}

// ------------------------------------------------------------------------------------------------ risk

// ------------------------------------------------------------------------------------------------ data-loss gate

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateItem {
    PlanOk,
    Backup,
    Accept,
    Unit,
    Phrase,
}

#[derive(Clone, Debug)]
pub struct ConfirmGate<'a> {
    pub phrase: &'a str,
    pub typed: &'a str,
    /// "Backup is tested"
    pub ack_backup: bool,
    /// "I accept data loss"
    pub ack_loss: bool,
    /// "This is the unit" (the identity line above was compared with the label on the NAS)
    pub ack_unit: bool,
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
        if !self.ack_unit {
            m.push(GateItem::Unit);
        }
        if self.phrase_state() != PhraseMatch::Match {
            m.push(GateItem::Phrase);
        }
        m
    }

    pub fn is_open(&self) -> bool {
        self.missing().is_empty()
    }

    /// Why the red button is disabled: "Missing: ..." (the tooltip of the disabled button, never silent).
    pub fn reason(&self) -> Option<String> {
        let m = self.missing();
        if m.is_empty() {
            return None;
        }
        if m.contains(&GateItem::PlanOk) {
            return Some("Missing: a valid plan. Go back and fix the errors.".into());
        }
        let names: Vec<&str> = m
            .iter()
            .map(|i| match i {
                GateItem::PlanOk => "a valid plan",
                GateItem::Backup => "Backup is tested",
                GateItem::Accept => "I accept data loss",
                GateItem::Unit => "This is the unit",
                GateItem::Phrase => "the typed phrase",
            })
            .collect();
        Some(format!("Missing: {}.", names.join(", ")))
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
            "After it, the installer stops instead of guessing and keeps its log and does not reboot by itself (power-cycle after reading it); DSM's operating system is gone. Reinstall DSM from find.synology.com with the .pat file for your model. No data recovery is promised.",
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
        assert_eq!((u.overlay, u.screen), (Overlay::Update, Screen::Inventory));
        assert!(DemoTarget::parse("failed-post").unwrap().past_no_return);
        assert!(!DemoTarget::parse("failed-pre").unwrap().past_no_return);
        assert!(DemoTarget::parse("system-offline").unwrap().offline);
        assert!(DemoTarget::parse("plan-keep").unwrap().keep);
        assert_eq!(DemoTarget::parse("help-topic").unwrap().overlay, Overlay::Help(Some("recovery")));
        assert!(DemoTarget::parse("nope").is_none());
        assert_eq!(DemoTarget::parse("  Confirm ").unwrap().screen, Screen::Confirm);
    }

    #[test]
    fn stepper_states_and_jumping() {
        assert_eq!(Step::ALL.len(), 6);
        assert_eq!(step_state(Step::Setup, Step::Connect), StepState::Done);
        assert_eq!(step_state(Step::Setup, Step::Setup), StepState::Current);
        assert_eq!(step_state(Step::Setup, Step::Confirm), StepState::Upcoming);
        assert!(can_jump(Step::Setup, Step::Connect, false));
        assert!(!can_jump(Step::Setup, Step::Confirm, false), "never forward through the stepper");
        assert!(!can_jump(Step::Install, Step::Goal, true), "no jumping while installing");
        assert_eq!(Screen::Failed.step(), Some(Step::Install));
        assert_eq!(Screen::BootMgr.step(), Some(Step::Goal));
        for s in [Screen::Settings, Screen::About, Screen::Manage] {
            assert_eq!(s.step(), None);
            assert!(s.is_side_page(), "{s:?} hides the stepper");
        }
        for s in Step::ALL {
            assert_eq!(s.screen().step(), Some(s), "stepper click lands on its own step");
        }
        assert_eq!(Step::Find.number(), 1);
        assert_eq!(Step::Install.number(), 6);
        assert_eq!(Screen::Done.step(), Some(Step::Install));
        assert_eq!(Screen::Inspect.step(), Some(Step::Connect));
        assert_eq!(Screen::System.step(), Some(Step::Setup));
    }

    fn gate<'a>(typed: &'a str, b: bool, l: bool, ok: bool) -> ConfirmGate<'a> {
        ConfirmGate { phrase: "ERASE DS918+", typed, ack_backup: b, ack_loss: l, ack_unit: true, plan_ok: ok }
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
        let mut g = gate("ERASE DS918+", true, true, true);
        g.ack_unit = false;
        assert!(!g.is_open(), "the third confirmation is required too");
    }

    #[test]
    fn the_gate_says_what_is_missing() {
        let r = gate("", false, false, true).reason().unwrap();
        assert!(
            r.starts_with("Missing:") && r.contains("Backup is tested") && r.contains("I accept data loss") && r.contains("typed phrase"),
            "{r}"
        );
        assert_eq!(gate("ERASE DS918+", true, false, true).reason().unwrap(), "Missing: I accept data loss.");
        assert_eq!(gate("", true, true, true).reason().unwrap(), "Missing: the typed phrase.");
        assert!(gate("ERASE DS918+", true, true, false).reason().unwrap().contains("valid plan"));
        let mut g = gate("ERASE DS918+", true, true, true);
        g.ack_unit = false;
        assert_eq!(g.reason().unwrap(), "Missing: This is the unit.");
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
