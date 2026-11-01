//! Offline mode: build a **bundle** while the desktop has Internet, serve it to the NAS over the LAN later.
//!
//! A bundle is a directory (copyable by USB stick) holding everything the install needs, so the NAS never
//! talks to the Internet:
//!
//! ```text
//! manifest.json            versions, suites, archs, kernel channel, creation date, every file with size + sha256
//! SHA256SUMS               the same hashes in `sha256sum -c` format
//! installer/<arch>/        kernel, initrd.gz, DTBs, plus the InRelease and SHA256SUMS the hashes came from
//! installer/firmware.cpio.gz   optional, shared by all archs (non-free firmware for the installer)
//! agent/agent-<arch>.{vmlinuz,initrd}   boot agent for internal-flash (DOM) units, optional
//! mgmt/                    the management-app update bundle (`synofurb_mgmt.py`, `VERSION`, `README.md`, the unit file and
//!                          `MANIFEST.sha256`): what `synofurb manage update --from-bundle <bundle>` pushes to a unit offline
//! mirror/debian/dists/<suite>[-backports]/{InRelease,Release,Release.gpg,<comp>/.../Packages.{gz,xz}}
//! mirror/debian/pool/...   only the .deb/.udeb files of the dependency closure
//! ```
//!
//! The `dists/` files are copied **verbatim** from `deb.debian.org`, so Debian's own PGP signatures stay valid and
//! `debootstrap`, `apt` and the installer's `anna` verify the packages exactly as they do online. Every `.deb` is
//! checked against the SHA-256 of the (verified) `Packages` index, which is checked against `InRelease`.
//!
//! The dependency closure (pure code, unit tested) resolves `Depends` and `Pre-Depends` only; see [`Resolver`] for
//! what it deliberately simplifies. The integration test (`tests/offline/`) proves the closure is complete by running
//! the real `debootstrap` and `apt-get` against the bundle.

use crate::config::{KernelChannel, valid_package_name};
use crate::debian::{self, InstallerFiles};
use crate::logs::Logger;
use crate::manage::{MANIFEST_NAME as MGMT_MANIFEST, MgmtBundle};
use crate::model::Arch;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::time::Duration;

pub const FORMAT_VERSION: u32 = 1;
pub const MANIFEST_FILE: &str = "manifest.json";
pub const SUMS_FILE: &str = "SHA256SUMS";
/// Marker inside a bundle directory that is still being built (or whose build was interrupted).
const BUILDING_MARKER: &str = ".building";
/// Scratch directory inside the bundle directory (agent downloads); never part of the manifest.
const WORK_DIR: &str = ".work";
/// Archive components the installer's `debootstrap --components=main,contrib,non-free-firmware` and apt use.
pub const COMPONENTS: [&str; 3] = ["main", "contrib", "non-free-firmware"];
/// Relative path of the served mirror root inside a bundle (what is reachable below `/<token>/mirror/`).
pub const MIRROR_ROOT: &str = "mirror";
/// Name of the mirror inside [`MIRROR_ROOT`]; the NAS uses `/<token>/mirror/debian` as `mirror_dir`.
pub const MIRROR_NAME: &str = "debian";
/// The management-app update bundle inside a bundle (`mgmt/`), in the layout `synofurb_mgmt.py update --from` takes.
pub const MGMT_SUBDIR: &str = "mgmt";

// ============================================================================================================
// Public data types
// ============================================================================================================

/// What `create_bundle` should put into the bundle.
#[derive(Debug, Clone)]
pub struct BundleSpec {
    /// Debian suite, normally `trixie`.
    pub suite: String,
    pub archs: Vec<Arch>,
    /// `Newest` adds `<suite>-backports` and the kernel from there; `Stable` ships the suite's own kernel.
    pub kernel: KernelChannel,
    /// Also store the installer firmware (`firmware.cpio.gz`, about 470 MiB, shared by all architectures).
    pub with_firmware: bool,
    pub extra_packages: Vec<String>,
    /// Boot agent for internal-flash (DOM) x86 units; when its release is unavailable this only warns.
    pub include_agent: bool,
    /// DTB file names of the installer's `device-tree/` (armhf U-Boot boards).
    pub dtbs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to the bundle directory, always with `/`.
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Per-architecture facts recorded at creation time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchManifest {
    /// Kernel ABI of the Debian installer image (`6.12.107+deb13-amd64`); the installer's module udebs match it.
    pub installer_kernel: Option<String>,
    /// Version of the kernel image package that will be installed (`7.2.11-1~bpo13+1`).
    pub kernel_package_version: Option<String>,
    pub kernel_from_backports: bool,
    pub packages: u32,
    pub udebs: u32,
    /// Pool files this architecture needs (shared files are listed for every architecture that needs them).
    pub pool_files: Vec<String>,
    pub installer_files: Vec<String>,
    pub agent_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub format: u32,
    pub tool_version: String,
    /// RFC 3339.
    pub created: String,
    pub suite: String,
    /// Suites present in the mirror (`trixie`, and `trixie-backports` for the newest kernel).
    pub suites: Vec<String>,
    pub kernel: KernelChannel,
    pub with_firmware: bool,
    pub extra_packages: Vec<String>,
    /// `Date:` of each suite's Release file (the snapshot the bundle was built from).
    pub release_dates: BTreeMap<String, String>,
    /// Whether the local `gpgv` + Debian keyring verified `InRelease` while building.
    pub pgp_verified: bool,
    pub warnings: Vec<String>,
    pub archs: BTreeMap<String, ArchManifest>,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub files_checked: usize,
    pub bytes_checked: u64,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    /// Files on disk that the manifest does not know (never trusted; reported so you notice).
    pub extra: Vec<String>,
    /// Mismatches between a `Packages` index and the `InRelease` that lists it.
    pub chain_problems: Vec<String>,
    /// Problems with the management-app update bundle in `mgmt/` (its own `MANIFEST.sha256` does not match its files, a file is
    /// missing, or it disagrees with `manifest.json`). Empty when `mgmt/` is fine or the bundle has none.
    pub mgmt_problems: Vec<String>,
    /// Version of the management app in `mgmt/` when it verified.
    pub mgmt_version: Option<String>,
    /// Things worth saying that are not errors (a bundle from an older synofurb without `mgmt/`).
    pub notes: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.changed.is_empty() && self.chain_problems.is_empty() && self.mgmt_problems.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchInfo {
    pub arch: String,
    pub installer_kernel: Option<String>,
    pub kernel_package_version: Option<String>,
    pub packages: u32,
    pub udebs: u32,
    /// Bytes of everything this architecture needs (installer, agent, pool files; shared files counted in full).
    pub bytes: u64,
    pub has_agent: bool,
}

/// Human summary of a bundle (`synofurb bundle info`).
#[derive(Debug, Clone, Serialize)]
pub struct BundleInfo {
    pub path: PathBuf,
    pub created: String,
    pub tool_version: String,
    pub suite: String,
    pub suites: Vec<String>,
    pub kernel: KernelChannel,
    pub release_dates: BTreeMap<String, String>,
    pub archs: Vec<ArchInfo>,
    pub with_firmware: bool,
    pub firmware_bytes: u64,
    pub total_bytes: u64,
    pub files: usize,
    pub pgp_verified: bool,
    pub extra_packages: Vec<String>,
    pub warnings: Vec<String>,
    /// Version of the management-app update bundle in `mgmt/`; `None` for a bundle from a synofurb that did not add it.
    pub mgmt_version: Option<String>,
}

/// The `--kernel` value of a channel.
fn kernel_flag(k: KernelChannel) -> &'static str {
    match k {
        KernelChannel::Stable => "stable",
        KernelChannel::Newest => "newest",
    }
}

pub fn human_bytes(b: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if b as f64 >= 1024.0 * MIB { format!("{:.2} GiB", b as f64 / (1024.0 * MIB)) } else { format!("{:.1} MiB", b as f64 / MIB) }
}

impl BundleInfo {
    /// Multi-line text for the terminal.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let mut line = |l: String| {
            s.push_str(&l);
            s.push('\n');
        };
        line(format!("Bundle:    {}", self.path.display()));
        line(format!("Created:   {} by synofurb {}", self.created, self.tool_version));
        line(format!("Debian:    {} (suites in the mirror: {})", self.suite, self.suites.join(", ")));
        for (suite, date) in &self.release_dates {
            line(format!("           {suite} snapshot of {date}"));
        }
        line(format!("Kernel:    {}", self.kernel.label()));
        line(format!(
            "Firmware:  {}",
            if self.with_firmware {
                format!("installer firmware included ({})", human_bytes(self.firmware_bytes))
            } else {
                "not included".into()
            }
        ));
        if !self.extra_packages.is_empty() {
            line(format!("Extra:     {}", self.extra_packages.join(", ")));
        }
        line(format!(
            "Signature: {}",
            if self.pgp_verified {
                "Debian InRelease PGP-verified when built"
            } else {
                "built on the HTTPS + SHA-256 chain (no local PGP check); the NAS still verifies Debian's signatures"
            }
        ));
        line(format!(
            "Mgmt app:  {}",
            match &self.mgmt_version {
                Some(v) => format!("synofurb-mgmt {v} (for `synofurb manage update --from-bundle`)"),
                None => "not included (built by an older synofurb; `manage update` then uses the copy built into synofurb)".into(),
            }
        ));
        for a in &self.archs {
            line(format!(
                "  {:<6} {:>10}  installer kernel {}, kernel package {}, {} packages + {} installer udebs, boot agent: {}",
                a.arch,
                human_bytes(a.bytes),
                a.installer_kernel.as_deref().unwrap_or("unknown"),
                a.kernel_package_version.as_deref().unwrap_or("unknown"),
                a.packages,
                a.udebs,
                if a.has_agent { "yes" } else { "no" }
            ));
        }
        line(format!("Total:     {} in {} files", human_bytes(self.total_bytes), self.files));
        for w in &self.warnings {
            line(format!("WARNING:   {w}"));
        }
        s
    }
}

/// An opened bundle. Opening reads `manifest.json` only (no network, no hashing); call [`Bundle::verify`] to re-hash.
#[derive(Debug, Clone)]
pub struct Bundle {
    root: PathBuf,
    pub manifest: BundleManifest,
}

/// Where the downloads come from. Tests point it at a loopback server.
#[derive(Debug, Clone)]
pub struct Source {
    pub debian_base: String,
    pub firmware_base: String,
    pub allow_loopback_http: bool,
}

impl Default for Source {
    fn default() -> Self {
        Source { debian_base: debian::DEBIAN_BASE.into(), firmware_base: debian::FIRMWARE_BASE.into(), allow_loopback_http: false }
    }
}

// ============================================================================================================
// Debian control-file parsing
// ============================================================================================================

/// One binary (`.deb`) or installer (`.udeb`) package stanza from a `Packages` index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub arch: String,
    pub depends: String,
    pub pre_depends: String,
    /// `(virtual name, optional exact version)`
    pub provides: Vec<(String, Option<String>)>,
    pub essential: bool,
    pub priority: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    /// udebs only: the kernel ABI a module udeb belongs to.
    pub kernel_version: Option<String>,
    /// Index into [`Universe::repos`].
    pub repo: usize,
}

/// Parse the text of a `Packages` index (any number of stanzas separated by blank lines).
pub fn parse_packages(text: &str) -> Vec<Pkg> {
    parse_packages_reader(text.as_bytes()).unwrap_or_default()
}

fn parse_packages_reader(r: impl BufRead) -> Result<Vec<Pkg>> {
    let mut out = vec![];
    let mut cur = Pkg::default();
    let mut have = false;
    let mut last_field = String::new();
    for line in r.lines() {
        let line = line?;
        if line.is_empty() {
            if have && !cur.name.is_empty() {
                out.push(std::mem::take(&mut cur));
            } else {
                cur = Pkg::default();
            }
            have = false;
            last_field.clear();
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // continuation: only the dependency fields can legitimately wrap
            match last_field.as_str() {
                "Depends" => {
                    cur.depends.push(' ');
                    cur.depends.push_str(line.trim());
                }
                "Pre-Depends" => {
                    cur.pre_depends.push(' ');
                    cur.pre_depends.push_str(line.trim());
                }
                "Provides" => cur.provides.extend(parse_provides(line.trim())),
                _ => {}
            }
            continue;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim();
        have = true;
        last_field = k.to_string();
        match k {
            "Package" => cur.name = v.to_string(),
            "Version" => cur.version = v.to_string(),
            "Architecture" => cur.arch = v.to_string(),
            "Depends" => cur.depends = v.to_string(),
            "Pre-Depends" => cur.pre_depends = v.to_string(),
            "Provides" => cur.provides = parse_provides(v),
            "Essential" => cur.essential = v.eq_ignore_ascii_case("yes"),
            "Priority" => cur.priority = v.to_string(),
            "Filename" => cur.filename = v.to_string(),
            "SHA256" => cur.sha256 = v.to_string(),
            "Size" => cur.size = v.parse().unwrap_or(0),
            "Kernel-Version" => cur.kernel_version = Some(v.to_string()),
            _ => {}
        }
    }
    if have && !cur.name.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

fn parse_provides(s: &str) -> Vec<(String, Option<String>)> {
    s.split(',')
        .filter_map(|item| {
            let atom = parse_atom(item.trim())?;
            let ver = match atom.ver {
                Some((Op::Eq, v)) => Some(v),
                _ => None,
            };
            Some((atom.name, ver))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

/// One alternative of a dependency: `name[:arch] [(op version)]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepAtom {
    pub name: String,
    /// Explicit architecture qualifier other than `any`/`native` (kept so it can be rejected).
    pub foreign_arch: Option<String>,
    pub ver: Option<(Op, String)>,
}

impl std::fmt::Display for DepAtom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)?;
        if let Some((op, v)) = &self.ver {
            let o = match op {
                Op::Lt => "<<",
                Op::Le => "<=",
                Op::Eq => "=",
                Op::Ge => ">=",
                Op::Gt => ">>",
            };
            write!(f, " ({o} {v})")?;
        }
        Ok(())
    }
}

fn parse_atom(s: &str) -> Option<DepAtom> {
    let s = s.trim();
    let (name_part, ver) = match s.find('(') {
        Some(i) => {
            let close = s[i..].find(')')? + i;
            let inner = s[i + 1..close].trim();
            let (op, rest) = ["<<", "<=", ">=", ">>", "=", "<", ">"].iter().find_map(|o| inner.strip_prefix(o).map(|r| (*o, r)))?;
            let op = match op {
                "<<" => Op::Lt,
                "<=" | "<" => Op::Le,
                "=" => Op::Eq,
                ">=" | ">" => Op::Ge,
                _ => Op::Gt,
            };
            (s[..i].trim(), Some((op, rest.trim().to_string())))
        }
        None => (s, None),
    };
    if name_part.is_empty() {
        return None;
    }
    let (name, foreign_arch) = match name_part.split_once(':') {
        Some((n, "any" | "native")) => (n, None),
        Some((n, a)) => (n, Some(a.to_string())),
        None => (name_part, None),
    };
    Some(DepAtom { name: name.to_string(), foreign_arch, ver })
}

/// `a (>= 1) | b, c` -> `[[a, b], [c]]`
pub fn parse_depends(s: &str) -> Vec<Vec<DepAtom>> {
    s.split(',')
        .filter_map(|group| {
            let alts: Vec<DepAtom> = group.split('|').filter_map(parse_atom).collect();
            (!alts.is_empty()).then_some(alts)
        })
        .collect()
}

// -- dpkg version comparison ---------------------------------------------------------------------------------

fn char_order(c: Option<u8>) -> i32 {
    match c {
        None => 0,
        Some(c) if c.is_ascii_digit() => 0,
        Some(c) if c.is_ascii_alphabetic() => c as i32,
        Some(b'~') => -1,
        Some(c) => c as i32 + 256,
    }
}

fn cmp_part(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() || j < b.len() {
        let mut first_diff = 0i32;
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let (ac, bc) = (char_order(a.get(i).copied()), char_order(b.get(j).copied()));
            if ac != bc {
                return ac.cmp(&bc);
            }
            i += 1;
            j += 1;
        }
        while i < a.len() && a[i] == b'0' {
            i += 1;
        }
        while j < b.len() && b[j] == b'0' {
            j += 1;
        }
        while i < a.len() && a[i].is_ascii_digit() && j < b.len() && b[j].is_ascii_digit() {
            if first_diff == 0 {
                first_diff = a[i] as i32 - b[j] as i32;
            }
            i += 1;
            j += 1;
        }
        if i < a.len() && a[i].is_ascii_digit() {
            return Ordering::Greater;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            return Ordering::Less;
        }
        if first_diff != 0 {
            return first_diff.cmp(&0);
        }
    }
    Ordering::Equal
}

fn split_version(v: &str) -> (u64, &str, &str) {
    let (epoch, rest) = match v.split_once(':') {
        Some((e, r)) if !e.is_empty() && e.bytes().all(|c| c.is_ascii_digit()) => (e.parse().unwrap_or(0), r),
        _ => (0, v),
    };
    match rest.rsplit_once('-') {
        Some((u, r)) => (epoch, u, r),
        None => (epoch, rest, ""),
    }
}

/// Compare two Debian version strings like `dpkg --compare-versions`.
pub fn dpkg_cmp(a: &str, b: &str) -> Ordering {
    let (ea, ua, ra) = split_version(a);
    let (eb, ub, rb) = split_version(b);
    ea.cmp(&eb).then_with(|| cmp_part(ua, ub)).then_with(|| cmp_part(ra, rb))
}

fn version_ok(have: &str, want: &Option<(Op, String)>) -> bool {
    let Some((op, v)) = want else { return true };
    let c = dpkg_cmp(have, v);
    match op {
        Op::Lt => c == Ordering::Less,
        Op::Le => c != Ordering::Greater,
        Op::Eq => c == Ordering::Equal,
        Op::Ge => c != Ordering::Less,
        Op::Gt => c == Ordering::Greater,
    }
}

// ============================================================================================================
// Release files
// ============================================================================================================

/// One line of the `SHA256:` section of a Release file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseEntry {
    pub sha256: String,
    pub size: u64,
    pub path: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReleaseInfo {
    pub fields: BTreeMap<String, String>,
    pub entries: Vec<ReleaseEntry>,
}

impl ReleaseInfo {
    pub fn entry(&self, path: &str) -> Option<&ReleaseEntry> {
        self.entries.iter().find(|e| e.path == path)
    }
    pub fn acquire_by_hash(&self) -> bool {
        self.fields.get("Acquire-By-Hash").is_some_and(|v| v.eq_ignore_ascii_case("yes"))
    }
}

/// Parse a `Release` or `InRelease` (PGP armour lines are ignored).
pub fn parse_release(text: &str) -> ReleaseInfo {
    let mut info = ReleaseInfo::default();
    let mut section = String::new();
    for line in text.lines() {
        if line.starts_with(' ') {
            if section == "SHA256" {
                let f: Vec<&str> = line.split_whitespace().collect();
                if f.len() == 3 && f[0].len() == 64 && f[0].bytes().all(|c| c.is_ascii_hexdigit()) {
                    info.entries.push(ReleaseEntry { sha256: f[0].to_string(), size: f[1].parse().unwrap_or(0), path: f[2].to_string() });
                }
            }
            continue;
        }
        section.clear();
        if line.starts_with("-----") {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if v.trim().is_empty() {
                section = k.to_string();
            } else {
                info.fields.entry(k.to_string()).or_insert_with(|| v.trim().to_string());
            }
        }
    }
    info
}

/// The signed text of a clear-signed document (`InRelease`), with dash-escaping removed. `None` if not clear-signed.
pub fn clearsign_body(text: &str) -> Option<String> {
    let rest = text.strip_prefix("-----BEGIN PGP SIGNED MESSAGE-----")?;
    let after_headers = rest.split_once("\n\n")?.1;
    let body = after_headers.split("\n-----BEGIN PGP SIGNATURE-----").next()?;
    let mut out = String::new();
    for (i, l) in body.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(l.strip_prefix("- ").unwrap_or(l));
    }
    Some(out)
}

// ============================================================================================================
// Universe and dependency resolver
// ============================================================================================================

/// How a candidate is chosen when a package exists in several suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// What a plain `apt-get install` does: the suite wins over backports (backports has priority 100).
    Stable,
    /// What `apt-get install -t <suite>-backports` does: backports wins whenever it has the package.
    BackportsFirst,
}

/// All packages of one architecture across the configured suites.
#[derive(Debug, Default)]
pub struct Universe {
    pub arch: String,
    pub repos: Vec<String>,
    pub pkgs: Vec<Pkg>,
    by_name: HashMap<String, Vec<usize>>,
    by_provide: HashMap<String, Vec<usize>>,
}

impl Universe {
    pub fn new(arch: &str) -> Universe {
        Universe { arch: arch.into(), ..Default::default() }
    }

    /// Add the packages of one suite. Packages for other architectures are ignored (`all` and the target are kept).
    pub fn add_repo(&mut self, suite: &str, pkgs: Vec<Pkg>) {
        let repo = self.repos.iter().position(|r| r == suite).unwrap_or_else(|| {
            self.repos.push(suite.to_string());
            self.repos.len() - 1
        });
        for mut p in pkgs {
            if p.arch != "all" && p.arch != self.arch {
                continue;
            }
            p.repo = repo;
            let i = self.pkgs.len();
            self.by_name.entry(p.name.clone()).or_default().push(i);
            for (v, _) in &p.provides {
                self.by_provide.entry(v.clone()).or_default().push(i);
            }
            self.pkgs.push(p);
        }
    }

    pub fn get(&self, name: &str) -> Vec<&Pkg> {
        self.by_name.get(name).map(|v| v.iter().map(|&i| &self.pkgs[i]).collect()).unwrap_or_default()
    }

    pub fn is_backports(&self, p: &Pkg) -> bool {
        self.repos[p.repo].ends_with("-backports")
    }

    fn repo_rank(&self, p: &Pkg, policy: Policy) -> u8 {
        match (policy, self.is_backports(p)) {
            (Policy::Stable, false) | (Policy::BackportsFirst, true) => 0,
            _ => 1,
        }
    }
}

fn priority_rank(p: &str) -> u8 {
    match p {
        "required" => 0,
        "important" => 1,
        "standard" => 2,
        "optional" => 3,
        _ => 4,
    }
}

#[derive(Debug, Clone)]
enum Origin {
    Seed(String),
    Dep(usize),
}

/// Computes an install closure.
///
/// Deliberate simplifications (the integration test shows they are sufficient; see `docs/OFFLINE.md`):
/// * Only `Depends` and `Pre-Depends` are followed - never `Recommends`/`Suggests` (the installer passes
///   `--no-install-recommends`).
/// * Version constraints are honoured when choosing between suites (a constraint only the backports version meets
///   selects backports), but there is no backtracking: the first candidate that satisfies a constraint is taken.
/// * For an alternative group `a | b` the first alternative that exists is selected unless something that
///   satisfies the group is already selected. A virtual package with several providers gets the provider with
///   the highest priority (`required` first), then the preferred suite, then the alphabetical first name.
pub struct Resolver<'u> {
    u: &'u Universe,
    order: Vec<usize>,
    sel: HashSet<usize>,
    sel_by_name: HashMap<&'u str, Vec<usize>>,
    sel_by_provide: HashMap<&'u str, Vec<usize>>,
    why: HashMap<usize, Origin>,
    queue: VecDeque<usize>,
    /// Names that are never installed (debootstrap's `usrmerge` exclusion).
    pub excluded: HashSet<String>,
    errors: Vec<String>,
}

impl<'u> Resolver<'u> {
    pub fn new(u: &'u Universe) -> Resolver<'u> {
        Resolver {
            u,
            order: vec![],
            sel: HashSet::new(),
            sel_by_name: HashMap::new(),
            sel_by_provide: HashMap::new(),
            why: HashMap::new(),
            queue: VecDeque::new(),
            excluded: HashSet::new(),
            errors: vec![],
        }
    }

    /// Selected packages in selection order.
    pub fn selected(&self) -> Vec<&'u Pkg> {
        self.order.iter().map(|&i| &self.u.pkgs[i]).collect()
    }

    pub fn is_selected(&self, name: &str) -> bool {
        self.sel_by_name.contains_key(name)
    }

    /// Problems found so far (each names the dependency chain).
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    fn select(&mut self, i: usize, why: Origin) {
        if !self.sel.insert(i) {
            return;
        }
        let u: &'u Universe = self.u;
        let p = &u.pkgs[i];
        self.order.push(i);
        self.sel_by_name.entry(p.name.as_str()).or_default().push(i);
        for (v, _) in &p.provides {
            self.sel_by_provide.entry(v.as_str()).or_default().push(i);
        }
        self.why.insert(i, why);
        self.queue.push_back(i);
    }

    fn chain(&self, mut i: usize) -> String {
        let mut parts = vec![];
        loop {
            parts.push(format!("{} {}", self.u.pkgs[i].name, self.u.pkgs[i].version));
            match self.why.get(&i) {
                Some(Origin::Dep(parent)) if parts.len() < 12 => i = *parent,
                Some(Origin::Seed(s)) => {
                    parts.push(format!("[{s}]"));
                    break;
                }
                _ => break,
            }
        }
        parts.join(" <- ")
    }

    fn satisfied_by_selected(&self, a: &DepAtom) -> bool {
        if let Some(v) = self.sel_by_name.get(a.name.as_str())
            && v.iter().any(|&i| version_ok(&self.u.pkgs[i].version, &a.ver))
        {
            return true;
        }
        if let Some(v) = self.sel_by_provide.get(a.name.as_str()) {
            for &i in v {
                for (n, pv) in &self.u.pkgs[i].provides {
                    if n == &a.name && provide_ok(pv, &a.ver) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn real_candidate(&self, a: &DepAtom, policy: Policy) -> Option<usize> {
        if a.foreign_arch.as_ref().is_some_and(|fa| fa != &self.u.arch) {
            return None;
        }
        let u = self.u;
        u.by_name.get(&a.name).into_iter().flatten().copied().filter(|&i| version_ok(&u.pkgs[i].version, &a.ver)).min_by(|&x, &y| {
            let (px, py) = (&u.pkgs[x], &u.pkgs[y]);
            u.repo_rank(px, policy).cmp(&u.repo_rank(py, policy)).then_with(|| dpkg_cmp(&py.version, &px.version))
        })
    }

    fn provider_candidate(&self, a: &DepAtom, policy: Policy) -> Option<usize> {
        if a.foreign_arch.as_ref().is_some_and(|fa| fa != &self.u.arch) {
            return None;
        }
        let u = self.u;
        u.by_provide
            .get(&a.name)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| u.pkgs[i].provides.iter().any(|(n, pv)| n == &a.name && provide_ok(pv, &a.ver)))
            .min_by(|&x, &y| {
                let (px, py) = (&u.pkgs[x], &u.pkgs[y]);
                priority_rank(&px.priority)
                    .cmp(&priority_rank(&py.priority))
                    .then_with(|| u.repo_rank(px, policy).cmp(&u.repo_rank(py, policy)))
                    .then_with(|| px.name.cmp(&py.name))
                    .then_with(|| dpkg_cmp(&py.version, &px.version))
            })
    }

    /// A real package of that name wins over providers of the same name (as in apt).
    fn candidate(&self, a: &DepAtom, policy: Policy) -> Option<usize> {
        self.real_candidate(a, policy).or_else(|| self.provider_candidate(a, policy))
    }

    fn satisfy(&mut self, group: &[DepAtom], policy: Policy, from: usize) {
        if group.iter().any(|a| self.satisfied_by_selected(a)) {
            return;
        }
        for a in group {
            if self.excluded.contains(&a.name) {
                continue;
            }
            if let Some(c) = self.candidate(a, policy) {
                self.select(c, Origin::Dep(from));
                return;
            }
        }
        if group.iter().all(|a| self.excluded.contains(&a.name)) {
            return;
        }
        let want = group.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(" | ");
        let msg = format!("nothing in the mirror satisfies '{want}', required by {}", self.chain(from));
        if !self.errors.contains(&msg) {
            self.errors.push(msg);
        }
    }

    fn drain(&mut self, policy: Policy) {
        while let Some(i) = self.queue.pop_front() {
            let u: &'u Universe = self.u;
            let p = &u.pkgs[i];
            for field in [&p.pre_depends, &p.depends] {
                for group in parse_depends(field) {
                    self.satisfy(&group, policy, i);
                }
            }
        }
    }

    /// Select `spec` (`name` or `name (>= version)`) and everything it needs. `label` appears in error chains.
    pub fn install(&mut self, spec: &str, policy: Policy, label: &str) {
        let Some(atom) = parse_atom(spec) else {
            self.errors.push(format!("[{label}] unparsable package spec '{spec}'"));
            return;
        };
        // Named explicitly: the package of that name is wanted, not just something that provides the name
        // (`base-files` provides `usr-is-merged`, yet debootstrap installs the real `usr-is-merged` package too).
        let real_selected =
            self.sel_by_name.get(atom.name.as_str()).is_some_and(|v| v.iter().any(|&i| version_ok(&self.u.pkgs[i].version, &atom.ver)));
        if real_selected {
            return;
        }
        let pick = match self.real_candidate(&atom, policy) {
            Some(c) => Some(c),
            None if self.satisfied_by_selected(&atom) => return,
            None => self.provider_candidate(&atom, policy),
        };
        match pick {
            Some(c) => {
                self.select(c, Origin::Seed(label.to_string()));
                self.drain(policy);
            }
            None => self.errors.push(format!("[{label}] package '{spec}' does not exist in the mirror for {}", self.u.arch)),
        }
    }

    /// Like [`Resolver::install`], but when `spec` cannot be installed completely, everything it added is undone and the
    /// reasons are returned instead of being kept as errors (an installer component whose dependency does not exist
    /// for this architecture is simply not installable there, and `anna` skips it too).
    pub fn try_install(&mut self, spec: &str, policy: Policy, label: &str) -> std::result::Result<(), Vec<String>> {
        let (order_len, err_len) = (self.order.len(), self.errors.len());
        self.install(spec, policy, label);
        if self.errors.len() == err_len {
            return Ok(());
        }
        let reasons = self.errors.split_off(err_len);
        for i in self.order.split_off(order_len) {
            self.sel.remove(&i);
            self.why.remove(&i);
            for v in self.sel_by_name.values_mut() {
                v.retain(|x| *x != i);
            }
            for v in self.sel_by_provide.values_mut() {
                v.retain(|x| *x != i);
            }
        }
        self.sel_by_name.retain(|_, v| !v.is_empty());
        self.sel_by_provide.retain(|_, v| !v.is_empty());
        self.queue.clear();
        Err(reasons)
    }

    /// Whether `spec` could be installed at all (a real package or a provider exists).
    pub fn exists(&self, spec: &str) -> bool {
        parse_atom(spec).is_some_and(|a| self.candidate(&a, Policy::Stable).is_some())
    }

    pub fn install_all<S: AsRef<str>>(&mut self, specs: &[S], policy: Policy, label: &str) {
        for s in specs {
            self.install(s.as_ref(), policy, label);
        }
    }

    /// Fail with every collected problem.
    pub fn finish(self) -> Result<Vec<&'u Pkg>> {
        if !self.errors.is_empty() {
            bail!("the package closure is incomplete:\n  {}", self.errors.join("\n  "));
        }
        Ok(self.selected())
    }
}

fn provide_ok(provided: &Option<String>, want: &Option<(Op, String)>) -> bool {
    match (want, provided) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(_), Some(v)) => version_ok(v, want),
    }
}

// ============================================================================================================
// Seeds: what the installation really installs
// ============================================================================================================

/// Installed in `chroot-setup.sh` before the kernel: `apt_install debian-archive-keyring tzdata`.
pub const CHROOT_KEYRING: &[&str] = &["debian-archive-keyring", "tzdata"];
/// `apt_install debconf locales`
pub const CHROOT_LOCALES: &[&str] = &["debconf", "locales"];
/// `apt_install initramfs-tools`
pub const CHROOT_INITRAMFS: &[&str] = &["initramfs-tools"];
/// The big `apt_install` of `chroot-setup.sh` after the bootloader/microcode packages and before `$EXTRA`.
/// A unit test fails when this list and the script drift apart.
pub const CHROOT_TOOLS: &[&str] = &[
    "lm-sensors",
    "mdadm",
    "lvm2",
    "xfsprogs",
    "btrfs-progs",
    "e2fsprogs",
    "dosfstools",
    "exfatprogs",
    "ntfs-3g",
    "parted",
    "gdisk",
    "smartmontools",
    "nvme-cli",
    "hdparm",
    "sudo",
    "openssh-server",
    "openssh-client",
    "systemd-resolved",
    "systemd-timesyncd",
    "avahi-daemon",
    "libnss-mdns",
    "python3",
    "openssl",
    "ca-certificates",
    "curl",
    "wget",
    "console-setup",
    "kbd",
    "gpm",
    "usbutils",
    "pciutils",
    "ethtool",
    "iproute2",
    "iputils-ping",
    "less",
    "nano",
    "htop",
    "rsync",
    "bash-completion",
    "unattended-upgrades",
    "firmware-linux-free",
    "firmware-misc-nonfree",
    "firmware-realtek",
];
/// debootstrap `--include=` of the generated stage-2 script.
pub const DEBOOTSTRAP_INCLUDE: &[&str] = &["ca-certificates", "locales", "dbus"];

/// Bootloader packages for every firmware type a unit of this architecture can have.
pub fn boot_packages(arch: Arch) -> Vec<&'static str> {
    match arch {
        Arch::X86_64 => vec!["grub-pc", "grub-efi-amd64", "efibootmgr"],
        Arch::Arm64 => vec!["grub-efi-arm64", "efibootmgr", "u-boot-tools"],
        Arch::Armhf => vec!["grub-efi-arm", "efibootmgr", "u-boot-tools"],
        Arch::Unsupported => vec![],
    }
}

/// CPU microcode packages (`MICROCODE_PKG` depends on the CPU vendor of the unit, so both ship).
pub fn microcode_packages(arch: Arch) -> Vec<&'static str> {
    match arch {
        Arch::X86_64 => vec!["intel-microcode", "amd64-microcode"],
        _ => vec![],
    }
}

/// Everything `payload/chroot-setup.sh` installs on top of the debootstrap base, in script order, for `arch`:
/// `(label, packages, policy)`.
pub fn chroot_phases(arch: Arch, kernel: KernelChannel, extra: &[String]) -> Vec<(&'static str, Vec<String>, Policy)> {
    let s = |l: &[&str]| l.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let mut bulk: Vec<String> = boot_packages(arch).into_iter().map(String::from).collect();
    bulk.extend(microcode_packages(arch).into_iter().map(String::from));
    bulk.extend(s(CHROOT_TOOLS));
    if arch == Arch::Armhf {
        bulk.push("haveged".into());
    }
    bulk.extend(extra.iter().cloned());
    vec![
        ("chroot: keyring+tzdata", s(CHROOT_KEYRING), Policy::Stable),
        ("chroot: locales", s(CHROOT_LOCALES), Policy::Stable),
        ("chroot: initramfs-tools", s(CHROOT_INITRAMFS), Policy::Stable),
        (
            "chroot: kernel",
            vec![arch.kernel_package().to_string()],
            if kernel == KernelChannel::Newest { Policy::BackportsFirst } else { Policy::Stable },
        ),
        ("chroot: tools", bulk, Policy::Stable),
    ]
}

/// Resolve everything the base install needs on one architecture. The universe must contain the suite (and
/// `<suite>-backports` for the newest kernel).
pub fn resolve_install_set<'u>(u: &'u Universe, arch: Arch, kernel: KernelChannel, extra: &[String]) -> Result<Vec<&'u Pkg>> {
    let mut r = Resolver::new(u);
    r.excluded.insert("usrmerge".into());
    // debootstrap --variant=minbase: Priority: required (+ Essential), apt, usr-is-merged and --include
    let mut base: Vec<String> =
        u.pkgs.iter().filter(|p| !u.is_backports(p) && (p.priority == "required" || p.essential)).map(|p| p.name.clone()).collect();
    base.sort();
    base.dedup();
    r.install_all(&base, Policy::Stable, "debootstrap: required");
    r.install_all(&["apt", "usr-is-merged"], Policy::Stable, "debootstrap: minbase");
    r.install_all(DEBOOTSTRAP_INCLUDE, Policy::Stable, "debootstrap: --include");
    for (label, pkgs, policy) in chroot_phases(arch, kernel, extra) {
        r.install_all(&pkgs, policy, label);
    }
    r.finish()
}

/// The udebs to ship for one architecture: those the stage-2 script loads with `anna-install`, everything the
/// installer marks `required`/`important`/`standard` (what `anna` loads on its own, including `base-installer` and
/// `debootstrap-udeb`) and every kernel-module udeb in the universe (what hardware detection loads). Filter the
/// universe to the installer kernel's ABI first with [`filter_udebs_for_kernel`].
///
/// Returns the selection, the `ANNA_PACKAGES` names the archive does not have at all (`try anna-install` ignores
/// those online and offline alike) and the udebs that cannot be installed for this architecture because a dependency
/// does not exist (for example `partman-xfs` where there is no `xfs-modules` udeb); both are reported as warnings.
pub fn resolve_udebs(u: &Universe) -> Result<(Vec<&Pkg>, Vec<String>, Vec<String>)> {
    let mut r = Resolver::new(u);
    let (known, unknown): (Vec<&str>, Vec<&str>) = crate::script::ANNA_PACKAGES.iter().copied().partition(|n| r.exists(n));
    let mut skipped = vec![];
    for n in &known {
        if let Err(why) = r.try_install(n, Policy::Stable, "anna-install") {
            skipped.push(format!("{n} ({})", why.join("; ")));
        }
    }
    let mut standard: Vec<&str> = u
        .pkgs
        .iter()
        .filter(|p| matches!(p.priority.as_str(), "required" | "important" | "standard") || p.kernel_version.is_some())
        .map(|p| p.name.as_str())
        .collect();
    standard.sort();
    standard.dedup();
    for n in standard {
        if let Err(why) = r.try_install(n, Policy::Stable, "installer: standard + kernel modules") {
            skipped.push(format!("{n} ({})", why.join("; ")));
        }
    }
    Ok((r.finish()?, unknown.into_iter().map(String::from).collect(), skipped))
}

/// Keep only module udebs of one kernel ABI (others are never loaded: `anna` matches `Kernel-Version`).
pub fn filter_udebs_for_kernel(pkgs: Vec<Pkg>, kernel_version: Option<&str>) -> Vec<Pkg> {
    match kernel_version {
        Some(kv) => pkgs.into_iter().filter(|p| p.kernel_version.as_deref().is_none_or(|k| k == kv)).collect(),
        None => pkgs,
    }
}

/// Kernel ABI of a Debian installer initrd: the directory below `lib/modules/` in its cpio archive.
pub fn initrd_kernel_version(initrd_gz: &Path) -> Result<Option<String>> {
    let f = std::fs::File::open(initrd_gz).with_context(|| format!("opening {}", initrd_gz.display()))?;
    let mut r = std::io::BufReader::with_capacity(1 << 20, flate2::read::MultiGzDecoder::new(f));
    let mut sink = vec![0u8; 64 * 1024];
    fn skip(r: &mut dyn Read, mut n: u64, sink: &mut [u8]) -> std::io::Result<()> {
        while n > 0 {
            let want = n.min(sink.len() as u64) as usize;
            let k = r.read(&mut sink[..want])?;
            if k == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            n -= k as u64;
        }
        Ok(())
    }
    loop {
        let mut hdr = [0u8; 110];
        if r.read_exact(&mut hdr).is_err() || (&hdr[..6] != b"070701" && &hdr[..6] != b"070702") {
            return Ok(None);
        }
        let hex = |o: usize| u64::from_str_radix(std::str::from_utf8(&hdr[o..o + 8]).unwrap_or("0"), 16).unwrap_or(0);
        let (filesize, namesize) = (hex(54), hex(94));
        let mut name = vec![0u8; namesize as usize];
        r.read_exact(&mut name)?;
        let name = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]).to_string();
        skip(&mut r, (4 - (110 + namesize) % 4) % 4, &mut sink)?;
        if name == "TRAILER!!!" {
            return Ok(None);
        }
        let n = name.trim_start_matches("./");
        let n = n.strip_prefix("usr/").unwrap_or(n);
        if let Some(rest) = n.strip_prefix("lib/modules/")
            && let Some(ver) = rest.split('/').next()
            && !ver.is_empty()
        {
            return Ok(Some(ver.to_string()));
        }
        skip(&mut r, filesize + (4 - filesize % 4) % 4, &mut sink)?;
    }
}

// ============================================================================================================
// Downloading
// ============================================================================================================

const ALLOWED_HOSTS: &[&str] =
    &["deb.debian.org", "cdimage.debian.org", "github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// `Ok(host)` for an https URL on an allowed host (or loopback http when `allow_loopback_http`).
pub fn check_url(url: &str, allow_loopback_http: bool) -> Result<String> {
    let (scheme, rest) = url.split_once("://").ok_or_else(|| anyhow!("not a URL: {url}"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('\\')
        || authority.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        bail!("refusing suspicious URL {url}");
    }
    let host = authority.split(':').next().unwrap_or("").to_ascii_lowercase();
    match scheme.to_ascii_lowercase().as_str() {
        "https" => {
            if authority.contains(':') && !authority.ends_with(":443") {
                bail!("refusing non-standard port in {url}");
            }
            if ALLOWED_HOSTS.contains(&host.as_str()) {
                Ok(host)
            } else {
                bail!("refusing to download from host '{host}' (allowed: {})", ALLOWED_HOSTS.join(", "))
            }
        }
        "http" if allow_loopback_http && (host == "127.0.0.1" || host == "localhost") => Ok(host),
        s => bail!("refusing {s}:// URL {url}: only https is allowed"),
    }
}

struct Fetcher {
    agent: ureq::Agent,
    src: Source,
}

#[derive(Clone)]
struct DlJob {
    /// Tried in order (by-hash first, then the plain path).
    urls: Vec<String>,
    dest: PathBuf,
    sha256: String,
    size: Option<u64>,
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn file_matches(path: &Path, size: Option<u64>, sha256: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => size.is_none_or(|s| s == m.len()) && sha256_file(path).is_ok_and(|h| h == sha256),
        _ => false,
    }
}

enum FetchError {
    /// 404 / 410: the file is not there (a by-hash URL may legitimately be absent).
    NotFound(String),
    RangeNotSatisfiable,
    Transient(String),
    Fatal(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NotFound(u) => write!(f, "not found: {u}"),
            FetchError::RangeNotSatisfiable => write!(f, "range not satisfiable"),
            FetchError::Transient(m) | FetchError::Fatal(m) => write!(f, "{m}"),
        }
    }
}

impl Fetcher {
    fn new(src: &Source) -> Fetcher {
        let mut b = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .redirects(0)
            .user_agent(concat!("synofurb/", env!("CARGO_PKG_VERSION"), " bundle"));
        if !src.allow_loopback_http {
            // honour HTTPS_PROXY / ALL_PROXY of the machine (corporate networks); TLS trust is the OS store
            b = b.try_proxy_from_env(true);
        }
        Fetcher { agent: b.build(), src: src.clone() }
    }

    /// GET with manual redirect following; every hop must pass the host allowlist.
    fn get(&self, url: &str, range_from: Option<u64>) -> std::result::Result<ureq::Response, FetchError> {
        let mut url = url.to_string();
        for _ in 0..6 {
            check_url(&url, self.src.allow_loopback_http).map_err(|e| FetchError::Fatal(e.to_string()))?;
            let mut req = self.agent.get(&url);
            if let Some(from) = range_from.filter(|f| *f > 0) {
                req = req.set("Range", &format!("bytes={from}-"));
            }
            match req.call() {
                Ok(resp) if (300..400).contains(&resp.status()) && resp.status() != 304 => {
                    let loc = resp
                        .header("location")
                        .ok_or_else(|| FetchError::Fatal(format!("redirect without Location from {url}")))?
                        .to_string();
                    url = if loc.starts_with("http") { loc } else { join_url(&url, &loc) };
                }
                Ok(resp) => return Ok(resp),
                Err(ureq::Error::Status(404 | 410, _)) => return Err(FetchError::NotFound(url)),
                Err(ureq::Error::Status(416, _)) => return Err(FetchError::RangeNotSatisfiable),
                Err(ureq::Error::Status(code @ (400..=499), _)) => {
                    return Err(FetchError::Fatal(format!("HTTP {code} from {url} (access denied?)")));
                }
                Err(ureq::Error::Status(code, _)) => return Err(FetchError::Transient(format!("HTTP {code} from {url}"))),
                Err(e) => return Err(FetchError::Transient(format!("{url}: {e}"))),
            }
        }
        Err(FetchError::Fatal(format!("too many redirects from {url}")))
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let mut last = String::new();
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(1 << attempt));
            }
            match self.get(url, None) {
                Ok(resp) => {
                    let mut buf = vec![];
                    match resp.into_reader().take(512 << 20).read_to_end(&mut buf) {
                        Ok(_) => return Ok(buf),
                        Err(e) => last = format!("{url}: {e}"),
                    }
                }
                Err(FetchError::Transient(m)) => last = m,
                Err(e) => bail!("{e}"),
            }
        }
        bail!("download failed: {last}")
    }

    fn get_text(&self, url: &str) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.get_bytes(url)?).to_string())
    }

    /// Size of a remote file without downloading it (`Content-Range` of a 1-byte ranged GET).
    fn remote_size(&self, url: &str) -> Option<u64> {
        let mut u = url.to_string();
        for _ in 0..6 {
            check_url(&u, self.src.allow_loopback_http).ok()?;
            let resp = self.agent.get(&u).set("Range", "bytes=0-0").call().ok()?;
            if (300..400).contains(&resp.status()) {
                let loc = resp.header("location")?.to_string();
                u = if loc.starts_with("http") { loc } else { join_url(&u, &loc) };
                continue;
            }
            if let Some(cr) = resp.header("content-range")
                && let Some((_, total)) = cr.rsplit_once('/')
            {
                return total.trim().parse().ok();
            }
            return resp.header("content-length").and_then(|v| v.parse().ok());
        }
        None
    }

    /// Download into `job.dest` (resuming a `.part`), verify, rename into place. Returns the bytes transferred.
    fn download_one(&self, job: &DlJob, counter: &AtomicU64) -> Result<u64> {
        if file_matches(&job.dest, job.size, &job.sha256) {
            counter
                .fetch_add(job.size.unwrap_or_else(|| std::fs::metadata(&job.dest).map(|m| m.len()).unwrap_or(0)), AtomicOrdering::Relaxed);
            return Ok(0);
        }
        if let Some(p) = job.dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let part = part_path(&job.dest);
        let mut transferred = 0u64;
        let mut last_err = String::new();
        let mut fresh_retry_done = false;
        let mut attempt = 0u32;
        'attempts: while attempt < 5 {
            attempt += 1;
            if attempt > 1 {
                std::thread::sleep(Duration::from_secs(1 << (attempt - 1).min(4)));
            }
            let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if job.size.is_some_and(|s| have > s) {
                let _ = std::fs::remove_file(&part);
                have = 0;
            }
            for url in &job.urls {
                let resp = match self.get(url, Some(have)) {
                    Ok(r) => r,
                    Err(FetchError::NotFound(_)) => continue, // try the next URL (by-hash -> plain path)
                    Err(FetchError::RangeNotSatisfiable) => {
                        let _ = std::fs::remove_file(&part);
                        continue 'attempts;
                    }
                    Err(FetchError::Fatal(m)) => bail!("{m}"),
                    Err(FetchError::Transient(m)) => {
                        last_err = m;
                        continue 'attempts;
                    }
                };
                let resumed = have > 0 && resp.status() == 206;
                let mut f = if resumed { std::fs::OpenOptions::new().append(true).open(&part)? } else { std::fs::File::create(&part)? };
                let mut r = resp.into_reader();
                let mut buf = vec![0u8; 256 * 1024];
                loop {
                    match r.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            f.write_all(&buf[..n])?;
                            transferred += n as u64;
                            counter.fetch_add(n as u64, AtomicOrdering::Relaxed);
                        }
                        Err(e) => {
                            last_err = format!("{url}: {e}");
                            continue 'attempts; // keep the .part; the next attempt resumes
                        }
                    }
                }
                f.flush()?;
                drop(f);
                let size_ok = job.size.is_none_or(|s| std::fs::metadata(&part).map(|m| m.len() == s).unwrap_or(false));
                if size_ok && sha256_file(&part)? == job.sha256 {
                    std::fs::rename(&part, &job.dest)?;
                    return Ok(transferred);
                }
                // corrupt or truncated: start from scratch once, then give up
                let _ = std::fs::remove_file(&part);
                if fresh_retry_done {
                    bail!(
                        "checksum mismatch for {url}: the downloaded file is not the one the signed index lists (the archive may have changed while building the bundle; run the command again)"
                    );
                }
                fresh_retry_done = true;
                last_err = format!("checksum mismatch for {url}");
                continue 'attempts;
            }
            // every URL answered 404
            bail!(
                "not found on the server: {} (the archive changed while building the bundle - superseded packages are removed from the pool; run the command again)",
                job.urls.last().cloned().unwrap_or_default()
            );
        }
        bail!("download failed after retries: {last_err}")
    }

    /// Run many downloads on a few threads. Progress is reported from the calling thread.
    fn download_all(&self, jobs: Vec<DlJob>, label: &str, log: &Logger, progress: &mut dyn FnMut(&str, f32)) -> Result<()> {
        let mut seen = HashSet::new();
        let jobs: Vec<DlJob> = jobs.into_iter().filter(|j| seen.insert(j.dest.clone())).collect();
        if jobs.is_empty() {
            return Ok(());
        }
        let total: u64 = jobs.iter().map(|j| j.size.unwrap_or(0)).sum();
        let count_based = jobs.iter().any(|j| j.size.is_none());
        let n_jobs = jobs.len();
        let queue = Mutex::new(jobs.into_iter().collect::<VecDeque<_>>());
        let counter = AtomicU64::new(0);
        let finished = AtomicUsize::new(0);
        let downloaded = AtomicU64::new(0);
        let abort = AtomicBool::new(false);
        let first_err: Mutex<Option<anyhow::Error>> = Mutex::new(None);
        let workers = 4.min(n_jobs);
        std::thread::scope(|s| {
            let mut handles = vec![];
            for _ in 0..workers {
                handles.push(s.spawn(|| {
                    loop {
                        if abort.load(AtomicOrdering::Relaxed) {
                            return;
                        }
                        let Some(job) = queue.lock().unwrap().pop_front() else { return };
                        match self.download_one(&job, &counter) {
                            Ok(n) => {
                                downloaded.fetch_add(n, AtomicOrdering::Relaxed);
                                finished.fetch_add(1, AtomicOrdering::Relaxed);
                            }
                            Err(e) => {
                                abort.store(true, AtomicOrdering::Relaxed);
                                let mut g = first_err.lock().unwrap();
                                if g.is_none() {
                                    *g = Some(e.context(format!("downloading {}", job.dest.display())));
                                }
                                return;
                            }
                        }
                    }
                }));
            }
            while handles.iter().any(|h| !h.is_finished()) {
                std::thread::sleep(Duration::from_millis(250));
                let f = if count_based || total == 0 {
                    finished.load(AtomicOrdering::Relaxed) as f32 / n_jobs as f32
                } else {
                    counter.load(AtomicOrdering::Relaxed) as f32 / total as f32
                };
                progress(label, f.min(1.0));
            }
        });
        if let Some(e) = first_err.into_inner().unwrap() {
            return Err(e);
        }
        progress(label, 1.0);
        log.info(format!(
            "{label}: {n_jobs} files ready ({} downloaded now, the rest were already on disk)",
            human_bytes(downloaded.load(AtomicOrdering::Relaxed))
        ));
        Ok(())
    }
}

fn join_url(base: &str, loc: &str) -> String {
    if loc.starts_with('/') {
        let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
        let host_end = base[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(base.len());
        format!("{}{}", &base[..host_end], loc)
    } else {
        let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
        format!("{dir}/{loc}")
    }
}

// ============================================================================================================
// create_bundle
// ============================================================================================================

fn validate_spec(spec: &BundleSpec) -> Result<()> {
    if spec.archs.is_empty() {
        bail!("choose at least one architecture for the bundle");
    }
    if spec.archs.contains(&Arch::Unsupported) {
        bail!("Debian 13 has no installer for that architecture");
    }
    if spec.suite.is_empty() || !spec.suite.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        bail!("invalid suite '{}'", spec.suite);
    }
    for p in &spec.extra_packages {
        if !valid_package_name(p) {
            bail!("invalid package name '{p}'");
        }
    }
    for d in &spec.dtbs {
        if !d.ends_with(".dtb") || !d.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
            bail!("invalid DTB file name '{d}'");
        }
    }
    Ok(())
}

fn rel_to_path(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
    }
    p
}

fn safe_pool_path(filename: &str) -> Result<()> {
    let ok = filename.starts_with("pool/")
        && !filename.contains("..")
        && !filename.contains("//")
        && filename.chars().all(|c| c.is_ascii_alphanumeric() || "/._+-~".contains(c));
    if ok { Ok(()) } else { bail!("the index lists an unsafe file name: {filename}") }
}

/// Index files (path below `dists/<suite>/`) one architecture needs, as far as the Release file lists them.
fn index_paths(suite: &str, base_suite: &str, arch: &str, rel: &ReleaseInfo) -> Vec<String> {
    let mut v = vec![];
    for comp in COMPONENTS {
        let mut dirs = vec![format!("{comp}/binary-{arch}"), format!("{comp}/binary-all")];
        if suite == base_suite {
            dirs.push(format!("{comp}/debian-installer/binary-{arch}"));
        }
        for d in dirs {
            for ext in ["gz", "xz"] {
                let p = format!("{d}/Packages.{ext}");
                if rel.entry(&p).is_some() {
                    v.push(p);
                }
            }
        }
    }
    v
}

struct SuiteData {
    name: String,
    release: ReleaseInfo,
}

/// Build (or resume building) a bundle in `out_dir`. Needs Internet. Interrupted runs leave `.part` files and are
/// resumed by running the same command again; files that already verify are not downloaded twice.
pub fn create_bundle(spec: &BundleSpec, out_dir: &Path, log: &Logger, progress: &mut dyn FnMut(&str, f32)) -> Result<BundleManifest> {
    create_bundle_from(&Source::default(), spec, out_dir, log, progress)
}

/// [`create_bundle`] with an explicit download source (tests use a loopback server).
pub fn create_bundle_from(
    src: &Source,
    spec: &BundleSpec,
    out_dir: &Path,
    log: &Logger,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<BundleManifest> {
    validate_spec(spec)?;
    let fetcher = Fetcher::new(src);
    let mut warnings: Vec<String> = vec![];
    let archs: Vec<Arch> = {
        let mut seen = HashSet::new();
        spec.archs.iter().copied().filter(|a| seen.insert(*a)).collect()
    };

    // -- the output directory: refuse to scribble into a directory that is not ours ---------------------------
    if out_dir.exists() {
        let non_empty = std::fs::read_dir(out_dir)?.next().is_some();
        if non_empty && !out_dir.join(MANIFEST_FILE).exists() && !out_dir.join(BUILDING_MARKER).exists() {
            bail!("{} is not empty and is not a synofurb bundle; choose an empty directory", out_dir.display());
        }
    }
    std::fs::create_dir_all(out_dir)?;
    std::fs::write(out_dir.join(BUILDING_MARKER), format!("synofurb {} bundle build in progress\n", env!("CARGO_PKG_VERSION")))?;
    let _ = std::fs::remove_file(out_dir.join(MANIFEST_FILE));
    let _ = std::fs::remove_file(out_dir.join(SUMS_FILE));
    let mirror = out_dir.join(MIRROR_ROOT).join(MIRROR_NAME);
    let _ = std::fs::remove_dir_all(mirror.join("dists")); // small; the pool is kept for resuming
    let work = out_dir.join(WORK_DIR);

    // -- 1. Release files of every suite -------------------------------------------------------------------------
    let mut suite_names = vec![spec.suite.clone()];
    if spec.kernel == KernelChannel::Newest {
        suite_names.push(format!("{}-backports", spec.suite));
    }
    progress("Release files", 0.0);
    let mut suites: Vec<SuiteData> = vec![];
    let mut pgp_verified = true;
    let mut release_dates = BTreeMap::new();
    for name in &suite_names {
        let base = format!("{}/dists/{name}", src.debian_base);
        let dir = mirror.join("dists").join(name);
        std::fs::create_dir_all(&dir)?;
        log.info(format!("fetching {name}/InRelease"));
        let inrelease = fetcher.get_text(&format!("{base}/InRelease"))?;
        if !debian::gpg_verify_inrelease(&inrelease, log) {
            pgp_verified = false;
        }
        std::fs::write(dir.join("InRelease"), &inrelease)?;
        let release = parse_release(&inrelease);
        if release.entries.is_empty() {
            bail!("{name}/InRelease lists no files; is '{name}' a Debian suite?");
        }
        if let Some(d) = release.fields.get("Date") {
            release_dates.insert(name.clone(), d.clone());
        }
        // Release + Release.gpg: old debootstrap versions (and net-retriever of the installer) use them
        match fetcher.get_text(&format!("{base}/Release")) {
            Ok(rel) => {
                if clearsign_body(&inrelease).is_some_and(|b| b.trim_end() == rel.trim_end()) {
                    std::fs::write(dir.join("Release"), &rel)?;
                    match fetcher.get_bytes(&format!("{base}/Release.gpg")) {
                        Ok(sig) => std::fs::write(dir.join("Release.gpg"), sig)?,
                        Err(e) => warnings
                            .push(format!("{name}/Release.gpg could not be fetched ({e}); only InRelease is available to old tools")),
                    }
                } else {
                    warnings.push(format!("{name}/Release differs from the signed InRelease; Release and Release.gpg are not included"));
                }
            }
            Err(e) => warnings.push(format!("{name}/Release could not be fetched ({e}); only InRelease is available to old tools")),
        }
        suites.push(SuiteData { name: name.clone(), release });
    }
    progress("Release files", 1.0);
    if !pgp_verified {
        warnings.push("the PGP signature of InRelease was not checked on this computer (no gpgv / current debian-archive-keyring); the HTTPS and SHA-256 chain was used. The NAS verifies Debian's signatures itself while installing".into());
    }

    // -- 2. package indexes for every architecture -----------------------------------------------------------------
    let mut index_jobs = vec![];
    for a in &archs {
        let da = a.debian();
        for s in &suites {
            let paths = index_paths(&s.name, &spec.suite, da, &s.release);
            if !paths.iter().any(|p| p == &format!("main/binary-{da}/Packages.gz") || p == &format!("main/binary-{da}/Packages.xz")) {
                bail!("{} has no package index for {da}; is the architecture available?", s.name);
            }
            for p in paths {
                let e = s.release.entry(&p).context("index vanished")?;
                let dir = p.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                let mut urls = vec![];
                if s.release.acquire_by_hash() {
                    urls.push(format!("{}/dists/{}/{dir}/by-hash/SHA256/{}", src.debian_base, s.name, e.sha256));
                }
                urls.push(format!("{}/dists/{}/{p}", src.debian_base, s.name));
                index_jobs.push(DlJob {
                    urls,
                    dest: rel_to_path(&mirror.join("dists").join(&s.name), &p),
                    sha256: e.sha256.clone(),
                    size: Some(e.size),
                });
            }
        }
    }
    fetcher.download_all(index_jobs, "package indexes", log, progress)?;

    // -- 3. installer files ---------------------------------------------------------------------------------------
    let main_release = &suites[0].release;
    let mut manifests: BTreeMap<String, ArchManifest> = BTreeMap::new();
    let mut installer_jobs = vec![];
    for a in &archs {
        let da = a.debian();
        let (kpath, ipath) = debian::netboot_paths(*a).context("no installer for this architecture")?;
        let sums_rel = format!("main/installer-{da}/current/images/SHA256SUMS");
        let want =
            main_release.entry(&sums_rel).map(|e| e.sha256.clone()).with_context(|| format!("{sums_rel} is not listed in InRelease"))?;
        let sums = fetcher.get_text(&format!("{}/dists/{}/{sums_rel}", src.debian_base, spec.suite))?;
        if debian::sha256_hex(sums.as_bytes()) != want {
            bail!("the installer SHA256SUMS of {da} does not match InRelease (the archive was updated just now; run the command again)");
        }
        let dir = out_dir.join("installer").join(da);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("SHA256SUMS"), &sums)?;
        std::fs::copy(mirror.join("dists").join(&spec.suite).join("InRelease"), dir.join("InRelease"))?;
        let base = format!("{}/dists/{}/main/installer-{da}/current/images", src.debian_base, spec.suite);
        let mut add = |rel: &str, name: &str| -> Result<()> {
            let h = debian::sums_sha256(&sums, rel).with_context(|| format!("{rel} is not in the installer SHA256SUMS"))?;
            installer_jobs.push(DlJob {
                urls: vec![format!("{base}/{}", rel.trim_start_matches("./"))],
                dest: dir.join(name),
                sha256: h,
                size: None,
            });
            Ok(())
        };
        add(kpath, "kernel")?;
        add(ipath, "initrd.gz")?;
        let mut m = ArchManifest {
            installer_files: vec![format!("installer/{da}/kernel"), format!("installer/{da}/initrd.gz")],
            ..Default::default()
        };
        if *a == Arch::Armhf {
            for d in &spec.dtbs {
                let rel = format!("./device-tree/{d}");
                if debian::sums_sha256(&sums, &rel).is_some() {
                    add(&rel, d)?;
                    m.installer_files.push(format!("installer/{da}/{d}"));
                } else {
                    warnings.push(format!("{d} is not shipped with Debian's installer; supply your own DTB"));
                }
            }
        } else if !spec.dtbs.is_empty() {
            log.debug(format!("DTBs are only used on armhf; ignoring them for {da}"));
        }
        m.installer_files.push(format!("installer/{da}/SHA256SUMS"));
        m.installer_files.push(format!("installer/{da}/InRelease"));
        manifests.insert(da.to_string(), m);
    }
    fetcher.download_all(installer_jobs, "installer images", log, progress)?;

    // -- 4. dependency closure per architecture ---------------------------------------------------------------------
    let mut pool_jobs: Vec<DlJob> = vec![];
    for a in &archs {
        let da = a.debian();
        progress(&format!("resolving packages for {da}"), 0.0);
        log.info(format!("resolving the package closure for {da}"));
        let mut u = Universe::new(da);
        for s in &suites {
            let dir = mirror.join("dists").join(&s.name);
            for comp in COMPONENTS {
                for sub in [format!("{comp}/binary-{da}"), format!("{comp}/binary-all")] {
                    if let Some(pk) = load_index(&dir.join(&sub))? {
                        u.add_repo(&s.name, pk);
                    }
                }
            }
        }
        let kver = initrd_kernel_version(&out_dir.join("installer").join(da).join("initrd.gz"))?;
        if kver.is_none() {
            warnings
                .push(format!("{da}: could not read the installer kernel version; the installer udebs of all kernel ABIs are included"));
        }
        let debs =
            resolve_install_set(&u, *a, spec.kernel, &spec.extra_packages).with_context(|| format!("resolving the {da} install set"))?;

        let mut ud = Universe::new(da);
        for comp in COMPONENTS {
            let d = mirror.join("dists").join(&spec.suite).join(comp).join("debian-installer").join(format!("binary-{da}"));
            if let Some(pk) = load_index(&d)? {
                ud.add_repo(&spec.suite, filter_udebs_for_kernel(pk, kver.as_deref()));
            }
        }
        let (udebs, unknown_anna, skipped_udebs) = resolve_udebs(&ud).with_context(|| format!("resolving the {da} installer udebs"))?;
        for sk in &skipped_udebs {
            warnings.push(format!("{da}: installer component not installable in this archive, left out: {sk}"));
        }
        if !unknown_anna.is_empty() {
            warnings.push(format!("{da}: the installer archive has no udeb named {} (the stage-2 script loads it with `try anna-install`, which ignores that; the install is not affected)", unknown_anna.join(", ")));
        }
        if let Some(kv) = &kver
            && !udebs.iter().any(|p| p.kernel_version.as_deref() == Some(kv.as_str()))
        {
            warnings.push(format!("{da}: the archive has no module udebs for the installer kernel {kv} (installer and archive are out of sync right now; the installer may not find disk drivers). Build the bundle again in a few hours"));
        }

        let m = manifests.get_mut(da).unwrap();
        m.installer_kernel = kver;
        m.packages = debs.len() as u32;
        m.udebs = udebs.len() as u32;
        let image = debs
            .iter()
            .filter(|p| p.name.strip_prefix("linux-image-").and_then(|r| r.chars().next()).is_some_and(|c| c.is_ascii_digit()))
            .max_by(|x, y| dpkg_cmp(&x.version, &y.version));
        if let Some(img) = image {
            m.kernel_package_version = Some(img.version.clone());
            m.kernel_from_backports = u.is_backports(img);
            if spec.kernel == KernelChannel::Newest && !m.kernel_from_backports {
                warnings
                    .push(format!("{da}: {}-backports has no newer kernel; the suite's own kernel {} is bundled", spec.suite, img.version));
            }
        }
        for p in debs.iter().chain(udebs.iter()) {
            safe_pool_path(&p.filename)?;
            if p.sha256.len() != 64 {
                bail!("the index lists no SHA-256 for {}", p.name);
            }
            m.pool_files.push(format!("{MIRROR_ROOT}/{MIRROR_NAME}/{}", p.filename));
            pool_jobs.push(DlJob {
                urls: vec![format!("{}/{}", src.debian_base, p.filename)],
                dest: mirror.join(&p.filename),
                sha256: p.sha256.clone(),
                size: Some(p.size),
            });
        }
        m.pool_files.sort();
        m.pool_files.dedup();
    }

    // -- 5. estimate, then the big downloads -----------------------------------------------------------------------
    let mut seen = HashSet::new();
    pool_jobs.retain(|j| seen.insert(j.dest.clone()));
    let total: u64 = pool_jobs.iter().map(|j| j.size.unwrap_or(0)).sum();
    let already: u64 =
        pool_jobs.iter().filter(|j| std::fs::metadata(&j.dest).is_ok_and(|m| Some(m.len()) == j.size)).map(|j| j.size.unwrap_or(0)).sum();
    let mut fw_note = String::new();
    let mut fw_job = None;
    let fw_dest = out_dir.join("installer").join("firmware.cpio.gz");
    if spec.with_firmware {
        let url = format!("{}/{}/current", src.firmware_base, spec.suite);
        let sums = fetcher.get_text(&format!("{url}/SHA256SUMS"))?;
        let h = debian::sums_sha256(&sums, "firmware.cpio.gz").context("firmware.cpio.gz is not listed")?;
        let size = fetcher.remote_size(&format!("{url}/firmware.cpio.gz"));
        std::fs::create_dir_all(out_dir.join("installer"))?;
        std::fs::write(out_dir.join("installer").join("firmware.SHA256SUMS"), &sums)?;
        fw_note = format!(" + installer firmware {}", size.map(human_bytes).unwrap_or_else(|| "(size unknown)".into()));
        fw_job = Some(DlJob { urls: vec![format!("{url}/firmware.cpio.gz")], dest: fw_dest.clone(), sha256: h, size });
    }
    log.info(format!(
        "bundle download estimate: {} of packages in {} files{} ({} of the packages are already on disk)",
        human_bytes(total),
        pool_jobs.len(),
        fw_note,
        human_bytes(already)
    ));
    if let Some(j) = fw_job {
        fetcher.download_all(vec![j], "installer firmware", log, progress)?;
    }
    fetcher.download_all(pool_jobs.clone(), "packages", log, progress)?;

    // -- 6. boot agent -----------------------------------------------------------------------------------------------
    if spec.include_agent {
        use crate::domboot::AgentFiles;
        for a in &archs {
            let da = a.debian();
            let res = AgentFiles::locate(*a, None).or_else(|_| {
                AgentFiles::fetch_release(*a, env!("CARGO_PKG_VERSION"), &work.join("agent"), log, &mut |n, f| progress(n, f))
            });
            match res {
                Ok(f) => {
                    let dir = out_dir.join("agent");
                    std::fs::create_dir_all(&dir)?;
                    std::fs::copy(&f.kernel, dir.join(format!("agent-{da}.vmlinuz")))?;
                    std::fs::copy(&f.initrd, dir.join(format!("agent-{da}.initrd")))?;
                    manifests.get_mut(da).unwrap().agent_files =
                        vec![format!("agent/agent-{da}.vmlinuz"), format!("agent/agent-{da}.initrd")];
                }
                Err(e) => warnings.push(format!(
                    "{da}: the boot agent is not in the bundle ({e}). Units with an internal boot flash need it; other units do not"
                )),
            }
        }
    }
    let _ = std::fs::remove_dir_all(&work);

    // -- 7. drop what this build does not reference (older builds in the same directory) ---------------------------
    let needed: HashSet<PathBuf> = pool_jobs.iter().map(|j| j.dest.clone()).collect();
    prune_pool(&mirror.join("pool"), &needed)?;
    if let Ok(rd) = std::fs::read_dir(out_dir.join("installer")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if e.path().is_dir() && !archs.iter().any(|a| a.debian() == n) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    if !spec.with_firmware {
        let _ = std::fs::remove_file(&fw_dest);
        let _ = std::fs::remove_file(out_dir.join("installer").join("firmware.SHA256SUMS"));
    }
    if let Ok(rd) = std::fs::read_dir(out_dir.join("agent")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !spec.include_agent || !archs.iter().any(|a| n.starts_with(&format!("agent-{}.", a.debian()))) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    // -- 7b. the management-app update bundle (small, from the copy built into this program, always rewritten) -------
    let mgmt = write_mgmt(out_dir).context("writing the management-app update bundle (mgmt/)")?;
    log.info(format!(
        "management app {} written to mgmt/ ({} files; `synofurb manage update --from-bundle` uses it offline)",
        mgmt.version,
        mgmt.files.len()
    ));

    // -- 8. manifest -------------------------------------------------------------------------------------------------
    progress("manifest", 0.0);
    let files = index_tree(out_dir, &mut |f| progress("hashing the bundle", f))?;
    let manifest = BundleManifest {
        format: FORMAT_VERSION,
        tool_version: env!("CARGO_PKG_VERSION").into(),
        created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        suite: spec.suite.clone(),
        suites: suite_names,
        kernel: spec.kernel,
        with_firmware: spec.with_firmware,
        extra_packages: spec.extra_packages.clone(),
        release_dates,
        pgp_verified,
        warnings,
        archs: manifests,
        files,
    };
    write_manifest(out_dir, &manifest)?;
    let _ = std::fs::remove_file(out_dir.join(BUILDING_MARKER));
    for w in &manifest.warnings {
        log.warn(w.clone());
    }
    let total_bytes: u64 = manifest.files.iter().map(|f| f.size).sum();
    log.info(format!("bundle ready: {} in {} files at {}", human_bytes(total_bytes), manifest.files.len(), out_dir.display()));
    progress("done", 1.0);
    Ok(manifest)
}

/// Write `mgmt/`: the management plane shipped in this program plus its `MANIFEST.sha256` (the layout of
/// `apps/mgmt-plane/tools/make-update-bundle.py`, which the tests compare it with). An existing `mgmt/` is replaced.
fn write_mgmt(out_dir: &Path) -> Result<MgmtBundle> {
    let dir = out_dir.join(MGMT_SUBDIR);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("replacing {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir)?;
    let b = MgmtBundle::embedded();
    for f in &b.files {
        std::fs::write(dir.join(&f.name), &f.data).with_context(|| format!("writing {}", dir.join(&f.name).display()))?;
    }
    std::fs::write(dir.join(MGMT_MANIFEST), b.manifest())?;
    Ok(b)
}

/// Read `Packages.gz` (or `Packages.xz`, the only compression `<suite>-backports` publishes) of one index directory;
/// `None` when neither is there.
fn load_index(dir: &Path) -> Result<Option<Vec<Pkg>>> {
    let gz = dir.join("Packages.gz");
    if gz.is_file() {
        return load_packages_gz(&gz).map(Some);
    }
    let xz = dir.join("Packages.xz");
    if xz.is_file() {
        let mut raw = std::io::BufReader::new(std::fs::File::open(&xz)?);
        let mut text = vec![];
        lzma_rs::xz_decompress(&mut raw, &mut text).map_err(|e| anyhow!("decompressing {}: {e}", xz.display()))?;
        return Ok(Some(parse_packages_reader(&text[..]).with_context(|| format!("reading {}", xz.display()))?));
    }
    Ok(None)
}

fn load_packages_gz(path: &Path) -> Result<Vec<Pkg>> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    parse_packages_reader(std::io::BufReader::with_capacity(1 << 20, flate2::read::MultiGzDecoder::new(f)))
        .with_context(|| format!("reading {}", path.display()))
}

fn prune_pool(pool: &Path, needed: &HashSet<PathBuf>) -> Result<()> {
    fn walk(dir: &Path, needed: &HashSet<PathBuf>) -> std::io::Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                walk(&p, needed)?;
                let _ = std::fs::remove_dir(&p); // only succeeds when empty
            } else if !needed.contains(&p) {
                std::fs::remove_file(&p)?;
            }
        }
        Ok(())
    }
    if pool.exists() {
        walk(pool, needed)?;
    }
    Ok(())
}

/// All files below `root` except the manifest, `SHA256SUMS`, scratch and partial files, with size and SHA-256.
fn index_tree(root: &Path, progress: &mut dyn FnMut(f32)) -> Result<Vec<FileEntry>> {
    let mut paths = vec![];
    collect_files(root, root, &mut paths)?;
    paths.sort();
    let total: u64 = paths.iter().map(|(_, p)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum();
    let mut done = 0u64;
    let mut out = vec![];
    for (rel, p) in paths {
        let size = std::fs::metadata(&p)?.len();
        out.push(FileEntry { path: rel, size, sha256: sha256_file(&p)? });
        done += size;
        progress(done as f32 / total.max(1) as f32);
    }
    Ok(out)
}

fn is_bookkeeping(rel: &str) -> bool {
    rel == MANIFEST_FILE
        || rel == SUMS_FILE
        || rel == BUILDING_MARKER
        || rel == "manifest.json.tmp"
        || rel == WORK_DIR
        || rel.starts_with(&format!("{WORK_DIR}/"))
        || rel.ends_with(".part")
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let p = e.path();
        let rel =
            p.strip_prefix(root).unwrap().components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
        if is_bookkeeping(&rel) {
            continue;
        }
        let ft = e.file_type()?;
        if ft.is_dir() {
            collect_files(root, &p, out)?;
        } else if ft.is_file() {
            out.push((rel, p));
        }
    }
    Ok(())
}

fn write_manifest(root: &Path, m: &BundleManifest) -> Result<()> {
    let mut sums = String::new();
    for f in &m.files {
        sums.push_str(&format!("{}  {}\n", f.sha256, f.path));
    }
    let json = serde_json::to_vec_pretty(m)?;
    sums.push_str(&format!("{}  {}\n", hex::encode(Sha256::digest(&json)), MANIFEST_FILE));
    let tmp = root.join("manifest.json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, root.join(MANIFEST_FILE))?;
    std::fs::write(root.join(SUMS_FILE), sums)?;
    Ok(())
}

// ============================================================================================================
// Opening, verifying, describing
// ============================================================================================================

impl Bundle {
    /// Read `manifest.json` and check the layout. No network, no hashing.
    pub fn open(dir: &Path) -> Result<Bundle> {
        if dir.join(BUILDING_MARKER).exists() {
            bail!(
                "{} is an unfinished bundle (its build was interrupted); run `synofurb bundle create` with the same directory to resume",
                dir.display()
            );
        }
        let mf = dir.join(MANIFEST_FILE);
        let text =
            std::fs::read_to_string(&mf).with_context(|| format!("{} has no {MANIFEST_FILE}; is it a synofurb bundle?", dir.display()))?;
        let manifest: BundleManifest = serde_json::from_str(&text).with_context(|| format!("{} is damaged", mf.display()))?;
        if manifest.format != FORMAT_VERSION {
            bail!(
                "the bundle has format {} but this synofurb understands format {}; update synofurb or build the bundle again",
                manifest.format,
                FORMAT_VERSION
            );
        }
        if !dir.join(MIRROR_ROOT).join(MIRROR_NAME).join("dists").is_dir() {
            bail!("{} has no mirror/ directory; the bundle is incomplete", dir.display());
        }
        Ok(Bundle { root: dir.to_path_buf(), manifest })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory the server publishes below `/<token>/mirror/` (contains `debian/`).
    pub fn mirror_dir(&self) -> PathBuf {
        self.root.join(MIRROR_ROOT)
    }

    pub fn has_arch(&self, arch: Arch) -> bool {
        self.manifest.archs.contains_key(arch.debian())
    }

    pub fn suites(&self) -> &[String] {
        &self.manifest.suites
    }

    /// Does the manifest list a `mgmt/` folder? (Bundles built before it existed have none and are still valid.)
    pub fn has_mgmt(&self) -> bool {
        let prefix = format!("{MGMT_SUBDIR}/");
        self.manifest.files.iter().any(|f| f.path.starts_with(&prefix))
    }

    /// The bundle's `mgmt/` folder when it has one.
    pub fn mgmt_dir(&self) -> Option<PathBuf> {
        self.has_mgmt().then(|| self.root.join(MGMT_SUBDIR))
    }

    /// Load and **check** the management-app update bundle: `mgmt/MANIFEST.sha256` must exist and match the files, and every file
    /// (and the manifest itself) must be what `manifest.json` recorded when the bundle was built. `Ok(None)` for a bundle without
    /// `mgmt/`. This is what `manage update --from-bundle <bundle>` uses, so a pushed update is always the one that was bundled.
    pub fn mgmt(&self) -> Result<Option<MgmtBundle>> {
        let Some(dir) = self.mgmt_dir() else { return Ok(None) };
        if !dir.join(MGMT_MANIFEST).is_file() {
            bail!("{MGMT_SUBDIR}/{MGMT_MANIFEST} is missing; the management-app update bundle is incomplete");
        }
        let b = MgmtBundle::from_plain_dir(&dir).with_context(|| format!("the management-app update bundle in {}", dir.display()))?;
        let recorded = |name: &str| self.manifest.files.iter().find(|f| f.path == format!("{MGMT_SUBDIR}/{name}"));
        let mut hashes: Vec<(String, String)> = b.files.iter().map(|f| (f.name.clone(), f.sha256())).collect();
        hashes.push((MGMT_MANIFEST.to_string(), sha256_file(&dir.join(MGMT_MANIFEST))?));
        for (name, have) in hashes {
            match recorded(&name) {
                None => bail!("{MGMT_SUBDIR}/{name} is not in the bundle manifest (it was added after the bundle was built)"),
                Some(e) if e.sha256 != have => {
                    bail!("{MGMT_SUBDIR}/{name} does not match the bundle manifest (the bundle is damaged or was modified)")
                }
                Some(_) => {}
            }
        }
        Ok(Some(b))
    }

    pub fn has_firmware(&self) -> bool {
        self.manifest.with_firmware && self.root.join("installer").join("firmware.cpio.gz").is_file()
    }

    /// The installer files of one architecture (not hashed; see [`Bundle::verify_installer`]).
    pub fn installer_files(&self, arch: Arch) -> Result<InstallerFiles> {
        if !self.has_arch(arch) {
            bail!(
                "the bundle has no {} installer; it contains: {}",
                arch.debian(),
                self.manifest.archs.keys().cloned().collect::<Vec<_>>().join(", ")
            );
        }
        let d = self.root.join("installer").join(arch.debian());
        Ok(InstallerFiles {
            kernel: d.join("kernel"),
            initrd: d.join("initrd.gz"),
            firmware: self.has_firmware().then(|| self.root.join("installer").join("firmware.cpio.gz")),
            dtb: None,
        })
    }

    /// Path of a bundled DTB for `arch` (armhf U-Boot boards).
    pub fn dtb(&self, arch: Arch, name: &str) -> Option<PathBuf> {
        let rel = format!("installer/{}/{name}", arch.debian());
        self.manifest.files.iter().any(|f| f.path == rel).then(|| rel_to_path(&self.root, &rel))
    }

    /// Directory with `agent-<arch>.{vmlinuz,initrd}` when the bundle has the agent for `arch`.
    pub fn agent_dir(&self, arch: Arch) -> Option<PathBuf> {
        let a = self.manifest.archs.get(arch.debian())?;
        (!a.agent_files.is_empty()).then(|| self.root.join("agent"))
    }

    /// Re-hash the installer files (and firmware) of `arch` against the manifest. Used right before serving them.
    pub fn verify_installer(&self, arch: Arch) -> Result<()> {
        let a = self.manifest.archs.get(arch.debian()).with_context(|| format!("the bundle has no {} installer", arch.debian()))?;
        let mut rels = a.installer_files.clone();
        if self.has_firmware() {
            rels.push("installer/firmware.cpio.gz".into());
        }
        for rel in rels {
            let e = self.manifest.files.iter().find(|f| f.path == rel).with_context(|| format!("{rel} is not in the manifest"))?;
            let p = rel_to_path(&self.root, &rel);
            if sha256_file(&p).with_context(|| format!("reading {rel}"))? != e.sha256 {
                bail!("{rel} does not match the bundle manifest (the bundle is damaged or was modified); run `synofurb bundle verify`");
            }
        }
        Ok(())
    }

    /// Re-hash every file and compare with the manifest; also checks `Packages` indexes against `InRelease`.
    pub fn verify(&self, progress: &mut dyn FnMut(&str, f32)) -> Result<VerifyReport> {
        let mut rep = VerifyReport::default();
        let total: u64 = self.manifest.files.iter().map(|f| f.size).sum();
        let mut done = 0u64;
        let mut known = HashSet::new();
        for f in &self.manifest.files {
            known.insert(f.path.clone());
            let p = rel_to_path(&self.root, &f.path);
            match std::fs::metadata(&p) {
                Ok(m) if m.is_file() => {
                    if m.len() != f.size || sha256_file(&p).map(|h| h != f.sha256).unwrap_or(true) {
                        rep.changed.push(f.path.clone());
                    }
                }
                _ => rep.missing.push(f.path.clone()),
            }
            rep.files_checked += 1;
            rep.bytes_checked += f.size;
            done += f.size;
            progress("verifying", done as f32 / total.max(1) as f32);
        }
        let mut on_disk = vec![];
        collect_files(&self.root, &self.root, &mut on_disk)?;
        for (rel, _) in on_disk {
            if !known.contains(&rel) {
                rep.extra.push(rel);
            }
        }
        // index chain: every Packages file must be what its InRelease says
        for suite in &self.manifest.suites {
            let dir = self.mirror_dir().join(MIRROR_NAME).join("dists").join(suite);
            let Ok(text) = std::fs::read_to_string(dir.join("InRelease")) else {
                rep.chain_problems.push(format!("dists/{suite}/InRelease is missing"));
                continue;
            };
            for e in &parse_release(&text).entries {
                let p = rel_to_path(&dir, &e.path);
                if p.is_file() && sha256_file(&p).is_ok_and(|h| h != e.sha256) {
                    rep.chain_problems.push(format!("dists/{suite}/{} does not match InRelease", e.path));
                }
            }
        }
        // the management-app update bundle: its own manifest, and agreement with manifest.json
        if self.has_mgmt() {
            match self.mgmt() {
                Ok(Some(b)) => rep.mgmt_version = Some(b.version),
                Ok(None) => {}
                Err(e) => rep.mgmt_problems.push(format!("{e:#}")),
            }
        } else {
            rep.notes.push(
                "the bundle has no mgmt/ folder (built by an older synofurb). It installs and verifies as before, but `manage update --from-bundle` cannot take the management app from it; run `synofurb bundle create` again with the same directory to add it"
                    .into(),
            );
        }
        progress("verifying", 1.0);
        Ok(rep)
    }

    pub fn info(&self) -> BundleInfo {
        let size_of: HashMap<&str, u64> = self.manifest.files.iter().map(|f| (f.path.as_str(), f.size)).collect();
        let archs = self
            .manifest
            .archs
            .iter()
            .map(|(name, a)| ArchInfo {
                arch: name.clone(),
                installer_kernel: a.installer_kernel.clone(),
                kernel_package_version: a.kernel_package_version.clone(),
                packages: a.packages,
                udebs: a.udebs,
                bytes: a
                    .pool_files
                    .iter()
                    .chain(&a.installer_files)
                    .chain(&a.agent_files)
                    .filter_map(|p| size_of.get(p.as_str()))
                    .sum::<u64>(),
                has_agent: !a.agent_files.is_empty(),
            })
            .collect();
        BundleInfo {
            path: self.root.clone(),
            created: self.manifest.created.clone(),
            tool_version: self.manifest.tool_version.clone(),
            suite: self.manifest.suite.clone(),
            suites: self.manifest.suites.clone(),
            kernel: self.manifest.kernel,
            release_dates: self.manifest.release_dates.clone(),
            archs,
            with_firmware: self.manifest.with_firmware,
            firmware_bytes: size_of.get("installer/firmware.cpio.gz").copied().unwrap_or(0),
            total_bytes: self.manifest.files.iter().map(|f| f.size).sum(),
            files: self.manifest.files.len(),
            pgp_verified: self.manifest.pgp_verified,
            extra_packages: self.manifest.extra_packages.clone(),
            warnings: self.manifest.warnings.clone(),
            mgmt_version: self.mgmt_dir().and_then(|d| std::fs::read_to_string(d.join("VERSION")).ok()).and_then(|t| {
                let v = t.lines().next().unwrap_or("").trim().to_string();
                (!v.is_empty()).then_some(v)
            }),
        }
    }

    /// Everything that must hold before this bundle can drive an install of `arch`. Returns warnings.
    pub fn check_for_install(
        &self,
        arch: Arch,
        suite: &str,
        kernel: KernelChannel,
        extra_packages: &[String],
        want_firmware: bool,
    ) -> Result<Vec<String>> {
        let mut warn = vec![];
        if !self.has_arch(arch) {
            bail!(
                "this offline bundle has no {} support (it contains: {}). Build a bundle with `synofurb bundle create --arch {}`",
                arch.debian(),
                self.manifest.archs.keys().cloned().collect::<Vec<_>>().join(", "),
                arch.debian()
            );
        }
        if suite != self.manifest.suite {
            bail!("the bundle is for Debian {} but the install asks for {suite}", self.manifest.suite);
        }
        if kernel != self.manifest.kernel {
            bail!(
                "the bundle was built for the {} kernel but the install asks for the {} kernel. Either install with `--kernel {}` (the GUI: set Kernel to {} on the System tab), or build a bundle for the kernel you want: `synofurb bundle create --kernel {} --out <dir>` (a bundle holds one kernel and has no fallback to the other one)",
                self.manifest.kernel.label(),
                kernel.label(),
                kernel_flag(self.manifest.kernel),
                kernel_flag(self.manifest.kernel),
                kernel_flag(kernel),
            );
        }
        if self.has_mgmt() {
            self.mgmt().context("the bundle's management-app update bundle is damaged; run `synofurb bundle verify`")?;
        }
        let missing: Vec<&str> = extra_packages.iter().filter(|p| !self.manifest.extra_packages.contains(p)).map(|s| s.as_str()).collect();
        if !missing.is_empty() {
            bail!(
                "these extra packages are not in the bundle and cannot be installed offline: {}. Add them with `synofurb bundle create --extra`",
                missing.join(", ")
            );
        }
        if want_firmware && !self.has_firmware() {
            warn.push(
                "the bundle was built without installer firmware; hardware that needs non-free firmware may not work in the installer"
                    .into(),
            );
        }
        Ok(warn)
    }
}

// ============================================================================================================
// Unit tests
// ============================================================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn pkg(name: &str, version: &str, depends: &str) -> Pkg {
        Pkg {
            name: name.into(),
            version: version.into(),
            arch: "amd64".into(),
            depends: depends.into(),
            filename: format!("pool/main/{name}.deb"),
            sha256: "0".repeat(64),
            size: 10,
            priority: "optional".into(),
            ..Default::default()
        }
    }

    fn universe(stable: Vec<Pkg>, bp: Vec<Pkg>) -> Universe {
        let mut u = Universe::new("amd64");
        u.add_repo("trixie", stable);
        if !bp.is_empty() {
            u.add_repo("trixie-backports", bp);
        }
        u
    }

    fn names(r: &Resolver) -> Vec<String> {
        r.selected().iter().map(|p| p.name.clone()).collect()
    }

    #[test]
    fn parses_packages_stanzas() {
        let text = "Package: foo\nVersion: 1:2.0-1\nArchitecture: amd64\nDepends: libc6 (>= 2.34),\n bar | baz:any\nProvides: foo-virt (= 2.0), other\nPriority: required\nEssential: yes\nFilename: pool/main/f/foo/foo_2_amd64.deb\nSize: 1234\nSHA256: abc\nDescription: x\n long\n\nPackage: bar\nVersion: 1\nArchitecture: all\nKernel-Version: 6.12.1+deb13-amd64\n";
        let p = parse_packages(text);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "foo");
        assert_eq!(p[0].depends, "libc6 (>= 2.34), bar | baz:any");
        assert_eq!(p[0].provides, vec![("foo-virt".to_string(), Some("2.0".to_string())), ("other".to_string(), None)]);
        assert!(p[0].essential);
        assert_eq!(p[0].size, 1234);
        assert_eq!(p[1].kernel_version.as_deref(), Some("6.12.1+deb13-amd64"));
    }

    #[test]
    fn parses_dependency_syntax() {
        let d = parse_depends("libc6 (>= 2.34), python3:any (<< 3.14) | python3-minimal:native, debconf (>= 0.5) | debconf-2.0");
        assert_eq!(d.len(), 3);
        assert_eq!(d[0][0].name, "libc6");
        assert_eq!(d[0][0].ver, Some((Op::Ge, "2.34".into())));
        assert_eq!(d[1].len(), 2);
        assert_eq!(d[1][0].name, "python3", ":any is stripped");
        assert_eq!(d[1][0].ver, Some((Op::Lt, "3.14".into())));
        assert_eq!(d[1][1].name, "python3-minimal", ":native is stripped");
        assert_eq!(parse_depends("foo:i386")[0][0].foreign_arch.as_deref(), Some("i386"));
        assert_eq!(d[2][1].name, "debconf-2.0");
        assert!(parse_depends("").is_empty());
    }

    #[test]
    fn dpkg_version_ordering() {
        use Ordering::*;
        for (a, b, want) in [
            ("1.0", "1.0", Equal),
            ("1.0", "1.1", Less),
            ("1.0~rc1", "1.0", Less),
            ("1:0.9", "2.0", Greater),
            ("1.0-1", "1.0-2", Less),
            ("1.10", "1.9", Greater),
            ("7.2.11-1~bpo13+1", "6.12.94-1", Greater),
            ("2.41-5", "2.41-5+b1", Less),
            ("1.0a", "1.0", Greater),
            ("1.0+dfsg", "1.0", Greater),
            ("1.0-1~bpo13+1", "1.0-1", Less),
        ] {
            assert_eq!(dpkg_cmp(a, b), want, "{a} vs {b}");
        }
    }

    #[test]
    fn follows_depends_and_pre_depends_but_not_recommends() {
        let mut a = pkg("a", "1", "b");
        a.pre_depends = "c".into();
        let u = universe(vec![a, pkg("b", "1", ""), pkg("c", "1", ""), pkg("rec", "1", "")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("a", Policy::Stable, "t");
        let n = names(&r);
        r.finish().unwrap();
        assert_eq!(n.len(), 3);
        assert!(n.contains(&"b".into()) && n.contains(&"c".into()) && !n.contains(&"rec".into()));
    }

    #[test]
    fn alternatives_prefer_selected_then_first_available() {
        let u = universe(vec![pkg("top", "1", "x | y"), pkg("y", "1", ""), pkg("z", "1", "y | w"), pkg("w", "1", "")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("top", Policy::Stable, "t");
        assert_eq!(names(&r), vec!["top", "y"], "x does not exist, y is the first available");
        let u = universe(vec![pkg("z", "1", "y | w"), pkg("y", "1", ""), pkg("w", "1", ""), pkg("seed", "1", "w")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("seed", Policy::Stable, "t");
        r.install("z", Policy::Stable, "t");
        let n = names(&r);
        assert!(n.contains(&"w".into()) && !n.contains(&"y".into()), "w was already selected so y is not pulled in: {n:?}");
    }

    #[test]
    fn virtual_packages_via_provides_and_version_constraints() {
        let mut mawk = pkg("mawk", "1.3", "");
        mawk.provides = vec![("awk".into(), None)];
        mawk.priority = "required".into();
        let mut gawk = pkg("gawk", "5", "");
        gawk.provides = vec![("awk".into(), None)];
        let mut impl1 = pkg("impl", "1", "");
        impl1.provides = vec![("api".into(), Some("2.0".into()))];
        let u = universe(
            vec![
                pkg("needs-awk", "1", "awk"),
                gawk,
                mawk,
                pkg("needs-api", "1", "api (>= 1.5)"),
                pkg("needs-api3", "1", "api (>= 3)"),
                impl1,
            ],
            vec![],
        );
        let mut r = Resolver::new(&u);
        r.install("needs-awk", Policy::Stable, "t");
        r.install("needs-api", Policy::Stable, "t");
        let n = names(&r);
        assert!(n.contains(&"mawk".into()) && !n.contains(&"gawk".into()), "highest priority provider wins: {n:?}");
        assert!(n.contains(&"impl".into()));
        let mut r = Resolver::new(&u);
        r.install("needs-api3", Policy::Stable, "t");
        assert!(r.finish().is_err(), "the provided version 2.0 does not satisfy >= 3");
    }

    #[test]
    fn a_seed_names_the_real_package_even_when_a_selected_package_provides_the_name() {
        // trixie: base-files `Provides: usr-is-merged` and a real usr-is-merged package exists; debootstrap installs both
        let mut base = pkg("base-files", "1", "");
        base.provides = vec![("usr-is-merged".into(), None)];
        base.priority = "required".into();
        let u = universe(vec![base, pkg("usr-is-merged", "39", "")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("base-files", Policy::Stable, "t");
        r.install("usr-is-merged", Policy::Stable, "t");
        assert_eq!(names(&r), vec!["base-files", "usr-is-merged"]);
        // but a *dependency* on the name is satisfied by the provider
        let mut base = pkg("base-files", "1", "");
        base.provides = vec![("usr-is-merged".into(), None)];
        let u = universe(vec![base, pkg("usr-is-merged", "39", ""), pkg("needs", "1", "usr-is-merged")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("base-files", Policy::Stable, "t");
        r.install("needs", Policy::Stable, "t");
        assert_eq!(names(&r), vec!["base-files", "needs"]);
    }

    #[test]
    fn backports_preference_follows_the_policy() {
        let u = universe(
            vec![pkg("linux-image-amd64", "6.12", "linux-base"), pkg("linux-base", "4.9", "")],
            vec![pkg("linux-image-amd64", "7.2~bpo", "linux-base (>= 4.12)"), pkg("linux-base", "4.13~bpo", "")],
        );
        let ver = |r: &Resolver| r.selected().iter().map(|p| (p.name.clone(), p.version.clone())).collect::<Vec<_>>();
        let mut r = Resolver::new(&u);
        r.install("linux-image-amd64", Policy::Stable, "t");
        assert_eq!(ver(&r)[0].1, "6.12", "stable policy keeps the suite's version");
        let mut r = Resolver::new(&u);
        r.install("linux-image-amd64", Policy::BackportsFirst, "t");
        let v = ver(&r);
        assert_eq!(v[0].1, "7.2~bpo");
        assert_eq!(v[1].1, "4.13~bpo");
        // a backports-only package is still installable under the stable policy (apt would install it too)
        let u = universe(vec![], vec![pkg("only-bp", "1", "")]);
        let mut r = Resolver::new(&u);
        r.install("only-bp", Policy::Stable, "t");
        assert_eq!(r.selected().len(), 1);
    }

    #[test]
    fn already_selected_old_version_does_not_satisfy_a_newer_constraint() {
        let u = universe(vec![pkg("early", "1", "lib"), pkg("lib", "1", "")], vec![pkg("kern", "9", "lib (>= 2)"), pkg("lib", "2", "")]);
        let mut r = Resolver::new(&u);
        r.install("early", Policy::Stable, "t");
        r.install("kern", Policy::BackportsFirst, "t");
        let libs: Vec<_> = r.selected().iter().filter(|p| p.name == "lib").map(|p| p.version.clone()).collect();
        assert_eq!(libs, vec!["1", "2"], "both versions end up in the mirror");
    }

    #[test]
    fn missing_dependency_is_reported_with_the_chain() {
        let u = universe(vec![pkg("top", "1", "mid"), pkg("mid", "2", "ghost (>= 1) | phantom")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("top", Policy::Stable, "seed-x");
        let e = r.finish().unwrap_err().to_string();
        assert!(e.contains("ghost (>= 1) | phantom"), "{e}");
        assert!(e.contains("mid 2 <- top 1 <- [seed-x]"), "{e}");
        let mut r = Resolver::new(&u);
        r.install("nonexistent", Policy::Stable, "s");
        assert!(r.finish().unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn excluded_names_are_skipped_and_foreign_arch_is_rejected() {
        let u = universe(
            vec![
                pkg("base-files", "1", "usrmerge | usr-is-merged"),
                pkg("usr-is-merged", "1", ""),
                pkg("usrmerge", "1", ""),
                pkg("x", "1", "libfoo:i386"),
            ],
            vec![],
        );
        let mut r = Resolver::new(&u);
        r.excluded.insert("usrmerge".into());
        r.install("base-files", Policy::Stable, "t");
        assert_eq!(names(&r), vec!["base-files", "usr-is-merged"]);
        let mut r = Resolver::new(&u);
        r.install("x", Policy::Stable, "t");
        assert!(r.finish().is_err());
    }

    #[test]
    fn cyclic_dependencies_terminate() {
        let u = universe(vec![pkg("a", "1", "b"), pkg("b", "1", "a, c"), pkg("c", "1", "a")], vec![]);
        let mut r = Resolver::new(&u);
        r.install("a", Policy::Stable, "t");
        assert_eq!(r.finish().unwrap().len(), 3);
    }

    #[test]
    fn other_architectures_are_ignored() {
        let mut arm = pkg("foo", "1", "");
        arm.arch = "arm64".into();
        assert!(universe(vec![arm], vec![]).get("foo").is_empty());
    }

    #[test]
    fn udeb_selection_resolves_virtual_modules_for_the_installer_kernel() {
        let mut md_old = pkg("md-modules-6.12.94+deb13-amd64-di", "6.12.94-1", "kernel-image-6.12.94+deb13-amd64-di");
        md_old.provides = vec![("md-modules".into(), None)];
        md_old.kernel_version = Some("6.12.94+deb13-amd64".into());
        let mut md_new = md_old.clone();
        md_new.name = "md-modules-6.12.107+deb13-amd64-di".into();
        md_new.kernel_version = Some("6.12.107+deb13-amd64".into());
        md_new.depends = "kernel-image-6.12.107+deb13-amd64-di".into();
        let mut ki = pkg("kernel-image-6.12.107+deb13-amd64-di", "6.12.107-1", "");
        ki.kernel_version = md_new.kernel_version.clone();
        let mut parted = pkg("parted-udeb", "1", "libparted-udeb");
        parted.priority = "standard".into();
        let all = vec![md_old, md_new, ki, parted, pkg("libparted-udeb", "1", ""), pkg("debian-archive-keyring-udeb", "1", "")];
        let kept = filter_udebs_for_kernel(all, Some("6.12.107+deb13-amd64"));
        assert_eq!(kept.len(), 5, "the module udeb of the other ABI is dropped");
        let mut u = Universe::new("amd64");
        u.add_repo("trixie", kept);
        let mut r = Resolver::new(&u);
        r.install_all(&["parted-udeb", "md-modules", "debian-archive-keyring-udeb"], Policy::Stable, "anna");
        let n: Vec<_> = r.finish().unwrap().iter().map(|p| p.name.clone()).collect();
        assert!(n.contains(&"md-modules-6.12.107+deb13-amd64-di".into()));
        assert!(n.contains(&"kernel-image-6.12.107+deb13-amd64-di".into()));
        assert!(n.contains(&"libparted-udeb".into()));
        assert!(!n.iter().any(|x| x.contains("6.12.94")));
    }

    #[test]
    fn try_install_undoes_a_failed_closure() {
        let u = universe(
            vec![pkg("partman-xfs", "1", "helper, xfs-modules"), pkg("helper", "1", "dep"), pkg("dep", "1", ""), pkg("ok", "1", "dep")],
            vec![],
        );
        let mut r = Resolver::new(&u);
        let why = r.try_install("partman-xfs", Policy::Stable, "t").unwrap_err();
        assert!(why[0].contains("xfs-modules"), "{why:?}");
        assert!(r.selected().is_empty() && r.errors().is_empty() && !r.is_selected("helper"), "everything it pulled in is undone");
        r.try_install("ok", Policy::Stable, "t").unwrap();
        assert_eq!(names(&r), vec!["ok", "dep"]);
        assert!(r.finish().is_ok());
    }

    #[test]
    fn release_parsing_and_clearsign() {
        let hash = "a".repeat(64);
        let inrel = format!(
            "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nOrigin: Debian\nCodename: trixie\nDate: Sat, 12 Sep 2026 07:55:41 UTC\nAcquire-By-Hash: yes\nMD5Sum:\n 00 1 old\nSHA256:\n {hash}  123 main/binary-amd64/Packages.gz\n- -dash-escaped\n-----BEGIN PGP SIGNATURE-----\n\nxxx\n-----END PGP SIGNATURE-----\n"
        );
        let r = parse_release(&inrel);
        assert_eq!(r.fields["Codename"], "trixie");
        assert!(r.acquire_by_hash());
        assert_eq!(r.entries.len(), 1, "MD5 section ignored");
        assert_eq!(r.entry("main/binary-amd64/Packages.gz").unwrap().size, 123);
        let body = clearsign_body(&inrel).unwrap();
        assert!(body.starts_with("Origin: Debian") && body.contains("\n-dash-escaped") && !body.contains("PGP"));
        assert!(clearsign_body("Origin: x\n").is_none());
    }

    #[test]
    fn url_policy() {
        assert!(check_url("https://deb.debian.org/debian/dists/trixie/InRelease", false).is_ok());
        assert!(check_url("https://cdimage.debian.org/cdimage/firmware/trixie/current/SHA256SUMS", false).is_ok());
        assert!(check_url("http://deb.debian.org/debian/", false).is_err(), "https only");
        assert!(check_url("https://evil.example/debian", false).is_err());
        assert!(check_url("https://deb.debian.org@evil.example/", false).is_err());
        assert!(check_url("https://deb.debian.org:8443/x", false).is_err());
        assert!(check_url("http://127.0.0.1:8080/x", false).is_err());
        assert!(check_url("http://127.0.0.1:8080/x", true).is_ok());
        assert_eq!(join_url("https://deb.debian.org/debian/a/b", "/x/y"), "https://deb.debian.org/x/y");
        assert_eq!(join_url("https://deb.debian.org/debian/a", "b"), "https://deb.debian.org/debian/b");
    }

    fn sample_manifest() -> BundleManifest {
        let mut archs = BTreeMap::new();
        archs.insert(
            "amd64".into(),
            ArchManifest { installer_kernel: Some("6.12.1+deb13-amd64".into()), packages: 3, ..Default::default() },
        );
        BundleManifest {
            format: FORMAT_VERSION,
            tool_version: "0.0.0".into(),
            created: "2026-11-01T00:00:00Z".into(),
            suite: "trixie".into(),
            suites: vec!["trixie".into()],
            kernel: KernelChannel::Stable,
            with_firmware: false,
            extra_packages: vec!["htop".into()],
            release_dates: BTreeMap::new(),
            pgp_verified: false,
            warnings: vec![],
            archs,
            files: vec![FileEntry { path: "hello.txt".into(), size: 2, sha256: debian::sha256_hex(b"hi") }],
        }
    }

    #[test]
    fn manifest_round_trips_and_open_validates() {
        let dir = std::env::temp_dir().join(format!("sf-bundle-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mirror/debian/dists/trixie")).unwrap();
        std::fs::write(dir.join("hello.txt"), "hi").unwrap();
        let inrelease = "Origin: Debian\nCodename: trixie\n";
        std::fs::write(dir.join("mirror/debian/dists/trixie/InRelease"), inrelease).unwrap();
        let mut m = sample_manifest();
        m.files.push(FileEntry {
            path: "mirror/debian/dists/trixie/InRelease".into(),
            size: inrelease.len() as u64,
            sha256: debian::sha256_hex(inrelease.as_bytes()),
        });
        write_manifest(&dir, &m).unwrap();
        let b = Bundle::open(&dir).unwrap();
        assert_eq!(b.manifest, m);
        assert!(b.has_arch(Arch::X86_64) && !b.has_arch(Arch::Arm64));
        assert!(b.installer_files(Arch::Arm64).is_err());
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok(), "{rep:?}");
        assert_eq!(rep.files_checked, 2);
        // tampering is detected
        std::fs::write(dir.join("hello.txt"), "ho").unwrap();
        std::fs::write(dir.join("stray.bin"), "x").unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert_eq!(rep.changed, vec!["hello.txt"]);
        assert_eq!(rep.extra, vec!["stray.bin"]);
        std::fs::remove_file(dir.join("hello.txt")).unwrap();
        assert_eq!(b.verify(&mut |_, _| {}).unwrap().missing, vec!["hello.txt"]);
        let sums = std::fs::read_to_string(dir.join(SUMS_FILE)).unwrap();
        assert!(sums.contains("  hello.txt") && sums.contains("  manifest.json"));
        // an unfinished build is refused
        std::fs::write(dir.join(BUILDING_MARKER), "").unwrap();
        assert!(Bundle::open(&dir).unwrap_err().to_string().contains("unfinished"));
        std::fs::remove_file(dir.join(BUILDING_MARKER)).unwrap();
        // wrong format version
        let mut m2 = m;
        m2.format = 99;
        std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_string(&m2).unwrap()).unwrap();
        assert!(Bundle::open(&dir).unwrap_err().to_string().contains("format 99"));
        assert!(Bundle::open(&dir.join("nope")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A small but structurally complete bundle written without any network: one mirror file, optionally the `mgmt/` folder that
    /// `create_bundle` writes, and a `manifest.json` that lists everything on disk.
    fn tiny_bundle(tag: &str, with_mgmt: bool) -> (PathBuf, Bundle) {
        let dir = std::env::temp_dir().join(format!("sf-tiny-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mirror/debian/dists/trixie")).unwrap();
        std::fs::write(dir.join("mirror/debian/dists/trixie/InRelease"), "Origin: Debian\nCodename: trixie\n").unwrap();
        if with_mgmt {
            write_mgmt(&dir).unwrap();
        }
        let mut m = sample_manifest();
        m.files = index_tree(&dir, &mut |_| {}).unwrap();
        write_manifest(&dir, &m).unwrap();
        let b = Bundle::open(&dir).unwrap();
        (dir, b)
    }

    /// Rewrite `mgmt/<name>` and keep `mgmt/MANIFEST.sha256` consistent with it (what a careless hand edit of a USB copy does),
    /// so only the bundle's own `manifest.json` can notice.
    fn edit_mgmt_consistently(dir: &Path, name: &str, data: &[u8]) {
        std::fs::write(dir.join("mgmt").join(name), data).unwrap();
        let mut b = MgmtBundle::embedded();
        for f in &mut b.files {
            if f.name == name {
                f.data = data.to_vec();
            }
        }
        std::fs::write(dir.join("mgmt").join(MGMT_MANIFEST), b.manifest()).unwrap();
    }

    #[test]
    fn a_bundle_carries_a_verified_management_app() {
        let (dir, b) = tiny_bundle("mgmt", true);
        for f in ["synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service", "MANIFEST.sha256"] {
            assert!(b.manifest.files.iter().any(|e| e.path == format!("mgmt/{f}")), "mgmt/{f} is in manifest.json");
        }
        assert!(b.has_mgmt() && b.mgmt_dir() == Some(dir.join("mgmt")));
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok() && rep.extra.is_empty() && rep.notes.is_empty(), "{rep:?}");
        assert_eq!(rep.mgmt_version.as_deref(), Some(crate::manage::bundled_mgmt_version()));
        // what the bundle gives `manage update` is exactly the copy built into this program
        let m = b.mgmt().unwrap().unwrap();
        assert_eq!(m.files, MgmtBundle::embedded().files);
        assert_eq!(m.version, crate::manage::bundled_mgmt_version());
        // `--from-bundle <bundle>` and `--from-bundle <bundle>/mgmt` both reach it
        for d in [dir.clone(), dir.join("mgmt")] {
            let l = MgmtBundle::from_dir(&d).unwrap();
            assert_eq!((l.files.clone(), l.version.clone()), (m.files.clone(), m.version.clone()), "{}", d.display());
            assert_eq!(l.source, crate::manage::BundleSource::Dir(dir.join("mgmt")));
        }
        // the offline plan: an empty unit gets an install of the bundled copy
        let plan = crate::manage::plan_update(&crate::manage::InstalledInfo::default(), &MgmtBundle::from_dir(&dir).unwrap(), false);
        assert!(plan.decision.applies(), "{plan:?}");
        // info and the install-time check
        let i = b.info();
        assert_eq!(i.mgmt_version.as_deref(), Some(crate::manage::bundled_mgmt_version()));
        assert!(i.render().contains(&format!("Mgmt app:  synofurb-mgmt {}", crate::manage::bundled_mgmt_version())), "{}", i.render());
        assert!(b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &["htop".into()], false).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_without_mgmt_still_verifies_with_a_note() {
        let (dir, b) = tiny_bundle("nomgmt", false);
        assert!(!b.has_mgmt() && b.mgmt_dir().is_none() && b.mgmt().unwrap().is_none());
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok(), "{rep:?}");
        assert!(rep.mgmt_version.is_none() && rep.mgmt_problems.is_empty());
        assert!(rep.notes.len() == 1 && rep.notes[0].contains("no mgmt/") && rep.notes[0].contains("bundle create"), "{:?}", rep.notes);
        assert!(b.info().mgmt_version.is_none() && b.info().render().contains("Mgmt app:  not included"));
        assert!(b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &["htop".into()], false).is_ok());
        // an old manifest.json is read as it always was (nothing in it names mgmt)
        let raw = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        assert!(!raw.contains("mgmt"));
        // `manage update --from-bundle <bundle>` says what to do instead of "not a management plane folder"
        let e = format!("{:#}", MgmtBundle::from_dir(&dir).unwrap_err());
        assert!(e.contains("no mgmt/ folder") && e.contains("bundle create") && e.contains("make-update-bundle"), "{e}");
        // adding the folder by hand without a new manifest.json is not accepted as part of the bundle
        write_mgmt(&dir).unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok() && rep.extra.iter().any(|e| e == "mgmt/VERSION"), "{rep:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn damage_to_the_management_app_is_found_by_verify_install_and_update() {
        // 1. a file changed behind the back of both manifests
        let (dir, b) = tiny_bundle("mgmt-bad1", true);
        std::fs::write(dir.join("mgmt/synofurb_mgmt.py"), "print('evil')\n").unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(!rep.ok() && rep.changed == vec!["mgmt/synofurb_mgmt.py"], "{rep:?}");
        assert!(rep.mgmt_problems.len() == 1 && rep.mgmt_problems[0].contains("does not match"), "{:?}", rep.mgmt_problems);
        assert!(rep.mgmt_version.is_none());
        let e = format!("{:#}", b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &[], false).unwrap_err());
        assert!(e.contains("management-app") && e.contains("bundle verify"), "{e}");
        for d in [dir.clone(), dir.join("mgmt")] {
            assert!(MgmtBundle::from_dir(&d).is_err(), "{}", d.display());
        }
        let _ = std::fs::remove_dir_all(&dir);

        // 2. the script and mgmt/MANIFEST.sha256 edited together: only manifest.json can tell
        let (dir, b) = tiny_bundle("mgmt-bad2", true);
        edit_mgmt_consistently(&dir, "synofurb_mgmt.py", b"print('patched')\n");
        assert!(MgmtBundle::from_plain_dir(&dir.join("mgmt")).is_ok(), "the folder alone looks fine");
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(!rep.ok() && rep.mgmt_problems.iter().any(|p| p.contains("does not match the bundle manifest")), "{rep:?}");
        let e = format!("{:#}", MgmtBundle::from_dir(&dir).unwrap_err());
        assert!(e.contains("does not match the bundle manifest"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);

        // 3. mgmt/MANIFEST.sha256 gone
        let (dir, b) = tiny_bundle("mgmt-bad3", true);
        std::fs::remove_file(dir.join("mgmt/MANIFEST.sha256")).unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(!rep.ok() && rep.missing == vec!["mgmt/MANIFEST.sha256"], "{rep:?}");
        assert!(rep.mgmt_problems.iter().any(|p| p.contains("MANIFEST.sha256 is missing")), "{:?}", rep.mgmt_problems);
        let _ = std::fs::remove_dir_all(&dir);

        // 4. a symbolic link in place of a file is not followed
        #[cfg(unix)]
        {
            let (dir, b) = tiny_bundle("mgmt-bad4", true);
            std::fs::remove_file(dir.join("mgmt/VERSION")).unwrap();
            std::os::unix::fs::symlink("/etc/hostname", dir.join("mgmt/VERSION")).unwrap();
            let rep = b.verify(&mut |_, _| {}).unwrap();
            assert!(!rep.ok(), "{rep:?}");
            assert!(MgmtBundle::from_dir(&dir).is_err());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// `create_bundle` writes what `apps/mgmt-plane/tools/make-update-bundle.py` writes (the documented way to make such a folder).
    #[test]
    fn mgmt_folder_equals_the_output_of_make_update_bundle() {
        let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/mgmt-plane/tools/make-update-bundle.py");
        let out = std::env::temp_dir().join(format!("sf-mub-{}", std::process::id()));
        let ours = std::env::temp_dir().join(format!("sf-mub-ours-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&ours);
        std::fs::create_dir_all(&ours).unwrap();
        let Ok(run) = std::process::Command::new("python3").arg("-I").arg(&tool).arg(&out).output() else {
            eprintln!("python3 is not installed: skipping the comparison with make-update-bundle.py");
            return;
        };
        assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
        write_mgmt(&ours).unwrap();
        let names = |d: &Path| -> BTreeSet<String> {
            std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect()
        };
        assert_eq!(names(&out), names(&ours.join("mgmt")));
        for n in names(&out) {
            let (a, b) = (std::fs::read_to_string(out.join(&n)).unwrap(), std::fs::read_to_string(ours.join("mgmt").join(&n)).unwrap());
            if n == MGMT_MANIFEST {
                // same lines; the order of the lines is not part of the format
                assert_eq!(a.lines().collect::<BTreeSet<_>>(), b.lines().collect::<BTreeSet<_>>());
            } else {
                assert_eq!(a, b, "{n}");
            }
        }
        assert_eq!(MgmtBundle::from_dir(&out).unwrap().files, MgmtBundle::from_dir(&ours.join("mgmt")).unwrap().files);
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&ours);
    }

    #[test]
    fn info_renders_sizes() {
        let mut m = sample_manifest();
        m.archs.get_mut("amd64").unwrap().pool_files = vec!["hello.txt".into()];
        let b = Bundle { root: PathBuf::from("/x"), manifest: m };
        let i = b.info();
        assert_eq!(i.archs[0].bytes, 2);
        let text = i.render();
        assert!(text.contains("amd64") && text.contains("Total:") && text.contains("6.12.1+deb13-amd64"), "{text}");
        assert_eq!(human_bytes(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn check_for_install_rejects_mismatches() {
        let b = Bundle { root: PathBuf::from("/nonexistent"), manifest: sample_manifest() };
        assert!(b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &[], false).unwrap().is_empty());
        assert!(
            b.check_for_install(Arch::Arm64, "trixie", KernelChannel::Stable, &[], false).unwrap_err().to_string().contains("no arm64")
        );
        assert!(b.check_for_install(Arch::X86_64, "bookworm", KernelChannel::Stable, &[], false).is_err());
        let e = b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Newest, &[], false).unwrap_err().to_string();
        // actionable: both ways out are named, with the exact flags
        assert!(
            e.contains("Stable") && e.contains("Newest") && e.contains("--kernel stable") && e.contains("bundle create --kernel newest"),
            "{e}"
        );
        assert!(e.contains("no fallback"), "{e}");
        assert!(
            b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &["vim".into()], false)
                .unwrap_err()
                .to_string()
                .contains("vim")
        );
        assert!(b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &["htop".into()], false).is_ok());
        assert_eq!(
            b.check_for_install(Arch::X86_64, "trixie", KernelChannel::Stable, &[], true).unwrap().len(),
            1,
            "firmware wanted but absent warns"
        );
    }

    /// The package lists in this module must be exactly the ones `payload/chroot-setup.sh` installs.
    #[test]
    fn closure_seeds_match_chroot_setup_script() {
        let script = include_str!("../../../payload/chroot-setup.sh");
        let joined = script.replace("\\\n", " ");
        let mut installs: Vec<Vec<String>> = vec![];
        for line in joined.lines() {
            if let Some(rest) = line.trim().strip_prefix("apt_install ") {
                let words: Vec<String> = rest.split_whitespace().map(String::from).collect();
                if words != ["\"$KERNEL\""] {
                    installs.push(words); // the kernel is installed by name via $KERNEL: see chroot_phases
                }
            }
        }
        let literal = |v: &[String]| -> Vec<String> { v.iter().filter(|w| !w.starts_with('$')).cloned().collect() };
        let find = |first: &str| -> Vec<String> {
            literal(
                installs
                    .iter()
                    .find(|v| v.first().is_some_and(|w| w == first))
                    .unwrap_or_else(|| panic!("no apt_install starting with {first}")),
            )
        };
        assert_eq!(find("debian-archive-keyring"), CHROOT_KEYRING);
        assert_eq!(find("debconf"), CHROOT_LOCALES);
        assert_eq!(find("initramfs-tools"), CHROOT_INITRAMFS);
        let bulk = literal(installs.iter().find(|v| v.iter().any(|w| w == "$BOOTPKG")).expect("the big apt_install"));
        assert_eq!(bulk, CHROOT_TOOLS, "CHROOT_TOOLS drifted from the apt_install of chroot-setup.sh");
        assert_eq!(installs.len(), 4, "chroot-setup.sh has an apt_install this module does not know: {installs:?}");
        // kernel packages per architecture
        for (arch, want) in [(Arch::X86_64, "amd64"), (Arch::Arm64, "arm64"), (Arch::Armhf, "armmp")] {
            assert!(script.contains(&format!("KERNEL=linux-image-{want}")), "{want}");
            assert_eq!(arch.kernel_package(), format!("linux-image-{want}"));
        }
        // boot packages: every BOOTPKG="..." value of the script is covered, and nothing else is claimed
        let mut boot: BTreeSet<String> = BTreeSet::new();
        for line in script.lines() {
            if let Some(i) = line.find("BOOTPKG=\"") {
                let v = &line[i + 9..];
                boot.extend(v.split('"').next().unwrap().split_whitespace().map(String::from));
            }
        }
        let ours: BTreeSet<String> =
            [Arch::X86_64, Arch::Arm64, Arch::Armhf].into_iter().flat_map(boot_packages).map(String::from).collect();
        assert_eq!(boot, ours, "boot packages drifted from chroot-setup.sh");
        assert!(script.contains("EXTRA=\"haveged\""));
        assert!(chroot_phases(Arch::Armhf, KernelChannel::Stable, &[]).iter().any(|p| p.1.contains(&"haveged".to_string())));
        // microcode comes from CpuVendor::microcode_package
        use crate::model::CpuVendor;
        let mc: BTreeSet<&str> = [CpuVendor::Intel, CpuVendor::Amd].into_iter().filter_map(|v| v.microcode_package()).collect();
        assert_eq!(mc, microcode_packages(Arch::X86_64).into_iter().collect::<BTreeSet<_>>());
        // debootstrap --include of the generated stage-2 script
        assert!(include_str!("script.rs").contains(&format!("--include={}", DEBOOTSTRAP_INCLUDE.join(","))));
    }

    #[test]
    fn chroot_phases_use_backports_only_for_the_kernel() {
        let ph = chroot_phases(Arch::X86_64, KernelChannel::Newest, &["vim".into()]);
        let k = ph.iter().find(|p| p.0 == "chroot: kernel").unwrap();
        assert_eq!((k.1.clone(), k.2), (vec!["linux-image-amd64".to_string()], Policy::BackportsFirst));
        assert!(ph.iter().filter(|p| p.0 != "chroot: kernel").all(|p| p.2 == Policy::Stable));
        let tools = ph.iter().find(|p| p.0 == "chroot: tools").unwrap();
        assert!(
            tools.1.contains(&"vim".to_string())
                && tools.1.contains(&"grub-pc".to_string())
                && tools.1.contains(&"intel-microcode".to_string())
        );
        let ph = chroot_phases(Arch::X86_64, KernelChannel::Stable, &[]);
        assert_eq!(ph.iter().find(|p| p.0 == "chroot: kernel").unwrap().2, Policy::Stable);
    }

    #[test]
    fn reads_the_kernel_version_from_an_initrd_cpio() {
        let gz = debian::cpio_gz(&[("bin/sh", b"x", 0o100755), ("usr/lib/modules/6.12.107+deb13-amd64/modules.dep", b"hello", 0o100644)]);
        let f = std::env::temp_dir().join(format!("sf-initrd-{}.gz", std::process::id()));
        std::fs::write(&f, gz).unwrap();
        assert_eq!(initrd_kernel_version(&f).unwrap().as_deref(), Some("6.12.107+deb13-amd64"));
        std::fs::write(&f, debian::cpio_gz(&[("bin/sh", b"x", 0o100755)])).unwrap();
        assert_eq!(initrd_kernel_version(&f).unwrap(), None);
        let _ = std::fs::remove_file(&f);
    }
    // ---- create_bundle against a fake archive served by the real mirror route --------------------------------------

    struct FakeArchive {
        dir: PathBuf,
        server: Option<crate::server::Server>,
        events: std::sync::mpsc::Receiver<crate::logs::Event>,
        base: String,
    }

    impl Drop for FakeArchive {
        fn drop(&mut self) {
            if let Some(s) = self.server.take() {
                s.stop();
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn stanza(name: &str, ver: &str, deps: &str, prio: &str, extra: &str, file: &str, content: &[u8]) -> String {
        format!(
            "Package: {name}\nVersion: {ver}\nArchitecture: amd64\nPriority: {prio}\n{}{extra}Filename: {file}\nSize: {}\nSHA256: {}\n\n",
            if deps.is_empty() { String::new() } else { format!("Depends: {deps}\n") },
            content.len(),
            debian::sha256_hex(content)
        )
    }

    /// A tiny but structurally complete archive: one suite + backports, every seed package, udebs, installer.
    fn fake_archive(name: &str, break_dependency: bool) -> FakeArchive {
        let dir = std::env::temp_dir().join(format!("sf-fakearch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("mirror/debian");
        let write = |rel: &str, data: &[u8]| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, data).unwrap();
        };
        // package universe from the real seed lists
        let mut names: Vec<String> = vec!["apt".into(), "usr-is-merged".into(), "base-files".into(), "libc6".into()];
        for l in [CHROOT_KEYRING, CHROOT_LOCALES, CHROOT_INITRAMFS, CHROOT_TOOLS, DEBOOTSTRAP_INCLUDE] {
            names.extend(l.iter().map(|s| s.to_string()));
        }
        names.extend(boot_packages(Arch::X86_64).iter().map(|s| s.to_string()));
        names.extend(microcode_packages(Arch::X86_64).iter().map(|s| s.to_string()));
        names.extend(["htop-extra".to_string(), "linux-base".to_string()]);
        names.sort();
        names.dedup();
        let mut main = String::new();
        for n in &names {
            let content = format!("deb:{n}").into_bytes();
            let file = format!("pool/main/{}/{n}/{n}_1_amd64.deb", &n[..1]);
            write(&file, &content);
            let (prio, deps) = match n.as_str() {
                "base-files" | "libc6" => ("required", ""),
                "apt" => ("important", "libc6"),
                _ if break_dependency && n == "lvm2" => ("optional", "libc6, a-package-nobody-has"),
                _ => ("optional", "libc6"),
            };
            main.push_str(&stanza(n, "1", deps, prio, "", &file, &content));
        }
        let idx = |rel: &str, text: &str| -> (String, usize) {
            let data = gz(text.as_bytes());
            write(rel, &data);
            (debian::sha256_hex(&data), data.len())
        };
        let mut lines = vec![];
        let add_idx = |suite_root: &str, rel: &str, text: &str, lines: &mut Vec<String>| {
            let (h, n) = idx(&format!("{suite_root}/{rel}.gz"), text);
            lines.push(format!(" {h} {n} {rel}.gz"));
            // the .xz is never parsed: any bytes will do, but it must be listed and hash right
            let xz = format!("xz-bytes-of-{rel}").into_bytes();
            write(&format!("{suite_root}/{rel}.xz"), &xz);
            lines.push(format!(" {} {} {rel}.xz", debian::sha256_hex(&xz), xz.len()));
        };
        add_idx("dists/trixie", "main/binary-amd64/Packages", &main, &mut lines);
        add_idx("dists/trixie", "main/binary-all/Packages", "", &mut lines);
        add_idx("dists/trixie", "contrib/binary-amd64/Packages", "", &mut lines);
        add_idx("dists/trixie", "non-free-firmware/binary-amd64/Packages", "", &mut lines);
        // udebs: virtual *-modules for the installer kernel (6.1.0-test-amd64) and a stale ABI
        let mut ud = String::new();
        for n in crate::script::ANNA_PACKAGES {
            let content = format!("udeb:{n}").into_bytes();
            if let Some(kind) = n.strip_suffix("-modules") {
                for abi in ["6.1.0-test-amd64", "6.0.0-old-amd64"] {
                    let real = format!("{kind}-modules-{abi}-di");
                    let file = format!("pool/main/l/linux/{real}_1_amd64.udeb");
                    let body = format!("udeb:{real}").into_bytes();
                    write(&file, &body);
                    ud.push_str(&stanza(&real, "1", "", "optional", &format!("Provides: {n}\nKernel-Version: {abi}\n"), &file, &body));
                }
            } else {
                let file = format!("pool/main/{}/{n}/{n}_1_amd64.udeb", &n[..1]);
                write(&file, &content);
                ud.push_str(&stanza(n, "1", "", "standard", "", &file, &content));
            }
        }
        add_idx("dists/trixie", "main/debian-installer/binary-amd64/Packages", &ud, &mut lines);
        // installer images
        let initrd = debian::cpio_gz(&[("usr/lib/modules/6.1.0-test-amd64/modules.dep", b"x", 0o100644)]);
        let (kernel, base) = (b"KERNEL".to_vec(), "dists/trixie/main/installer-amd64/current/images");
        write(&format!("{base}/netboot/debian-installer/amd64/linux"), &kernel);
        write(&format!("{base}/netboot/debian-installer/amd64/initrd.gz"), &initrd);
        let sums = format!(
            "{}  ./netboot/debian-installer/amd64/linux\n{}  ./netboot/debian-installer/amd64/initrd.gz\n",
            debian::sha256_hex(&kernel),
            debian::sha256_hex(&initrd)
        );
        write(&format!("{base}/SHA256SUMS"), sums.as_bytes());
        lines.push(format!(" {} {} main/installer-amd64/current/images/SHA256SUMS", debian::sha256_hex(sums.as_bytes()), sums.len()));
        let inrelease = |codename: &str, lines: &[String]| {
            format!(
                "Origin: Debian\nCodename: {codename}\nDate: Sat, 12 Sep 2026 07:55:41 UTC\nValid-Until: Sat, 19 Sep 2026 07:55:41 UTC\nSHA256:\n{}\n",
                lines.join("\n")
            )
        };
        write("dists/trixie/InRelease", inrelease("trixie", &lines).as_bytes());
        // backports: a newer kernel
        let kimg = b"deb:linux-image-7";
        write("pool/main/l/linux/linux-image-7.0-amd64_7.0-1~bpo13+1_amd64.deb", kimg);
        let mut bp = stanza(
            "linux-image-amd64",
            "7.0-1~bpo13+1",
            "linux-image-7.0-amd64 (= 7.0-1~bpo13+1)",
            "optional",
            "",
            "pool/main/l/linux/linux-image-amd64_7.0-1~bpo13+1_amd64.deb",
            b"deb:meta",
        );
        write("pool/main/l/linux/linux-image-amd64_7.0-1~bpo13+1_amd64.deb", b"deb:meta");
        bp.push_str(&stanza(
            "linux-image-7.0-amd64",
            "7.0-1~bpo13+1",
            "linux-base (>= 98)",
            "optional",
            "",
            "pool/main/l/linux/linux-image-7.0-amd64_7.0-1~bpo13+1_amd64.deb",
            kimg,
        ));
        bp.push_str(&stanza(
            "linux-base",
            "99~bpo13+1",
            "",
            "optional",
            "",
            "pool/main/l/linux-base/linux-base_99~bpo13+1_all.deb",
            b"deb:lb",
        ));
        write("pool/main/l/linux-base/linux-base_99~bpo13+1_all.deb", b"deb:lb");
        let mut bl = vec![];
        // like the real trixie-backports: no .gz, only .xz (real xz data here, it has to be parsed)
        for (rel, text) in [("main/binary-amd64/Packages", bp.as_str()), ("main/binary-all/Packages", "")] {
            let mut xz = vec![];
            lzma_rs::xz_compress(&mut text.as_bytes(), &mut xz).unwrap();
            write(&format!("dists/trixie-backports/{rel}.xz"), &xz);
            bl.push(format!(" {} {} {rel}.xz", debian::sha256_hex(&xz), xz.len()));
        }
        write("dists/trixie-backports/InRelease", inrelease("trixie-backports", &bl).as_bytes());
        // firmware
        let fw = b"FIRMWARE-CPIO".to_vec();
        write("firmware/trixie/current/firmware.cpio.gz", &fw);
        write("firmware/trixie/current/SHA256SUMS", format!("{}  firmware.cpio.gz\n", debian::sha256_hex(&fw)).as_bytes());

        let (tx, events) = std::sync::mpsc::channel();
        let served = crate::server::Served {
            files: Default::default(),
            token: "up".into(),
            stage2: String::new(),
            payload: vec![],
            preseed: String::new(),
            mirror: Some(dir.join("mirror")),
        };
        let server = crate::server::Server::start("127.0.0.1".parse().unwrap(), served, Logger::memory(Some(tx)), (0, 0, 0)).unwrap();
        let base = format!("http://127.0.0.1:{}/up/mirror", server.http_port);
        FakeArchive { dir, server: Some(server), events, base }
    }

    fn fake_spec() -> BundleSpec {
        BundleSpec {
            suite: "trixie".into(),
            archs: vec![Arch::X86_64],
            kernel: KernelChannel::Newest,
            with_firmware: true,
            extra_packages: vec!["htop-extra".into()],
            include_agent: false,
            dtbs: vec![],
        }
    }

    fn fake_source(a: &FakeArchive) -> Source {
        Source {
            debian_base: format!("{}/debian", a.base),
            firmware_base: format!("{}/debian/firmware", a.base),
            allow_loopback_http: true,
        }
    }

    fn upstream_requests(a: &FakeArchive) -> Vec<String> {
        a.events
            .try_iter()
            .filter_map(|e| match e {
                crate::logs::Event::Log(l) if l.msg.contains(" GET mirror/") => Some(l.msg),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn create_bundle_end_to_end_against_a_fake_archive() {
        let a = fake_archive("e2e", false);
        let out = std::env::temp_dir().join(format!("sf-fakebundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let log = Logger::memory(None);
        let spec = fake_spec();
        let m = create_bundle_from(&fake_source(&a), &spec, &out, &log, &mut |_, _| {}).unwrap();
        let first = upstream_requests(&a);
        assert!(first.iter().any(|r| r.contains("linux-image-7.0-amd64")), "kernel from backports downloaded");
        assert!(!first.iter().any(|r| r.contains("6.0.0-old")), "module udebs of the other kernel ABI are not fetched");
        assert!(first.iter().any(|r| r.contains("md-modules-6.1.0-test-amd64-di")), "virtual md-modules resolved for the installer kernel");
        let am = &m.archs["amd64"];
        assert_eq!(am.installer_kernel.as_deref(), Some("6.1.0-test-amd64"));
        assert_eq!(am.kernel_package_version.as_deref(), Some("7.0-1~bpo13+1"));
        assert!(am.kernel_from_backports);
        assert!(m.files.iter().any(|f| f.path == "installer/firmware.cpio.gz"));
        assert!(m.files.iter().any(|f| f.path == "mirror/debian/dists/trixie-backports/InRelease"));
        assert!(
            m.files.iter().any(|f| f.path == "installer/amd64/InRelease") && m.files.iter().any(|f| f.path == "installer/amd64/SHA256SUMS")
        );
        assert!(!out.join(BUILDING_MARKER).exists());
        // the bundle opens and verifies without any network (the fake archive keeps running but is not used)
        let b = Bundle::open(&out).unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok() && rep.extra.is_empty(), "{rep:?}");
        assert!(rep.notes.is_empty() && rep.mgmt_version.as_deref() == Some(crate::manage::bundled_mgmt_version()), "{rep:?}");
        assert!(m.files.iter().any(|f| f.path == "mgmt/synofurb_mgmt.py") && m.files.iter().any(|f| f.path == "mgmt/MANIFEST.sha256"));
        assert_eq!(b.mgmt().unwrap().unwrap().files, MgmtBundle::embedded().files);
        b.verify_installer(Arch::X86_64).unwrap();
        let inst = b.installer_files(Arch::X86_64).unwrap();
        assert!(inst.firmware.is_some() && inst.kernel.is_file());
        // the same bundle, served by the real route, hands out exactly what the manifest lists
        let (tx, _rx) = std::sync::mpsc::channel();
        let served = crate::server::Served {
            files: Default::default(),
            token: "tt".into(),
            stage2: String::new(),
            payload: vec![],
            preseed: String::new(),
            mirror: Some(b.mirror_dir()),
        };
        let srv = crate::server::Server::start("127.0.0.1".parse().unwrap(), served, Logger::memory(Some(tx)), (0, 0, 0)).unwrap();
        let fetch = |path: &str| -> Option<Vec<u8>> {
            let mut s = std::net::TcpStream::connect(("127.0.0.1", srv.http_port)).unwrap();
            write!(s, "GET /tt/mirror/{path} HTTP/1.0\r\n\r\n").unwrap();
            let mut raw = vec![];
            s.read_to_end(&mut raw).unwrap();
            let i = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            String::from_utf8_lossy(&raw[..12]).contains("200").then(|| raw[i..].to_vec())
        };
        for f in m.files.iter().filter(|f| f.path.starts_with("mirror/")) {
            let got = fetch(f.path.strip_prefix("mirror/").unwrap()).unwrap_or_else(|| panic!("{} not served", f.path));
            assert_eq!(debian::sha256_hex(&got), f.sha256, "{}", f.path);
        }
        assert!(fetch("debian/pool/main/x/ghost/ghost_1_amd64.deb").is_none(), "a package outside the closure 404s");
        srv.stop();

        // second run: nothing is downloaded again (resume = skip what verifies)
        let _ = upstream_requests(&a); // drain
        let a2 = fake_archive("e2e", false); // fresh server on the same files
        let m2 = create_bundle_from(&fake_source(&a2), &spec, &out, &log, &mut |_, _| {}).unwrap();
        let again = upstream_requests(&a2);
        assert!(!again.iter().any(|r| r.contains("/pool/")), "pool files already on disk are not fetched again: {again:?}");
        assert_eq!(m2.files.len(), m.files.len());
        // a damaged file is repaired on the next run, a stale pool file is pruned
        let victim = out.join("mirror/debian/pool/main/h/htop/htop_1_amd64.deb");
        std::fs::create_dir_all(out.join("mirror/debian/pool/main/z")).unwrap();
        std::fs::write(out.join("mirror/debian/pool/main/z/stale_1_amd64.deb"), b"old").unwrap();
        assert!(victim.exists());
        std::fs::write(&victim, b"corrupt").unwrap();
        let _ = upstream_requests(&a2);
        // a partial download resumes with a Range request
        let part = part_path(&out.join("mirror/debian/pool/main/s/sudo/sudo_1_amd64.deb"));
        std::fs::remove_file(out.join("mirror/debian/pool/main/s/sudo/sudo_1_amd64.deb")).unwrap();
        std::fs::write(&part, b"deb:").unwrap();
        let m3 = create_bundle_from(&fake_source(&a2), &spec, &out, &log, &mut |_, _| {}).unwrap();
        let third = upstream_requests(&a2);
        assert!(third.iter().any(|r| r.contains("/htop_1_amd64.deb")), "the corrupted file is downloaded again: {third:?}");
        assert!(
            third.iter().any(|r| r.contains("sudo_1_amd64.deb") && r.contains("partial")),
            "the .part resumed with a Range request: {third:?}"
        );
        assert!(!out.join("mirror/debian/pool/main/z/stale_1_amd64.deb").exists(), "stale pool files are pruned");
        assert!(Bundle::open(&out).unwrap().verify(&mut |_, _| {}).unwrap().ok());
        assert_eq!(m3.files.len(), m.files.len());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn create_bundle_reports_an_incomplete_closure_with_the_chain_and_leaves_no_manifest() {
        let a = fake_archive("broken", true);
        let out = std::env::temp_dir().join(format!("sf-fakebundle-broken-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let e = create_bundle_from(&fake_source(&a), &fake_spec(), &out, &Logger::memory(None), &mut |_, _| {}).unwrap_err();
        let text = format!("{e:#}");
        assert!(text.contains("a-package-nobody-has") && text.contains("lvm2 1 <-"), "{text}");
        assert!(!out.join(MANIFEST_FILE).exists());
        assert!(Bundle::open(&out).unwrap_err().to_string().contains("unfinished"), "an interrupted build is not usable");
        // a directory that is not a bundle is never written into
        let other = std::env::temp_dir().join(format!("sf-notabundle-{}", std::process::id()));
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("precious.txt"), "keep").unwrap();
        let e = create_bundle_from(&fake_source(&a), &fake_spec(), &other, &Logger::memory(None), &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("not a synofurb bundle"));
        assert!(other.join("precious.txt").exists());
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn a_tampered_package_is_rejected() {
        let a = fake_archive("tamper", false);
        // the archive serves different bytes than its signed index lists
        std::fs::write(a.dir.join("mirror/debian/pool/main/c/curl/curl_1_amd64.deb"), b"evil").unwrap();
        let out = std::env::temp_dir().join(format!("sf-fakebundle-tamper-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let e = create_bundle_from(&fake_source(&a), &fake_spec(), &out, &Logger::memory(None), &mut |_, _| {}).unwrap_err();
        assert!(format!("{e:#}").contains("checksum mismatch"), "{e:#}");
        assert!(
            !out.join("mirror/debian/pool/main/c/curl/curl_1_amd64.deb").exists(),
            "a file that fails verification is never left in place"
        );
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn create_bundle_rejects_bad_specs() {
        let a = fake_archive("spec", false);
        let log = Logger::memory(None);
        let out = std::env::temp_dir().join(format!("sf-fakebundle-spec-{}", std::process::id()));
        for bad in [
            BundleSpec { archs: vec![], ..fake_spec() },
            BundleSpec { archs: vec![Arch::Unsupported], ..fake_spec() },
            BundleSpec { suite: "tri xie".into(), ..fake_spec() },
            BundleSpec { extra_packages: vec!["Bad;pkg".into()], ..fake_spec() },
            BundleSpec { dtbs: vec!["../x.dtb".into()], ..fake_spec() },
        ] {
            assert!(create_bundle_from(&fake_source(&a), &bad, &out, &log, &mut |_, _| {}).is_err());
        }
        assert!(!out.exists(), "validation happens before anything is written");
    }
    #[test]
    fn stage_offline_points_everything_at_the_served_bundle() {
        use crate::config::InstallConfig;
        use crate::inspect::{fixtures, parse_inspect};
        use crate::plan::{PlanRequest, build_plan};
        use crate::remote::Auth;
        use crate::session::{BootMethod, InstallJob, SshTarget, stage};
        let a = fake_archive("stage", false);
        let tmp = std::env::temp_dir().join(format!("sf-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let bundle = tmp.join("bundle");
        let log = Logger::memory(None);
        create_bundle_from(&fake_source(&a), &fake_spec(), &bundle, &log, &mut |_, _| {}).unwrap();
        let facts = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&facts, &PlanRequest::default());
        let mut cfg = InstallConfig::new("nas", "nasadmin", format!("$6$saltsaltsaltsalt${}", "x".repeat(86)));
        cfg.extra_packages = vec!["htop-extra".into()];
        let job = InstallJob {
            target: SshTarget {
                host: "192.0.2.1".into(),
                port: 22,
                user: "root".into(),
                auth: Auth::Password("x".into()),
                sudo_password: None,
            },
            facts,
            plan,
            cfg,
            method: BootMethod::Kit { dir: tmp.join("kit") },
            cache_dir: tmp.join("cache"),
            with_firmware: true,
            verbose_installer: false,
            display_password: "pw".into(),
            offline_bundle: Some(bundle.clone()),
        };
        let staged = stage(&job, "127.0.0.1".parse().unwrap(), "KV|x|y\nEND|ok\n", &log, &mut |_, _| {}).unwrap();
        let base = staged.http_base.clone();
        let token = base.rsplit('/').next().unwrap().to_string();
        let hostport = base.trim_start_matches("http://").split('/').next().unwrap().to_string();
        let mirror = format!("http://{hostport}/{token}/mirror/debian");
        // preseed: the installer's own mirror is the bundle
        assert!(staged.preseed.contains(&format!("d-i mirror/http/hostname string {hostport}")), "{}", staged.preseed);
        assert!(staged.preseed.contains(&format!("d-i mirror/http/directory string /{token}/mirror/debian")));
        assert!(!staged.preseed.contains("deb.debian.org"));
        // stage-2 script: preflight + debootstrap use it
        let stage2 = String::from_utf8(http_get(&format!("{base}/stage2.sh")).unwrap()).unwrap();
        assert!(stage2.contains(&format!("'{mirror}/dists/trixie/Release'")), "preflight wget");
        assert!(stage2.contains(&format!("trixie /target '{mirror}'")), "debootstrap mirror");
        assert!(!stage2.contains("deb.debian.org"));
        // payload: env.sh carries OFFLINE and the mirror
        let payload = http_get(&format!("{base}/payload.tar.gz")).unwrap();
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&payload[..]));
        let mut env = String::new();
        for e in ar.entries().unwrap() {
            let mut e = e.unwrap();
            if e.path().unwrap().to_string_lossy() == "env.sh" {
                e.read_to_string(&mut env).unwrap();
            }
        }
        assert!(env.contains("OFFLINE='1'") && env.contains("OFFLINE_SUITES='trixie trixie-backports'"), "{env}");
        assert!(
            env.contains(&format!("MIRROR_HOST='{hostport}'")) && env.contains(&format!("MIRROR_DIR='/{token}/mirror/debian'")),
            "{env}"
        );
        // the mirror is reachable under the token only, and is byte-identical to the bundle
        let want = std::fs::read(bundle.join("mirror/debian/dists/trixie/InRelease")).unwrap();
        assert_eq!(http_get(&format!("{mirror}/dists/trixie/InRelease")).unwrap(), want);
        assert_eq!(http_get(&format!("http://{hostport}/other/mirror/debian/dists/trixie/InRelease")).unwrap_err(), 404);
        // installer files come from the bundle (firmware appended to the initrd)
        assert_eq!(staged.files.kernel, bundle.join("installer/amd64/kernel"));
        assert!(staged.files.firmware.is_some() && staged.initrd.is_file());
        let kernel = http_get(&format!("{base}/files/installer-kernel")).unwrap();
        assert_eq!(kernel, b"KERNEL");
        staged.server.stop();

        // mismatches are refused before anything is served
        let mut other = job_clone(&job);
        other.cfg.kernel = KernelChannel::Stable;
        assert!(stage(&other, "127.0.0.1".parse().unwrap(), "x", &log, &mut |_, _| {}).err().unwrap().to_string().contains("kernel"));
        let mut other = job_clone(&job);
        other.cfg.extra_packages = vec!["vim".into()];
        assert!(stage(&other, "127.0.0.1".parse().unwrap(), "x", &log, &mut |_, _| {}).err().unwrap().to_string().contains("vim"));
        let mut other = job_clone(&job);
        other.offline_bundle = Some(tmp.join("nope"));
        assert!(stage(&other, "127.0.0.1".parse().unwrap(), "x", &log, &mut |_, _| {}).is_err());
        // a damaged installer file in the bundle is detected before it is served
        std::fs::write(bundle.join("installer/amd64/kernel"), b"EVIL").unwrap();
        let e = stage(&job, "127.0.0.1".parse().unwrap(), "x", &log, &mut |_, _| {}).err().unwrap();
        assert!(e.to_string().contains("does not match the bundle manifest"), "{e}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn job_clone(j: &crate::session::InstallJob) -> crate::session::InstallJob {
        use crate::session::{BootMethod, InstallJob};
        InstallJob {
            target: j.target.clone(),
            facts: j.facts.clone(),
            plan: j.plan.clone(),
            cfg: j.cfg.clone(),
            method: BootMethod::Kit { dir: PathBuf::from("/nonexistent") },
            cache_dir: j.cache_dir.clone(),
            with_firmware: j.with_firmware,
            verbose_installer: j.verbose_installer,
            display_password: j.display_password.clone(),
            offline_bundle: j.offline_bundle.clone(),
        }
    }

    // ---- live tests (network, root, debootstrap): run with `tests/offline/run.sh` or `cargo test -- --ignored` ------

    fn live_dir() -> PathBuf {
        PathBuf::from(std::env::var("SYNOFURB_OFFLINE_DIR").expect("set SYNOFURB_OFFLINE_DIR to an empty directory for the bundle"))
    }

    fn live_archs() -> Vec<Arch> {
        std::env::var("SYNOFURB_OFFLINE_ARCHS")
            .unwrap_or_else(|_| "amd64".into())
            .split(',')
            .map(|a| match a.trim() {
                "amd64" => Arch::X86_64,
                "arm64" => Arch::Arm64,
                "armhf" => Arch::Armhf,
                o => panic!("unknown arch {o}"),
            })
            .collect()
    }

    /// Builds a real bundle from deb.debian.org. `SYNOFURB_OFFLINE_FIRMWARE=1` adds the ~470 MiB firmware cpio.
    #[test]
    #[ignore = "needs Internet; downloads hundreds of MiB"]
    fn live_create_bundle() {
        let spec = BundleSpec {
            suite: "trixie".into(),
            archs: live_archs(),
            kernel: if std::env::var("SYNOFURB_OFFLINE_KERNEL").as_deref() == Ok("stable") {
                KernelChannel::Stable
            } else {
                KernelChannel::Newest
            },
            with_firmware: std::env::var("SYNOFURB_OFFLINE_FIRMWARE").as_deref() == Ok("1"),
            extra_packages: vec![],
            include_agent: false,
            dtbs: vec![],
        };
        let log = Logger::memory(None);
        let (tx, rx) = std::sync::mpsc::channel();
        log.add_sender(tx);
        let printer = std::thread::spawn(move || {
            for e in rx {
                if let crate::logs::Event::Log(l) = e {
                    eprintln!("{} {}", l.ts, l.msg);
                }
            }
        });
        let mut last = String::new();
        let m = create_bundle(&spec, &live_dir(), &log, &mut |l, f| {
            let line = format!("{l} {:.0}%", f * 100.0);
            if line != last && (f == 0.0 || f >= 0.999) {
                eprintln!("[progress] {line}");
            }
            last = line;
        })
        .unwrap();
        drop(log);
        let _ = printer.join();
        let b = Bundle::open(&live_dir()).unwrap();
        eprintln!("{}", b.info().render());
        assert_eq!(m.files.len(), b.manifest.files.len());
    }

    /// Serves the live bundle and returns the `http://127.0.0.1:PORT/<token>/mirror/debian` URL.
    fn live_serve(b: &Bundle) -> (crate::server::Server, String) {
        let token = crate::config::random_token(8);
        let served = crate::server::Served {
            files: Default::default(),
            token: token.clone(),
            stage2: String::new(),
            payload: vec![],
            preseed: String::new(),
            mirror: Some(b.mirror_dir()),
        };
        let srv = crate::server::Server::start("127.0.0.1".parse().unwrap(), served, Logger::memory(None), (0, 0, 0)).unwrap();
        let url = format!("http://127.0.0.1:{}/{token}/mirror/debian", srv.http_port);
        (srv, url)
    }

    fn http_get(url: &str) -> std::result::Result<Vec<u8>, u16> {
        let agent = ureq::AgentBuilder::new().build();
        match agent.get(url).call() {
            Ok(r) => {
                let mut v = vec![];
                r.into_reader().read_to_end(&mut v).unwrap();
                Ok(v)
            }
            Err(ureq::Error::Status(c, _)) => Err(c),
            Err(e) => panic!("{url}: {e}"),
        }
    }

    /// Simulates the installer's `anna`: every udeb the stage-2 script loads (and everything `anna` loads by itself) must
    /// be reachable over HTTP with the right hash, including all of its dependencies. The index used is the one served.
    #[test]
    #[ignore = "needs a bundle: run live_create_bundle first (SYNOFURB_OFFLINE_DIR)"]
    fn live_udeb_closure_is_served() {
        let b = Bundle::open(&live_dir()).unwrap();
        let (srv, url) = live_serve(&b);
        let inrelease = String::from_utf8(http_get(&format!("{url}/dists/{}/InRelease", b.manifest.suite)).unwrap()).unwrap();
        let rel = parse_release(&inrelease);
        for (arch, am) in &b.manifest.archs {
            let mut u = Universe::new(arch);
            for comp in COMPONENTS {
                let path = format!("{comp}/debian-installer/binary-{arch}/Packages.gz");
                let Some(e) = rel.entry(&path) else { continue };
                let data = http_get(&format!("{url}/dists/{}/{path}", b.manifest.suite)).unwrap_or_else(|c| panic!("{path}: HTTP {c}"));
                assert_eq!(debian::sha256_hex(&data), e.sha256, "{path} must match the signed InRelease");
                let pkgs = parse_packages_reader(std::io::BufReader::new(flate2::read::MultiGzDecoder::new(&data[..]))).unwrap();
                u.add_repo(&b.manifest.suite, filter_udebs_for_kernel(pkgs, am.installer_kernel.as_deref()));
            }
            // 1. what the stage-2 script loads, one by one
            let mut total = 0usize;
            for name in crate::script::ANNA_PACKAGES {
                let mut r = Resolver::new(&u);
                if !r.exists(name) {
                    eprintln!("{arch}: {name} does not exist in the archive (anna-install ignores it)");
                    continue;
                }
                r.install(name, Policy::Stable, "anna-install");
                let set = r.finish().unwrap_or_else(|e| panic!("{arch} {name}: {e:#}"));
                assert!(!set.is_empty(), "{arch}: {name} resolved to nothing");
                for p in &set {
                    let data = http_get(&format!("{url}/{}", p.filename))
                        .unwrap_or_else(|c| panic!("{arch}: {name} needs {} which is not served (HTTP {c})", p.filename));
                    assert_eq!(debian::sha256_hex(&data), p.sha256, "{}", p.filename);
                    assert_eq!(data.len() as u64, p.size);
                    total += 1;
                }
                if name.ends_with("-modules") {
                    assert!(
                        set.iter().any(|p| p.kernel_version.as_deref() == am.installer_kernel.as_deref() && am.installer_kernel.is_some()),
                        "{arch}: {name} must resolve to a module udeb of the installer kernel {:?}",
                        am.installer_kernel
                    );
                }
            }
            // 2. everything anna loads on its own (standard priority and up) + all module udebs of the installer kernel
            let (all, unknown, skipped) = resolve_udebs(&u).unwrap_or_else(|e| panic!("{arch}: {e:#}"));
            eprintln!("{arch}: ANNA_PACKAGES entries the archive does not have: {unknown:?}; udebs not installable here: {skipped:?}");
            let in_set: HashSet<&str> = all.iter().map(|p| p.name.as_str()).collect();
            for p in &all {
                // independent dependency check: every Depends group has an alternative inside the set (or a provider)
                for field in [&p.pre_depends, &p.depends] {
                    for group in parse_depends(field) {
                        let ok = group
                            .iter()
                            .any(|a| in_set.contains(a.name.as_str()) || all.iter().any(|q| q.provides.iter().any(|(n, _)| n == &a.name)));
                        assert!(ok, "{arch}: {} depends on {group:?} which is not in the udeb set", p.name);
                    }
                }
                let data =
                    http_get(&format!("{url}/{}", p.filename)).unwrap_or_else(|c| panic!("{arch}: {} not served (HTTP {c})", p.filename));
                assert_eq!(debian::sha256_hex(&data), p.sha256, "{}", p.filename);
            }
            // base-installer (priority required) pulls debootstrap-udeb: the stage-2 script needs the `debootstrap` tool
            for must in ["base-installer", "debootstrap-udeb", "partman-base", "anna", "hw-detect", "apt-setup-udeb"] {
                assert!(in_set.contains(must), "{arch}: udeb {must} missing from the bundle");
            }
            eprintln!(
                "{arch}: {total} files for the ANNA_PACKAGES closure verified over HTTP; {} udebs in the full installer set, all served",
                all.len()
            );
        }
        srv.stop();
    }

    /// Runs the REAL tools (debootstrap, apt-get) against the served bundle: `tests/offline/verify-mirror.sh`.
    /// `SYNOFURB_OFFLINE_WORK` is the scratch directory (chroots), `SYNOFURB_OFFLINE_MODES` e.g. `amd64=chroot,arm64=foreign`.
    #[test]
    #[ignore = "needs a bundle, root, debootstrap, apt and network for the keyring (run tests/offline/run.sh)"]
    fn live_real_tools_against_the_bundle() {
        let b = Bundle::open(&live_dir()).unwrap();
        let rep = b.verify(&mut |_, _| {}).unwrap();
        assert!(rep.ok(), "{rep:?}");
        let (srv, url) = live_serve(&b);
        let work = PathBuf::from(std::env::var("SYNOFURB_OFFLINE_WORK").expect("SYNOFURB_OFFLINE_WORK"));
        let modes = std::env::var("SYNOFURB_OFFLINE_MODES").unwrap_or_default();
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/offline/verify-mirror.sh");
        let chroot_script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../payload/chroot-setup.sh");
        for arch in b.manifest.archs.keys() {
            let mode = modes.split(',').find_map(|m| m.strip_prefix(&format!("{arch}="))).unwrap_or("foreign");
            let kernel = if b.manifest.kernel == KernelChannel::Newest { "newest" } else { "stable" };
            let extra = b.manifest.extra_packages.join(" ");
            let st = std::process::Command::new("sh")
                .arg(&script)
                .args([mode, arch, &url, work.to_str().unwrap(), chroot_script.to_str().unwrap(), kernel, &b.manifest.suite, &extra])
                .status()
                .unwrap();
            assert!(st.success(), "verify-mirror.sh {mode} {arch} failed");
        }
        srv.stop();
    }
}
