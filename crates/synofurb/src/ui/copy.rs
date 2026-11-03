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
        STEP_SETUP: Label, "Setup", "", "", "";
        STEP_SYSTEM: Label, "System", "", "", "";
        STEP_CONFIRM: Label, "Confirm", "", "", "";
        STEP_INSTALL: Label, "Install", "", "", "";
        STEP_DONE: Label, "Done", "", "", "";
        RISK_READ_ONLY: Label, "Read-only", "", "recovery", "Nothing to undo.";
        RISK_PLAN_ONLY: Label, "Plan only", "", "recovery", "Nothing to undo.";
        RISK_FLASH_CHANGED: Label, "Flash changed", "", "recovery", "Power-cycle or Cancel: DSM boots. Our entries stay until removed.";
        RISK_DISKS_CHANGED: Label, "Disks changed", "", "recovery", "Read the log, then the DSM loader; no data promise.";
        RISK_FAILED_SAFELY: Label, "Failed safely", "", "recovery", "DSM boots again by itself.";
        RISK_DEBIAN_RUNNING: Label, "Debian running", "", "recovery", "Recovery stays under Manage.";
        RISK_BOOT_UNCONFIRMED: Label, "Boot unconfirmed", "", "recovery", "DSM may not return by itself: see Help, Recovery.";
        RISK_NEEDS_ATTENTION: Label, "Needs attention", "", "recovery", "Recovery stays under Manage. Repair the flagged item first.";
        RISK_CRITICAL: Label, "Critical", "", "recovery", "Recovery stays under Manage. Replace the failed disk soon.";
        RISK_UNREACHABLE: Label, "Unreachable", "", "recovery", "Check power and network; Recovery stays under Manage.";
        WAY_BACK: Button, "Way back", "What returns the unit to DSM from here.", "recovery", "";
        TAG_DEMO: Label, "Demo", "Built-in sample unit; nothing touches the network or any disk.", "", "";
        TAG_OFFLINE: Label, "Offline", "Compatibility list is {n} days old; LAN scan still works.", "offline-mode", "";
        MENU_HELP: Label, "Help", "Help for this screen.", "", "";
        MENU_SETTINGS: Label, "Settings", "Theme, text size, updates, data.", "", "";
        MENU_SETTINGS_UPDATE: Label, "Settings, update available", "An update is waiting: see Settings, Updates.", "", "";
        MENU_ABOUT: Label, "About", "Version, credits, diagnostics.", "", "";
        MENU_UPDATES: Label, "Check for updates", "Looks for a newer synofurb.", "", "";

        // ===================================================================== common
        BTN_BACK: Button, "Back", "", "", "";
        BTN_CLOSE: Button, "Close", "", "", "";
        BTN_CANCEL: Button, "Cancel", "", "", "";
        BTN_OPEN_HELP: Button, "Open in Help", "", "", "";
        BTN_COPY_CMD: Button, "Copy command", "Copies the equivalent synofurb command.", "cli", "";
        BTN_COPY_CMD_CONFIRM: Button, "Copy command", "Includes the confirmation flags only after you confirm here.", "cli", "";
        DLG_COMMAND: Dialog, "Copy command", "Includes the confirmation flags only after you confirm here.", "cli", "Edit the line if you like: the tag says whether it still matches the plan.";
        CMD_MATCHES: Label, "Matches plan", "Every choice of the plan is on the line, with the same value.", "cli", "";
        CMD_DIFFERS: Label, "Differs from plan", "A choice is missing, extra or different.", "cli", "";
        CMD_READ_ONLY: Label, "Plan form", "Read-only: it shows the plan and changes nothing.", "cli", "";
        BTN_COPY: Button, "Copy", "", "", "";
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
        INV_ADD_HINT: Label, "IP or host name", "", "", "";
        INV_ADD_CHECKING: Label, "Checking {}", "", "", "";
        ADD_ERR_EMPTY: Prose, "Enter an IP address or a host name.", "", "", "";
        ADD_ERR_CHARS: Prose, "Use one address, without spaces or other characters.", "", "", "";
        ADD_ERR_IP: Prose, "That is not a valid IPv4 or IPv6 address.", "", "", "";
        ADD_ERR_PORT: Prose, "The port must be a number from 1 to 65535.", "", "", "";
        ADD_ERR_NAME: Prose, "That is not a valid host name.", "", "", "";
        ADD_ERR_NOT_HOST: Prose, "No NAS can have that address.", "", "", "";
        ADD_ERR_THIS: Prose, "That is this computer, not a NAS.", "", "", "";
        INV_ADDED: Label, "Added by you", "", "", "";
        INV_UNKNOWN: Label, "Unknown device", "Something answered at this address, but nothing says it is a Synology unit.", "", "";
        INV_UNKNOWN_TAG: Label, "Not recognised", "", "", "";
        INV_UNREACH: Label, "Unreachable", "Nothing answered at this address.", "", "";
        INV_UNREACH_TAG: Label, "No answer", "", "", "";
        INV_CHECK_AGAIN: Button, "Check again", "Look at this address again.", "", "";
        INV_REMOVE: Button, "Remove", "Remove this address from the list.", "", "";
        INV_CHECKED: Label, "What was checked", "", "", "";
        INV_UNKNOWN_WARN: Prose, "Not recognised as a Synology NAS. Check the address first.", "", "", "";
        INV_UNREACH_WHY: Prose, "Nothing answered at this address. Use Check again, or remove it.", "", "", "";
        NET_CHIP: Button, "Networks: {} of {}", "Choose which networks the scan covers.", "", "";
        NET_TITLE: Dialog, "Networks to scan", "", "", "Scan the networks your NAS is on. VPN and virtual networks are off unless you select them.";
        NET_RECOMMENDED: Label, "Recommended", "", "", "";
        NET_SEL_REC: Button, "Select recommended", "", "", "";
        NET_SEL_ALL: Button, "Select all", "", "", "";
        NET_ADD_LABEL: Label, "Add a network", "", "", "";
        NET_ADD_HINT: Label, "192.168.1.0/24", "", "", "";
        NET_ADD_BTN: Button, "Add", "", "", "";
        NET_GONE: Prose, "A chosen network is gone. The recommended ones stand in.", "", "", "";
        SCAN_STOPPING: Label, "Stopping", "", "", "";
        NET_NONE: Prose, "Select at least one network.", "", "", "";
        NET_DONE: Button, "Done", "", "", "";
        NET_CUSTOM: Label, "Custom", "", "", "";
        SCAN_NOT_STARTED: Label, "Scan not started", "", "", "";
        SCAN_WHY: Label, "Why", "", "", "";
        SCAN_PROGRESS: Label, "Scan progress", "", "", "";
        SCAN_WHAT_RAN: Label, "What ran", "", "", "";
        SCAN_STOP: Button, "Stop", "Stop scanning now.", "", "";
        SCAN_WAITING: Label, "Waiting", "", "", "";
        SCAN_RUNNING: Label, "Running", "", "", "";
        SCAN_DONE: Label, "Done", "", "", "";
        SCAN_FAILED: Label, "Failed", "", "", "";
        SCAN_SKIPPED: Label, "Skipped", "", "", "";
        SCAN_STOPPED: Label, "Stopped", "", "", "";
        SCAN_CHOOSE: Button, "Choose networks", "", "", "";
        SCAN_HEADLINE_WORDS: Prose, "Scanning of hosts found", "", "", "";
        SCAN_EMPTY_WORDS: Prose, "Scanned with methods in", "", "", "";
        HELP_LINK: Label, "Help", "", "", "";
        SCAN_NONE_SELECTED: Prose, "No network is selected. Choose at least one.", "", "", "";
        INV_EMPTY_WHY: Prose, "The NAS may be on another network, asleep or firewalled.", "", "", "";
        NET_VPN_WARN: Prose, "VPN and virtual networks scan slowly; a NAS is rarely there.", "", "", "";
        NET_RESCAN: Prose, "Done scans again.", "", "", "";
        SCAN_NO_NETWORK: Prose, "This computer is on no network. Connect it, or add a network.", "", "", "";

        // ===================================================================== D1b welcome guide (first launch, Help, menu)
        WELCOME_MENU: Label, "Welcome guide", "Replay the short intro.", "", "";
        WELCOME_1_TITLE: Title, "Replace DSM with Debian", "", "", "";
        WELCOME_1_A: Prose, "synofurb installs Debian 13 on supported Synology units over SSH.", "", "", "";
        WELCOME_1_B: Prose, "Nothing changes until you confirm.", "", "", "";
        WELCOME_1_NOTE: Prose, "Screen readers are not supported; use the command line.", "", "", "";
        WELCOME_1_INFO: Label, "Good to know", "The Find list shows which models are supported.", "cli",
            "synofurb only reads until you confirm.\n\nThe same steps work from the command line (see Help, Command line).";
        WELCOME_2_TITLE: Title, "Six steps to Debian", "", "", "";
        WELCOME_2_A: Prose, "Changes begin after Confirm.", "", "", "";
        WELCOME_2_B: Prose, "The NAS restarts; installing takes a while.", "", "", "";
        WELCOME_2_INFO: Label, "More about the steps", "Find to Setup only read from the NAS.", "first-install",
            "You can go back to any earlier step until Install starts.\n\nBefore the disks change, synofurb saves the boot flash and the first and last MiB of each changed disk on this computer.";
        WELCOME_FLOW_FIND: Label, "Look for it", "", "", "";
        WELCOME_FLOW_CONNECT: Label, "Sign in", "", "", "";
        WELCOME_FLOW_GOAL: Label, "Pick a goal", "", "", "";
        WELCOME_FLOW_SETUP: Label, "Plan disks", "", "", "";
        WELCOME_FLOW_CONFIRM: Label, "You agree", "", "", "";
        WELCOME_FLOW_INSTALL: Label, "Changes begin", "", "", "";
        WELCOME_3_TITLE: Title, "Before you start", "", "", "";
        WELCOME_3_A: Prose, "The disks you pick will be erased.", "", "", "";
        WELCOME_3_B: Prose, "Back up your files and test it; keep the NAS on.", "", "", "";
        WELCOME_3_INFO: Dialog, "What synofurb saves", "synofurb saves the boot flash and disk headers, not your files.", "backups",
            "Before it writes anything, synofurb saves on this computer the whole boot flash and the first and last MiB of each changed disk.\n\nThat is not a backup of your files: make and test your own.\n\nKeep the NAS powered on and on the network until the install ends.";
        WELCOME_4_TITLE: Title, "If something goes wrong", "", "", "";
        WELCOME_4_A: Prose, "Before the disks change, a failure returns the NAS to DSM.", "", "", "";
        WELCOME_4_B: Prose, "After that, DSM must be reinstalled. Keep your backup.", "", "", "";
        WELCOME_4_INFO: Dialog, "More about recovery", "The Way back link in the status bar shows what applies now.", "recovery",
            "After a finished install the default boot entry is Debian. Past the point of no return DSM's own system may be gone; reinstall it from Synology.\n\nIf the boot agent never starts, a power cycle may boot it again: see Manual recovery in Help.\n\nRecovery restores the saved boot flash only when an image of this unit exists and the unit answers.";
        WELCOME_5_TITLE: Title, "Ready when you are", "", "", "";
        WELCOME_5_A: Prose, "Press F1 any time for Help.", "", "", "";
        WELCOME_5_B: Prose, "Reopen this guide from Help or the menu.", "", "", "";
        WELCOME_PAGES: Label, "Guide steps", "", "", "";
        WELCOME_GOTO: Label, "Go to step {n}", "", "", "";
        WELCOME_HIDE: Label, "Don't show again", "", "", "";
        WELCOME_NEXT: Button, "Next", "", "", "";
        WELCOME_SKIP: Button, "Skip", "Close for now; the guide returns at the next launch.", "", "";
        WELCOME_START: Button, "Find NAS", "", "", "";

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
        INSPECT_VOLUMES: Label, "DSM volumes", "The volumes DSM shows, and the disks behind them.", "", "";
        INSPECT_ARRAYS: Label, "RAID arrays", "DSM's RAID arrays and what they hold.", "", "";
        INSPECT_NO_VOLUMES: Label, "No volumes found", "", "", "";
        INSPECT_COL_VOLUME: Label, "Volume", "", "", "";
        INSPECT_COL_FS: Label, "Filesystem", "", "", "";
        INSPECT_COL_USED: Label, "Used", "", "", "";
        INSPECT_COL_BACKED: Label, "Backed by", "", "", "";
        INSPECT_COL_STATUS: Label, "Status", "", "", "";
        INSPECT_USE_TARGET: Label, "Install target", "", "", "";
        INSPECT_USE_NOT: Label, "Not used: {}", "", "", "";
        INSPECT_ERASE_NOTE: Prose, "Install targets are erased in Wipe mode.", "", "", "";
        INSPECT_NOT_CHECKED: Label, "Not checked", "", "", "";
        INSPECT_LAYOUT: Label, "Current layout", "DSM's partitions on each disk.", "", "";
        INSPECT_CHOOSE: Button, "Choose goal", "", "", "";
        INSPECT_BLOCKED: Label, "Cannot be converted", "Debian 13 cannot run on this hardware.", "compat-status", "";

        // ===================================================================== D4 goal
        GOAL_TITLE: Title, "Choose goal", "", "", "";
        GOAL_SUB: Subtitle, "What should happen to this NAS?", "", "", "";
        GOAL_BOOTMGR: Label, "Boot manager", "Adds a small SSH agent to the boot flash.", "boot-manager",
            "The flash changes, the disks stay untouched. The agent is one-shot and has a 60 minute watchdog.";
        GOAL_BOOTMGR_LINE: Prose, "Try Linux. Disks untouched.", "", "", "";
        GOAL_RECOMMENDED: Label, "Recommended", "No one has reported this model; try the agent first.", "compat-status", "";
        GOAL_SET_UP: Button, "Set up", "", "", "";
        GOAL_INSTALL: Label, "Install Debian", "Erases disks; needs your backup.", "first-install",
            "Wipe, or the advanced Keep mode. It takes about 25 minutes.";
        GOAL_INSTALL_LINE: Prose, "Replace DSM with Debian.", "", "", "";
        GOAL_TAG_SAFE: Label, "Disks untouched", "", "", "";
        GOAL_TAG_ERASE: Label, "Erases disks", "", "", "";
        GOAL_TAG_READS: Label, "Reads state first", "", "", "";
        GOAL_QUICK: Button, "Quick setup", "Recommended defaults, then straight to Confirm.", "first-install", "";
        GOAL_CUSTOMIZE: Button, "Customize", "", "", "";
        QUICK_TITLE: Dialog, "Quick setup", "Recommended defaults, then straight to Confirm.", "first-install", "";
        QUICK_USE: Button, "Use these", "", "", "";
        QUICK_HYBRID_LABEL: Label, "Hybrid 1-disk", "", "", "";
        QUICK_HYBRID_CAUTION: Prose, "Hybrid is built by synofurb; DSM may not adopt it.", "", "", "";
        QUICK_INTRO: Prose, "Recommended defaults for this unit:", "", "", "";
        QUICK_NOTES: Dialog, "Notes", "", "", "The password is generated and shown once on Done. Confirm shows what is erased before anything changes.";
        QUICK_CHOOSE: Label, "Why unavailable", "Choose a layout first.", "", "";
        GOAL_PLAN_INSTALL: Button, "Plan install", "Debian 13 cannot run on this hardware.", "", "";
        GOAL_RECOVER: Label, "Restore / Recover", "Reads the unit's state before offering anything.", "recovery", "";
        GOAL_RECOVER_LINE: Prose, "Undo or repair.", "", "", "";
        GOAL_OPEN_RECOVERY: Button, "Open recovery", "", "", "";
        GOAL_BOOTMGR_OFF: Label, "Boot manager", "Needs an editable internal boot flash, which this unit does not have.", "boot-manager", "";

        // ===================================================================== D5 plan
        SETUP_STORAGE: Label, "Storage", "", "", "";
        SETUP_SYSTEM: Label, "System", "", "", "";

        // ===================================================================== Setup: Add-ons (installer integration, docs/ADDONS.md)
        SETUP_ADDONS: Label, "Add-ons", "", "", "";
        ADDONS_SUB: Subtitle, "Optional apps, installed after the first boot.", "", "", "";
        ADDONS_INTRO: Prose, "Tick what to install with the system.", "", "", "";
        ADDON_NEEDS_DOCKER: Label, "Needs Docker", "Docker is installed with it, nothing to do.", "", "";
        ADDON_NEEDS_NET: Label, "Needs Internet", "The container images are downloaded when the NAS first starts.", "", "";
        ADDON_REQUIRED: Label, "Required by {names}", "", "", "";
        ADDON_PRIV_SOCKET: Label, "Controls Docker", "It can start and stop every container, which is as good as administrator rights on the NAS.", "", "";
        ADDON_PRIV_HOSTNET: Label, "Uses NAS network", "It shares the NAS's own network address instead of an isolated one.", "", "";
        ADDON_PRIV_FULL: Label, "Full access", "It runs with almost unrestricted access to the NAS.", "", "";
        ADDON_PRIV_OTHER: Label, "Extra rights", "It asks for more rights than an ordinary container.", "", "";
        ADDON_RETAIN: Label, "Keep recordings (days)", "Older recordings are deleted.", "", "Cameras and the detector are set up later, in Frigate's own web page.";
        ADDON_PLEX_CLAIM: Label, "Claim token (leave empty)", "Ties the server to your Plex account.",
            "", "Get a token at plex.tv/claim. It is valid for only 4 minutes and the first boot comes much later, so normally leave this empty and claim the server in its web page.";
        ADDON_DEFAULT: Label, "Default: {value}", "", "", "";
        ADDON_PLEX_HINT: Prose, "Valid 4 minutes, the first boot comes later. Leave it empty.", "", "", "";
        ADDON_UNTESTED: Label, "Untested", "Add-ons have only run against simulated Docker so far.", "",
            "These add-ons have been tested only against simulated Docker, never on a real NAS. The first boot is their first real run. If one fails it shows \"needs attention\" in the web interface; the system is unaffected.";
        ADDON_ABOUT: Label, "About this add-on", "", "", "";
        ADDON_NEEDS_DATA: Prose, "Needs the data volume, which Keep mode does not create.", "", "", "";
        ADDON_KEEP_BLOCK: Prose, "Needs a data volume: {names}. Keep mode does not create one.", "", "", "";
        ADDON_QUICK_ACCEPT: Prose, "You accept this on the next screen.", "", "", "";
        ADDON_QUICK: Label, "Add-ons", "Optional apps, installed after the first boot.", "", "";
        ADDON_QUICK_NONE: Label, "None", "", "", "";
        ADDON_NEEDS_WEB: Prose, "Add-ons need the web interface: switch it on in System.", "", "", "";
        SYS_NEXT_ADDONS: Button, "Next: Add-ons", "", "", "";
        CONF_ADDONS: Prose, "Will be installed after first boot:", "", "", "";
        CONF_ADDONS_NET: Prose, "The first boot needs Internet to pull the container images.", "", "", "";
        CONF_ADDONS_OFFLINE: Prose, "Without Internet at first boot, Docker and the add-ons are not installed.", "", "", "";
        CONF_ADDON_ACK: Label, "I accept these privileges", "", "", "";
        CONF_WARNINGS: Label, "Warnings", "", "", "";
        DONE_ADDONS: Label, "Installing after first boot", "What the first boot does.", "",
            "Without Internet at the first boot nothing is downloaded and the add-ons show \"needs attention\". Connect the NAS, then press Retry on the Add-ons page of the web interface.";
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
        SYS_KERNEL: Label, "Kernel", "Newest, the default, is Linux 7.x from backports; Stable is 6.12 LTS.", "first-install",
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
        CONF_BACKUP: Prose, "I have a tested backup", "", "", "";
        CONF_LOSS: Label, "I accept data loss", "", "", "";
        CONF_UNIT: Label, "This is {unit}", "Compare the identity line with the label on the NAS.", "", "";
        CONF_PHRASE: Label, "Type phrase", "Case matters; pasting is fine.", "", "";
        CONF_MATCH: Label, "Matches", "", "", "";
        CONF_NO_MATCH: Label, "No match", "", "", "";
        CONF_FALLBACK: Prose, "Free to cancel until step 3; then no undo.", "", "", "";
        CONF_NO_BACKUP: Label, "Test your backup.", "synofurb does not look at DSM's backup tasks; only you can confirm a backup.", "backups", "";
        CONF_KEEP_RISK: Prose, "Kept volumes are best effort; data can still be lost.", "", "", "";
        CONF_CHANGED_KEEP: Label, "Partition tables", "", "", "";
        CONF_NOTHING_ELSE: Label, "Nothing else", "", "", "";
        CONF_LIST: Label, "Erased disks", "", "", "";
        CONF_LIST_KEEP: Label, "Rewritten disks", "", "", "";
        CONF_ERASED: Prose, "Everything on the disks is erased.", "", "", "";
        CONF_AFTER: Label, "After", "", "", "";
        CONF_ADD_SERIAL: Prose, "Add serial last 4.", "", "", "";
        CONF_DETAILS: Label, "Details", "", "", "";
        CONF_BACKED_UP: Prose, "Boot flash \u{b7} backed up first", "", "", "";
        CONF_HOW_RECOVERY: Button, "How recovery works", "DSM is the default; failures return to it.", "recovery", "";
        CONF_ERASE: Button, "Erase and install", "", "", "";
        CONF_FLASH: Label, "Boot flash", "Entries and files are added; the whole flash is backed up first.", "backups", "";
        CONF_TWIN: Label, "Same model nearby", "Another unit of this model is on the network; compare serials.", "first-install", "";
        DLG_RECOVERY: Dialog, "How recovery works", "DSM is the default; failures return to it.", "recovery",
            "DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds.\n\nFrom step 3, a failure keeps the log. On boot-flash units the NAS reboots after 15 minutes into the DSM loader, which shows Synology's install page; other units stop and wait for a power cycle. DSM's own system may be gone; reinstall it from Synology. There is no rescue shell.\n\nWe promise nothing about your data.";

        // ===================================================================== D8 install
        INSTALL_TITLE: Title, "Installing", "", "", "";
        INSTALL_SUB: Subtitle, "Keep this window open.", "", "", "";
        INSTALL_PHASES: Label, "Phases", "Seven phases from connecting to verifying.", "first-install", "";
        INSTALL_FLASH: Label, "Boot flash", "Backed up and verified before the first write.", "backups", "";
        INSTALL_STEPS: Label, "Steps", "", "", "";
        INSTALL_NO_RETURN: Label, "No return", "From step 3 the first disk changes; cancel is free before.", "recovery", "";
        INSTALL_LOG: Label, "Log", "", "", "";
        INSTALL_CANCEL: Button, "Cancel", "Not possible after step 3.", "recovery", "";
        CONN_HOST_WHY: Label, "Why unavailable", "Enter the NAS address first.", "", "";
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
        FAIL_LOADER_IN: Label, "DSM loader in", "On boot-flash units the NAS reboots into Synology's loader after 15 minutes, which is time to read the log.", "recovery", "";
        FAIL_HALTS: Label, "Will not reboot itself", "The installer stops and waits: read the log on the console, then power-cycle the NAS.", "recovery", "";
        FAIL_COPY_LOG: Button, "Copy log", "", "", "";
        FAIL_DSM_IN: Label, "DSM returns in", "The installer's failure handler sets DSM as default and reboots.", "recovery", "";
        FAIL_WAITING: Label, "Waiting for DSM", "", "", "";
        FAIL_DSM_BACK: Label, "DSM is back.", "The NAS answered on its DSM address.", "", "";
        FAIL_DSM_LOADER: Prose, "The NAS is in the DSM loader.", "DSM's own system may be gone; reinstall it from find.synology.com.", "recovery", "";
        FAIL_DSM_AGENT_UP: Prose, "The boot agent is up.", "The unit is in the agent, not in DSM; reboot it into DSM from the agent.", "recovery", "";
        FAIL_DSM_NOTBACK: Label, "DSM did not return.", "Power-cycle the NAS and check again.", "recovery", "";
        FAIL_DSM_NOTBACK_AGENT: Label, "DSM not back.", "The agent may never have run, so use the manual recovery.", "recovery", "";
        FAIL_WAIT_DSM: Button, "Wait for DSM", "", "", "";
        FAIL_EXPORT: Button, "Export diagnostics", "Do this before the clock ends; secrets are removed.", "troubleshooting", "";
        FAIL_RECOVERY: Button, "Recovery options", "", "", "";
        FAIL_NEXT: Button, "What happens next", "Order of actions and what survives.", "recovery", "";
        FAIL_LOG_EMPTY: Label, "No log lines yet.", "", "", "";
        FAIL_LOG: Label, "Log excerpt", "", "", "";
        FAIL_AGENT_TITLE: Label, "Agent did not answer", "The engine message is in the log excerpt.", "recovery", "";
        FAIL_AGENT_TEXT: Prose, "No answer within 10 minutes. Nothing was reset after the wait.", "", "", "";
        FAIL_DEFAULT_UNCONFIRMED: Label, "Default: not confirmed", "We cannot tell if the agent ran; if it did not, a power cycle boots it again.", "recovery", "";
        FAIL_CLOCK_AGENT: Label, "DSM may return in", "An estimate, not a promise.", "recovery",
            "If the agent started, its watchdog counts 30 minutes from when it booted. The app already waited 10, so 20 or more remain. If it never started there is no watchdog and DSM does not come back by itself.";
        FAIL_CLOCK_AGENT_VALUE: Label, "20 min or more", "", "", "";
        DLG_NEXT_AGENT: Dialog, "What happens next", "", "recovery",
            "The app waited 10 minutes and cannot tell what happened. If the agent started, its watchdog reboots the NAS into DSM, 20 minutes or more from now.\n\nIf it never started, the NAS may keep booting the agent after every power cycle. Then choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer.\n\nDSM is not confirmed. Your disks are untouched.";
        DLG_NEXT_PRE: Dialog, "What happens next", "", "recovery",
            "No disk was changed. The installer sets DSM as the default and reboots the NAS in 60 seconds, then DSM starts as before.\n\nIf DSM is not back after 10 minutes, power-cycle the NAS. Our entries stay on the boot flash until you remove them under Recovery.";
        DLG_NEXT_PRE_HALT: Dialog, "What happens next", "", "recovery",
            "No disk was changed. This installer does not reboot the NAS by itself: it stops and waits. Read the log, then power-cycle the NAS and DSM starts as before.";
        DLG_NEXT_WIPE: Dialog, "What happens next", "", "recovery",
            "The first disk was changed and DSM's own system may be gone. Export diagnostics first; there is no rescue shell.\n\nThe installer sets DSM as the default and reboots the NAS after 15 minutes into the DSM loader, which shows Synology's install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep.";
        DLG_NEXT_WIPE_HALT: Dialog, "What happens next", "", "recovery",
            "The first disk was changed and DSM's own system may be gone. Export diagnostics first; there is no rescue shell.\n\nThis installer will NOT reboot the NAS by itself. Read the log on the console, then power-cycle it: the DSM loader shows Synology's install page. Reinstall DSM from Synology.";
        DLG_NEXT_KEEP: Dialog, "What happens next", "", "recovery",
            "The first disk was changed. Export diagnostics first; there is no rescue shell.\n\nAfter 15 minutes the NAS reboots into the DSM loader and Synology's install page. Your data volume was not part of any write command and should be intact unless the failure was a hardware problem. We cannot guarantee it.";
        DLG_NEXT_KEEP_HALT: Dialog, "What happens next", "", "recovery",
            "The first disk was changed. Export diagnostics first; there is no rescue shell.\n\nThis installer will NOT reboot the NAS by itself. Power-cycle it after reading the log; the DSM loader shows Synology's install page. Your data volume was not part of any write command, but we cannot guarantee it.";
        INSTALL_CANCEL_DIALOG_HALT: Dialog, "Cancel install?", "Nothing is written before step 3.", "recovery", "Before step 3 no disk is changed. This installer does not reboot the NAS by itself: after stopping, power-cycle it and DSM starts as before.";

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
        UPD_CHECK: Button, "Check for updates", "", "", "";
        UPD_CHECK_AGAIN: Button, "Check again", "", "", "";
        UPD_OPEN_PAGE: Button, "Open release page", "", "", "";
        UPD_ST_NONE: Prose, "Not checked yet", "", "", "";
        UPD_ST_CHECKING: Prose, "Checking for updates", "", "", "";
        UPD_ST_CURRENT: Prose, "{v} is current", "", "", "";
        UPD_ST_AVAILABLE: Prose, "{v} available", "", "", "";
        UPD_ST_UNSIGNED: Prose, "This release is not signed. synofurb checks its SHA-256 only.", "", "", "";
        UPD_CONSENT: Prose, "I understand and want to install it", "", "", "";
        UPD_CONSENT_INFO: Dialog, "Unsigned install", "A tampered release would not be detected.", "troubleshooting",
            "synofurb compares the download with the SHA-256 listed on the same GitHub release. That catches a damaged file.\n\nIt cannot tell whether the release itself was changed by someone else, because there is no signature to check. Install only if you trust the release page.";
        UPD_CONSENT_WHY: Label, "Why unavailable", "Confirm that you understand the risk first.", "", "";
        UPD_DOWNLOAD_INSTALL: Button, "Download and install", "", "", "";
        UPD_VERIFIED_UNSIGNED: Label, "Unsigned, checksum verified", "Only the SHA-256 was checked; there is no signature.", "", "";
        UPD_ST_FAILED: Prose, "Couldn't check: {reason}", "", "", "";
        UPD_FAIL_OFFLINE: Label, "No Internet connection", "", "", "synofurb could not reach GitHub. Check the Internet connection and try again.";
        UPD_FAIL_NOTFOUND: Label, "Releases not found", "", "", "GitHub did not find the releases. If the repository is private, the update check cannot see them.";
        UPD_FAIL_RATE: Label, "GitHub rate limit", "", "", "";
        UPD_FAIL_NORELEASE: Label, "No release found", "", "", "No release found for this system.";
        UPD_FAIL_SERVER: Label, "GitHub error", "", "", "";
        UPD_FAIL_PARSE: Label, "Unreadable answer", "", "", "";
        UPD_FAIL_OTHER: Label, "Check failed", "", "", "";
        ABOUT_BUILD_RELEASE: Label, "release", "", "", "";
        ABOUT_BUILD_DEV: Label, "dev", "", "", "";
        ABOUT_BUILD_LOCAL: Label, "local build", "", "", "";

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
        SET_ACCENT: Label, "Accent", "The colour of buttons, links and focus.", "", "";
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
        SET_BUNDLE_WHY: Label, "Why unavailable", "Not built into the app yet: run synofurb-cli bundle to build or download one.", "", "";
        UPD_DOWNLOAD_KEY_WHY: Label, "Why unavailable", "This build cannot verify updates: its signing key is a placeholder.", "", "";
        UPD_DOWNLOAD_ASSET_WHY: Label, "Why unavailable", "This release has no signed package for this system.", "", "";
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
        BM_LOGIN_TIMEOUT: Label, "Login timeout", "The agent reboots to DSM after 60 minutes without a login.", "boot-manager", "";
        BM_OTHER_OS: Prose, "Other OS: power cycle returns.", "After kexec nothing of ours runs; power-cycling returns to the default.", "boot-manager", "";
        BM_AFTER_OS: Prose, "After another OS starts, only a power cycle returns.", "", "boot-manager", "";
        BM_STEP1: Label, "Boot flash backup", "", "", "";
        BM_STEP2: Label, "Agent added", "", "", "";
        BM_STEP3: Label, "Reboot into the agent", "", "", "";
        BM_STEP4: Label, "Agent answers", "", "", "";
        BM_READY: Label, "Agent ready", "", "", "";
        BM_WATCHDOG: Label, "60 min watchdog", "The agent reboots to DSM after 60 minutes without a login.", "boot-manager", "";
        BM_RETURN: Button, "Return to DSM", "Sets DSM as the default; you reboot the unit afterwards.", "recovery", "";
        BM_DONE: Button, "Done", "", "", "";
        BM_CANCEL_WHY: Prose, "Not possible after the first flash write.", "", "", "";
        DLG_BM_CONFIRM: Dialog, "Install the agent?", "Entries and files are added; DSM's entry stays.", "boot-manager", "The whole boot flash is saved to this computer first and verified. Then the agent is added and the unit reboots into it once. DSM stays the default and your disks are not touched.";
        BM_INSTALL: Button, "Install agent", "", "", "";
        BM_NOT_EDITABLE: Label, "Flash not editable", "Needs an editable internal boot flash (DOM).", "boot-manager", "";

        // ===================================================================== D14 recovery
        REC_TITLE: Title, "Recovery", "", "", "";
        REC_SUB: Subtitle, "From this session.", "The app cannot read the boot flash yet.", "recovery", "These facts come from what this app did in this session. If you closed it after a failure, they are gone: use Manual recovery.";
        REC_BOOT_DSM: Label, "Boot DSM next", "Sets the default boot entry back to DSM.", "recovery", "";
        REC_OPEN_AGENT: Label, "Open agent", "Boots the SSH agent once; DSM stays the way back.", "boot-manager", "";
        REC_RESTORE_FLASH: Label, "Restore flash", "Writes the saved boot-flash image back; needs a working Linux on the unit.", "recovery", "";
        REC_REMOVE: Label, "Remove our entries", "Takes the agent out of the boot flash again.", "recovery", "";
        REC_REINSTALL: Label, "Reinstall DSM", "Use Synology's page; no data promise.", "recovery", "";
        REC_SUB_READ: Subtitle, "Read from the unit.", "", "", "";
        REC_READING: Label, "Reading the unit", "", "", "";
        REC_READ_FAILED: Label, "Unit not read", "The unit did not answer, so the facts below come from this session.", "", "";
        REC_NOT_READ: Label, "Boot flash not read", "", "", "";
        REC_TOOL_MISSING: Label, "Why not read", "synofurb-dom is not on the unit, so its boot menu could not be read.", "", "";
        REC_NOTHING_READ: Label, "Nothing to recover.", "The boot menu was read: DSM is the default and none of our entries are on it.", "", "";
        REC_BOOT_RUNNING: Label, "Setting default to DSM", "", "", "";
        REC_BOOT_DONE: Label, "Default set to DSM", "Reboot the NAS to start DSM; this app does not reboot it.", "recovery", "";
        REC_BOOT_DSM_OK: Button, "Set default", "", "", "";
        DLG_BOOT_DSM: Dialog, "Boot DSM next?", "Makes DSM the default boot entry.", "recovery", "This makes DSM the default boot entry on the unit and reads it back. Set default does not reboot it; Set and reboot also asks the unit to reboot, and DSM is back only when it answers. If the unit does not answer, use Manual recovery.";
        REC_BOOT_DSM_REBOOT: Button, "Set and reboot", "", "", "";
        DLG_AGENT: Dialog, "Reboot into the agent?", "The unit reboots once into the SSH agent.", "boot-manager", "The unit reboots into the boot agent and this app waits up to 10 minutes for it. The agent resets the default to DSM when it starts. If it does not answer, the default is not confirmed.";
        DLG_AGENT_DEBIAN: Dialog, "Reboot into the agent?", "This unit runs Debian; the agent makes DSM the default.", "boot-manager", "This unit runs Debian. The agent makes DSM the default boot entry. After the agent's watchdog or any reboot the unit starts DSM's loader instead of Debian, until you choose Debian again from the agent. Your Debian stays on the disks. The app waits up to 10 minutes for the agent.";
        REC_AGENT_OK: Button, "Reboot", "", "", "";
        REC_STOP_WAITING: Button, "Stop waiting", "The unit may still boot the agent; use Recovery to check.", "", "";
        REC_AGENT_RUNNING: Label, "Waiting for the agent", "", "", "";
        REC_RESTORE_RUNNING: Label, "Restoring the flash", "", "", "";
        REC_REMOVE_RUNNING: Label, "Removing our entries", "", "", "";
        REC_SHOW_COMMAND: Button, "Show command", "", "", "";
        REC_NO_SERIAL: Label, "Why unavailable", "This unit's serial number is not known, so no saved image can be matched to it; use Manual recovery.", "", "";
        REC_NO_BACKUP: Label, "No saved image", "No saved boot-flash image is in this folder.", "", "";
        DLG_RESTORE: Dialog, "Restore boot flash?", "Writes the saved image back to the boot flash.", "backups", "This writes the saved image back to the boot flash through the unit. The engine checks the file, snapshots the flash, writes, reads back and compares. File:";
        DLG_REMOVE: Dialog, "Remove our entries?", "Leaves only Synology's own files on the flash.", "boot-manager", "This removes our entries from the boot flash and sets DSM as the default. The result is read back from the unit.";
        MGMT_ROLLBACK_OK: Button, "Roll back", "", "", "";
        DLG_MGMT_ROLLBACK: Dialog, "Roll back the update?", "Goes back to the version the unit had before.", "", "The unit goes back to the management app version it had before the last update. The web UI restarts and is unavailable for a few seconds. Disks and data are not touched.";
        DLG_HOSTKEY: Dialog, "Trust this NAS?", "Checks that you are talking to your NAS.", "troubleshooting",
            "Compare this fingerprint with DSM (Control Panel, Terminal and SNMP). If you did not expect a new key, cancel. Nothing, not even your password, is sent before you trust it.";
        HK_FIRST: Label, "First connection to", "", "", "";
        HK_REMAINING: Label, "Answer within", "", "", "";
        HK_ENDED: Label, "Connection ended", "Close this dialog and start again.", "", "";
        HK_TRUST: Button, "Trust", "", "", "";
        DLG_HOSTKEY_CHANGED: Dialog, "Host key changed", "The NAS shows another key than the one saved.", "troubleshooting",
            "This NAS shows a different key than the one saved. DSM may have been reinstalled, or another device took this address. synofurb stopped before sending anything, not even your password. If you replaced the unit on purpose, forget the old key with the command below.";
        HK_PINNED_LABEL: Label, "Saved key", "", "", "";
        HK_NOW_LABEL: Label, "Key it shows now", "", "", "";
        HK_PIN: Label, "Host key", "The fingerprint saved when you first trusted this NAS; later connections must match it.", "troubleshooting", "";
        HK_PINNED_TAG: Label, "Pinned", "", "", "";
        HK_NOT_PINNED: Label, "Not saved yet", "", "", "";
        MAN_READ_FAILED: Label, "Health not read", "", "", "";
        MAN_CHANGE_SIGNIN: Button, "Change sign-in", "", "", "";
        MAN_SIGNIN: Subtitle, "Sign in to read health", "Use the user and password or key of the NAS at this address.", "", "";
        MAN_READ_HEALTH: Button, "Read health", "", "", "";
        MAN_SIGNIN_WHY: Label, "Why unavailable", "Enter the user and the password or key of this NAS first.", "", "";
        SET_REFRESH_OFFLINE: Label, "Why unavailable", "The app is offline; the list works from its cached copy.", "", "";
        PLAN_SIZE_FIXED: Label, "Why unavailable", "These disks leave no room to choose another system size.", "", "";
        REC_RUN: Button, "Run", "", "", "";
        REC_SAVED: Label, "Saved here", "First and last 1 MiB of changed disks; not your files.", "backups", "";
        REC_OPEN_FOLDER: Button, "Open folder", "", "", "";
        REC_FLASH_OURS: Label, "Flash: ours present", "", "", "";
        REC_FLASH_CLEAN: Label, "Flash: clean", "", "", "";
        REC_DEFAULT_DSM: Label, "Default: DSM", "", "", "";
        REC_DEFAULT_DEBIAN: Label, "Default: Debian", "", "", "";
        REC_DEFAULT_UNKNOWN: Label, "Default: not confirmed", "We cannot tell if the agent ran; if it did not, a power cycle boots it again.", "recovery", "";
        REC_DISKS_MODIFIED: Label, "Disks: modified", "", "", "";
        REC_DISKS_UNKNOWN: Label, "Disks: not checked", "", "", "";
        REC_DISKS_UNTOUCHED: Label, "Disks: untouched", "", "", "";
        REC_UNKNOWN_ALERT: Prose, "The agent may still be the default.", "DSM may not return by itself.", "recovery", "";
        REC_MANUAL: Button, "Manual recovery", "Steps that work without the unit answering.", "recovery", "";
        REC_NOTHING: Label, "Nothing known to recover.", "This session changed nothing; the app cannot read earlier changes.", "", "";
        DLG_MANUAL: Dialog, "Manual recovery", "Steps that work without the unit answering.", "recovery",
            "Both ways need physical access to the NAS.\n\n1. On a serial or VGA console, choose SYNOLOGY_2 in the boot menu. When DSM runs, run the first command on the NAS. synofurb-dom is there only if the app installed the agent; if not, use route 2.\n\n2. Or put the boot flash in another Linux computer: with synofurb-dom there run the second command, without it the third. Replace /dev/sdX with the 128 MB flash; check twice.\n\nThese steps do not touch your disks. No promise about your data.";        REC_TYPE: Label, "Type {phrase} to continue.", "", "", "";
        REC_COPY_CMD: Button, "Copy command", "", "", "";
        REC_UNAVAILABLE: Label, "Unavailable", "", "", "";
        DLG_REINSTALL: Dialog, "Reinstall DSM", "Use Synology's page; no data promise.", "recovery",
            "The NAS boots Synology's loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model.\n\nSynology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it.";
        BTN_OPEN_SYNOLOGY: Button, "Open find.synology.com", "", "", "";

        // ===================================================================== D15 manage
        MAN_TITLE: Title, "Manage", "", "", "";
        MAN_SUB: Subtitle, "Running Debian 13.", "", "", "";
        MAN_STORAGE_TAB: Label, "Storage", "", "", "";
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


        // ===================================================================== RAID (D5 additions, D6, D18)
        PLAN_USABLE: Label, "Usable", "What the data volume holds after redundancy.", "raid-levels", "";
        PLAN_SURVIVES: Label, "Survives", "How many disks may fail without losing data.", "raid-levels", "";
        PLAN_SLICES: Button, "Slices", "Each slice becomes one array.", "hybrid-raid", "";
        PLAN_WEAKER: Label, "Weaker slice", "A slice with fewer than four disks survives fewer failures.", "hybrid-raid", "";
        PLAN_HYBRID_TIP: Label, "Hybrid (SHR-style)", "Same technique as Synology SHR; DSM may not adopt it.", "hybrid-raid", "";
        SLICE_COL_SLICE: Label, "Slice", "", "", "";
        SLICE_COL_DISKS: Label, "Disks", "", "", "";
        SLICE_COL_LEVEL: Label, "Level", "RAID level and minimum disks.", "raid-levels", "";
        SLICE_COL_USABLE: Label, "Usable", "", "", "";
        DLG_SLICES: Dialog, "Hybrid slices", "Each slice becomes one array.", "hybrid-raid",
            "Hybrid (SHR-style) cuts your disks' data space into slices at the sizes of the different disks. Each slice present on two or more disks becomes one array: RAID 1 for two members, RAID 5 for three or more (Hybrid 2-disk: RAID 6 for four or more). A slice on one disk has no partner and stays unused.\n\nThe arrays are joined by LVM into one volume. This is built by synofurb with mdadm and LVM; DSM may not adopt it, and nothing here has run on a real Synology.";
        SYS_DATA_LAYOUT: Label, "Data layout", "Chosen on the Plan screen.", "raid-levels", "";
        MAN_STORAGE: Label, "Storage", "", "", "";
        RAID_ARRAY: Label, "Array", "", "", "";
        RAID_LEVEL: Label, "Level", "RAID level and minimum disks.", "raid-levels", "";
        RAID_STATE: Label, "State", "Clean, degraded, rebuilding or foreign.", "replace-disk", "";
        RAID_SIZE: Label, "Size", "", "", "";
        RAID_PROGRESS: Label, "Progress", "Speed and time left; the array stays usable.", "replace-disk", "";
        RAID_CLEAN: Label, "Clean", "", "", "";
        RAID_DEGRADED_TAG: Label, "Degraded", "", "", "";
        RAID_REBUILDING: Label, "Rebuilding", "Speed and time left; the array stays usable.", "replace-disk", "";
        RAID_FOREIGN: Label, "Foreign", "Created by DSM or another system; you can assemble and scrub it.", "hybrid-raid", "";
        RAID_INACTIVE: Label, "Inactive", "", "", "";
        RAID_CREATE: Button, "Create array", "Opens the plan; the web UI runs it.", "raid-levels", "";
        RAID_CREATE_RAID: Label, "RAID array", "", "", "";
        RAID_CREATE_HYBRID: Label, "Hybrid array", "", "", "";
        RAID_ACTIONS: Label, "Actions", "Changes run in the web UI.", "raid-levels", "";
        RAID_ADD_SPARE: Label, "Add spare", "", "", "";
        RAID_REPLACE: Button, "Replace disk", "Swap a failed or old member.", "replace-disk", "";
        RAID_GROW: Label, "Grow", "", "", "";
        RAID_CHANGE_LEVEL: Label, "Change level", "RAID 1 to 5 and 5 to 6 only.", "raid-levels", "";
        RAID_SCRUB: Label, "Scrub", "", "scrubbing", "";
        RAID_STOP: Label, "Stop", "", "", "";
        RAID_TUNE: Label, "Tune", "", "", "";
        RAID_OPEN_WEB: Button, "Open web UI", "Full array detail and history.", "raid-levels", "";
        RAID_DEGRADED_ALERT: Label, "Array degraded", "One more failure may lose data on RAID 5.", "replace-disk", "";
        RAID_NONE: Label, "No arrays", "", "", "";
        RAID_UNREACHABLE: Label, "Cannot reach unit", "", "troubleshooting", "";
        RAID_READ: Button, "Read arrays", "Reads /proc/mdstat over SSH.", "", "";
        DLG_OPERATION: Dialog, "Operation plan", "Review the steps and exact commands first.", "raid-levels",
            "Shows the ordered steps for this operation, re-derived on the server when you run it, and the exact commands (collapsed). Destructive steps ask you to type a phrase such as REPLACE sdc.\n\nIt refuses to touch a device that is mounted, holds the system or swap, belongs to another array, or while the array is resyncing or reshaping.";
        RAID_COMMANDS: Label, "Commands", "", "", "";
        RAID_RUN: Button, "Run", "Run it from the web UI.", "", "";

        // ===================================================================== Help topics (bodies: markdown, at most 150 words)
        T_FIRST_INSTALL: Topic, "First install", "Scan, connect, inspect, plan, confirm.", "",
            "1. Scan, pick your unit and connect. Enable SSH in DSM first (Control Panel, Terminal and SNMP).\n2. Inspect is read-only.\n3. Choose Boot manager to try the agent, or Install Debian.\n4. Plan storage, set the system options, then confirm.\n\nThe install takes about 25 minutes. Kernel: Newest (the default) is 7.x from trixie-backports and falls back to Stable if it cannot be installed; Stable is Debian's 6.12 LTS. Source: Online needs Internet on the NAS; Offline uses a bundle you downloaded in Settings. Afterwards the data array syncs for about 9 hours; the NAS works meanwhile with reduced protection. Copy the generated password on the last screen: synofurb does not keep it.";
        T_COMPAT_STATUS: Topic, "Status meanings", "What Verified, Untested and the others mean.", "",
            "Each model has one status.\n\n**Verified**: maintainers or two independent people installed it, with logs. No model has this yet.\n\n**Community-reported**: at least one report exists. DS923+ has one, from another project.\n\n**Untested**: no report. The hardware class should work in theory; that is not a promise, and your install is the test.\n\n**Known issues**: reports show something fails or only partly works.\n\n**Unsupported**: Debian 13 cannot run on this hardware; synofurb refuses to install.\n\nOnly Unsupported blocks. The list ships with the app and refreshes from the project repository; offline it uses the cached copy.";
        T_BACKUPS: Topic, "Backups", "Your backup and what synofurb saves.", "",
            "Your own backup is essential. In Wipe mode nothing on the disks is preserved; in Keep mode the data volume is left alone on a best-effort basis with no guarantee.\n\nsynofurb also saves on this computer:\n\n- **Boot-flash image**: the whole flash, hash-verified, before the first flash write. Keep it off the NAS.\n- **Disk metadata backup**: the first and last 1 MiB of every disk we change (partition tables, GPT headers). That is metadata only, not your files, and it cannot bring back a formatted filesystem.\n- The plan, the install script and the log.";
        T_RECOVERY: Topic, "Recovery", "What returns the unit to DSM, by phase.", "",
            "DSM stays the default boot entry and failures point it back at DSM, except when the agent never answered: then the default is not confirmed.\n\n- **Before the boot flash is changed**: nothing to undo.\n- **Agent installed**: the agent resets the default to DSM first; it reboots to DSM if nobody logs in.\n- **Before step 3**: a failure reboots to DSM after 60 s; a power cycle also works.\n- **After step 3**: no rescue shell. Boot-flash units reboot after 15 minutes into the DSM loader; others stop and wait for a power cycle. DSM's own system may be gone, reinstall it from Synology (find.synology.com); no data promise.\n- **Agent never answered**: DSM is not confirmed and may not return by itself; use Manual recovery (SYNOLOGY_2 in the boot menu).\n- **Debian installed**: pick the previous kernel in the boot menu or boot DSM.\n\nCommands: `synofurb recover`, `synofurb boot-manager enter`.";
        T_BOOT_MANAGER: Topic, "Boot manager", "A one-shot SSH agent on the boot flash.", "",
            "x86 units boot through a small internal boot flash. Boot manager adds an SSH agent entry to it. The whole flash is saved to this computer first; disks are untouched; DSM's entry stays and stays the default.\n\nThe agent is one-shot: it resets the default to DSM when it starts and reboots to DSM after 60 minutes without a login. If it never starts, the unit may keep booting it; see Recovery. From it, `sf-boot kexec <kernel-url> <initrd-url>` starts another Linux kernel. kexec starts Linux kernels only, not Windows or UEFI-only ISOs. Once another system runs, nothing of ours is running: power-cycling returns to the default entry. That system can see and change your disks.";
        T_SERIAL_CONSOLE: Topic, "Serial console", "Boot messages without a network.", "",
            "A serial console shows boot messages and lets you log in without a network. Intel units usually use `ttyS0` at 115200 8N1; the app reads the setting from DSM's kernel command line. AMD units have a memory-mapped UART; the exact console argument is copied from DSM. ARM boards need a 3.3 V TTL serial adapter (not RS-232 levels); without it you cannot recover an ARM unit over SSH alone. Debian also opens a login on any USB serial adapter you plug in later.";
        T_OFFLINE_MODE: Topic, "Offline mode", "Install from a downloaded bundle.", "",
            "Offline installs use a bundle: the installer files and packages for one CPU architecture. Download it in Settings, Data (this computer needs Internet once). In System, choose Source: Offline and select the bundle. The NAS then needs no Internet during the install. The compatibility list also works offline from its cached copy. Update checks, list refresh and result reports need Internet and are disabled while offline.";
        T_CLI: Topic, "Command line", "Everything in the app is also a command.", "",
            "Everything in the app is also a command, and the command line is the screen-reader accessible path (the app window does not support screen readers).\n\n`synofurb scan` discovers units.\n\n`synofurb plan --host <ip> --user <admin>` shows the plan, read-only.\n\n`synofurb install ... --have-backup --accept-data-loss --confirm` converts a unit.\n\n`synofurb recover` and `synofurb boot-manager enter` handle recovery.\n\nThe Copy command button on Plan, System and Confirm copies the equivalent command. On Confirm the three confirmation flags are included only after you confirm.";
        T_TROUBLESHOOTING: Topic, "Troubleshooting", "No units, SSH off, login failed, installer stuck.", "",
            "**No units found**: wrong network or VLAN, NAS still booting, firewall (UDP 9999, mDNS 5353, TCP 22). Use Add by IP.\n\n**SSH is off**: enable it in DSM, Control Panel, Terminal and SNMP.\n\n**Login failed**: DSM may auto-block this computer after a few failures; check the account is an administrator.\n\n**Installer did not come up**: before step 3, DSM returns by itself. If the app says the default is not confirmed, it may not: use Manual recovery.\n\n**Boot flash write failed**: the tool restores the flash and verifies it; read the install log and use Recovery.\n\nLogs: `~/.local/share/synofurb/logs` (secrets removed). Settings, Logs opens the folder; About copies diagnostics.";
        T_REPORT_RESULT: Topic, "Report a result", "How a model moves from Untested to Verified.", "",
            "Reports move a model from Untested towards Verified. After an install, press Report result on the Done screen. It opens a pre-filled GitHub issue (compat report template) with the model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click, and Settings, Privacy has an off switch. A maintainer checks the log and changes the status through a reviewed pull request.";
        T_RAID_LEVELS: Topic, "RAID levels", "What each level costs and survives.", "",
            "RAID combines disks into one array. **None**: one disk, no protection. **RAID 0**: striped and fast; any failure loses everything (2+ disks). **RAID 1**: mirror, survives all but one disk (2+). **RAID 5**: survives one failed disk; usable space is (n-1) disks (3+). **RAID 6**: survives two; usable (n-2) disks (4+). **RAID 10**: mirrored stripes, survives one disk per mirror, half the capacity (4+).\n\nEqual-size levels use the smallest disk's size on every member, so larger disks waste space; Hybrid avoids that. RAID is not a backup: it does not protect against deletion, corruption or fire. A new array syncs for hours and works meanwhile with reduced protection.";
        T_HYBRID_RAID: Topic, "Hybrid RAID", "Mixed disk sizes, SHR-style.", "",
            "Hybrid (SHR-style) mixes disk sizes. synofurb cuts the disks' data space into slices at the sizes of the different disks; every slice on two or more disks becomes an array (RAID 1 for two members, RAID 5 for three or more; Hybrid 2-disk uses RAID 6 for four or more). LVM joins the arrays into one volume.\n\nExample: 4, 4 and 8 TB with Hybrid 1-disk gives 8 TB usable; the extra 4 TB on the large disk has no partner and stays unused. Hybrid 1-disk survives any one disk; Hybrid 2-disk any two where a slice has four or more members. It is built with mdadm and LVM, so Debian manages it. DSM may not adopt it, and DSM-made volumes import read-only. Nothing here has run on a real Synology.";
        T_REPLACE_DISK: Topic, "Replacing a failed disk", "Degraded arrays and rebuilds.", "",
            "A failed member leaves the array degraded: it works, but one more failure may lose data (RAID 5).\n\n1. Check the disk on the Dashboard or RAID page.\n2. Choose the new disk; it is erased and must be at least as large.\n3. The tool marks the old member failed, removes it and adds the new one.\n4. The rebuild runs for hours; progress and speed are shown, and other operations wait. Do not power off.\n\nTo upgrade a healthy member, the tool adds a spare and uses a replace operation, so redundancy is kept during the swap. Each step shows its exact commands and asks you to type REPLACE and the disk name.";
        T_SCRUBBING: Topic, "Scrubbing", "Find silent errors before they matter.", "",
            "A scrub reads every block and compares copies or parity to find silent errors. Check only counts mismatches; Repair also rewrites them and needs a typed confirmation. Debian's mdcheck timer runs a scrub monthly; you can start one or stop it (idle) from the RAID page.\n\nThe array stays usable but slower while it runs, and speed limits can be tuned. A non-zero mismatch count on a healthy array deserves attention: check SMART on every member. A scrub cannot start while the array is resyncing or reshaping.";
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
    ("raid-levels", &T_RAID_LEVELS, "docs/RAID.md"),
    ("hybrid-raid", &T_HYBRID_RAID, "docs/RAID.md"),
    ("replace-disk", &T_REPLACE_DISK, "docs/RAID.md"),
    ("scrubbing", &T_SCRUBBING, "docs/RAID.md"),
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

/// The inline sentence for an address that cannot be used.
pub fn add_error(e: super::addhost::AddError) -> &'static str {
    use super::addhost::AddError::*;
    match e {
        Empty => ADD_ERR_EMPTY.short,
        BadChars => ADD_ERR_CHARS.short,
        BadIp => ADD_ERR_IP.short,
        BadPort => ADD_ERR_PORT.short,
        BadName => ADD_ERR_NAME.short,
        NotHost => ADD_ERR_NOT_HOST.short,
        ThisComputer => ADD_ERR_THIS.short,
    }
}

/// The shell around every wizard screen: stepper (six steps), risk tag, way back, wordmark (11 words).
pub static SHELL: &[&Item] =
    &[&APP_NAME, &STEP_FIND, &STEP_CONNECT, &STEP_GOAL, &STEP_SETUP, &STEP_CONFIRM, &STEP_INSTALL, &RISK_FLASH_CHANGED, &WAY_BACK];

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
            &NET_CHIP,
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
    // the scan panel is on screen with the table (headline words, a line's state, Stop)
    ScreenCopy {
        name: "inventory-scanning",
        visible: &[
            &INV_TITLE,
            &INV_SUB,
            &INV_FILTER,
            &INV_RESCAN,
            &INV_ADD,
            &NET_CHIP,
            &INV_COL_MODEL,
            &INV_COL_ADDR,
            &INV_COL_DSM,
            &INV_COL_FOUND,
            &INV_COL_STATUS,
            &INV_CONNECT,
            &SCAN_HEADLINE_WORDS,
            &SCAN_RUNNING,
            &SCAN_WAITING,
            &SCAN_DONE,
            &SCAN_STOP,
        ],
    },
    // nothing found: the empty state instead of the table
    ScreenCopy {
        name: "inventory-empty",
        visible: &[
            &INV_TITLE,
            &INV_SUB,
            &INV_FILTER,
            &INV_RESCAN,
            &INV_ADD,
            &NET_CHIP,
            &INV_EMPTY,
            &INV_EMPTY_WHY,
            &SCAN_EMPTY_WORDS,
            &SCAN_CHOOSE,
            &INV_SCAN_AGAIN,
            &HELP_LINK,
            &INV_CONNECT,
        ],
    },
    ScreenCopy {
        name: "connect",
        visible: &[
            &CONN_TITLE,
            &CONN_SUB,
            &CONN_PORT,
            &HK_PIN,
            &HK_PINNED_TAG,
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
            &HK_PIN,
            &HK_PINNED_TAG,
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
            &INSPECT_ARRAYS,
            &INSPECT_NO_VOLUMES,
            &INSPECT_COL_VOLUME,
            &INSPECT_COL_FS,
            &INSPECT_COL_USED,
            &INSPECT_COL_BACKED,
            &INSPECT_COL_STATUS,
            &INSPECT_USE_TARGET,
            &INSPECT_USE_NOT,
            &INSPECT_ERASE_NOTE,
            &INSPECT_NOT_CHECKED,
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
            &GOAL_QUICK,
            &GOAL_CUSTOMIZE,
            &GOAL_RECOVER,
            &GOAL_RECOVER_LINE,
            &GOAL_TAG_SAFE,
            &GOAL_TAG_ERASE,
            &GOAL_TAG_READS,
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
            &SYS_NEXT_ADDONS,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "addons",
        visible: &[&SETUP_ADDONS, &ADDONS_SUB, &ADDONS_INTRO, &ADDON_UNTESTED, &ADDON_REQUIRED, &ADDON_NEEDS_DATA, &SYS_NEXT, &BTN_BACK],
    },
    ScreenCopy {
        name: "confirm",
        visible: &[
            &CONF_TITLE,
            &CONF_SUB,
            &CONF_DESTROYED,
            &CONF_CHANGED,
            &CONF_PRESERVED,
            &CONF_NO_BACKUP,
            &CONF_ERASED,
            &CONF_LIST,
            &CONF_FALLBACK,
            &CONF_AFTER,
            &CONF_BACKUP,
            &CONF_LOSS,
            &CONF_UNIT,
            &CONF_ADD_SERIAL,
            &CONF_ERASE,
            &BTN_BACK,
        ],
    },
    ScreenCopy {
        name: "install",
        visible: &[&INSTALL_TITLE, &INSTALL_SUB, &INSTALL_FLASH, &INSTALL_STEPS, &INSTALL_NO_RETURN, &INSTALL_LOG, &INSTALL_CANCEL],
    },
    ScreenCopy {
        name: "failed",
        visible: &[
            &FAIL_TITLE,
            &FAIL_DISKS_CHANGED,
            &FAIL_FLASH_CHANGED,
            &FAIL_LOADER_IN,
            &FAIL_COPY_LOG,
            &FAIL_EXPORT,
            &FAIL_RECOVERY,
            &FAIL_NEXT,
            &FAIL_LOG,
        ],
    },
    ScreenCopy {
        name: "failed-agent",
        visible: &[
            &FAIL_TITLE,
            &FAIL_AGENT_TITLE,
            &FAIL_AGENT_TEXT,
            &FAIL_DISKS_UNTOUCHED,
            &FAIL_FLASH_CHANGED,
            &FAIL_DEFAULT_UNCONFIRMED,
            &FAIL_CLOCK_AGENT,
            &FAIL_CLOCK_AGENT_VALUE,
            &REC_MANUAL,
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
    ScreenCopy {
        name: "update",
        // the dialog of an unsigned release before the download: banner, one warning, the consent box, Skip, Later, Download and install
        visible: &[&UPD_BANNER, &UPD_DETAILS, &UPD_LATER, &UPD_ST_UNSIGNED, &UPD_CONSENT, &UPD_DOWNLOAD_INSTALL, &UPD_SKIP],
    },
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
            // the update row, worst state (an unsigned update): status, note, button
            &UPD_ST_AVAILABLE,
            &UPD_ST_UNSIGNED,
            &UPD_DETAILS,
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
    ScreenCopy {
        name: "storage",
        visible: &[
            &MAN_STORAGE,
            &RAID_ARRAY,
            &RAID_LEVEL,
            &RAID_STATE,
            &RAID_SIZE,
            &RAID_PROGRESS,
            &RAID_CREATE,
            &RAID_ACTIONS,
            &RAID_OPEN_WEB,
        ],
    },
    // D1b Welcome guide: one entry per step (a dialog over Find; the shell is counted anyway, to stay conservative)
    ScreenCopy { name: "welcome", visible: &[&WELCOME_1_TITLE, &WELCOME_1_A, &WELCOME_1_B, &WELCOME_1_NOTE, &WELCOME_SKIP, &WELCOME_NEXT] },
    ScreenCopy {
        name: "welcome-2",
        visible: &[
            &WELCOME_2_TITLE,
            &WELCOME_2_A,
            &WELCOME_2_B,
            &WELCOME_FLOW_FIND,
            &WELCOME_FLOW_CONNECT,
            &WELCOME_FLOW_GOAL,
            &WELCOME_FLOW_SETUP,
            &WELCOME_FLOW_CONFIRM,
            &WELCOME_FLOW_INSTALL,
            &WELCOME_SKIP,
            &BTN_BACK,
            &WELCOME_NEXT,
        ],
    },
    ScreenCopy { name: "welcome-3", visible: &[&WELCOME_3_TITLE, &WELCOME_3_A, &WELCOME_3_B, &WELCOME_SKIP, &BTN_BACK, &WELCOME_NEXT] },
    ScreenCopy { name: "welcome-4", visible: &[&WELCOME_4_TITLE, &WELCOME_4_A, &WELCOME_4_B, &WELCOME_SKIP, &BTN_BACK, &WELCOME_NEXT] },
    ScreenCopy { name: "welcome-5", visible: &[&WELCOME_5_TITLE, &WELCOME_5_A, &WELCOME_5_B, &WELCOME_HIDE, &BTN_BACK, &WELCOME_START] },
    ScreenCopy { name: "help", visible: &[&HELP_TITLE, &HELP_SEARCH, &HELP_TOPICS, &HELP_SHORTCUTS, &HELP_DOCS, &BTN_BACK, &WELCOME_MENU] },
    ScreenCopy {
        name: "about",
        visible: &[
            &ABOUT_TITLE,
            &ABOUT_TAGLINE,
            &ABOUT_COPY_DIAG,
            &ABOUT_DOCS,
            &ABOUT_SOURCE,
            &ABOUT_REPORT,
            &ABOUT_CREDITS,
            // the update line, worst state (an unsigned update): status, note, button
            &UPD_ST_AVAILABLE,
            &UPD_ST_UNSIGNED,
            &UPD_DETAILS,
        ],
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
        assert_eq!(sentences("Read the log, then the DSM loader; no data promise."), 1);
    }

    #[test]
    fn every_item_stays_inside_its_word_budget() {
        let mut bad: Vec<String> = vec![];
        for it in ALL {
            let n = words(it.short);
            let max = match it.kind {
                Kind::Title => 4,
                Kind::Subtitle => 12,
                Kind::Label => 4,
                Kind::Button => 3,
                Kind::Prose => 12,
                Kind::Topic => 4,
                Kind::Dialog => 4,
            };
            if n > max {
                bad.push(format!("{}: {:?} has {n} words (max {max} for {:?})", it.key, it.short, it.kind));
            }
            if !it.tip.is_empty() {
                if words(it.tip) > 24 {
                    bad.push(format!("{}: tooltip has {} words (max 24)", it.key, words(it.tip)));
                }
                if sentences(it.tip) != 1 {
                    bad.push(format!("{}: a tooltip is exactly one sentence: {:?}", it.key, it.tip));
                }
            }
            let body_max = match it.kind {
                Kind::Topic => 150,
                Kind::Dialog => 90,
                _ => 40,
            };
            if words(it.body) > body_max {
                bad.push(format!("{}: body has {} words (max {body_max})", it.key, words(it.body)));
            }
            if it.kind == Kind::Topic && (it.short.len() <= 2 || it.body.is_empty()) {
                bad.push(format!("{}: a topic needs a title and a body", it.key));
            }
        }
        assert!(bad.is_empty(), "copy budget violations:\n{}", bad.join("\n"));
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
        assert_eq!(TOPICS.len(), 14);
        for (id, it, docs) in TOPICS {
            assert_eq!(it.kind, Kind::Topic, "{id}");
            assert!(docs.starts_with("docs/"), "{id}");
        }
    }

    #[test]
    fn shell_is_eleven_words() {
        let n: usize = SHELL.iter().map(|i| words(i.short)).sum();
        assert_eq!(n, 11, "six steps, the wordmark, a two-word risk Tag and Way back");
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
            let total = own + shell;
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
            "addons",
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
            "welcome",
            "welcome-2",
            "welcome-3",
            "welcome-4",
            "welcome-5",
        ] {
            assert!(SCREENS.iter().any(|s| s.name == n), "screen {n} has no copy declaration");
        }
    }

    #[test]
    fn help_search_finds_titles_and_bodies() {
        assert_eq!(search_topics("").len(), 14);
        assert!(search_topics("rescue shell").iter().any(|(id, _, _)| *id == "recovery"));
        assert!(search_topics("kexec").iter().any(|(id, _, _)| *id == "boot-manager"));
        assert!(search_topics("zzzzqq").is_empty());
        assert!(search_topics("FIRST install").iter().any(|(id, _, _)| *id == "first-install"));
    }

    #[test]
    fn the_numbers_in_the_text_are_the_engines_timing_constants() {
        use synofurb_core::timing as t;
        let has = |it: &Item, n: u32, unit: &str| {
            let w = format!("{n} {unit}");
            assert!(it.short.contains(&w) || it.tip.contains(&w) || it.body.contains(&w), "{}: expected {w:?}", it.key);
        };
        has(&GOAL_BOOTMGR, t::BOOT_MANAGER_WATCHDOG_MIN, "minute");
        has(&BM_WATCHDOG, t::BOOT_MANAGER_WATCHDOG_MIN, "min");
        has(&BM_LOGIN_TIMEOUT, t::BOOT_MANAGER_WATCHDOG_MIN, "minutes");
        has(&FAIL_AGENT_TEXT, t::AGENT_WAIT_MIN, "minutes");
        has(&FAIL_CLOCK_AGENT, t::INSTALL_WATCHDOG_MIN, "minutes");
        has(&DLG_NEXT_AGENT, t::AGENT_WAIT_MIN, "minutes");
        has(&DLG_NEXT_PRE, t::FAIL_REBOOT_BEFORE_PONR_SECS, "seconds");
        has(&INSTALL_CANCEL_DIALOG, t::FAIL_REBOOT_BEFORE_PONR_SECS, "seconds");
        has(&DLG_NEXT_WIPE, t::FAIL_REBOOT_AFTER_PONR_SECS / 60, "minutes");
        has(&FAIL_LOADER_IN, t::FAIL_REBOOT_AFTER_PONR_SECS / 60, "minutes");
        assert_eq!(t::watchdog_minutes_possibly_left(t::INSTALL_WATCHDOG_MIN, t::AGENT_WAIT_MIN), 20);
        assert!(FAIL_CLOCK_AGENT_VALUE.short.starts_with("20 min"));
    }
}
