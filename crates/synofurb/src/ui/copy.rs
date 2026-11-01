//! All user-visible prose of the desktop app, in one place (BRIEF-v2: "one copy module per surface").
//!
//! Every string is an [`Item`] with levels of disclosure (UX spec rule 4):
//! * `short` - what the screen shows (a title, a label, a button, one line);
//! * `tip`   - one sentence for a tooltip (hover; a keyboard user opens the same text with Enter on the info icon);
//! * `body`  - a popover paragraph, a dialog text or a Help topic (markdown);
//! * `topic` - the id of the Help topic the info icon links to.
//!
//! Items are `static`s, so a screen that names a missing string does not compile. Each screen declares in
//! [`SCREENS`] which items it shows; the tests at the bottom enforce the word budgets of `design/BRIEF-v2.md`:
//! title <= 4 words, subtitle <= 12, labels <= 4, buttons <= 3, tooltip = one sentence of <= 24 words, topics <= 150
//! words, dialogs <= 90, and at most 60 words of static prose per screen (the 14 words of the shell included). Data (names, sizes, addresses, log lines) is not copy and is never counted.

/// What an item is, which decides its budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Title,
    Subtitle,
    /// Field label, column header, menu entry, tag text.
    Label,
    /// A button: a verb and at most two more words.
    Button,
    /// Static prose on the screen (counts against the 60-word budget).
    Prose,
    /// A Help-centre topic: `short` is the title, `body` the markdown text (at most 150 words).
    Topic,
    /// A dialog: `short` is the title, `body` the paragraphs (at most 90 words).
    Dialog,
}

#[derive(Debug)]
pub struct Item {
    pub key: &'static str,
    pub kind: Kind,
    /// What is drawn.
    pub short: &'static str,
    /// The tooltip: one sentence, at most 24 words.
    pub tip: &'static str,
    /// Id of the Help topic this item links to ("" = none).
    pub topic: &'static str,
    /// Popover / dialog paragraphs, or the markdown of a Help topic ("" = none).
    pub body: &'static str,
}

impl Item {
    /// The text with `{name}` placeholders replaced in order by `args`.
    pub fn fmt(&self, args: &[&str]) -> String {
        fill(self.short, args)
    }

    /// The tooltip with placeholders replaced.
    pub fn tip_fmt(&self, args: &[&str]) -> String {
        fill(self.tip, args)
    }

    /// Does `query` (case-insensitive, every word) occur in this item? Used by the Help-centre search.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {} {}", self.short, self.tip, self.body).to_lowercase();
        query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
    }

    /// Has something more to say than the tooltip (a popover body, a dialog text or a Help topic)?
    pub fn has_more(&self) -> bool {
        !self.body.is_empty() || !self.topic.is_empty()
    }
}

/// Replace `{...}` placeholders, left to right, by `args` (missing arguments become an empty string).
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut it = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { break };
        out.push_str(&rest[..i]);
        out.push_str(it.next().copied().unwrap_or(""));
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

/// Words a person reads: whitespace separated, `{placeholders}` and bare symbols do not count.
pub fn words(s: &str) -> usize {
    let mut rest = s;
    let mut clean = String::new();
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { break };
        clean.push_str(&rest[..i]);
        clean.push(' ');
        rest = &rest[i + j + 1..];
    }
    clean.push_str(rest);
    clean.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).count()
}

/// Number of sentences (terminal punctuation followed by a space or the end).
pub fn sentences(s: &str) -> usize {
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().filter(|(i, c)| matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace())).count()
}

macro_rules! items {
    ($($name:ident: $kind:ident, $short:expr, $tip:expr, $topic:expr, $body:expr;)*) => {
        $(pub static $name: Item = Item { key: stringify!($name), kind: Kind::$kind, short: $short, tip: $tip, topic: $topic, body: $body };)*
        /// Every item, for the Help-centre search and the tests.
        pub static ALL: &[&Item] = &[$(&$name,)*];
    };
}

/// One screen's declared copy: the items it shows without any click (conditional elements are listed only when they
/// are part of the screen's normal state).
pub struct ScreenCopy {
    pub name: &'static str,
    pub visible: &'static [&'static Item],
}

pub use self::strings::*;

mod strings {
    use super::{Item, Kind};

    items! {
        // ===================================================================== shell (14 words)
        APP_NAME: Label, "synofurb", "", "", "";
        STEP_FIND: Label, "Find", "", "", "";
        STEP_CONNECT: Label, "Connect", "", "", "";
        STEP_INSPECT: Label, "Inspect", "", "", "";
        STEP_GOAL: Label, "Goal", "", "", "";
        STEP_PLAN: Label, "Plan", "", "", "";
        STEP_SYSTEM: Label, "System", "", "", "";
        STEP_CONFIRM: Label, "Confirm", "", "", "";
        STEP_INSTALL: Label, "Install", "", "", "";
        STEP_DONE: Label, "Done", "", "", "";
        RISK_READ_ONLY: Label, "Read-only", "Nothing to undo.", "recovery", "";
        RISK_PLAN_ONLY: Label, "Plan only", "Nothing to undo.", "recovery", "";
        RISK_FLASH_CHANGED: Label, "Flash changed", "Cancel or power-cycle: DSM boots.", "recovery", "";
        RISK_DISKS_CHANGED: Label, "Disks changed", "Rescue shell 15 min, then the DSM loader; no data promise.", "recovery", "";
        RISK_FAILED_SAFELY: Label, "Failed safely", "DSM boots again by itself.", "recovery", "";
        RISK_DEBIAN_RUNNING: Label, "Debian running", "Recovery stays under Manage.", "recovery", "";
        WAY_BACK: Button, "Way back", "What returns the unit to DSM from here.", "recovery", "";
        TAG_DEMO: Label, "Demo", "Built-in sample unit. Nothing touches the network or any disk.", "", "";
        TAG_OFFLINE: Label, "Offline", "Compatibility list is {n} days old; LAN scan still works.", "offline-mode", "";
        MENU_HELP: Label, "Help", "Help for this screen.", "", "";
        MENU_SETTINGS: Label, "Settings", "Theme, text size, updates, data.", "", "";
        MENU_ABOUT: Label, "About", "Version, credits, diagnostics.", "", "";
        MENU_UPDATES: Label, "Check updates", "Looks for a newer synofurb.", "", "";

        // ===================================================================== common
        BTN_BACK: Button, "Back", "", "", "";
        BTN_CLOSE: Button, "Close", "", "", "";
        BTN_CANCEL: Button, "Cancel", "", "", "";
        BTN_OPEN_HELP: Button, "Open in Help", "", "", "";
        BTN_COPY_CMD: Button, "Copy command", "Copies the equivalent synofurb command.", "cli", "";
        BTN_COPY_CMD_CONFIRM: Button, "Copy command", "Includes the confirmation flags only after you confirm here.", "cli", "";
        BTN_LEARN_MORE: Button, "Learn more", "", "", "";

        // ===================================================================== D1 inventory
        INV_TITLE: Title, "Find NAS", "", "", "";
        INV_SUB: Subtitle, "Select the unit to work on.", "", "", "";
        INV_FILTER: Label, "Filter units", "", "", "";
        INV_RESCAN: Button, "Rescan", "Five discovery methods run at once.", "troubleshooting", "Each method reports its own result; a failing one is a warning, never an error.";
        INV_ADD: Button, "Add by IP", "Enter an address to probe.", "", "";
        INV_COL_MODEL: Label, "Model", "", "", "";
        INV_COL_ADDR: Label, "Address", "", "", "";
        INV_COL_DSM: Label, "DSM", "", "", "";
        INV_COL_FOUND: Label, "Found by", "Discovery methods that saw this unit.", "first-install", "";
        INV_COL_STATUS: Label, "Status", "How much this model is proven.", "compat-status", "";
        INV_RUNNING: Label, "Running Debian", "", "", "";
        INV_MANAGE: Button, "Manage", "Open this unit's page.", "", "";
        INV_CONNECT: Button, "Connect", "Select a unit first.", "", "";
        INV_TWIN: Label, "Twin", "Another unit of this model is on the network; check serials.", "first-install", "";
        INV_EMPTY: Title, "No NAS found", "", "troubleshooting", "";
        INV_SCAN_AGAIN: Button, "Scan again", "", "", "";
        INV_ADD_TITLE: Dialog, "Add NAS", "", "", "Enter the address of a unit that discovery did not find.";
        INV_ADD_ADDR: Label, "Address", "", "", "";
        INV_ADD_BTN: Button, "Add", "", "", "";
        FIRST_LAUNCH: Dialog, "Before you start", "Read-only until you confirm.", "cli",
            "synofurb only reads until you confirm.\n\nMake a tested backup first.\n\nScreen readers are not supported in this window; the same steps work from the command line (see Help, Command line).";
        BTN_GOT_IT: Button, "Got it", "", "", "";

        // ===================================================================== D2 connect
        CONN_TITLE: Title, "Connect", "Hardware, disks and RAID state; nothing is written.", "first-install",
            "synofurb runs read-only commands over SSH: model, CPU, memory, disks, partition tables, RAID and LVM state, network cards and the kernel command line.";
        CONN_SUB: Subtitle, "Sign in over SSH. Nothing changes.", "This step only reads.", "", "";
        CONN_PORT: Label, "Port", "", "", "";
        CONN_USER: Label, "User", "An administrator account; DSM 7 admins get root through sudo.", "", "";
        CONN_SIGNIN: Label, "Sign in with", "", "", "";
        CONN_PASSWORD: Label, "Password", "", "", "";
        CONN_KEY: Label, "Key", "A private key file in OpenSSH format.", "", "";
        CONN_KEY_FILE: Label, "Key file", "Read on this computer and used for this connection only.", "", "";
        CONN_SUDO: Label, "Same for sudo", "DSM asks for the password to read disk details.", "", "";
        CONN_SUDO_PW: Label, "sudo password", "", "", "";
        CONN_BROWSE: Button, "Browse", "", "", "";
        CONN_SSH_OFF: Label, "SSH is off", "Control Panel, Terminal and SNMP, Enable SSH.", "troubleshooting", "";
        CONN_LOGIN_FAILED: Label, "Login failed", "DSM may auto-block this computer after a few failures.", "troubleshooting", "";

        // ===================================================================== D3 inspect
        INSPECT_TITLE: Title, "Inspection", "", "", "";
        INSPECT_SUB: Subtitle, "Read-only results.", "", "", "";
        INSPECT_MODEL: Label, "Model", "", "", "";
        INSPECT_CPU: Label, "CPU", "", "", "";
        INSPECT_RAM: Label, "RAM", "", "", "";
        INSPECT_FIRMWARE: Label, "Firmware", "", "", "";
        INSPECT_FLASH: Label, "Boot flash", "Synology's internal boot module; we add entries to it.", "boot-manager", "";
        INSPECT_CONSOLE: Label, "Console", "Read from DSM; a USB serial adapter also works.", "serial-console", "";
        INSPECT_BACKUPS: Label, "Backups", "synofurb does not look at DSM's backup tasks; only you can confirm a backup.", "backups", "";
        INSPECT_COMPAT: Label, "Compatibility", "How much this model is proven.", "compat-status", "";
        INSPECT_DISK: Label, "Disk", "", "", "";
        INSPECT_COL_MODEL: Label, "Model", "", "", "";
        INSPECT_COL_SERIAL: Label, "Serial", "", "", "";
        INSPECT_COL_SIZE: Label, "Size", "", "", "";
        INSPECT_COL_USE: Label, "Use", "", "", "";
        INSPECT_VOLUMES: Label, "DSM volumes", "DSM's RAID arrays and what they hold.", "", "";
        INSPECT_LAYOUT: Label, "Current layout", "DSM's partitions on each disk.", "", "";
        INSPECT_CHOOSE: Button, "Choose goal", "", "", "";
        INSPECT_BLOCKED: Label, "Cannot be converted", "Debian 13 cannot run on this hardware.", "compat-status", "";

        // ===================================================================== D4 goal
        GOAL_TITLE: Title, "Choose goal", "", "", "";
        GOAL_SUB: Subtitle, "What should happen to this NAS?", "", "", "";
        GOAL_BOOTMGR: Label, "Boot manager", "Adds a small SSH agent to the boot flash.", "boot-manager",
            "The flash changes, the disks stay untouched. The agent is one-shot and has a 30 minute watchdog.";
        GOAL_BOOTMGR_LINE: Prose, "Try Linux. Disks untouched.", "", "", "";
        GOAL_RECOMMENDED: Label, "Recommended", "No one has reported this model; try the agent first.", "compat-status", "";
        GOAL_SET_UP: Button, "Set up", "", "", "";
        GOAL_INSTALL: Label, "Install Debian", "Erases disks; needs your backup.", "first-install",
            "Wipe, or the advanced Keep mode. It takes about 25 minutes.";
        GOAL_INSTALL_LINE: Prose, "Replace DSM with Debian.", "", "", "";
        GOAL_PLAN_INSTALL: Button, "Plan install", "Debian 13 cannot run on this hardware.", "", "";
        GOAL_RECOVER: Label, "Restore / Recover", "Reads the unit's state before offering anything.", "recovery", "";
        GOAL_RECOVER_LINE: Prose, "Undo or repair.", "", "", "";
        GOAL_OPEN_RECOVERY: Button, "Open recovery", "", "", "";
        GOAL_BOOTMGR_OFF: Label, "Boot manager", "Needs an editable internal boot flash, which this unit does not have.", "boot-manager", "";

        // ===================================================================== D5 plan
        PLAN_TITLE: Title, "Storage plan", "", "", "";
        PLAN_SUB: Subtitle, "Nothing is written yet.", "", "", "";
        PLAN_MODE: Label, "Mode", "Wipe erases all {n} disks.", "backups", "";
        PLAN_WIPE: Label, "Wipe all", "", "", "";
        PLAN_KEEP: Label, "Keep volumes", "Best effort; no guarantee.", "", "";
        PLAN_ADVANCED: Button, "Advanced", "", "", "";
        PLAN_KEEP_DIALOG: Dialog, "Keep existing volumes", "Best effort; no guarantee.", "",
            "Keep mode reuses only DSM's system and swap area, about 10 GiB per disk; the system is capped near 9.5 GiB. DSM, its settings, users and packages are erased.\n\nYour data volume is left alone on a best-effort basis, with no guarantee. It is not mounted automatically; import it read-only later.";
        PLAN_USE_KEEP: Button, "Use Keep", "", "", "";
        PLAN_USE_WIPE: Button, "Use Wipe", "", "", "";
        PLAN_SYSTEM: Label, "System size", "Mirrored on every copy; {n} GiB is recommended.", "first-install", "";
        PLAN_COPIES: Label, "Mirror copies", "More copies survive more disk failures.", "recovery", "";
        PLAN_RAID: Label, "Data RAID", "Auto picks the level from the disk count.", "first-install", "";
        PLAN_FREE: Label, "Free space", "Left unallocated for growth and snapshots.", "", "";
        PLAN_DESTROYED: Label, "Destroyed", "Hover for names and sizes.", "", "";
        PLAN_CHANGED: Label, "Changed", "Hover for names and sizes.", "", "";
        PLAN_PRESERVED: Label, "Preserved", "Wipe mode preserves nothing on the disks.", "backups", "";
        PLAN_PRESERVED_KEEP: Label, "Preserved", "Best effort; no guarantee.", "backups", "";
        PLAN_NEXT: Button, "Next: System", "Fix the error first.", "", "";
        PLAN_LAYOUT_LEGEND: Label, "Layout", "Disks, partitions, RAID arrays, then LVM volumes and filesystems.", "", "";
        PLAN_FIX_SIZE: Button, "Set to {n} GiB", "", "", "";
        DIAG_DISKS: Label, "Disks", "", "", "";
        DIAG_PARTITIONS: Label, "Partitions", "", "", "";
        DIAG_RAID: Label, "RAID", "", "", "";
        DIAG_LVM: Label, "LVM", "", "", "";
        DIAG_UNTOUCHED: Label, "Not modified", "", "", "";
        PLAN_ERROR: Label, "Plan cannot be installed", "", "", "";
        PLAN_WARNING: Label, "Warning", "", "", "";

        // ===================================================================== D6 system
        SYS_TITLE: Title, "System", "", "", "";
        SYS_SUB: Subtitle, "Name, login and boot.", "", "", "";
        SYS_HOSTNAME: Label, "Hostname", "Also reachable as <name>.local.", "", "";
        SYS_USER: Label, "User", "Gets sudo; root login over SSH is off.", "", "";
        SYS_PASSWORD: Label, "Password", "Generated; copy it on the last screen.", "first-install", "";
        SYS_SSH_KEY: Label, "SSH key", "The key pair is created on this computer.", "first-install", "";
        SYS_KEY_GENERATE: Label, "Generate", "", "", "";
        SYS_KEY_PASTE: Label, "Paste", "", "", "";
        SYS_KEY_NONE: Label, "None", "", "", "";
        SYS_TIMEZONE: Label, "Time zone", "", "", "";
        SYS_CONSOLE: Label, "Console", "Read from DSM; a USB serial adapter also works.", "serial-console", "";
        SYS_WEB_UI: Label, "Web UI", "HTTPS on port 8443, self-signed certificate.", "first-install", "";
        SYS_KERNEL: Label, "Kernel", "Stable is 6.12 LTS; Newest comes from backports.", "first-install",
            "Newest falls back to Stable if it cannot be installed. Neither has run on this model.";
        SYS_STABLE: Label, "Stable", "", "", "";
        SYS_NEWEST: Label, "Newest", "", "", "";
        SYS_SOURCE: Label, "Source", "Offline uses a downloaded bundle.", "offline-mode",
            "Online needs Internet on the NAS. Offline needs a bundle for this architecture (Settings, Data).";
        SYS_ONLINE: Label, "Online", "", "", "";
        SYS_OFFLINE: Label, "Offline", "", "", "";
        SYS_BUNDLE: Label, "Bundle", "Installer files for one CPU architecture.", "offline-mode", "";
        SYS_NO_BUNDLE: Label, "No bundle", "Download one in Settings or switch to Online.", "offline-mode", "";
        SYS_DOWNLOAD: Button, "Download", "", "", "";
        SYS_BOOT_VIA: Label, "Boot via", "Debian starts through the boot flash on this unit.", "boot-manager",
            "The flash is backed up first and DSM stays the default. Units without a boot flash use the installer kit.";
        SYS_VIA_FLASH: Label, "Boot flash", "", "", "";
        SYS_VIA_KEXEC: Label, "kexec", "", "", "";
        SYS_VIA_KIT: Label, "Installer kit", "", "", "";
        SYS_NEXT: Button, "Next: Confirm", "", "", "";
        SYS_DTB: Label, "Device tree", "File name known to Debian for this board; needed on ARM units.", "serial-console", "";
        SYS_KIT_DIR: Label, "Kit folder", "Kernel, initrd and instructions are written here.", "", "";
        SYS_MORE: Label, "More options", "", "", "";
        SYS_FIRMWARE: Label, "Non-free firmware", "Needed by some network chips.", "", "";
        SYS_VERBOSE: Label, "Verbose installer", "Detailed installer logging; recommended.", "", "";
        SYS_MIRROR: Label, "Mirror", "Where the NAS downloads packages during an online install.", "", "";

        // ===================================================================== D7 confirm
        CONF_TITLE: Title, "Confirm erase", "", "", "";
        CONF_SUB: Subtitle, "Last step before changes.", "", "", "";
        CONF_DESTROYED: Label, "Destroyed", "", "", "";
        CONF_CHANGED: Label, "Changed", "", "", "";
        CONF_PRESERVED: Label, "Preserved", "Wipe mode preserves nothing on the disks.", "backups", "";
        CONF_NOTHING_PRESERVED: Label, "Nothing on the disks", "", "", "";
        CONF_DISK_WARNING: Label, "Disk warning", "{what}; consider replacing it first.", "troubleshooting", "";
        CONF_UNTESTED: Label, "Untested", "No one has reported this model; your install is the test.", "compat-status", "";
        CONF_ABOUT_TIME: Label, "About 25 min", "Keep this computer awake and the NAS online.", "first-install",
            "Stay awake and on the network.\n\nThe NAS needs Internet unless Source is Offline.\n\nUnplug USB and eSATA disks.\n\nThe first RAID sync takes about 9 hours.";
        CONF_BACKUP: Label, "Backup is tested", "", "", "";
        CONF_LOSS: Label, "I accept data loss", "", "", "";
        CONF_UNIT: Label, "This is the unit", "Compare the identity line with the label on the NAS.", "", "";
        CONF_PHRASE: Label, "Type phrase", "Case matters; pasting is fine.", "", "";
        CONF_MATCH: Label, "Matches", "", "", "";
        CONF_NO_MATCH: Label, "No match", "", "", "";
        CONF_FALLBACK: Prose, "Fallback: DSM stays default.", "", "", "";
        CONF_HOW_RECOVERY: Button, "How recovery works", "DSM is the default; failures return to it.", "recovery", "";
        CONF_ERASE: Button, "Erase and install", "", "", "";
        CONF_FLASH: Label, "Boot flash", "Entries and files are added; the whole flash is backed up first.", "backups", "";
        CONF_TWIN: Label, "Same model nearby", "Another unit of this model is on the network; compare serials.", "first-install", "";
        DLG_RECOVERY: Dialog, "How recovery works", "DSM is the default; failures return to it.", "recovery",
            "DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds.\n\nFrom step 3, a failure leaves a rescue shell open for 15 minutes, then the NAS reboots into the DSM loader, which shows Synology's install page. DSM's own system may be gone by then; reinstall it from Synology.\n\nWe promise nothing about your data.";

        // ===================================================================== D8 install
        INSTALL_TITLE: Title, "Installing", "", "", "";
        INSTALL_SUB: Subtitle, "Keep this window open.", "", "", "";
        INSTALL_PHASES: Label, "Phases", "Seven phases from connecting to verifying.", "first-install", "";
        INSTALL_FLASH: Label, "Boot flash", "Backed up and verified before the first write.", "backups", "";
        INSTALL_STEPS: Label, "Steps", "", "", "";
        INSTALL_NO_RETURN: Label, "No return", "From step 3 the first disk changes; cancel is free before.", "recovery", "";
        INSTALL_LOG: Label, "Log", "", "", "";
        INSTALL_CANCEL: Button, "Cancel", "Not possible after step 3.", "recovery", "";
        INSTALL_OPEN_LOG: Label, "Open log file", "Opens the log folder.", "", "";
        INSTALL_CANCEL_DIALOG: Dialog, "Cancel install?", "The NAS returns to DSM within 60 seconds.", "recovery", "The NAS returns to DSM within 60 seconds.";
        INSTALL_STOP: Button, "Stop install", "", "", "";
        INSTALL_KEEP_GOING: Button, "Keep going", "", "", "";
        INSTALL_CONN_LOST: Label, "Connection lost", "", "", "";
        INSTALL_RETRY: Button, "Retry", "", "", "";

        // ===================================================================== D9 failed
        FAIL_TITLE: Title, "Install stopped", "", "", "";
        FAIL_DISKS_CHANGED: Label, "Disks changed", "Repartitioned; DSM's own system may be gone.", "recovery", "";
        FAIL_DISKS_UNTOUCHED: Label, "Disks untouched", "No disk was written.", "recovery", "";
        FAIL_FLASH_CHANGED: Label, "Flash changed", "Entries are on it; the full backup is on this computer.", "backups", "";
        FAIL_FLASH_UNTOUCHED: Label, "Flash untouched", "No boot entry was written.", "backups", "";
        FAIL_RESCUE_IN: Label, "Rescue shell closes in", "Then DSM becomes the default and the NAS reboots.", "recovery", "";
        FAIL_DSM_IN: Label, "DSM returns in", "The installer's failure handler sets DSM as default and reboots.", "recovery", "";
        FAIL_OPEN_RESCUE: Button, "Open rescue shell", "", "", "";
        FAIL_WAIT_DSM: Button, "Wait for DSM", "", "", "";
        FAIL_EXPORT: Button, "Export diagnostics", "Do this before the clock ends; secrets are removed.", "troubleshooting", "";
        FAIL_RECOVERY: Button, "Recovery options", "", "", "";
        FAIL_NEXT: Button, "What happens next", "Order of actions and what survives.", "recovery", "";
        FAIL_LOG: Label, "Log excerpt", "", "", "";
        DLG_NEXT_PRE: Dialog, "What happens next", "", "recovery",
            "No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before.\n\nIf DSM is not back after 10 minutes, power-cycle the NAS. Our entries stay on the boot flash until you remove them under Recovery.";
        DLG_NEXT_WIPE: Dialog, "What happens next", "", "recovery",
            "The first disk was changed and DSM's own system may be gone. Export diagnostics first, then open the rescue shell.\n\nAt zero DSM becomes the default and the NAS reboots into the DSM loader, which shows Synology's install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep.";
        DLG_NEXT_KEEP: Dialog, "What happens next", "", "recovery",
            "The first disk was changed. Export diagnostics, then open the rescue shell.\n\nAt zero the NAS reboots into the DSM loader and Synology's install page. Your data volume was not part of any write command and should be intact unless the failure was a hardware problem.\n\nSynology documents a reinstall that can keep data volumes; check its current documentation. We cannot guarantee it.";

        // ===================================================================== D10 done
        DONE_TITLE: Title, "Done", "", "", "";
        DONE_SUB: Subtitle, "Debian is running.", "", "", "";
        DONE_ADDRESS: Label, "Address", "", "", "";
        DONE_USER: Label, "User", "", "", "";
        DONE_PASSWORD: Label, "Password", "Copy it now; it is not stored or shown again.", "first-install", "";
        DONE_SSH: Label, "SSH", "", "", "";
        DONE_HOST_KEY: Label, "Host key", "Compare the full value when the browser warns.", "troubleshooting", "";
        DONE_WEB_UI: Label, "Web UI", "HTTPS on port 8443, self-signed certificate.", "first-install", "";
        DONE_OPEN: Button, "Open", "", "", "";
        DONE_COPIED: Label, "Password copied", "", "", "";
        DONE_FINISH: Button, "Finish", "Copy the password first.", "", "";
        DONE_REPORT: Button, "Report result", "Opt-in; nothing is sent without your click.", "report-result", "";
        DONE_NEXT_STEPS: Label, "Next steps", "Open the web UI; replace warned disks; check BIOS boot order.", "first-install", "";
        DONE_SAVE: Button, "Save logins", "Writes the logins to a file only you can read.", "", "";
        DONE_ARM_ACTION: Label, "Action needed", "Read from this unit; not verified on real boards.", "serial-console", "";
        DONE_SHOW_COMMANDS: Button, "Show commands", "", "", "";
        DONE_UNREACHABLE: Title, "Unreachable", "", "troubleshooting", "";
        DLG_REPORT: Dialog, "Report a result", "Opt-in; nothing is sent without your click.", "report-result",
            "Opens a pre-filled GitHub issue with: model, DSM version, synofurb version, result, boot method and a log excerpt without secrets.\n\nThe serial number is left out. Nothing is sent without your click.";
        BTN_OPEN_ISSUE: Button, "Open issue", "", "", "";

        // ===================================================================== D11 update
        UPD_BANNER: Label, "Update available", "Never offered while an install is running.", "troubleshooting", "";
        UPD_DETAILS: Button, "Details", "", "", "";
        UPD_LATER: Button, "Later", "", "", "";
        UPD_DOWNLOAD: Button, "Download", "", "", "";
        UPD_RESTART: Button, "Restart and update", "Waiting for verification or for the install to finish.", "", "";
        UPD_SKIP: Button, "Skip version", "", "", "";
        UPD_SIGNATURE: Label, "Signature", "Checked with the project key and SHA-256 before use.", "troubleshooting", "";
        UPD_DISCARDED: Label, "Update discarded", "", "", "";

        // ===================================================================== D12 settings
        SET_TITLE: Title, "Settings", "", "", "";
        SET_GENERAL: Label, "General", "", "", "";
        SET_UPDATES: Label, "Updates", "", "", "";
        SET_DATA: Label, "Data", "", "", "";
        SET_LOGS: Label, "Logs", "", "", "";
        SET_PRIVACY: Label, "Privacy", "", "", "";
        SET_THEME: Label, "Theme", "", "", "";
        SET_SYSTEM: Label, "System", "", "", "";
        SET_LIGHT: Label, "Light", "", "", "";
        SET_DARK: Label, "Dark", "", "", "";
        SET_TEXT_SIZE: Label, "Text size", "Scales the whole window.", "", "";
        SET_DEFAULT: Label, "Default", "", "", "";
        SET_LARGE: Label, "Large", "", "", "";
        SET_LARGEST: Label, "Largest", "", "", "";
        SET_CHANNEL: Label, "Channel", "Beta also offers pre-releases.", "", "";
        SET_STABLE: Label, "Stable", "", "", "";
        SET_BETA: Label, "Beta", "", "", "";
        SET_AUTO_UPDATE: Label, "Auto-update", "Never installs while a conversion is running.", "troubleshooting", "";
        SET_COMPAT_LIST: Label, "Compatibility list", "Fetched from the project repository; works offline from the cache.", "compat-status", "";
        SET_REFRESH: Button, "Refresh", "", "", "";
        SET_BUNDLE: Label, "Offline bundle", "Installer files for one CPU architecture.", "offline-mode", "";
        SET_BUNDLE_DOWNLOAD: Button, "Download", "", "", "";
        SET_LOG_FOLDER: Label, "Log folder", "", "", "";
        SET_OPEN: Button, "Open", "", "", "";
        SET_SEND_RESULTS: Label, "Send results", "Asks before every report; serial numbers are left out.", "report-result", "";
        SET_MOTION: Label, "Reduce motion", "Shows a static glyph instead of the spinner.", "", "";
        SET_NONE: Label, "None", "", "", "";

        // ===================================================================== D13 boot manager
        BM_TITLE: Title, "Boot manager", "It resets the default to DSM when it starts.", "boot-manager",
            "kexec starts Linux kernels only. The agent is one-shot: it resets the default to DSM when it starts.";
        BM_SUB: Subtitle, "SSH agent on the boot flash.", "", "", "";
        BM_FLASH_CHANGES: Label, "Flash changes", "Entries and files are added; DSM's entry stays.", "boot-manager",
            "The whole flash is saved to this computer first and verified.";
        BM_DISKS_UNTOUCHED: Label, "Disks untouched", "No partition table is written.", "boot-manager", "";
        BM_SSH_KEY: Label, "SSH key", "The key pair is created on this computer.", "first-install", "";
        BM_LOGIN_TIMEOUT: Label, "Login timeout", "The agent reboots to DSM after 30 minutes without a login.", "boot-manager", "";
        BM_OTHER_OS: Prose, "Other OS: power cycle returns.", "After kexec nothing of ours runs; power-cycling returns to the default.", "boot-manager", "";
        BM_INSTALL: Button, "Install agent", "", "", "";
        BM_NOT_EDITABLE: Label, "Flash not editable", "Needs an editable internal boot flash (DOM).", "boot-manager", "";

        // ===================================================================== D14 recovery
        REC_TITLE: Title, "Recovery", "", "", "";
        REC_SUB: Subtitle, "State read from the unit.", "", "", "";
        REC_BOOT_DSM: Label, "Boot DSM next", "Sets the default boot entry back to DSM.", "recovery", "";
        REC_OPEN_AGENT: Label, "Open agent", "Boots the SSH agent once; DSM stays the way back.", "boot-manager", "";
        REC_RESTORE_FLASH: Label, "Restore flash", "Writes the saved boot-flash image back; needs a working Linux on the unit.", "recovery", "";
        REC_REMOVE: Label, "Remove our entries", "Takes the agent out of the boot flash again.", "recovery", "";
        REC_REINSTALL: Label, "Reinstall DSM", "Use Synology's page; no data promise.", "recovery", "";
        REC_RUN: Button, "Run", "", "", "";
        REC_SAVED: Label, "Saved here", "First and last 1 MiB of changed disks; not your files.", "backups", "";
        REC_OPEN_FOLDER: Button, "Open folder", "", "", "";
        REC_UNAVAILABLE: Label, "Unavailable", "", "", "";
        DLG_REINSTALL: Dialog, "Reinstall DSM", "Use Synology's page; no data promise.", "recovery",
            "The NAS boots Synology's loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model.\n\nSynology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it.";
        BTN_OPEN_SYNOLOGY: Button, "Open find.synology.com", "", "", "";

        // ===================================================================== D15 manage
        MAN_SUB: Subtitle, "Running Debian 13.", "", "", "";
        MAN_OVERVIEW: Label, "Overview", "", "", "";
        MAN_LOGINS: Label, "Logins", "", "", "";
        MAN_RECOVERY: Label, "Recovery", "", "", "";
        MAN_HISTORY: Label, "History", "", "", "";
        MAN_ADDRESS: Label, "Address", "", "", "";
        MAN_DEFAULT_BOOT: Label, "Default boot", "Debian; the DSM loader entry remains.", "recovery", "";
        MAN_WEB_UI: Label, "Web UI", "HTTPS on port 8443, self-signed certificate.", "first-install", "";
        MAN_OPEN_WEB: Button, "Open web UI", "", "", "";
        MAN_DIAG: Button, "Diagnostics", "Copies version, system and paths; no secrets.", "troubleshooting", "";
        MAN_SSH_CMD: Label, "SSH command", "", "", "";
        MAN_PW_NOT_KEPT: Label, "Password not kept", "synofurb does not store it; the NAS keeps a hash.", "first-install", "";
        MAN_NO_HISTORY: Label, "No history", "", "", "";

        // ===================================================================== D16 help
        HELP_TITLE: Title, "Help", "", "", "";
        HELP_SEARCH: Label, "Search help", "", "", "";
        HELP_TOPICS: Label, "Topics", "", "", "";
        HELP_SHORTCUTS: Label, "Shortcuts", "", "", "";
        HELP_DOCS: Button, "Open docs", "Needs Internet.", "", "";
        HELP_NO_TOPICS: Label, "No topics match", "", "", "";
        HELP_MORE: Button, "More", "Opens the full page in your browser.", "", "";

        // ===================================================================== D17 about
        ABOUT_TITLE: Title, "About", "", "", "";
        ABOUT_TAGLINE: Prose, "Reinstall Synology NAS units with Debian.", "", "", "";
        ABOUT_COPY_DIAG: Button, "Copy diagnostics", "Version, OS, settings; no secrets.", "troubleshooting", "";
        ABOUT_DOCS: Label, "Documentation", "", "", "";
        ABOUT_SOURCE: Label, "Source code", "", "", "";
        ABOUT_REPORT: Label, "Report issue", "", "", "";
        ABOUT_CREDITS: Label, "Credits", "", "", "";
        ABOUT_SYNODEB: Label, "synodeb", "Boot-flash technique documented by synodeb (GPL-2.0).", "",
            "The boot-flash technique is documented by the synodeb project (GPL-2.0). synofurb contains none of its code and is MIT licensed.";
        ABOUT_LICENCE: Label, "Licence", "", "", "";

        // ===================================================================== Help topics (bodies: markdown, at most 150 words)
        T_FIRST_INSTALL: Topic, "First install", "Scan, connect, inspect, plan, confirm.", "",
            "1. Scan, pick your unit and connect. Enable SSH in DSM first (Control Panel, Terminal and SNMP).\n2. Inspect is read-only.\n3. Choose Boot manager to try the agent, or Install Debian.\n4. Plan storage, set the system options, then confirm.\n\nThe install takes about 25 minutes. Kernel: Stable is Debian's 6.12 LTS; Newest is 7.x from trixie-backports and falls back to Stable if it cannot be installed. Source: Online needs Internet on the NAS; Offline uses a bundle you downloaded in Settings. Afterwards the data array syncs for about 9 hours; the NAS works meanwhile with reduced protection. Copy the generated password on the last screen: synofurb does not keep it.";
        T_COMPAT_STATUS: Topic, "Status meanings", "What Verified, Untested and the others mean.", "",
            "Each model has one status.\n\n**Verified**: maintainers or two independent people installed it, with logs. No model has this yet.\n\n**Community-reported**: at least one report exists. DS923+ has one, from another project.\n\n**Untested**: no report. The hardware class should work in theory; that is not a promise, and your install is the test.\n\n**Known issues**: reports show something fails or only partly works.\n\n**Unsupported**: Debian 13 cannot run on this hardware; synofurb refuses to install.\n\nOnly Unsupported blocks. The list ships with the app and refreshes from the project repository; offline it uses the cached copy.";
        T_BACKUPS: Topic, "Backups", "Your backup and what synofurb saves.", "",
            "Your own backup is essential. In Wipe mode nothing on the disks is preserved; in Keep mode the data volume is left alone on a best-effort basis with no guarantee.\n\nsynofurb also saves on this computer:\n\n- **Boot-flash image**: the whole flash, hash-verified, before the first flash write. Keep it off the NAS.\n- **Disk metadata backup**: the first and last 1 MiB of every disk we change (partition tables, GPT headers). That is metadata only, not your files, and it cannot bring back a formatted filesystem.\n- The plan, the install script and the log.";
        T_RECOVERY: Topic, "Recovery", "What returns the unit to DSM, by phase.", "",
            "DSM stays the default boot entry; every failure points it back at DSM.\n\n- **Before the boot flash is changed**: nothing to undo.\n- **Agent installed**: the agent resets the default to DSM first; it reboots to DSM if nobody logs in.\n- **Before step 3**: a failure reboots to DSM after 60 s; a power cycle also works.\n- **After step 3**: rescue shell for 15 minutes, then the DSM loader runs; DSM's own system may be gone, reinstall it from Synology (find.synology.com); no data promise.\n- **Debian installed**: pick the previous kernel in the boot menu or boot DSM.\n\nCommands: `synofurb recover`, `synofurb boot-manager enter`.";
        T_BOOT_MANAGER: Topic, "Boot manager", "A one-shot SSH agent on the boot flash.", "",
            "x86 units boot through a small internal boot flash. Boot manager adds an SSH agent entry to it. The whole flash is saved to this computer first; disks are untouched; DSM's entry stays and stays the default.\n\nThe agent is one-shot: it resets the default to DSM when it starts and reboots to DSM after 30 minutes without a login. From it, `sf-boot kexec <kernel-url> <initrd-url>` starts another Linux kernel. kexec starts Linux kernels only, not Windows or UEFI-only ISOs. Once another system runs, nothing of ours is running: power-cycling returns to the default entry. That system can see and change your disks.";
        T_SERIAL_CONSOLE: Topic, "Serial console", "Boot messages without a network.", "",
            "A serial console shows boot messages and lets you log in without a network. Intel units usually use `ttyS0` at 115200 8N1; the app reads the setting from DSM's kernel command line. AMD units have a memory-mapped UART; the exact console argument is copied from DSM. ARM boards need a 3.3 V TTL serial adapter (not RS-232 levels); without it you cannot recover an ARM unit over SSH alone. Debian also opens a login on any USB serial adapter you plug in later.";
        T_OFFLINE_MODE: Topic, "Offline mode", "Install from a downloaded bundle.", "",
            "Offline installs use a bundle: the installer files and packages for one CPU architecture. Download it in Settings, Data (this computer needs Internet once). In System, choose Source: Offline and select the bundle. The NAS then needs no Internet during the install. The compatibility list also works offline from its cached copy. Update checks, list refresh and result reports need Internet and are disabled while offline.";
        T_CLI: Topic, "Command line", "Everything in the app is also a command.", "",
            "Everything in the app is also a command, and the command line is the screen-reader accessible path (the app window does not support screen readers).\n\n`synofurb scan` discovers units.\n\n`synofurb plan --host <ip> --user <admin>` shows the plan, read-only.\n\n`synofurb install ... --have-backup --accept-data-loss --confirm` converts a unit.\n\n`synofurb recover` and `synofurb boot-manager enter` handle recovery.\n\nThe Copy command button on Plan, System and Confirm copies the equivalent command. On Confirm the three confirmation flags are included only after you confirm.";
        T_TROUBLESHOOTING: Topic, "Troubleshooting", "No units, SSH off, login failed, installer stuck.", "",
            "**No units found**: wrong network or VLAN, NAS still booting, firewall (UDP 9999, mDNS 5353, TCP 22). Use Add by IP.\n\n**SSH is off**: enable it in DSM, Control Panel, Terminal and SNMP.\n\n**Login failed**: DSM may auto-block this computer after a few failures; check the account is an administrator.\n\n**Installer did not come up**: DSM is already the default; wait, then power-cycle.\n\n**Boot flash write failed**: the tool restores the flash and verifies it; use the rescue shell within 15 minutes.\n\nLogs: `~/.local/share/synofurb/logs` (secrets removed). Settings, Logs opens the folder; About copies diagnostics.";
        T_REPORT_RESULT: Topic, "Report a result", "How a model moves from Untested to Verified.", "",
            "Reports move a model from Untested towards Verified. After an install, press Report result on the Done screen. It opens a pre-filled GitHub issue (compat report template) with the model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click, and Settings, Privacy has an off switch. A maintainer checks the log and changes the status through a reviewed pull request.";
    }
}

/// Help topics in display order, with the docs page each one links to.
pub static TOPICS: &[(&str, &Item, &str)] = &[
    ("first-install", &T_FIRST_INSTALL, "docs/INSTALL.md"),
    ("compat-status", &T_COMPAT_STATUS, "docs/COMPAT.md"),
    ("backups", &T_BACKUPS, "docs/RECOVERY.md"),
    ("recovery", &T_RECOVERY, "docs/RECOVERY.md"),
    ("boot-manager", &T_BOOT_MANAGER, "docs/BOOT.md"),
    ("serial-console", &T_SERIAL_CONSOLE, "docs/BOOT.md"),
    ("offline-mode", &T_OFFLINE_MODE, "docs/OFFLINE.md"),
    ("cli", &T_CLI, "docs/CLI.md"),
    ("troubleshooting", &T_TROUBLESHOOTING, "docs/RECOVERY.md"),
    ("report-result", &T_REPORT_RESULT, "docs/COMPAT.md"),
];

/// The topic with this id.
pub fn topic(id: &str) -> Option<(&'static Item, &'static str)> {
    TOPICS.iter().find(|(i, _, _)| *i == id).map(|(_, it, docs)| (*it, *docs))
}

/// Topics matching a search query (titles, summaries and bodies); all of them for an empty query.
pub fn search_topics(query: &str) -> Vec<&'static (&'static str, &'static Item, &'static str)> {
    TOPICS.iter().filter(|(_, it, _)| query.trim().is_empty() || it.matches(query)).collect()
}

/// `(keystroke in GPUI syntax, action)` of the Shortcuts tab; the keys are drawn by the `Kbd` widget.
pub static SHORTCUTS: &[(&str, &str)] = &[
    ("f1", "Help for this screen"),
    ("?", "Help (outside text fields)"),
    ("ctrl-,", "Settings"),
    ("ctrl-f", "Search (inventory filter, log, help)"),
    ("ctrl-shift-c", "Copy command"),
    ("alt-left", "Back"),
    ("enter", "Main action (never Erase)"),
    ("escape", "Close popovers and dialogs"),
];

/// The shell around every wizard screen: stepper, risk tag, way back, wordmark (14 words).
pub static SHELL: &[&Item] = &[
    &APP_NAME,
    &STEP_FIND,
    &STEP_CONNECT,
    &STEP_INSPECT,
    &STEP_GOAL,
    &STEP_PLAN,
    &STEP_SYSTEM,
    &STEP_CONFIRM,
    &STEP_INSTALL,
    &STEP_DONE,
    &RISK_FLASH_CHANGED,
    &WAY_BACK,
];

/// Items each screen shows without a click (UX spec section 5; conditional elements are left out).
pub static SCREENS: &[ScreenCopy] = &[
    ScreenCopy {
        name: "inventory",
        visible: &[
            &INV_TITLE,
            &INV_SUB,
            &INV_FILTER,
            &INV_RESCAN,
            &INV_ADD,
            &INV_COL_MODEL,
            &INV_COL_ADDR,
            &INV_COL_DSM,
            &INV_COL_FOUND,
            &INV_COL_STATUS,
            &INV_RUNNING,
            &INV_MANAGE,
            &INV_CONNECT,
        ],
    },
    ScreenCopy {
        name: "connect",
        visible: &[
            &CONN_TITLE,
            &CONN_SUB,
            &CONN_PORT,
            &CONN_USER,
            &CONN_SIGNIN,
            &CONN_PASSWORD,
            &CONN_KEY,
            &CONN_PASSWORD,
            &CONN_SUDO,
            &CONN_SUDO,
            &INV_CONNECT,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "inspect",
        visible: &[
            &INSPECT_TITLE,
            &INSPECT_SUB,
            &INSPECT_MODEL,
            &INSPECT_CPU,
            &INSPECT_RAM,
            &INSPECT_FIRMWARE,
            &INSPECT_FLASH,
            &INSPECT_CONSOLE,
            &INSPECT_BACKUPS,
            &INSPECT_DISK,
            &INSPECT_COL_MODEL,
            &INSPECT_COL_SIZE,
            &INSPECT_COL_USE,
            &INSPECT_VOLUMES,
            &INSPECT_LAYOUT,
            &INSPECT_CHOOSE,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "goal",
        visible: &[
            &GOAL_TITLE,
            &GOAL_SUB,
            &GOAL_BOOTMGR,
            &GOAL_RECOMMENDED,
            &GOAL_BOOTMGR_LINE,
            &GOAL_SET_UP,
            &GOAL_INSTALL,
            &GOAL_INSTALL_LINE,
            &GOAL_PLAN_INSTALL,
            &GOAL_RECOVER,
            &GOAL_RECOVER_LINE,
            &GOAL_OPEN_RECOVERY,
        ],
    },
    ScreenCopy {
        name: "plan",
        visible: &[
            &PLAN_TITLE,
            &PLAN_SUB,
            &PLAN_MODE,
            &PLAN_WIPE,
            &PLAN_ADVANCED,
            &PLAN_SYSTEM,
            &PLAN_COPIES,
            &PLAN_RAID,
            &PLAN_FREE,
            &PLAN_DESTROYED,
            &PLAN_CHANGED,
            &PLAN_PRESERVED,
            &PLAN_NEXT,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "system",
        visible: &[
            &SYS_TITLE,
            &SYS_SUB,
            &SYS_HOSTNAME,
            &SYS_USER,
            &SYS_PASSWORD,
            &SYS_SSH_KEY,
            &SYS_KEY_GENERATE,
            &SYS_KEY_PASTE,
            &SYS_KEY_NONE,
            &SYS_TIMEZONE,
            &SYS_CONSOLE,
            &SYS_WEB_UI,
            &SYS_KERNEL,
            &SYS_STABLE,
            &SYS_NEWEST,
            &SYS_SOURCE,
            &SYS_ONLINE,
            &SYS_OFFLINE,
            &SYS_BOOT_VIA,
            &SYS_VIA_FLASH,
            &SYS_NEXT,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "confirm",
        visible: &[
            &CONF_TITLE,
            &CONF_SUB,
            &CONF_DESTROYED,
            &CONF_CHANGED,
            &CONF_PRESERVED,
            &CONF_DISK_WARNING,
            &CONF_UNTESTED,
            &CONF_ABOUT_TIME,
            &CONF_BACKUP,
            &CONF_LOSS,
            &CONF_UNIT,
            &CONF_PHRASE,
            &CONF_FALLBACK,
            &CONF_HOW_RECOVERY,
            &CONF_ERASE,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "install",
        visible: &[
            &INSTALL_TITLE,
            &INSTALL_SUB,
            &INSTALL_PHASES,
            &INSTALL_FLASH,
            &INSTALL_STEPS,
            &INSTALL_NO_RETURN,
            &INSTALL_LOG,
            &INSTALL_CANCEL,
        ],
    },
    ScreenCopy {
        name: "failed",
        visible: &[
            &FAIL_TITLE,
            &FAIL_DISKS_CHANGED,
            &FAIL_FLASH_CHANGED,
            &FAIL_RESCUE_IN,
            &FAIL_OPEN_RESCUE,
            &FAIL_EXPORT,
            &FAIL_RECOVERY,
            &FAIL_NEXT,
            &FAIL_LOG,
        ],
    },
    ScreenCopy {
        name: "done",
        visible: &[
            &DONE_TITLE,
            &DONE_SUB,
            &DONE_ADDRESS,
            &DONE_USER,
            &DONE_PASSWORD,
            &DONE_SSH,
            &DONE_HOST_KEY,
            &DONE_WEB_UI,
            &DONE_OPEN,
            &DONE_COPIED,
            &DONE_FINISH,
            &DONE_REPORT,
            &DONE_NEXT_STEPS,
        ],
    },
    ScreenCopy { name: "update", visible: &[&UPD_BANNER, &UPD_DETAILS, &UPD_LATER, &UPD_DOWNLOAD, &UPD_RESTART, &UPD_SKIP] },
    ScreenCopy {
        name: "settings",
        visible: &[
            &SET_TITLE,
            &SET_GENERAL,
            &SET_UPDATES,
            &SET_DATA,
            &SET_LOGS,
            &SET_PRIVACY,
            &SET_THEME,
            &SET_SYSTEM,
            &SET_LIGHT,
            &SET_DARK,
            &SET_TEXT_SIZE,
            &SET_DEFAULT,
            &SET_LARGE,
            &SET_LARGEST,
            &SET_CHANNEL,
            &SET_AUTO_UPDATE,
            &SET_COMPAT_LIST,
            &SET_REFRESH,
            &SET_BUNDLE,
            &SET_BUNDLE_DOWNLOAD,
            &SET_LOG_FOLDER,
            &SET_OPEN,
            &SET_SEND_RESULTS,
        ],
    },
    ScreenCopy {
        name: "bootmgr",
        visible: &[
            &BM_TITLE,
            &BM_SUB,
            &BM_FLASH_CHANGES,
            &BM_DISKS_UNTOUCHED,
            &BM_SSH_KEY,
            &SYS_KEY_GENERATE,
            &SYS_KEY_PASTE,
            &BM_LOGIN_TIMEOUT,
            &BM_OTHER_OS,
            &BM_INSTALL,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "recover",
        visible: &[
            &REC_TITLE,
            &REC_SUB,
            &REC_BOOT_DSM,
            &REC_OPEN_AGENT,
            &REC_RESTORE_FLASH,
            &REC_REMOVE,
            &REC_REINSTALL,
            &REC_RUN,
            &REC_SAVED,
            &REC_OPEN_FOLDER,
        ],
    },
    ScreenCopy {
        name: "manage",
        visible: &[
            &MAN_SUB,
            &MAN_OVERVIEW,
            &MAN_LOGINS,
            &MAN_RECOVERY,
            &MAN_HISTORY,
            &MAN_ADDRESS,
            &MAN_DEFAULT_BOOT,
            &MAN_WEB_UI,
            &MAN_OPEN_WEB,
            &MAN_DIAG,
            &MAN_SSH_CMD,
            &MAN_PW_NOT_KEPT,
        ],
    },
    ScreenCopy { name: "help", visible: &[&HELP_TITLE, &HELP_SEARCH, &HELP_TOPICS, &HELP_SHORTCUTS, &HELP_DOCS, &BTN_BACK] },
    ScreenCopy {
        name: "about",
        visible: &[&ABOUT_TITLE, &ABOUT_TAGLINE, &ABOUT_COPY_DIAG, &ABOUT_DOCS, &ABOUT_SOURCE, &ABOUT_REPORT, &ABOUT_CREDITS],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_placeholders_in_order() {
        assert_eq!(fill("{a} of {b} done", &["2", "5"]), "2 of 5 done");
        assert_eq!(fill("no placeholder", &["x"]), "no placeholder");
        assert_eq!(fill("{a} {b}", &["1"]), "1 ");
    }

    #[test]
    fn word_counter_ignores_placeholders_and_symbols() {
        assert_eq!(words("Plan valid, {n} warnings - fine"), 4);
        assert_eq!(words("  "), 0);
        assert_eq!(words("Back"), 1);
        assert_eq!(words("Next: System"), 2);
    }

    #[test]
    fn sentence_counter() {
        assert_eq!(sentences("One sentence only."), 1);
        assert_eq!(sentences("One. Two."), 2);
        assert_eq!(sentences("Version 1.2.3 is out"), 0);
        assert_eq!(sentences("Rescue shell 15 min, then the DSM loader; no data promise."), 1);
    }

    #[test]
    fn every_item_stays_inside_its_word_budget() {
        for it in ALL {
            let n = words(it.short);
            let max = match it.kind {
                Kind::Title => 4,
                Kind::Subtitle => 12,
                Kind::Label => 4,
                Kind::Button => 3,
                Kind::Prose => 6,
                Kind::Topic => 4,
                Kind::Dialog => 4,
            };
            assert!(n <= max, "{}: {:?} has {n} words (max {max} for {:?})", it.key, it.short, it.kind);
            if !it.tip.is_empty() {
                assert!(words(it.tip) <= 24, "{}: tooltip has {} words (max 24)", it.key, words(it.tip));
                assert_eq!(sentences(it.tip), 1, "{}: a tooltip is exactly one sentence: {:?}", it.key, it.tip);
            }
            let body_max = match it.kind {
                Kind::Topic => 150,
                Kind::Dialog => 90,
                _ => 40,
            };
            assert!(words(it.body) <= body_max, "{}: body has {} words (max {body_max})", it.key, words(it.body));
            if it.kind == Kind::Topic {
                assert!(it.short.len() > 2 && !it.body.is_empty(), "{}: a topic needs a title and a body", it.key);
            }
        }
    }

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<&str> = ALL.iter().map(|i| i.key).collect();
        keys.sort_unstable();
        let n = keys.len();
        keys.dedup();
        assert_eq!(n, keys.len(), "duplicate copy keys");
    }

    #[test]
    fn every_help_id_exists() {
        for it in ALL {
            if !it.topic.is_empty() {
                assert!(topic(it.topic).is_some(), "{}: unknown help topic {:?}", it.key, it.topic);
            }
        }
        assert_eq!(TOPICS.len(), 10);
        for (id, it, docs) in TOPICS {
            assert_eq!(it.kind, Kind::Topic, "{id}");
            assert!(docs.starts_with("docs/"), "{id}");
        }
    }

    #[test]
    fn shell_is_fourteen_words() {
        let n: usize = SHELL.iter().map(|i| words(i.short)).sum();
        assert_eq!(n, 14, "the UX spec counts 14 words of shell on every wizard screen");
        // the risk tag is at most two words in every state
        for r in [&RISK_READ_ONLY, &RISK_PLAN_ONLY, &RISK_FLASH_CHANGED, &RISK_DISKS_CHANGED, &RISK_FAILED_SAFELY, &RISK_DEBIAN_RUNNING] {
            assert!(words(r.short) <= 2, "{}", r.key);
        }
    }

    #[test]
    fn no_screen_exceeds_sixty_words_of_static_prose() {
        let shell: usize = SHELL.iter().map(|i| words(i.short)).sum();
        for s in SCREENS {
            let mut seen: Vec<&str> = vec![];
            let own: usize = s
                .visible
                .iter()
                .filter(|i| {
                    // an item that appears twice (two buttons with the same label) is counted once, like the spec does
                    let fresh = !seen.contains(&i.key);
                    seen.push(i.key);
                    fresh
                })
                .map(|i| words(i.short))
                .sum();
            let side_page = matches!(s.name, "settings" | "about" | "help" | "manage" | "update");
            let total = own + if side_page { 0 } else { shell };
            assert!(total <= 60, "screen {}: {total} words of static prose (own {own}, shell {shell}; max 60)", s.name);
            let titles = s.visible.iter().filter(|i| i.kind == Kind::Title).count();
            assert!(titles <= 1, "screen {}: more than one title", s.name);
        }
    }

    #[test]
    fn every_screen_declares_copy() {
        for n in [
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
            "manage",
            "help",
            "about",
        ] {
            assert!(SCREENS.iter().any(|s| s.name == n), "screen {n} has no copy declaration");
        }
    }

    #[test]
    fn help_search_finds_titles_and_bodies() {
        assert_eq!(search_topics("").len(), 10);
        assert!(search_topics("rescue shell").iter().any(|(id, _, _)| *id == "recovery"));
        assert!(search_topics("kexec").iter().any(|(id, _, _)| *id == "boot-manager"));
        assert!(search_topics("zzzzqq").is_empty());
        assert!(search_topics("FIRST install").iter().any(|(id, _, _)| *id == "first-install"));
    }
}
