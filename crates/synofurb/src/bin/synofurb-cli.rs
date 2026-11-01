//! synofurb-cli - the command line of synofurb with NO graphical dependencies.
//!
//! Same subcommands as `synofurb` except `gui`. Build: `cargo build --release -p synofurb --no-default-features --bin synofurb-cli`.
//! Meant for SSH sessions, servers, containers and CI. Everything lives in the shared library; see `synofurb_app::cli`.

fn main() -> std::process::ExitCode {
    synofurb_app::cli::main_cli()
}
