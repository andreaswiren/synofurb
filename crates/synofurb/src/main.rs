//! synofurb - reinstall Synology NAS units with plain Debian 13.
mod cli;
mod ui;
mod update_cmd;

use clap::Parser;

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    update_cmd::finish_previous_update();
    let args = cli::Cli::parse();
    match args.cmd {
        None | Some(cli::Cmd::Gui { .. }) => {
            let (demo, screen) = match args.cmd {
                Some(cli::Cmd::Gui { demo, screen }) => (demo, screen),
                _ => (false, String::new()),
            };
            ui::run(demo.then_some(screen.as_str()))
        }
        Some(cmd) => cli::run(cmd),
    }
}
