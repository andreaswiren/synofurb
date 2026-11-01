//! synofurb - reinstall Synology NAS units with plain Debian 13.
//!
//! No arguments (or `gui`) opens the desktop app; every other subcommand is the command line from `synofurb_app::cli`, identical to
//! `synofurb-cli` (which has no GUI libraries and runs on headless servers).
mod ui;

use clap::{CommandFactory, Parser, Subcommand};
use std::process::ExitCode;
use synofurb_app::{cli, update_cmd};

#[derive(Parser)]
#[command(name = "synofurb", version, about = cli::args::ABOUT, after_help = cli::args::AFTER_HELP, propagate_version = true)]
struct Args {
    #[command(flatten)]
    global: cli::Global,
    #[command(subcommand)]
    cmd: Option<GuiOrCli>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once at startup; boxing would only obscure the clap derive
enum GuiOrCli {
    /// Open the graphical wizard (default)
    Gui {
        /// load a built-in demo NAS (4-bay DS918+) instead of scanning; nothing touches the network
        #[arg(long)]
        demo: bool,
        /// with --demo: start on this screen (inventory, connect, inspect, goal, plan, system, confirm, install, failed,
        /// done, update, settings, bootmgr, recover)
        #[arg(long, default_value = "inventory")]
        screen: String,
    },
    #[command(flatten)]
    Cli(cli::Cmd),
}

fn main() -> ExitCode {
    update_cmd::finish_previous_update();
    let args = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => e.exit(),
    };
    let program = cli::Program { name: "synofurb", gui: true, command: Args::command };
    match args.cmd {
        None | Some(GuiOrCli::Gui { .. }) => {
            let (demo, screen) = match args.cmd {
                Some(GuiOrCli::Gui { demo, screen }) => (demo, screen),
                _ => (false, String::new()),
            };
            match ui::run(demo.then_some(screen.as_str())) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
        Some(GuiOrCli::Cli(cmd)) => cli::dispatch(&program, &args.global, cmd),
    }
}
