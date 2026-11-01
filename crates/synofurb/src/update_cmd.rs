//! `synofurb self-update`: check GitHub for a newer release, download it, verify it, install it.
//!
//! The logic lives in `synofurb_core::updater`; this file is only the terminal front end. Wiring (not done here):
//!
//! ```ignore
//! mod update_cmd;
//! // in the clap enum:
//! //   /// Check for a newer synofurb and install it
//! //   SelfUpdate { #[arg(long)] check: bool, #[arg(long)] channel: Option<String> },
//! // in the match:
//! //   Cmd::SelfUpdate { check, channel } => update_cmd::run(check, channel)?,
//! // early in main(), after the app started fine:
//! //   update_cmd::finish_previous_update();
//! ```

use anyhow::{Result, anyhow, bail};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use synofurb_core::model::human_bytes;
use synofurb_core::updater::{self, ApplyOutcome, Channel, Progress, ReleaseCache, UpdateError, UpdateInfo};

const MAX_NOTE_LINES: usize = 25;

/// Entry point. `check_only` stops after printing what is available; `channel` is `stable` (default) or `prerelease`.
pub fn run(check_only: bool, channel: Option<String>) -> Result<()> {
    let channel = match channel.as_deref() {
        None => Channel::Stable,
        Some(c) => c.parse::<Channel>().map_err(|e| anyhow!(e))?,
    };
    let current = env!("CARGO_PKG_VERSION");
    let base = cache_base();
    let cache = ReleaseCache::on_disk(&base.join("releases"));

    eprintln!("Checking github.com/{} for updates ({} channel)...", updater::DEFAULT_REPO, channel.label());
    let info = match updater::check_with_cache(current, updater::DEFAULT_REPO, channel, &cache) {
        Ok(i) => i,
        Err(e @ UpdateError::RateLimited { .. }) => bail!("{e}. Nothing was changed."),
        Err(e) => return Err(anyhow!(e).context("could not check for updates")),
    };
    print_summary(&info);

    if !info.update_available {
        println!("\nsynofurb {} is up to date.", info.current);
        return Ok(());
    }
    let key_ok = !updater::signing_key_is_placeholder(updater::UPDATE_PUBLIC_KEY);
    if check_only {
        println!("\nRun `synofurb self-update` (without --check) to install it.");
        if !key_ok {
            println!("Note: installing is currently disabled because the release signing key has not been set up (placeholder key).");
        }
        return Ok(());
    }

    // Refuse before downloading anything: never skip verification.
    updater::ensure_can_verify(updater::UPDATE_PUBLIC_KEY)?;
    if !info.installable() {
        bail!("this release cannot be installed automatically:\n  - {}", info.problems.join("\n  - "));
    }

    let dir = base.join("downloads").join(&info.latest);
    make_private_dir(&dir)?;
    let size = info.asset.as_ref().map(|a| a.size).unwrap_or(0);
    println!("\nDownloading {} ({})...", info.asset.as_ref().map(|a| a.name.as_str()).unwrap_or("?"), human_bytes(size));
    let mut bar = Bar::new();
    let verified = updater::download_verified(&info, &dir, &mut |p| bar.update(p))?;
    bar.finish();
    println!("Verified: signature of SHA256SUMS is valid and SHA-256 matches ({}).", verified.sha256);

    match updater::apply(&verified.path)? {
        ApplyOutcome::InstallerLaunched => {
            println!("The installer for synofurb {} has been started. synofurb will now exit so it can be replaced.", info.latest);
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        ApplyOutcome::Opened => {
            println!("Opened {}. Finish the installation from there, then start synofurb again.", verified.path.display());
        }
        ApplyOutcome::Replaced { target, backup } => {
            println!("Updated {} to {}.", target.display(), info.latest);
            println!("The previous version is kept at {} until the new version has started successfully.", backup.display());
            println!("Start synofurb again to use the new version.");
        }
        ApplyOutcome::PackageInstalled { command } => {
            println!("Installed with: {command}");
        }
        ApplyOutcome::PackageCommand { command } => {
            println!("This is a system package. Install it with:\n\n  {command}\n");
            println!("(pkexec was not found, so nothing was run.)");
        }
        ApplyOutcome::Manual { reason } => {
            println!("{reason}\nThe verified file is at {}", verified.path.display());
        }
    }
    Ok(())
}

/// Call once after a normal start: removes the `.old` backup left by a previous self-update.
#[allow(dead_code)]
pub fn finish_previous_update() {
    if let Ok(exe) = updater::running_executable() {
        updater::mark_start_ok(&exe);
    }
}

fn print_summary(info: &UpdateInfo) {
    println!("Installed version : {}", info.current);
    println!("Latest on channel : {}{}", info.latest, if info.prerelease { " (pre-release)" } else { "" });
    if let Some(d) = &info.published_at {
        println!("Published         : {d}");
    }
    if let Some(u) = &info.html_url {
        println!("Release page      : {u}");
    }
    match &info.asset {
        Some(a) => println!("Download          : {} ({})", a.name, human_bytes(a.size)),
        None => println!("Download          : none for this system"),
    }
    for p in &info.problems {
        println!("Problem           : {p}");
    }
    let notes = info.notes.trim();
    if info.update_available && !notes.is_empty() {
        println!("\nRelease notes:");
        for line in notes.lines().take(MAX_NOTE_LINES) {
            println!("  {line}");
        }
        if notes.lines().count() > MAX_NOTE_LINES {
            println!("  ... (see the release page for the rest)");
        }
    }
}

/// Per-user cache directory (`~/.cache/synofurb`, `%LOCALAPPDATA%\synofurb`, ...).
fn cache_base() -> PathBuf {
    directories::ProjectDirs::from("", "", "synofurb")
        .map(|d| d.cache_dir().join("updates"))
        .unwrap_or_else(|| std::env::temp_dir().join("synofurb-updates"))
}

fn make_private_dir(dir: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Progress bar on stderr: redraws in place on a terminal, prints a line per 10 % otherwise.
struct Bar {
    tty: bool,
    last_pct: i64,
    drew: bool,
}

impl Bar {
    fn new() -> Bar {
        Bar { tty: std::io::stderr().is_terminal(), last_pct: -1, drew: false }
    }

    fn update(&mut self, p: Progress) {
        let Some(total) = p.total.filter(|t| *t > 0) else {
            if self.tty {
                eprint!("\r  {} downloaded", human_bytes(p.downloaded));
                self.drew = true;
            }
            return;
        };
        let pct = ((p.downloaded.min(total) * 100) / total) as i64;
        if self.tty {
            if pct == self.last_pct && p.downloaded < total {
                return;
            }
            let filled = (pct as usize * 30) / 100;
            eprint!(
                "\r  [{}{}] {:>3}%  {} / {}",
                "#".repeat(filled),
                "-".repeat(30 - filled),
                pct,
                human_bytes(p.downloaded),
                human_bytes(total)
            );
            let _ = std::io::stderr().flush();
            self.drew = true;
        } else if pct / 10 != self.last_pct / 10 || self.last_pct < 0 {
            eprintln!("  {pct}%  {} / {}", human_bytes(p.downloaded), human_bytes(total));
        }
        self.last_pct = pct;
    }

    fn finish(&mut self) {
        if self.drew {
            eprintln!();
        }
    }
}
