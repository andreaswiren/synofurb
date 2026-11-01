//! The clap definitions: global flags, shared argument groups and every subcommand.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use synofurb_core::config::KernelChannel;
use synofurb_core::plan::{DataRaid, PlanRequest, StorageMode};

// ---------------------------------------------------------------------------------------------- top level

pub const ABOUT: &str = "Reinstall Synology NAS units with plain Debian 13 (trixie) - from a terminal";

pub const AFTER_HELP: &str = "\
EXIT CODES
  0 ok   1 error   2 usage   3 refused by the safety gate   4 plan has errors   5 unsupported hardware
  6 install failed AFTER the point of no return (disks changed)   7 install failed/cancelled BEFORE it (disks untouched)
  8 `manage health`: the unit reports WARN   9 `manage health`: the unit reports CRIT

EXAMPLES
  synofurb-cli scan                                   find units on the LAN
  synofurb-cli compat DS923+                          look a model up in the compatibility list
  synofurb-cli inspect --host 192.168.1.20 --json     read-only look at a running DSM
  synofurb-cli plan --host 192.168.1.20 --mode keep   what would be kept and what destroyed
  synofurb-cli wizard                                 guided, step by step, in the terminal
  synofurb-cli install --host 192.168.1.20 --dry-run  render everything, touch nothing
  synofurb-cli manage health --host 192.168.1.20      health of an installed unit (exit 8 warn, 9 crit)
  synofurb-cli manage update --host 192.168.1.20      update the web UI on the unit (rolls back by itself on failure)
  synofurb-cli completions bash > /etc/bash_completion.d/synofurb-cli

Docs: docs/CLI.md in the repository. Reinstalling repartitions the disks: make a verified backup first.";

/// The command line of `synofurb-cli` (no GUI). The `synofurb` program wraps [`Cmd`] with its own `gui` subcommand.
#[derive(Parser, Debug)]
#[command(name = "synofurb-cli", version, about = ABOUT, after_help = AFTER_HELP, propagate_version = true, arg_required_else_help = true)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Flags every command understands (they may be written before or after the subcommand).
#[derive(Args, Debug, Clone, Default)]
pub struct Global {
    /// `text` (default): readable output. `json`: one JSON event per line on stdout (phase, step, progress, log, point_of_no_return,
    /// failure, done, result); human text then goes to stderr
    #[arg(long, global = true, value_enum, value_name = "FORMAT", env = "SYNOFURB_LOG_FORMAT")]
    pub log_format: Option<LogFormat>,
    /// never use colour (also: NO_COLOR in the environment, TERM=dumb, output that is not a terminal)
    #[arg(long, global = true)]
    pub no_color: bool,
    /// only errors and the final result
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// more detail: -v installer and ssh messages, -vv every remote command
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// never ask questions; fail with a message instead (implied when stdin/stderr is not a terminal, or CI is set)
    #[arg(long, global = true, alias = "no-input")]
    pub non_interactive: bool,
    /// answer yes to ordinary confirmations. It NEVER satisfies the data-loss gate (--have-backup, --accept-data-loss, --confirm)
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    Text,
    Json,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)] // parsed once at startup; boxing would only obscure the derive
pub enum Cmd {
    /// Find Synology NAS units on the network (and remember them)
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli scan\n  synofurb-cli scan --timeout 8 --json > units.json\n  synofurb-cli scan --no-sweep --ndjson"
    )]
    Scan(ScanArgs),
    /// List, add or forget the units remembered from earlier scans
    Inventory {
        #[command(subcommand)]
        action: InventoryCmd,
    },
    /// Read-only look at a running DSM: disks, arrays, volumes, firmware, boot flash, serial console and the compatibility verdict
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli inspect --host 192.168.1.20\n  synofurb-cli inspect --host nas.lan --user admin --key ~/.ssh/id_ed25519 --json\n  synofurb-cli inspect --host 192.168.1.20 --save facts.json    # reuse later with --facts facts.json\n  synofurb-cli inspect --facts demo                              # built-in sample unit, no network"
    )]
    Inspect(InspectArgs),
    /// Look a Synology model up in the compatibility list (embedded copy; --refresh fetches the newest from the repository)
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli compat\n  synofurb-cli compat DS923+ --json\n  synofurb-cli compat --refresh\n  synofurb-cli compat --list --status verified"
    )]
    Compat(CompatArgs),
    /// Inspect a running DSM and show the install plan (read-only). Exit 4 when the plan has errors, 5 when the hardware is unsupported
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli plan --host 192.168.1.20\n  synofurb-cli plan --host 192.168.1.20 --mode keep --keep md2 --json\n  synofurb-cli plan --host 192.168.1.20 --system-gib 64 --copies 2 --raid 5 --free-percent 10\n  synofurb-cli plan --facts facts.json --mode wipe"
    )]
    Plan(PlanArgs),
    /// Install Debian. DESTROYS DATA. Needs --have-backup --accept-data-loss --confirm "ERASE <MODEL>" (or a terminal to type them)
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli install --host 192.168.1.20 --dry-run --out ./review     # render stage-2 script, payload, plan, preseed; change nothing\n  synofurb-cli install --host 192.168.1.20 --mode keep --keep md2 --hostname nas --username nasadmin \\\n      --have-backup --accept-data-loss --confirm \"ERASE DS918+\"\n  SYNOFURB_PASSWORD=... synofurb-cli --log-format json install --host 192.168.1.20 --password-file pw.txt --have-backup --accept-data-loss --confirm \"ERASE DS923+\"\n  synofurb-cli install --host 10.0.0.5 --source offline --bundle /srv/bundle ..."
    )]
    Install(Box<InstallArgs>),
    /// Guided install / boot manager / recovery in plain terminal prompts (the same nine steps as the app)
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli wizard\n  synofurb-cli wizard --demo        # rehearse with a built-in sample unit; nothing is contacted or changed\n  TERM=dumb synofurb-cli wizard --no-color"
    )]
    Wizard(WizardArgs),
    /// Boot manager mode: an SSH-controllable boot environment on the unit's internal boot flash, DSM stays the fail-safe default
    BootManager {
        #[command(subcommand)]
        action: BootManagerCmd,
    },
    /// Recovery: make the unit boot the original DSM again (from DSM, Debian or the boot agent)
    Recover(ConnArgs),
    /// Back up the unit's internal boot flash (DOM) to this computer, verified against the NAS, and change nothing
    BackupDom(BackupDomArgs),
    /// Back up the partition tables (first and last MiB) of the unit's disks to this computer
    DiskBackup(DiskBackupArgs),
    /// Day two: health and updates of the web management UI on a converted unit (health, version, update, rollback, open)
    Manage {
        #[command(subcommand)]
        action: ManageCmd,
    },
    /// Look at the log files of earlier runs
    Logs {
        #[command(subcommand)]
        action: LogsCmd,
    },
    /// Offline bundles: everything an install needs, for units or networks without Internet
    Bundle {
        #[command(subcommand)]
        action: BundleCmd,
    },
    /// Download and verify the Debian installer files (kernel/initrd) for an architecture
    FetchInstaller(FetchArgs),
    /// Check for (and install) a newer version from the GitHub releases; verifies the minisign signature and SHA-256
    SelfUpdate(SelfUpdateArgs),
    /// Show the effective configuration and where files live
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Print a shell completion script
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli completions bash | sudo tee /etc/bash_completion.d/synofurb-cli\n  synofurb-cli completions zsh > ~/.zfunc/_synofurb-cli\n  synofurb-cli completions fish > ~/.config/fish/completions/synofurb-cli.fish\n  synofurb-cli completions powershell | Out-String | Invoke-Expression"
    )]
    Completions {
        #[arg(value_enum)]
        shell: ShellArg,
    },
    /// Version, build and enabled features
    Version {
        #[arg(long)]
        json: bool,
    },
}

// ---------------------------------------------------------------------------------------------- shared groups

/// How to log in to the NAS. The password is never an argument: environment variable, file or prompt.
#[derive(Args, Debug, Clone)]
pub struct SshOpts {
    #[arg(long, default_value_t = 22)]
    pub port: u16,
    /// SSH user (a DSM administrator; sudo is used when it is not root)
    #[arg(long, default_value = "admin")]
    pub user: String,
    /// use this OpenSSH private key (unencrypted) instead of a password
    #[arg(long, value_name = "FILE")]
    pub key: Option<PathBuf>,
    /// environment variable holding the SSH password (asked for on a terminal when unset)
    #[arg(long, default_value = "SYNOFURB_PASSWORD", value_name = "VAR")]
    pub ssh_password_env: String,
    /// file whose first line is the SSH password
    #[arg(long, value_name = "FILE", conflicts_with = "key")]
    pub ssh_password_file: Option<PathBuf>,
    /// environment variable with the sudo password when it differs from the SSH password
    #[arg(long, default_value = "SYNOFURB_SUDO_PASSWORD", value_name = "VAR")]
    pub sudo_password_env: String,
}

/// A NAS reachable over SSH (required).
#[derive(Args, Debug, Clone)]
pub struct Conn {
    /// IP address or name of the NAS
    #[arg(long, value_name = "ADDRESS")]
    pub host: String,
    #[command(flatten)]
    pub ssh: SshOpts,
}

/// A NAS reachable over SSH, or a saved inspection (`--facts`) for offline work.
#[derive(Args, Debug, Clone)]
pub struct LiveOrFacts {
    /// IP address or name of the NAS
    #[arg(long, value_name = "ADDRESS", required_unless_present = "facts", conflicts_with = "facts")]
    pub host: Option<String>,
    #[command(flatten)]
    pub ssh: SshOpts,
    /// use a saved inspection instead of connecting: a file written by `inspect --save`/`--json`, raw inspection output, or `demo`
    /// for the built-in 4-bay DS918+
    #[arg(long, value_name = "FILE|demo")]
    pub facts: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ConnArgs {
    #[command(flatten)]
    pub conn: Conn,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ModeArg {
    /// destroy everything on every disk and start clean
    Wipe,
    /// reclaim only DSM's system/swap partitions and keep the data arrays
    Keep,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum RaidArg {
    Auto,
    None,
    #[value(name = "0")]
    R0,
    #[value(name = "1")]
    R1,
    #[value(name = "5")]
    R5,
    #[value(name = "6")]
    R6,
    #[value(name = "10")]
    R10,
    /// Synology-Hybrid-RAID-like (mdadm slices + LVM), any 1 disk may fail, 2+ disks of any size
    Shr1,
    /// Hybrid RAID with RAID 6 slices, any 2 disks may fail, 4+ disks of any size
    Shr2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum KernelArg {
    /// Linux 7.x from trixie-backports
    Newest,
    /// Debian 13's 6.12 LTS kernel
    Stable,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum MethodArg {
    /// choose by hardware: boot flash (DOM) units use `dom`, units with kexec use `kexec`, otherwise `kit`
    Auto,
    /// through the internal boot flash and the boot agent (x86 Synology), SSH only
    Dom,
    /// upload a static kexec to DSM
    Kexec,
    /// export an installer kit and boot it yourself (USB/TFTP/U-Boot)
    Kit,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum SourceArg {
    /// download the installer and packages from the Internet
    Online,
    /// use an offline bundle (--bundle DIR); nothing is downloaded
    Offline,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ArchArg {
    Amd64,
    Arm64,
    Armhf,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ShellArg {
    Bash,
    Zsh,
    Fish,
    Powershell,
}

/// Every planner knob.
#[derive(Args, Debug, Clone)]
pub struct PlanOpts {
    /// wipe: destroy everything and start clean. keep: reclaim only DSM's system and swap, keep the data arrays
    #[arg(long, value_enum, default_value = "wipe")]
    pub mode: ModeArg,
    /// system size in GiB (0 = recommended; see the plan for the allowed range)
    #[arg(long, default_value_t = 0, value_name = "GIB")]
    pub system_gib: u64,
    /// number of disks carrying a copy of the system (default: automatic)
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..=64))]
    pub copies: Option<u64>,
    /// RAID level of the new data volume; shr1/shr2 = Synology-Hybrid-RAID-like layout (mixed disk sizes, wiped disks only)
    #[arg(long, value_enum, default_value = "auto")]
    pub raid: RaidArg,
    /// percent of the data volume group left unallocated (0-90)
    #[arg(long, default_value_t = 20, value_name = "PERCENT", value_parser = clap::value_parser!(u8).range(0..=90))]
    pub free_percent: u8,
    /// with --mode keep: data arrays to preserve (repeat or comma-separate; default all), e.g. --keep md2
    #[arg(long, value_name = "MDx", value_delimiter = ',')]
    pub keep: Vec<String>,
}

impl PlanOpts {
    pub fn request(&self) -> PlanRequest {
        PlanRequest {
            mode: match self.mode {
                ModeArg::Wipe => StorageMode::WipeAll,
                ModeArg::Keep => StorageMode::KeepVolumes,
            },
            system_copies: self.copies.map(|c| c as usize),
            system_size_mib: self.system_gib * 1024,
            data_raid: match self.raid {
                RaidArg::Auto => DataRaid::Auto,
                RaidArg::None => DataRaid::None,
                RaidArg::R0 => DataRaid::Raid0,
                RaidArg::R1 => DataRaid::Raid1,
                RaidArg::R5 => DataRaid::Raid5,
                RaidArg::R6 => DataRaid::Raid6,
                RaidArg::R10 => DataRaid::Raid10,
                RaidArg::Shr1 => DataRaid::Shr1,
                RaidArg::Shr2 => DataRaid::Shr2,
            },
            data_free_percent: self.free_percent,
            keep_arrays: if self.keep.is_empty() { None } else { Some(self.keep.clone()) },
        }
    }
}

impl KernelArg {
    pub fn channel(self) -> KernelChannel {
        match self {
            KernelArg::Newest => KernelChannel::Newest,
            KernelArg::Stable => KernelChannel::Stable,
        }
    }
}

// ---------------------------------------------------------------------------------------------- scan / inventory

#[derive(Args, Debug, Clone)]
pub struct ScanArgs {
    /// how long each discovery method listens
    #[arg(long, default_value_t = 4, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..=120))]
    pub timeout: u64,
    /// skip the /24 port sweep (multicast/broadcast methods only)
    #[arg(long)]
    pub no_sweep: bool,
    /// print the final result as one JSON document: {"devices":[...]}
    #[arg(long, conflicts_with = "ndjson")]
    pub json: bool,
    /// stream one JSON object per line as units are found (the last line per unit is the most complete)
    #[arg(long)]
    pub ndjson: bool,
    /// also list hosts that were not identified as Synology
    #[arg(long)]
    pub all: bool,
    /// do not update the saved inventory
    #[arg(long)]
    pub no_save: bool,
}

#[derive(Subcommand, Debug)]
pub enum InventoryCmd {
    /// Show the remembered units
    List {
        #[arg(long)]
        json: bool,
        /// include hosts that were not identified as Synology
        #[arg(long)]
        all: bool,
    },
    /// Forget units matching an address, MAC, serial, model or host name (or everything with --all)
    Forget {
        /// address, MAC, serial, model or host name
        pattern: Option<String>,
        #[arg(long, conflicts_with = "pattern")]
        all: bool,
    },
    /// Add a unit by address (it is probed for a DSM web API; added either way)
    Add {
        #[arg(long, value_name = "ADDRESS")]
        ip: std::net::IpAddr,
        /// name to remember it by
        #[arg(long)]
        name: Option<String>,
    },
}

// ---------------------------------------------------------------------------------------------- inspect / compat / plan

#[derive(Args, Debug, Clone)]
pub struct InspectArgs {
    #[command(flatten)]
    pub src: LiveOrFacts,
    /// one JSON document: facts, platform, compatibility verdict, confirmation phrase
    #[arg(long)]
    pub json: bool,
    /// write the facts (reusable with --facts) to this file
    #[arg(long, value_name = "FILE")]
    pub save: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct CompatArgs {
    /// model name, e.g. "DS923+" or "RS3618xs" (omit for a summary)
    pub model: Option<String>,
    /// fetch the newest list from the repository first
    #[arg(long)]
    pub refresh: bool,
    #[arg(long)]
    pub json: bool,
    /// list every model (one per line)
    #[arg(long, conflicts_with = "model")]
    pub list: bool,
    /// with --list: only this status (verified, community, untested, known-issues, unsupported)
    #[arg(long, requires = "list", value_name = "STATUS")]
    pub status: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct PlanArgs {
    #[command(flatten)]
    pub src: LiveOrFacts,
    #[command(flatten)]
    pub opts: PlanOpts,
    /// which Linux kernel the new system would get (does not change the storage layout; shown so the whole plan reads in one place)
    #[arg(long, value_enum, default_value = "newest")]
    pub kernel: KernelArg,
    /// the whole plan as JSON
    #[arg(long)]
    pub json: bool,
}

// ---------------------------------------------------------------------------------------------- install

/// Everything that ends up on the NAS besides the storage layout.
#[derive(Args, Debug, Clone)]
pub struct SystemOpts {
    #[arg(long, default_value = "nas")]
    pub hostname: String,
    /// name of the new administrator account
    #[arg(long, default_value = "nasadmin")]
    pub username: String,
    /// read the new account's password from the first line of this file
    #[arg(long, value_name = "FILE", group = "pw")]
    pub password_file: Option<PathBuf>,
    /// read the new account's password from this environment variable
    #[arg(long, value_name = "VAR", group = "pw")]
    pub password_env: Option<String>,
    /// ask for the new account's password on the terminal (twice)
    #[arg(long, group = "pw")]
    pub prompt_password: bool,
    /// generate a strong password and show it at the end (the default when no other source is given)
    #[arg(long, group = "pw")]
    pub generate_password: bool,
    /// SSH key for the new account: `generate` (default, saved under the data directory), `none`, or the path of a PUBLIC key file
    #[arg(long, default_value = "generate", value_name = "generate|none|FILE")]
    pub authorize_key: String,
    #[arg(long, default_value = "UTC")]
    pub timezone: String,
    #[arg(long, default_value = "en_US.UTF-8")]
    pub locale: String,
    /// keyboard layout for the console
    #[arg(long, default_value = "us")]
    pub keymap: String,
    /// Debian mirror host name
    #[arg(long, default_value = "deb.debian.org", value_name = "HOST")]
    pub mirror: String,
    #[arg(long, default_value = "/debian", value_name = "PATH")]
    pub mirror_dir: String,
    /// extra Debian packages to install (repeat or comma-separate)
    #[arg(long = "package", value_name = "PKG", value_delimiter = ',')]
    pub packages: Vec<String>,
    /// serial console: a tty name (ttyS0) or a raw console such as uart8250,mmio32,0xfedc9000,115200n8 (default: what DSM uses)
    #[arg(long, value_name = "TTY|RAW")]
    pub serial: Option<String>,
    /// serial console speed in baud (default: what DSM uses, else 115200)
    #[arg(long, value_name = "BAUD")]
    pub serial_speed: Option<u32>,
    /// board device tree for ARM U-Boot units (a file name known to Debian, e.g. armada-385-synology-ds116.dtb)
    #[arg(long, value_name = "FILE.dtb")]
    pub dtb: Option<String>,
    /// do not install the web management plane on the new system
    #[arg(long)]
    pub no_mgmt: bool,
    /// which Linux kernel to install
    #[arg(long, value_enum, default_value = "newest")]
    pub kernel: KernelArg,
}

#[derive(Args, Debug, Clone)]
pub struct BootOpts {
    /// how the installer is started on the unit
    #[arg(long, value_enum, default_value = "auto")]
    pub method: MethodArg,
    /// directory with agent-<arch>.vmlinuz/.initrd (default: $SYNOFURB_AGENT_DIR, assets/agent, else downloaded from the release)
    #[arg(long, value_name = "DIR")]
    pub agent_dir: Option<PathBuf>,
    /// where the boot flash backup is written (default: <data dir>/dom-backups)
    #[arg(long, value_name = "DIR")]
    pub backup_dir: Option<PathBuf>,
    /// static kexec binary for the NAS architecture (see docs/kexec.md)
    #[arg(long, value_name = "FILE")]
    pub kexec: Option<PathBuf>,
    /// output folder for --method kit
    #[arg(long, default_value = "synofurb-kit", value_name = "DIR")]
    pub kit_dir: PathBuf,
    /// do not install non-free firmware packages
    #[arg(long)]
    pub no_firmware: bool,
    /// no debug logging inside the Debian installer
    #[arg(long)]
    pub no_verbose_installer: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SourceOpts {
    /// where the installer files and packages come from (default: online, or offline when --bundle is given)
    #[arg(long, value_enum)]
    pub source: Option<SourceArg>,
    /// offline bundle directory (made by `bundle create`); implies --source offline
    #[arg(long, value_name = "DIR", alias = "offline-bundle")]
    pub bundle: Option<PathBuf>,
}

/// The three-part data-loss gate. All three are needed; there is no flag that skips them.
#[derive(Args, Debug, Clone)]
pub struct GateOpts {
    /// I have a complete, verified backup of everything on this NAS
    #[arg(long)]
    pub have_backup: bool,
    /// I understand that data on this NAS can be destroyed, even data marked as preserved
    #[arg(long)]
    pub accept_data_loss: bool,
    /// type the confirmation phrase shown by `plan`/`inspect` (for example "ERASE DS918+")
    #[arg(long, value_name = "PHRASE")]
    pub confirm: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct InstallArgs {
    #[command(flatten)]
    pub src: LiveOrFacts,
    #[command(flatten)]
    pub plan: PlanOpts,
    #[command(flatten)]
    pub system: SystemOpts,
    #[command(flatten)]
    pub boot: BootOpts,
    #[command(flatten)]
    pub source: SourceOpts,
    #[command(flatten)]
    pub gate: GateOpts,
    /// do everything up to rendering and print (or write with --out) the generated stage-2 script, payload manifest, plan and
    /// preseed. The NAS is only READ (or not contacted at all with --facts); secrets are replaced by placeholders
    #[arg(long)]
    pub dry_run: bool,
    /// with --dry-run: write the artefacts into this directory instead of printing them
    #[arg(long, value_name = "DIR", requires = "dry_run")]
    pub out: Option<PathBuf>,
    /// with --dry-run: print one JSON document instead of text
    #[arg(long, requires = "dry_run")]
    pub json: bool,
    /// after a successful install, also write the logins to this file (mode 0600)
    #[arg(long, value_name = "FILE", conflicts_with = "dry_run")]
    pub save_logins: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct WizardArgs {
    /// rehearse with a built-in sample 4-bay DS918+: nothing is contacted or changed, the end is a dry run
    #[arg(long, conflicts_with = "facts")]
    pub demo: bool,
    /// rehearse with a saved inspection (`inspect --save`): nothing is contacted, the end is a dry run
    #[arg(long, value_name = "FILE")]
    pub facts: Option<String>,
    /// start at this address instead of asking
    #[arg(long, value_name = "ADDRESS")]
    pub host: Option<String>,
}

// ---------------------------------------------------------------------------------------------- boot manager / recovery / backups

#[derive(Subcommand, Debug)]
pub enum BootManagerCmd {
    /// Back up the boot flash, then install the agent (DSM stays the default unless --arm)
    Install {
        #[command(flatten)]
        conn: Conn,
        #[arg(long, value_name = "DIR")]
        agent_dir: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        backup_dir: Option<PathBuf>,
        /// host name of the agent on the network
        #[arg(long, default_value = "synofurb-agent")]
        agent_hostname: String,
        /// boot into the agent on the next reboot (it resets the default to DSM itself)
        #[arg(long)]
        arm: bool,
    },
    /// Arm the agent for the next boot, reboot DSM, wait for it and print how to use it
    Enter {
        #[command(flatten)]
        conn: Conn,
    },
    /// Show whether the unit can run the boot manager and what the boot flash tool reports
    Status {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ManageCmd {
    /// Health of the unit as its management plane reports it: arrays, disks, volumes, services, pending updates. Exit 0 ok, 8 warn, 9 crit
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli manage health --host 192.168.1.20\n  synofurb-cli manage health --host nas.lan --user nasadmin --key ~/.ssh/id_ed25519 --json\n  SYNOFURB_PASSWORD=... synofurb-cli --log-format json manage health --host 192.168.1.20\n\nExit codes: 0 ok, 8 warn, 9 crit, 1 the unit could not be asked (not installed, too old, unreachable)."
    )]
    Health {
        #[command(flatten)]
        conn: Conn,
        /// one JSON document: the unit's report plus the derived summary
        #[arg(long)]
        json: bool,
    },
    /// Version of the management plane on the unit compared with the one shipped in this program (read-only)
    #[command(
        after_help = "EXAMPLES\n  synofurb-cli manage version --host 192.168.1.20\n  synofurb-cli manage version --host 192.168.1.20 --json"
    )]
    Version {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        json: bool,
    },
    /// Update the management plane on the unit: upload, verify, swap, restart the web UI, check. The unit rolls back by itself on failure
    #[command(
        after_help = "Restarts the web UI for a few seconds. Disks, arrays, shares and the boot flash are not touched.\n\nEXAMPLES\n  synofurb-cli manage update --host 192.168.1.20\n  synofurb-cli manage update --host 192.168.1.20 --dry-run          # only say what would happen\n  synofurb-cli manage update --host 192.168.1.20 --yes              # no question\n  synofurb-cli manage update --host 10.0.0.5 --from-bundle /srv/bundle/mgmt   # an offline copy\n  synofurb-cli manage update --host 192.168.1.20 --force            # reinstall, or go back to an older version"
    )]
    Update {
        #[command(flatten)]
        conn: Conn,
        /// take the management plane from this folder instead of the copy built into this program: the files themselves, or an
        /// offline bundle / release folder that has them in `mgmt/` (a MANIFEST.sha256 in it is checked)
        #[arg(long, value_name = "DIR")]
        from_bundle: Option<PathBuf>,
        /// reinstall identical files, or install an older version than the unit has
        #[arg(long)]
        force: bool,
        /// connect and say what would happen, change nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Go back to the management plane version the unit had before its last update
    Rollback {
        #[command(flatten)]
        conn: Conn,
    },
    /// Print the address of the unit's web UI (https://<address>:8443); no connection is made
    Open {
        /// IP address or name of the NAS
        #[arg(long, value_name = "ADDRESS")]
        host: String,
    },
}

#[derive(Args, Debug, Clone)]
pub struct BackupDomArgs {
    #[command(flatten)]
    pub conn: Conn,
    /// where the image is written (default: <data dir>/dom-backups)
    #[arg(long, value_name = "DIR")]
    pub backup_dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct DiskBackupArgs {
    #[command(flatten)]
    pub conn: Conn,
    /// disks to back up, by their name in DSM (repeat or comma-separate; default: every internal disk)
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub disk: Vec<String>,
    /// root folder for the backup (default: <data dir>/backups)
    #[arg(long, value_name = "DIR")]
    pub out_dir: Option<PathBuf>,
}

// ---------------------------------------------------------------------------------------------- logs / bundle / misc

#[derive(Subcommand, Debug)]
pub enum LogsCmd {
    /// List the log files, newest first
    List {
        #[arg(long)]
        json: bool,
        /// include the generated stage-2 scripts and plans saved next to the logs
        #[arg(long)]
        all: bool,
    },
    /// Print a log file: N counts from the newest (1, the default)
    Show {
        #[arg(default_value_t = 1, value_name = "N")]
        n: usize,
        /// only the last LINES lines
        #[arg(long, value_name = "LINES")]
        tail: Option<usize>,
    },
    /// Print the folder the logs are written to (--latest: the newest file)
    Path {
        #[arg(long)]
        latest: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum BundleCmd {
    /// Download everything an offline install needs into a directory
    Create(BundleCreateArgs),
    /// Check a bundle against its manifest
    Verify {
        #[arg(value_name = "DIR")]
        dir: PathBuf,
    },
    /// Show what a bundle contains
    Info {
        #[arg(value_name = "DIR")]
        dir: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Debug, Clone)]
pub struct BundleCreateArgs {
    /// architectures to include (repeat or comma-separate): amd64, arm64, armhf, or all (default: amd64)
    #[arg(long = "arch", value_name = "ARCH", value_delimiter = ',')]
    pub archs: Vec<String>,
    /// Debian suite
    #[arg(long, default_value = "trixie")]
    pub suite: String,
    /// include the newest (backports) kernel as well as the stable one
    #[arg(long)]
    pub kernel_newest: bool,
    /// leave out non-free firmware packages
    #[arg(long)]
    pub no_firmware: bool,
    /// extra Debian packages to include (repeat or comma-separate)
    #[arg(long = "package", value_name = "PKG", value_delimiter = ',')]
    pub extra: Vec<String>,
    /// output directory
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// leave the boot agent out of the bundle
    #[arg(long)]
    pub no_agent: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FetchArgs {
    #[arg(value_enum)]
    pub arch: ArchArg,
    #[arg(long, default_value = "trixie")]
    pub suite: String,
    /// also fetch the non-free firmware archive
    #[arg(long)]
    pub firmware: bool,
    /// ARM only: board device tree to fetch
    #[arg(long, value_name = "FILE.dtb")]
    pub dtb: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SelfUpdateArgs {
    /// only report whether an update exists
    #[arg(long)]
    pub check: bool,
    /// `stable` (default) or `prerelease`
    #[arg(long, value_name = "CHANNEL")]
    pub channel: Option<String>,
    /// with --check: print the result as JSON
    #[arg(long, requires = "check")]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// Effective configuration: version, paths, environment, update channel, compatibility list
    Show {
        #[arg(long)]
        json: bool,
    },
    /// Where everything lives
    Paths {
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut v = vec!["synofurb-cli"];
        v.extend_from_slice(args);
        Cli::try_parse_from(v)
    }

    fn ok(args: &[&str]) -> Cli {
        parse(args).unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn err(args: &[&str]) -> String {
        parse(args).err().unwrap_or_else(|| panic!("{args:?} should not parse")).to_string()
    }

    #[test]
    fn the_command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_top_level_command_exists_and_has_no_gui() {
        let cmd = Cli::command();
        let names: Vec<&str> = cmd.get_subcommands().map(|c| c.get_name()).collect();
        for want in [
            "scan",
            "inventory",
            "inspect",
            "compat",
            "plan",
            "install",
            "wizard",
            "boot-manager",
            "recover",
            "backup-dom",
            "disk-backup",
            "manage",
            "logs",
            "bundle",
            "fetch-installer",
            "self-update",
            "config",
            "completions",
            "version",
        ] {
            assert!(names.contains(&want), "missing subcommand {want}: {names:?}");
        }
        assert!(!names.contains(&"gui"), "the headless CLI has no gui subcommand");
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        let c = ok(&["--log-format", "json", "-vv", "--no-color", "-q", "scan"]);
        assert_eq!(c.global.log_format, Some(LogFormat::Json));
        assert_eq!((c.global.verbose, c.global.no_color, c.global.quiet), (2, true, true));
        let c = ok(&["scan", "--log-format", "json", "-v", "--no-color", "--non-interactive", "-y"]);
        assert_eq!(c.global.log_format, Some(LogFormat::Json));
        assert!(c.global.no_color && c.global.non_interactive && c.global.yes);
        assert!(err(&["--log-format", "yaml", "scan"]).contains("invalid value"));
    }

    #[test]
    fn scan_flags() {
        let Cmd::Scan(a) = ok(&["scan", "--timeout", "9", "--no-sweep", "--json"]).cmd else { panic!() };
        assert_eq!((a.timeout, a.no_sweep, a.json, a.ndjson), (9, true, true, false));
        assert!(err(&["scan", "--json", "--ndjson"]).contains("cannot be used"));
        assert!(err(&["scan", "--timeout", "0"]).contains("not in 1..=120"));
    }

    #[test]
    fn inventory_subcommands() {
        assert!(matches!(ok(&["inventory", "list", "--json"]).cmd, Cmd::Inventory { action: InventoryCmd::List { json: true, .. } }));
        let Cmd::Inventory { action: InventoryCmd::Add { ip, name } } =
            ok(&["inventory", "add", "--ip", "192.168.1.9", "--name", "attic"]).cmd
        else {
            panic!()
        };
        assert_eq!((ip.to_string(), name.as_deref()), ("192.168.1.9".into(), Some("attic")));
        assert!(err(&["inventory", "add", "--ip", "not-an-ip"]).contains("invalid"));
        assert!(matches!(ok(&["inventory", "forget", "--all"]).cmd, Cmd::Inventory { action: InventoryCmd::Forget { all: true, .. } }));
        assert!(err(&["inventory", "forget", "x", "--all"]).contains("cannot be used"));
    }

    #[test]
    fn inspect_needs_a_host_or_saved_facts() {
        assert!(err(&["inspect"]).contains("--host"));
        let Cmd::Inspect(a) = ok(&["inspect", "--host", "10.0.0.2", "--user", "root", "--port", "2222", "--key", "k", "--json"]).cmd else {
            panic!()
        };
        assert_eq!((a.src.host.as_deref(), a.src.ssh.port, a.src.ssh.user.as_str(), a.json), (Some("10.0.0.2"), 2222, "root", true));
        let Cmd::Inspect(a) = ok(&["inspect", "--facts", "demo", "--save", "f.json"]).cmd else { panic!() };
        assert_eq!(a.src.facts.as_deref(), Some("demo"));
        assert!(err(&["inspect", "--host", "h", "--facts", "demo"]).contains("cannot be used"));
        assert!(err(&["inspect", "--host", "h", "--key", "k", "--ssh-password-file", "p"]).contains("cannot be used"));
    }

    #[test]
    fn plan_knobs() {
        let Cmd::Plan(a) = ok(&[
            "plan",
            "--host",
            "h",
            "--mode",
            "keep",
            "--keep",
            "md2,md3",
            "--keep",
            "md4",
            "--system-gib",
            "64",
            "--copies",
            "2",
            "--raid",
            "10",
            "--free-percent",
            "35",
            "--kernel",
            "stable",
            "--json",
        ])
        .cmd
        else {
            panic!()
        };
        assert_eq!(a.kernel, KernelArg::Stable);
        let r = a.opts.request();
        assert_eq!(r.mode, StorageMode::KeepVolumes);
        assert_eq!(r.keep_arrays, Some(vec!["md2".into(), "md3".into(), "md4".into()]));
        assert_eq!((r.system_size_mib, r.system_copies, r.data_free_percent), (64 * 1024, Some(2), 35));
        assert_eq!(r.data_raid, DataRaid::Raid10);
        let Cmd::Plan(d) = ok(&["plan", "--host", "h"]).cmd else { panic!() };
        let r = d.opts.request();
        assert_eq!(
            (r.mode, r.system_size_mib, r.system_copies, r.data_free_percent, r.keep_arrays),
            (StorageMode::WipeAll, 0, None, 20, None)
        );
        for (v, want) in [
            ("auto", DataRaid::Auto),
            ("none", DataRaid::None),
            ("0", DataRaid::Raid0),
            ("1", DataRaid::Raid1),
            ("5", DataRaid::Raid5),
            ("6", DataRaid::Raid6),
            ("10", DataRaid::Raid10),
            ("shr1", DataRaid::Shr1),
            ("shr2", DataRaid::Shr2),
        ] {
            let Cmd::Plan(a) = ok(&["plan", "--host", "h", "--raid", v]).cmd else { panic!() };
            assert_eq!(a.opts.request().data_raid, want, "--raid {v}");
        }
        assert!(err(&["plan", "--host", "h", "--free-percent", "95"]).contains("not in 0..=90"));
        assert!(err(&["plan", "--host", "h", "--raid", "7"]).contains("invalid value"));
        assert!(err(&["plan", "--host", "h", "--copies", "0"]).contains("not in 1..=64"));
        assert!(err(&["plan", "--host", "h", "--mode", "shred"]).contains("invalid value"));
    }

    #[test]
    fn install_accepts_every_knob() {
        let Cmd::Install(a) = ok(&[
            "install",
            "--host",
            "192.168.1.20",
            "--mode",
            "keep",
            "--keep",
            "md2",
            "--hostname",
            "vault",
            "--username",
            "andreas",
            "--password-file",
            "pw.txt",
            "--authorize-key",
            "id.pub",
            "--timezone",
            "Europe/Stockholm",
            "--locale",
            "sv_SE.UTF-8",
            "--keymap",
            "se",
            "--mirror",
            "ftp.se.debian.org",
            "--mirror-dir",
            "/debian",
            "--package",
            "htop,tmux",
            "--package",
            "iperf3",
            "--serial",
            "uart8250,mmio32,0xfedc9000,115200n8",
            "--serial-speed",
            "115200",
            "--dtb",
            "armada-385-synology-ds116.dtb",
            "--no-mgmt",
            "--kernel",
            "stable",
            "--method",
            "dom",
            "--agent-dir",
            "/a",
            "--backup-dir",
            "/b",
            "--kexec",
            "/k",
            "--kit-dir",
            "/kit",
            "--no-firmware",
            "--no-verbose-installer",
            "--source",
            "offline",
            "--bundle",
            "/bundle",
            "--have-backup",
            "--accept-data-loss",
            "--confirm",
            "ERASE DS918+",
        ])
        .cmd
        else {
            panic!()
        };
        assert_eq!(a.system.packages, vec!["htop", "tmux", "iperf3"]);
        assert_eq!(a.system.serial.as_deref(), Some("uart8250,mmio32,0xfedc9000,115200n8"));
        assert!(a.system.no_mgmt && a.boot.no_firmware && a.boot.no_verbose_installer);
        assert_eq!((a.boot.method, a.system.kernel, a.source.source), (MethodArg::Dom, KernelArg::Stable, Some(SourceArg::Offline)));
        assert_eq!(a.source.bundle.as_deref(), Some(std::path::Path::new("/bundle")));
        assert!(a.gate.have_backup && a.gate.accept_data_loss);
        assert_eq!(a.gate.confirm.as_deref(), Some("ERASE DS918+"));
        assert!(!a.dry_run);
    }

    #[test]
    fn install_defaults_have_no_gate_flags_set() {
        let Cmd::Install(a) = ok(&["install", "--host", "h"]).cmd else { panic!() };
        assert!(!a.gate.have_backup && !a.gate.accept_data_loss && a.gate.confirm.is_none(), "the gate is never on by default");
        assert_eq!(
            (a.system.hostname.as_str(), a.system.username.as_str(), a.system.authorize_key.as_str()),
            ("nas", "nasadmin", "generate")
        );
        assert_eq!((a.boot.method, a.source.source), (MethodArg::Auto, None));
    }

    #[test]
    fn passwords_never_travel_on_argv() {
        for flag in ["--password", "--new-password", "--ssh-password", "--sudo-password"] {
            assert!(err(&["install", "--host", "h", flag, "hunter2"]).contains("unexpected argument"), "{flag} must not exist");
        }
        assert!(err(&["install", "--host", "h", "--password-file", "a", "--password-env", "B"]).contains("cannot be used"));
        assert!(err(&["install", "--host", "h", "--prompt-password", "--generate-password"]).contains("cannot be used"));
    }

    #[test]
    fn dry_run_options_depend_on_dry_run() {
        assert!(err(&["install", "--host", "h", "--out", "d"]).contains("--dry-run"));
        let Cmd::Install(a) = ok(&["install", "--facts", "demo", "--dry-run", "--out", "d", "--json"]).cmd else { panic!() };
        assert!(a.dry_run && a.json && a.out.is_some());
        assert!(err(&["install", "--host", "h", "--dry-run", "--save-logins", "f"]).contains("cannot be used"));
    }

    #[test]
    fn recover_boot_manager_and_backups() {
        assert!(err(&["recover"]).contains("--host"));
        assert!(matches!(ok(&["recover", "--host", "h"]).cmd, Cmd::Recover(_)));
        let Cmd::BootManager { action: BootManagerCmd::Install { arm, backup_dir, .. } } =
            ok(&["boot-manager", "install", "--host", "h", "--arm", "--backup-dir", "/b"]).cmd
        else {
            panic!()
        };
        assert!(arm && backup_dir.is_some());
        assert!(matches!(ok(&["boot-manager", "enter", "--host", "h"]).cmd, Cmd::BootManager { action: BootManagerCmd::Enter { .. } }));
        assert!(matches!(
            ok(&["boot-manager", "status", "--host", "h", "--json"]).cmd,
            Cmd::BootManager { action: BootManagerCmd::Status { json: true, .. } }
        ));
        let Cmd::DiskBackup(d) = ok(&["disk-backup", "--host", "h", "--disk", "sata1,sata2"]).cmd else { panic!() };
        assert_eq!(d.disk, vec!["sata1", "sata2"]);
        assert!(matches!(ok(&["backup-dom", "--host", "h"]).cmd, Cmd::BackupDom(_)));
    }

    #[test]
    fn logs_bundle_config_misc() {
        assert!(matches!(ok(&["logs", "list"]).cmd, Cmd::Logs { action: LogsCmd::List { json: false, all: false } }));
        let Cmd::Logs { action: LogsCmd::Show { n, tail } } = ok(&["logs", "show", "3", "--tail", "50"]).cmd else { panic!() };
        assert_eq!((n, tail), (3, Some(50)));
        let Cmd::Logs { action: LogsCmd::Show { n, .. } } = ok(&["logs", "show"]).cmd else { panic!() };
        assert_eq!(n, 1);
        assert!(matches!(ok(&["logs", "path", "--latest"]).cmd, Cmd::Logs { action: LogsCmd::Path { latest: true } }));
        let Cmd::Bundle { action: BundleCmd::Create(b) } =
            ok(&["bundle", "create", "--arch", "amd64,arm64", "--out", "o", "--kernel-newest", "--no-agent", "--package", "htop"]).cmd
        else {
            panic!()
        };
        assert_eq!(
            (b.archs.clone(), b.kernel_newest, b.no_agent, b.suite.as_str()),
            (vec!["amd64".to_string(), "arm64".into()], true, true, "trixie")
        );
        let Cmd::Bundle { action: BundleCmd::Create(d) } = ok(&["bundle", "create", "--out", "o"]).cmd else { panic!() };
        assert!(d.archs.is_empty(), "--arch is optional: the bundle code defaults to amd64");
        let Cmd::Install(i) = ok(&["install", "--host", "h", "--offline-bundle", "/b"]).cmd else { panic!() };
        assert_eq!(i.source.bundle.as_deref(), Some(std::path::Path::new("/b")), "alias used in the bundle command's own hint");
        assert!(matches!(ok(&["bundle", "verify", "d"]).cmd, Cmd::Bundle { action: BundleCmd::Verify { .. } }));
        assert!(matches!(ok(&["bundle", "info", "d", "--json"]).cmd, Cmd::Bundle { action: BundleCmd::Info { json: true, .. } }));
        assert!(matches!(ok(&["config", "paths", "--json"]).cmd, Cmd::Config { action: ConfigCmd::Paths { json: true } }));
        assert!(matches!(ok(&["config", "show"]).cmd, Cmd::Config { action: ConfigCmd::Show { json: false } }));
        for s in ["bash", "zsh", "fish", "powershell"] {
            assert!(matches!(ok(&["completions", s]).cmd, Cmd::Completions { .. }), "{s}");
        }
        assert!(err(&["completions", "tcsh"]).contains("invalid value"));
        assert!(matches!(ok(&["version", "--json"]).cmd, Cmd::Version { json: true }));
        let Cmd::SelfUpdate(u) = ok(&["self-update", "--check", "--channel", "prerelease", "--json"]).cmd else { panic!() };
        assert!(u.check && u.json && u.channel.as_deref() == Some("prerelease"));
        assert!(err(&["self-update", "--json"]).contains("--check"));
        let Cmd::FetchInstaller(f) = ok(&["fetch-installer", "arm64", "--suite", "trixie", "--firmware"]).cmd else { panic!() };
        assert_eq!((f.arch, f.firmware), (ArchArg::Arm64, true));
        let Cmd::Compat(c) = ok(&["compat", "DS923+", "--json", "--refresh"]).cmd else { panic!() };
        assert_eq!((c.model.as_deref(), c.json, c.refresh), (Some("DS923+"), true, true));
        assert!(err(&["compat", "--status", "verified"]).contains("--list"));
    }

    #[test]
    fn manage_commands() {
        let Cmd::Manage { action: ManageCmd::Health { conn, json } } =
            ok(&["manage", "health", "--host", "10.0.0.2", "--user", "root", "--port", "2222", "--key", "k", "--json"]).cmd
        else {
            panic!()
        };
        assert_eq!((conn.host.as_str(), conn.ssh.port, conn.ssh.user.as_str(), json), ("10.0.0.2", 2222, "root", true));
        assert!(err(&["manage", "health"]).contains("--host"));
        assert!(matches!(
            ok(&["manage", "version", "--host", "h", "--json"]).cmd,
            Cmd::Manage { action: ManageCmd::Version { json: true, .. } }
        ));
        let Cmd::Manage { action: ManageCmd::Update { from_bundle, force, dry_run, .. } } =
            ok(&["manage", "update", "--host", "h", "--from-bundle", "/b", "--force", "--dry-run"]).cmd
        else {
            panic!()
        };
        assert_eq!((from_bundle.as_deref(), force, dry_run), (Some(std::path::Path::new("/b")), true, true));
        let Cmd::Manage { action: ManageCmd::Update { from_bundle, force, dry_run, .. } } = ok(&["manage", "update", "--host", "h"]).cmd
        else {
            panic!()
        };
        assert_eq!((from_bundle, force, dry_run), (None, false, false), "an update never forces or skips the question by default");
        // --yes is the global flag, accepted after the subcommand
        let c = ok(&["manage", "update", "--host", "h", "--yes", "--log-format", "json"]);
        assert!(c.global.yes && c.global.log_format == Some(LogFormat::Json));
        assert!(matches!(ok(&["manage", "rollback", "--host", "h"]).cmd, Cmd::Manage { action: ManageCmd::Rollback { .. } }));
        let Cmd::Manage { action: ManageCmd::Open { host } } = ok(&["manage", "open", "--host", "nas.lan"]).cmd else { panic!() };
        assert_eq!(host, "nas.lan");
        assert!(err(&["manage", "open"]).contains("--host"));
        assert!(err(&["manage"]).contains("Usage") || err(&["manage"]).contains("subcommand"));
        // passwords never travel on argv here either
        for flag in ["--password", "--ssh-password", "--sudo-password"] {
            assert!(err(&["manage", "health", "--host", "h", flag, "x"]).contains("unexpected argument"), "{flag}");
        }
    }

    #[test]
    fn wizard_flags() {
        let Cmd::Wizard(w) = ok(&["wizard", "--demo"]).cmd else { panic!() };
        assert!(w.demo);
        assert!(err(&["wizard", "--demo", "--facts", "x"]).contains("cannot be used"));
    }

    #[test]
    fn no_subcommand_is_a_usage_error() {
        let e = parse(&[]).err().unwrap();
        assert_eq!(e.exit_code(), 2);
        assert_eq!(parse(&["--help"]).err().unwrap().exit_code(), 0);
        assert_eq!(parse(&["bogus"]).err().unwrap().exit_code(), 2);
    }
}
