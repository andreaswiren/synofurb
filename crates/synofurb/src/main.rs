//! synofurb - reinstall Synology NAS units with plain Debian 13.
mod cli;
mod gui;

use clap::Parser;

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    match args.cmd {
        None | Some(cli::Cmd::Gui { .. }) => {
            let (demo, screen) = match args.cmd {
                Some(cli::Cmd::Gui { demo, screen }) => (demo, screen),
                _ => (false, String::new()),
            };
            let opts = eframe::NativeOptions {
                viewport: eframe::egui::ViewportBuilder::default()
                    .with_inner_size([1000.0, 760.0])
                    .with_min_inner_size([760.0, 520.0])
                    .with_title("synofurb"),
                ..Default::default()
            };
            eframe::run_native("synofurb", opts, Box::new(move |cc| Ok(Box::new(gui::App::new(cc, demo.then_some(screen.as_str()))))))
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        Some(cmd) => cli::run(cmd),
    }
}
