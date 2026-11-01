//! The command line: every capability of the desktop app, scriptable, plus a text wizard.
//!
//! Layout: `args` (clap definitions), `exit` (stable exit codes), `out` (colour/verbosity/NDJSON/progress), `runner` (worker thread +
//! Ctrl-C), `facts` (live/saved inspection, JSON documents), `spec` (install choices -> config/job), `gate` (the three-part safety gate),
//! `dryrun`, `install` (inspect/plan/install), `scan`, `compat_cmd`, `boot`, `manage_cmd` (health and updates of the web UI on a converted unit), `logs_cmd`, `bundle`, `misc`, `prompt` and `wizard`.
//! Both programs (`synofurb` and `synofurb-cli`) call [`dispatch`].

pub mod args;
pub mod boot;
pub mod bundle;
pub mod compat_cmd;
pub mod dryrun;
pub mod exit;
pub mod facts;
pub mod gate;
pub mod install;
pub mod logs_cmd;
pub mod manage_cmd;
pub mod misc;
pub mod out;
pub mod prompt;
pub mod render;
pub mod runner;
pub mod scan;
pub mod spec;
pub mod wizard;

pub use args::{Cli, Cmd, Global};
pub use facts::config_dir;

use clap::{CommandFactory, Parser};
use exit::{CliError, CliResult};
use out::Out;
use std::process::ExitCode;

/// Which program is running; decides the name in `--help`, `version` and `completions` and which release asset `self-update` fetches.
#[derive(Clone, Copy)]
pub struct Program {
    pub name: &'static str,
    /// The desktop UI is part of this program.
    pub gui: bool,
    /// Builds the program's whole command tree (for `completions`).
    pub command: fn() -> clap::Command,
}

/// `main` of `synofurb-cli`.
pub fn main_cli() -> ExitCode {
    crate::update_cmd::finish_previous_update();
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => e.exit(), // help/version -> 0, misuse -> 2
    };
    let program = Program { name: "synofurb-cli", gui: false, command: Cli::command };
    dispatch(&program, &cli.global, cli.cmd)
}

/// Run one command and turn its result into the process exit code (printing errors as the output mode asks).
pub fn dispatch(p: &Program, g: &Global, cmd: Cmd) -> ExitCode {
    let out = Out::from_global(g);
    match run(&out, p, cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            report_error(&out, &e);
            ExitCode::from(e.code())
        }
    }
}

pub fn report_error(out: &Out, e: &CliError) {
    runner::failure_event(out, e);
    // a health verdict was already printed as the command's result; the exit code carries the rest
    if matches!(e, CliError::Health { .. }) {
        return;
    }
    let msg = e.to_string();
    if out.quiet && out.json_events() {
        return;
    }
    out.error(&msg);
    if matches!(e, CliError::Usage(_)) {
        out.say("Try --help for the options.");
    }
}

pub fn run(out: &Out, p: &Program, cmd: Cmd) -> CliResult<()> {
    match cmd {
        Cmd::Scan(a) => scan::scan(out, a),
        Cmd::Inventory { action } => scan::inventory(out, action),
        Cmd::Inspect(a) => install::inspect(out, a),
        Cmd::Compat(a) => compat_cmd::compat(out, a),
        Cmd::Plan(a) => install::plan(out, a),
        Cmd::Install(a) => install::install(out, *a),
        Cmd::Wizard(a) => wizard::run_wizard(out, a),
        Cmd::BootManager { action } => boot::boot_manager(out, action),
        Cmd::Recover(a) => boot::recover(out, a),
        Cmd::BackupDom(a) => boot::backup_dom(out, a),
        Cmd::DiskBackup(a) => boot::disk_backup(out, a),
        Cmd::Logs { action } => logs_cmd::logs(out, action),
        Cmd::Manage { action } => manage_cmd::manage(out, action),
        Cmd::Bundle { action } => bundle::bundle(out, action),
        Cmd::FetchInstaller(a) => boot::fetch_installer(out, a),
        Cmd::SelfUpdate(a) => crate::update_cmd::run(out, a.check, a.channel, a.json, p.gui).map_err(CliError::from),
        Cmd::Config { action } => misc::config(out, p, action),
        Cmd::Completions { shell } => misc::completions(p, shell),
        Cmd::Version { json } => misc::version(out, p, json),
    }
}
