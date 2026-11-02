//! synofurb - reinstall Synology NAS units with plain Debian 13.
//!
//! No arguments (or `gui`) opens the desktop app; every other subcommand is the command line from `synofurb_app::cli`, identical to
//! `synofurb-cli` (which has no GUI libraries and runs on headless servers).
// Release builds on Windows use the GUI subsystem so that opening the app does not also open an empty console window.
// The command-line subcommands of this binary still print: `attach_parent_console` connects them to the console they were started from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
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

/// A GUI-subsystem process starts without standard handles. When it was started from a console (cmd, PowerShell, Windows Terminal) and
/// asked to do a command-line job, attach to that console and point stdin/stdout/stderr at it, unless the caller redirected them.
/// Started by a double click there is no parent console and nothing to attach to (the app just opens its window).
#[cfg(windows)]
fn attach_parent_console() {
    use std::fs::OpenOptions;
    use std::os::windows::io::IntoRawHandle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(std_handle: u32) -> isize;
        fn SetStdHandle(std_handle: u32, handle: isize) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    // SAFETY: plain Win32 calls with integer arguments; the handles we install are leaked on purpose (they live for the process).
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for (std, device, writable) in
            [(STD_INPUT_HANDLE, "CONIN$", false), (STD_OUTPUT_HANDLE, "CONOUT$", true), (STD_ERROR_HANDLE, "CONOUT$", true)]
        {
            let current = GetStdHandle(std);
            if current == 0 || current == -1 {
                if let Ok(f) = OpenOptions::new().read(!writable).write(writable).open(device) {
                    SetStdHandle(std, f.into_raw_handle() as isize);
                }
            }
        }
    }
}

fn main() -> ExitCode {
    #[cfg(windows)]
    {
        // any argument other than the GUI itself means a command-line job
        if std::env::args_os().nth(1).is_some_and(|a| a != "gui") {
            attach_parent_console();
        }
    }
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
