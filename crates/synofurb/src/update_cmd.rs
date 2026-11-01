//! `self-update`: check GitHub for a newer release, download it, verify it, install it.
//!
//! The logic lives in `synofurb_core::updater`; this file is the terminal front end. Two flavours:
//! * the desktop program (`synofurb`) updates from its own asset, exactly as the core chooses it;
//! * the headless program (`synofurb-cli`) updates from the CLI asset `synofurb-cli-<version>-<os>-<arch>.tar.gz` (`.zip` on Windows).
//!   The core's asset picker only knows the `synofurb-` prefix and a binary called `synofurb`, so this file builds the
//!   `UpdateInfo` for the CLI asset itself and reuses the core's verified download (minisign signature of SHA256SUMS, then SHA-256)
//!   and its safe executable replacement. See docs/CLI.md ("Updating") for the API change that would make this a one-liner.

use crate::cli::out::{Out, err_raw};
use anyhow::{Result, anyhow, bail};
use serde_json::json;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use synofurb_core::model::human_bytes;
use synofurb_core::updater::{self, ApplyOutcome, AssetInfo, Channel, Progress, ReleaseCache, Target, UpdateError, UpdateInfo};

const MAX_NOTE_LINES: usize = 25;
/// Largest binary accepted from an archive.
const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

/// `synofurb-cli-<version>-<os>-<arch>.<tar.gz|zip>`
pub fn cli_asset_name(version: &str, os: &str, arch: &str) -> Option<String> {
    let ext = match os {
        "linux" | "macos" => "tar.gz",
        "windows" => "zip",
        _ => return None,
    };
    matches!(arch, "x86_64" | "aarch64").then(|| format!("synofurb-cli-{version}-{os}-{arch}.{ext}"))
}

/// Point `info` at the CLI asset of the same release. `Err(why)` when this system has no CLI build.
pub fn retarget_to_cli(info: &mut UpdateInfo, repo: &str, target: &Target) -> std::result::Result<(), String> {
    let name = cli_asset_name(&info.latest, &target.os, &target.arch)
        .ok_or_else(|| format!("there is no synofurb-cli build for {}", target.label()))?;
    info.asset = Some(AssetInfo { url: format!("https://github.com/{repo}/releases/download/{}/{name}", info.tag), name, size: 0 });
    // the core complained that the desktop asset for this system is missing; that is not our asset
    info.problems.retain(|p| !p.contains("has no build for this system"));
    Ok(())
}

/// Pull `synofurb-cli` (or `.exe`) out of a `.tar.gz` into `out`.
pub fn extract_cli_binary(archive: &Path, bin_name: &str, out: &Path) -> Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    for entry in ar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        if entry.path()?.file_name().is_some_and(|n| n == bin_name) {
            if entry.size() > MAX_BINARY_BYTES {
                bail!("the binary inside the archive is too large");
            }
            let mut dst = std::fs::File::create(out)?;
            std::io::copy(&mut entry.by_ref().take(MAX_BINARY_BYTES), &mut dst)?;
            dst.sync_all()?;
            return Ok(());
        }
    }
    bail!("{bin_name} not found in {}", archive.display())
}

/// Entry point. `check_only` stops after printing what is available; `channel` is `stable` (default) or `prerelease`; `gui`: this is
/// the desktop program.
pub fn run(out: &Out, check_only: bool, channel: Option<String>, json: bool, gui: bool) -> Result<()> {
    let channel = match channel.as_deref() {
        None => Channel::Stable,
        Some(c) => c.parse::<Channel>().map_err(|e| anyhow!(e))?,
    };
    let current = env!("CARGO_PKG_VERSION");
    let base = cache_base();
    let cache = ReleaseCache::on_disk(&base.join("releases"));
    let flavour = if gui { "desktop app" } else { "command-line build" };

    out.info(&format!("Checking github.com/{} for updates ({} channel, {flavour}) ...", updater::DEFAULT_REPO, channel.label()));
    let mut info = match updater::check_with_cache(current, updater::DEFAULT_REPO, channel, &cache) {
        Ok(i) => i,
        Err(e @ UpdateError::RateLimited { .. }) => bail!("{e}. Nothing was changed."),
        Err(e) => return Err(anyhow!(e).context("could not check for updates")),
    };
    if !gui && let Err(why) = retarget_to_cli(&mut info, updater::DEFAULT_REPO, &Target::current()) {
        info.asset = None;
        info.problems.push(why);
    }
    let key_ok = !updater::signing_key_is_placeholder(updater::UPDATE_PUBLIC_KEY);

    if check_only && json {
        let mut v = serde_json::to_value(&info)?;
        v["schema"] = json!(1);
        v["program"] = json!(if gui { "synofurb" } else { "synofurb-cli" });
        v["installable"] = json!(info.installable() && key_ok);
        out.json_doc(&v);
        return Ok(());
    }
    print_summary(out, &info);

    if !info.update_available {
        out.data(&format!("{} {} is up to date.", if gui { "synofurb" } else { "synofurb-cli" }, info.current));
        return Ok(());
    }
    if check_only {
        out.data(&format!(
            "Update available: {} -> {}. Run `{} self-update` (without --check) to install it.",
            info.current,
            info.latest,
            if gui { "synofurb" } else { "synofurb-cli" }
        ));
        if !key_ok {
            out.warn("installing is currently disabled because the release signing key has not been set up (placeholder key)");
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
    out.info(&format!(
        "Downloading {}{} ...",
        info.asset.as_ref().map(|a| a.name.as_str()).unwrap_or("?"),
        if size > 0 { format!(" ({})", human_bytes(size)) } else { String::new() }
    ));
    let mut bar = Bar::new(!out.quiet);
    let verified = match updater::download_verified(&info, &dir, &mut |p| bar.update(p)) {
        Ok(v) => v,
        Err(e @ UpdateError::ChecksumMissing(_)) if !gui => {
            bar.finish();
            bail!(
                "release {} has no command-line build for this system ({e}). Use the desktop program's updater, or install it again with install.sh --cli",
                info.tag
            );
        }
        Err(e) => {
            bar.finish();
            return Err(e.into());
        }
    };
    bar.finish();
    out.info(&format!("Verified: signature of SHA256SUMS is valid and SHA-256 matches ({}).", verified.sha256));

    if gui { apply_gui(out, &verified.path, &info) } else { apply_cli(out, &verified.path, &info) }
}

fn apply_cli(out: &Out, archive: &Path, info: &UpdateInfo) -> Result<()> {
    let name = archive.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if name.ends_with(".zip") {
        out.data(&format!(
            "The verified archive for Windows is at {}\nStop synofurb-cli, extract synofurb-cli.exe from it over the installed copy, then start it again (a running .exe cannot replace itself).",
            archive.display()
        ));
        return Ok(());
    }
    let exe = updater::running_executable()?;
    let staged = archive.with_file_name(format!("synofurb-cli-{}.new", info.latest));
    extract_cli_binary(archive, "synofurb-cli", &staged)?;
    let result = updater::replace_executable(&staged, &exe, false);
    let _ = std::fs::remove_file(&staged);
    match result {
        Ok(backup) => {
            out.data(&format!("Updated {} to {}.", exe.display(), info.latest));
            out.info(&format!("The previous version is kept at {} until the new version has started successfully.", backup.display()));
            out.info("Run synofurb-cli again to use the new version.");
            Ok(())
        }
        Err(e) => Err(anyhow!(e).context(format!(
            "could not replace {} (is it in a directory you can write to? a package-managed copy must be updated with the package manager)",
            exe.display()
        ))),
    }
}

fn apply_gui(out: &Out, path: &Path, info: &UpdateInfo) -> Result<()> {
    match updater::apply(path)? {
        ApplyOutcome::InstallerLaunched => {
            out.data(&format!(
                "The installer for synofurb {} has been started. synofurb will now exit so it can be replaced.",
                info.latest
            ));
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        ApplyOutcome::Opened => {
            out.data(&format!("Opened {}. Finish the installation from there, then start synofurb again.", path.display()))
        }
        ApplyOutcome::Replaced { target, backup } => {
            out.data(&format!("Updated {} to {}.", target.display(), info.latest));
            out.info(&format!("The previous version is kept at {} until the new version has started successfully.", backup.display()));
            out.info("Start synofurb again to use the new version.");
        }
        ApplyOutcome::PackageInstalled { command } => out.data(&format!("Installed with: {command}")),
        ApplyOutcome::PackageCommand { command } => {
            out.data(&format!("This is a system package. Install it with:\n\n  {command}\n"));
            out.info("(pkexec was not found, so nothing was run.)");
        }
        ApplyOutcome::Manual { reason } => out.data(&format!("{reason}\nThe verified file is at {}", path.display())),
    }
    Ok(())
}

/// Call once after a normal start: removes the `.old` backup left by a previous self-update.
pub fn finish_previous_update() {
    if let Ok(exe) = updater::running_executable() {
        updater::mark_start_ok(&exe);
    }
}

fn print_summary(out: &Out, info: &UpdateInfo) {
    out.info(&format!("Installed version : {}", info.current));
    out.info(&format!("Latest on channel : {}{}", info.latest, if info.prerelease { " (pre-release)" } else { "" }));
    if let Some(d) = &info.published_at {
        out.info(&format!("Published         : {d}"));
    }
    if let Some(u) = &info.html_url {
        out.info(&format!("Release page      : {u}"));
    }
    match &info.asset {
        Some(a) => out.info(&format!(
            "Download          : {}{}",
            a.name,
            if a.size > 0 { format!(" ({})", human_bytes(a.size)) } else { String::new() }
        )),
        None => out.info("Download          : none for this system"),
    }
    for p in &info.problems {
        out.info(&format!("Problem           : {p}"));
    }
    let notes = info.notes.trim();
    if info.update_available && !notes.is_empty() {
        out.info("\nRelease notes:");
        for line in notes.lines().take(MAX_NOTE_LINES) {
            out.info(&format!("  {line}"));
        }
        if notes.lines().count() > MAX_NOTE_LINES {
            out.info("  ... (see the release page for the rest)");
        }
    }
}

/// Per-user cache directory (`~/.cache/synofurb`, `%LOCALAPPDATA%\synofurb`, ...).
fn cache_base() -> PathBuf {
    directories::ProjectDirs::from("", "", "synofurb")
        .map(|d| d.cache_dir().join("updates"))
        .unwrap_or_else(|| std::env::temp_dir().join("synofurb-updates"))
}

fn make_private_dir(dir: &Path) -> Result<()> {
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
    enabled: bool,
    tty: bool,
    last_pct: i64,
    drew: bool,
}

impl Bar {
    fn new(enabled: bool) -> Bar {
        let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
        Bar { enabled, tty: std::io::stderr().is_terminal() && !dumb, last_pct: -1, drew: false }
    }

    fn update(&mut self, p: Progress) {
        if !self.enabled {
            return;
        }
        let Some(total) = p.total.filter(|t| *t > 0) else {
            if self.tty {
                err_raw(&format!("\r  {} downloaded", human_bytes(p.downloaded)));
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
            err_raw(&format!(
                "\r  [{}{}] {:>3}%  {} / {}",
                "#".repeat(filled),
                "-".repeat(30 - filled),
                pct,
                human_bytes(p.downloaded),
                human_bytes(total)
            ));
            self.drew = true;
        } else if pct / 10 != self.last_pct / 10 || self.last_pct < 0 {
            err_raw(&format!("  {pct}%  {} / {}\n", human_bytes(p.downloaded), human_bytes(total)));
        }
        self.last_pct = pct;
    }

    fn finish(&mut self) {
        if self.drew {
            err_raw("\n");
            self.drew = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> UpdateInfo {
        UpdateInfo {
            current: "0.2.0".into(),
            latest: "0.3.0".into(),
            tag: "v0.3.0".into(),
            update_available: true,
            channel: Channel::Stable,
            prerelease: false,
            published_at: None,
            notes: String::new(),
            html_url: None,
            asset: None,
            checksums: Some(AssetInfo {
                name: "SHA256SUMS".into(),
                url: "https://github.com/o/r/releases/download/v0.3.0/SHA256SUMS".into(),
                size: 0,
            }),
            signature: Some(AssetInfo {
                name: "SHA256SUMS.minisig".into(),
                url: "https://github.com/o/r/releases/download/v0.3.0/SHA256SUMS.minisig".into(),
                size: 0,
            }),
            problems: vec!["release v0.3.0 has no build for this system (looked for synofurb-0.3.0-linux-x86_64.AppImage)".into()],
        }
    }

    #[test]
    fn asset_names_follow_the_release_contract() {
        assert_eq!(cli_asset_name("0.3.0", "linux", "x86_64").unwrap(), "synofurb-cli-0.3.0-linux-x86_64.tar.gz");
        assert_eq!(cli_asset_name("0.3.0", "macos", "aarch64").unwrap(), "synofurb-cli-0.3.0-macos-aarch64.tar.gz");
        assert_eq!(cli_asset_name("0.3.0", "windows", "x86_64").unwrap(), "synofurb-cli-0.3.0-windows-x86_64.zip");
        assert!(cli_asset_name("0.3.0", "freebsd", "x86_64").is_none());
        assert!(cli_asset_name("0.3.0", "linux", "riscv64").is_none());
    }

    #[test]
    fn retargeting_makes_the_release_installable_with_the_cli_asset() {
        let mut i = info();
        assert!(!i.installable());
        retarget_to_cli(&mut i, "andreaswiren/synofurb", &Target::new("linux", "aarch64")).unwrap();
        let a = i.asset.clone().unwrap();
        assert_eq!(a.name, "synofurb-cli-0.3.0-linux-aarch64.tar.gz");
        assert_eq!(a.url, "https://github.com/andreaswiren/synofurb/releases/download/v0.3.0/synofurb-cli-0.3.0-linux-aarch64.tar.gz");
        assert!(i.problems.is_empty(), "{:?}", i.problems);
        assert!(i.installable());
        let mut j = info();
        assert!(retarget_to_cli(&mut j, "o/r", &Target::new("solaris", "sparc")).is_err());
    }

    #[test]
    fn extracts_the_binary_from_the_archive() {
        let dir = std::env::temp_dir().join(format!("sf-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tgz = dir.join("a.tar.gz");
        {
            let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tgz).unwrap(), flate2::Compression::fast());
            let mut tb = tar::Builder::new(gz);
            for (name, data) in [("LICENSE", &b"MIT"[..]), ("synofurb-cli", &b"#!/bin/sh\necho new\n"[..])] {
                let mut h = tar::Header::new_gnu();
                h.set_size(data.len() as u64);
                h.set_mode(0o755);
                h.set_cksum();
                tb.append_data(&mut h, name, data).unwrap();
            }
            tb.into_inner().unwrap().finish().unwrap();
        }
        let out = dir.join("bin");
        extract_cli_binary(&tgz, "synofurb-cli", &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"#!/bin/sh\necho new\n");
        assert!(extract_cli_binary(&tgz, "synofurb", &dir.join("x")).is_err(), "the desktop binary name is not in a CLI archive");
        // and the core's safe replacement keeps a backup of the old executable
        let exe = dir.join("synofurb-cli");
        std::fs::write(&exe, b"old").unwrap();
        let backup = updater::replace_executable(&out, &exe, false).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"#!/bin/sh\necho new\n");
        assert_eq!(std::fs::read(backup).unwrap(), b"old");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
