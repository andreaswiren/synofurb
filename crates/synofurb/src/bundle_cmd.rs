//! `synofurb bundle create|verify|info`: CLI glue for the offline bundle (see `synofurb_core::offline` and
//! `docs/OFFLINE.md`). Progress and log lines go to stderr; stdout carries only the final result so it can be piped.

use anyhow::{Context, Result, bail};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use synofurb_core::config::KernelChannel;
use synofurb_core::logs::{Event, Level, Logger};
use synofurb_core::model::Arch;
use synofurb_core::offline::{Bundle, BundleSpec, create_bundle, human_bytes};

/// `amd64`, `x86_64`, `arm64`, `aarch64`, `armhf`, `armv7l` (case-insensitive).
fn parse_arch(s: &str) -> Result<Arch> {
    let a = match s.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" | "x86-64" => Arch::X86_64,
        "arm64" | "aarch64" => Arch::Arm64,
        "armhf" | "armv7" | "armv7l" => Arch::Armhf,
        other => bail!("unknown architecture '{other}': choose amd64, arm64 or armhf"),
    };
    Ok(a)
}

/// Print log lines to stderr from a helper thread; the returned logger also writes the usual log file when possible.
fn stderr_logger() -> (Logger, std::thread::JoinHandle<()>) {
    let (tx, rx) = channel();
    let log =
        Logger::to_dir(&synofurb_core::session::data_dir().join("logs"), Some(tx.clone())).unwrap_or_else(|_| Logger::memory(Some(tx)));
    let printer = std::thread::spawn(move || {
        for e in rx {
            if let Event::Log(l) = e {
                match l.level {
                    Level::Warn => eprintln!("warning: {}", l.msg),
                    Level::Error => eprintln!("error: {}", l.msg),
                    Level::Debug => {}
                    _ => eprintln!("{}", l.msg),
                }
            }
        }
    });
    (log, printer)
}

/// Progress as one rewritten line on a terminal, or a line per ten percent when piped.
struct Progress {
    tty: bool,
    last_label: String,
    last_bucket: i32,
}

impl Progress {
    fn new() -> Progress {
        Progress { tty: std::io::stderr().is_terminal(), last_label: String::new(), last_bucket: -1 }
    }

    fn update(&mut self, label: &str, f: f32) {
        let pct = (f.clamp(0.0, 1.0) * 100.0) as i32;
        if self.tty {
            eprint!("\r\x1b[2K{label}: {pct}%");
            if pct >= 100 {
                eprintln!();
            }
            return;
        }
        let bucket = pct / 10;
        if label != self.last_label || bucket != self.last_bucket {
            eprintln!("{label}: {pct}%");
            self.last_label = label.to_string();
            self.last_bucket = bucket;
        }
    }
}

/// Build (or resume) a bundle. Needs Internet; run it again after an interruption (Ctrl-C leaves only `.part` files).
pub fn create(
    archs: Vec<String>,
    suite: String,
    kernel_newest: bool,
    no_firmware: bool,
    extra: Vec<String>,
    out: PathBuf,
    no_agent: bool,
) -> Result<()> {
    let archs: Vec<Arch> = if archs.is_empty() {
        eprintln!("no --arch given: building the bundle for amd64 (use --arch amd64 --arch arm64 --arch armhf for more)");
        vec![Arch::X86_64]
    } else {
        let mut v = vec![];
        for a in archs.iter().flat_map(|a| a.split(',')) {
            if a.trim().eq_ignore_ascii_case("all") {
                v.extend([Arch::X86_64, Arch::Arm64, Arch::Armhf]);
            } else {
                v.push(parse_arch(a)?);
            }
        }
        v
    };
    let spec = BundleSpec {
        suite,
        archs,
        kernel: if kernel_newest { KernelChannel::Newest } else { KernelChannel::Stable },
        with_firmware: !no_firmware,
        extra_packages: extra,
        include_agent: !no_agent,
        dtbs: vec![],
    };
    eprintln!(
        "Building the offline bundle in {} for {} (kernel: {}). Needs Internet; stop with Ctrl-C and run the same command again to resume.",
        out.display(),
        spec.archs.iter().map(|a| a.debian()).collect::<Vec<_>>().join(", "),
        spec.kernel.label()
    );
    let (log, printer) = stderr_logger();
    let mut prog = Progress::new();
    let res = create_bundle(&spec, &out, &log, &mut |l, f| prog.update(l, f));
    drop(log);
    let _ = printer.join();
    let m = res.with_context(|| format!("building the bundle in {}", out.display()))?;
    let total: u64 = m.files.iter().map(|f| f.size).sum();
    println!("bundle ready: {} ({}, {} files)", out.display(), human_bytes(total), m.files.len());
    println!("install with: synofurb install --offline-bundle {}   (the bundle can be copied to another computer)", out.display());
    Ok(())
}

/// Re-hash everything in the bundle; non-zero exit when anything is missing or changed.
pub fn verify(dir: PathBuf) -> Result<()> {
    let b = Bundle::open(&dir)?;
    let mut prog = Progress::new();
    let rep = b.verify(&mut |l, f| prog.update(l, f))?;
    println!("{} files, {} checked", rep.files_checked, human_bytes(rep.bytes_checked));
    for m in &rep.missing {
        println!("MISSING  {m}");
    }
    for c in &rep.changed {
        println!("CHANGED  {c}");
    }
    for c in &rep.chain_problems {
        println!("CHAIN    {c}");
    }
    for e in &rep.extra {
        println!("EXTRA    {e}   (not in the manifest, never served as part of the bundle)");
    }
    if rep.ok() {
        println!("bundle OK");
        Ok(())
    } else {
        bail!(
            "the bundle is damaged: {} missing, {} changed, {} index problems. Build it again with `synofurb bundle create` (only the broken files are downloaded)",
            rep.missing.len(),
            rep.changed.len(),
            rep.chain_problems.len()
        )
    }
}

/// Describe a bundle without verifying it (fast, no hashing).
pub fn info(dir: PathBuf, json: bool) -> Result<()> {
    let b = Bundle::open(&dir)?;
    let i = b.info();
    if json {
        println!("{}", serde_json::to_string_pretty(&i)?);
    } else {
        print!("{}", i.render());
    }
    Ok(())
}
