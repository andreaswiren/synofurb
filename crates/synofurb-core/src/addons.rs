//! Add-ons chosen in the installer (docs/ADDONS.md, "Installer integration").
//!
//! The management plane (`apps/mgmt-plane`) owns the authoritative add-on manifests and installs the add-ons. The
//! installer only needs a small built-in list to offer them before the unit exists: id, name, one-line summary,
//! what an add-on needs, which privileges it asks for and the few settings that matter at install time. A test checks
//! this list against the manifests in the repository, so the two cannot drift apart unnoticed.
//!
//! What the installer does with a choice:
//!
//! * [`resolve`] adds what is required (Docker for everything else) and puts the add-ons in install order.
//! * [`pending_json`] / [`pending_secrets_json`] render the files stage 2 writes to
//!   `/etc/synofurb/addons.d/pending/<id>.json` and `.../pending-secrets/<id>.json` (0600). Secrets are never part of
//!   `env.sh`, a command line, a plan or a log: they travel in the payload, next to the password hash.
//! * The first-boot unit `synofurb-addons-firstboot.service` hands the pending files to
//!   `synofurb_mgmt.py addons apply-pending`.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Debian 13 (trixie) packages for the `docker` add-on. `docker.io` is Docker Engine, `docker-cli` the client (only a
/// "Recommends" of `docker.io`, and the installer installs without recommends) and `docker-compose` is Compose v2 (Go):
/// it ships `/usr/libexec/docker/cli-plugins/docker-compose`, so `docker compose` works. (`docker-compose-v2` does not
/// exist in trixie.) Verified against the trixie `Packages` index: docker.io 26.1.5, docker-compose 2.26.1.
pub const DOCKER_PACKAGES: &[&str] = &["docker.io", "docker-cli", "docker-compose"];

/// Where stage 2 writes the choices in the installed system.
pub const PENDING_DIR: &str = "/etc/synofurb/addons.d/pending";
/// Secret settings (0600, directory 0700), one file per add-on. Not under `pending/` so that `pending/*.json` is only choices.
pub const PENDING_SECRETS_DIR: &str = "/etc/synofurb/addons.d/pending-secrets";
/// Written by the first-boot unit: what happened (JSON).
pub const RESULT_FILE: &str = "/var/lib/synofurb/addons-firstboot.json";

/// What kind of value an install-time setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKind {
    Int {
        min: i64,
        max: i64,
    },
    Text {
        max_len: usize,
    },
    /// Stored 0600 on the unit, never shown again, never logged. `prefix` is required at the start; `min`/`max` bound the
    /// whole length; the characters are letters, digits and `. _ ~ + / = : @ % -`.
    Secret {
        prefix: &'static str,
        min: usize,
        max: usize,
    },
}

/// One setting that is worth asking for while installing (everything else is done later in the web UI).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SettingSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: SettingKind,
    pub default: &'static str,
    /// Longer explanation, shown behind an info button.
    pub help: &'static str,
    pub required: bool,
}

impl SettingSpec {
    pub fn is_secret(&self) -> bool {
        matches!(self.kind, SettingKind::Secret { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddonKind {
    /// A host package/service installed by a named handler.
    System,
    /// A Docker app (compose project).
    Compose,
}

/// The web page of an add-on (the manifest's `web`): `scheme://<host>:port`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WebSpec {
    pub scheme: &'static str,
    pub port: u16,
}

/// A catalog entry as the installer shows it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub kind: AddonKind,
    /// Other add-on ids that must be installed first.
    pub requires: &'static [&'static str],
    /// `docker-socket`, `host-network`, `privileged`, `cap:<NAME>`: shown before the install.
    pub privileges: &'static [&'static str],
    /// Container images are pulled from the Internet at first boot.
    pub needs_internet: bool,
    /// Rough size of the downloaded software on the system volume, MiB.
    pub approx_size_mib: u32,
    /// Free space the add-on's data wants on the data volume, GiB (0: no fixed need).
    pub data_gib: u32,
    /// Keeps bulk data below `/srv/data`, which only exists when the install creates a data volume (not when volumes are kept).
    pub needs_data_volume: bool,
    /// The add-on's web page after the first boot, if it has one.
    pub web: Option<WebSpec>,
    /// What to do the first time it is opened (empty: nothing special).
    pub first_use: &'static str,
    pub settings: &'static [SettingSpec],
}

const CLAIM_HELP: &str = "A Plex claim token ties the server to your account. Get one at plex.tv/claim. It is valid for only 4 minutes and the \
first boot happens much later, so normally leave this empty and claim the server later in its web page.";

static CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "docker",
        name: "Docker",
        summary: "Docker Engine and the compose plugin from Debian. The base for every container add-on.",
        kind: AddonKind::System,
        requires: &[],
        privileges: &[],
        needs_internet: false,
        approx_size_mib: 500,
        data_gib: 0,
        needs_data_volume: false,
        web: None,
        first_use: "",
        settings: &[],
    },
    CatalogEntry {
        id: "portainer",
        name: "Portainer",
        summary: "Web page to manage Docker containers, images, volumes and networks.",
        kind: AddonKind::Compose,
        requires: &["docker"],
        privileges: &["docker-socket"],
        needs_internet: true,
        approx_size_mib: 300,
        data_gib: 0,
        needs_data_volume: false,
        web: Some(WebSpec { scheme: "https", port: 9443 }),
        first_use: "Open it within 5 minutes and create the administrator, or it locks itself.",
        settings: &[],
    },
    CatalogEntry {
        id: "frigate",
        name: "Frigate NVR",
        summary: "Local AI camera recorder. Recordings and clips go to the redundant data volume.",
        kind: AddonKind::Compose,
        requires: &["docker"],
        privileges: &[],
        needs_internet: true,
        approx_size_mib: 3000,
        data_gib: 50,
        needs_data_volume: true,
        web: Some(WebSpec { scheme: "https", port: 8971 }),
        first_use: "",
        settings: &[SettingSpec {
            key: "retain_days",
            label: "Keep recordings (days)",
            kind: SettingKind::Int { min: 1, max: 365 },
            default: "7",
            help: "Older recordings are deleted. Cameras and the detector are set up later in Frigate's web page.",
            required: false,
        }],
    },
    CatalogEntry {
        id: "plex",
        name: "Plex Media Server",
        summary: "Stream your movies, series and music to TVs, phones and browsers. Media folders on the data volume.",
        kind: AddonKind::Compose,
        requires: &["docker"],
        privileges: &["host-network"],
        needs_internet: true,
        approx_size_mib: 700,
        data_gib: 0,
        needs_data_volume: true,
        web: Some(WebSpec { scheme: "http", port: 32400 }),
        first_use: "",
        settings: &[SettingSpec {
            key: "plex_claim",
            label: "Claim token (leave empty)",
            kind: SettingKind::Secret { prefix: "claim-", min: 14, max: 70 },
            default: "",
            help: CLAIM_HELP,
            required: false,
        }],
    },
];

/// The built-in list, in display order (Docker first).
pub fn catalog() -> &'static [CatalogEntry] {
    CATALOG
}

pub fn entry(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id == id)
}

/// A quick-setup preset: ids to tick (what they require is added by [`resolve`]).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub addons: &'static [&'static str],
}

pub const PRESETS: &[Preset] = &[
    Preset { id: "docker-portainer", label: "Docker + Portainer", addons: &["docker", "portainer"] },
    Preset { id: "frigate", label: "Frigate NVR", addons: &["docker", "portainer", "frigate"] },
    Preset { id: "plex", label: "Plex", addons: &["docker", "portainer", "plex"] },
    Preset { id: "frigate-plex", label: "Frigate + Plex", addons: &["docker", "portainer", "frigate", "plex"] },
];

/// What the add-ons of a preset are, with what they need: `Docker, Portainer, Plex Media Server`.
pub fn preset_names(p: &Preset) -> String {
    let c: Vec<AddonChoice> = p.addons.iter().map(|i| AddonChoice::new(i)).collect();
    resolve(&c).map(|r| r.iter().map(|x| x.entry.name).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// One add-on picked for the install: its id and the settings typed for it.
///
/// `secrets` holds the values of secret settings (for example a Plex claim token). They are not serialized (a saved or
/// printed configuration never contains them) and are redacted in `Debug`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonChoice {
    pub id: String,
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    #[serde(default, skip_serializing)]
    pub secrets: BTreeMap<String, String>,
}

impl std::fmt::Debug for AddonChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden: Vec<&String> = self.secrets.keys().collect();
        f.debug_struct("AddonChoice").field("id", &self.id).field("settings", &self.settings).field("secrets", &hidden).finish()
    }
}

impl AddonChoice {
    pub fn new(id: &str) -> AddonChoice {
        AddonChoice { id: id.to_string(), ..Default::default() }
    }

    /// `id`, with its settings (secret keys listed by name only): what the CLI flag looks like without secret values.
    pub fn spec_string(&self) -> String {
        let mut parts: Vec<String> = self.settings.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.extend(self.secrets.keys().map(|k| format!("{k}=<secret>")));
        if parts.is_empty() { self.id.clone() } else { format!("{}:{}", self.id, parts.join(",")) }
    }
}

/// An add-on of the final install, after dependencies were added.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub entry: &'static CatalogEntry,
    pub choice: AddonChoice,
    /// Added only because another add-on requires it.
    pub auto: bool,
    /// The chosen add-ons that need this one (empty when nothing does).
    pub required_by: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    let mut c = id.chars();
    id.len() <= 32
        && c.next().is_some_and(|f| f.is_ascii_lowercase() || f.is_ascii_digit())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '-')
}

fn check_value(spec: &SettingSpec, v: &str) -> Result<()> {
    if v.chars().any(|c| c.is_control()) {
        bail!("{}: the value must be a single line without control characters", spec.label);
    }
    match spec.kind {
        SettingKind::Int { min, max } => match v.trim().parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => Ok(()),
            _ => bail!("{}: '{v}' is not a whole number from {min} to {max}", spec.label),
        },
        SettingKind::Text { max_len } => {
            if v.chars().count() > max_len {
                bail!("{}: at most {max_len} characters", spec.label);
            }
            Ok(())
        }
        SettingKind::Secret { prefix, min, max } => {
            // letters, digits and common punctuation only: neutral for JSON, shells and YAML
            if !v.starts_with(prefix)
                || v.len() < min
                || v.len() > max
                || !v.chars().all(|c| c.is_ascii_alphanumeric() || "._~+/=:@%-".contains(c))
            {
                let start = if prefix.is_empty() { String::new() } else { format!(" starting with '{prefix}'") };
                bail!("{}: {min}-{max} characters{start}, from letters, digits and . _ ~ + / = : @ % -", spec.label);
            }
            Ok(())
        }
    }
}

/// Check one typed value of an install-time setting (the GUI checks fields as they are typed). An empty value is "not set" and fine
/// unless the setting is required.
pub fn check_setting(id: &str, key: &str, value: &str) -> Result<()> {
    let Some(e) = entry(id) else { bail!("unknown add-on '{id}'") };
    let Some(spec) = e.settings.iter().find(|s| s.key == key) else { bail!("add-on '{id}' has no install-time setting '{key}'") };
    if value.trim().is_empty() {
        if spec.required {
            bail!("{} is required", spec.label);
        }
        return Ok(());
    }
    check_value(spec, value.trim())
}

/// Check the choices: known ids, no duplicates, known settings with valid values. Does not add dependencies.
pub fn validate(choices: &[AddonChoice]) -> Result<()> {
    let mut seen: Vec<&str> = vec![];
    for c in choices {
        if !valid_id(&c.id) {
            bail!("invalid add-on id '{}'", c.id);
        }
        let Some(e) = entry(&c.id) else {
            let known: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
            bail!("unknown add-on '{}' (available: {})", c.id, known.join(", "));
        };
        if seen.contains(&c.id.as_str()) {
            bail!("add-on '{}' is listed twice", c.id);
        }
        seen.push(&c.id);
        for (k, v) in &c.settings {
            let Some(spec) = e.settings.iter().find(|s| s.key == k) else {
                bail!("add-on '{}' has no install-time setting '{k}'{}", e.id, settings_hint(e));
            };
            if spec.is_secret() {
                bail!("'{k}' of '{}' is a secret: it must not be given as a plain setting", e.id);
            }
            check_value(spec, v)?;
        }
        for (k, v) in &c.secrets {
            let Some(spec) = e.settings.iter().find(|s| s.key == k) else {
                bail!("add-on '{}' has no install-time setting '{k}'{}", e.id, settings_hint(e));
            };
            if !spec.is_secret() {
                bail!("'{k}' of '{}' is not a secret setting", e.id);
            }
            check_value(spec, v)?;
        }
        for s in e.settings.iter().filter(|s| s.required) {
            if !c.settings.contains_key(s.key) && !c.secrets.contains_key(s.key) {
                bail!("add-on '{}' needs the setting '{}' ({})", e.id, s.key, s.label);
            }
        }
    }
    Ok(())
}

fn settings_hint(e: &CatalogEntry) -> String {
    if e.settings.is_empty() {
        format!("; {} has none", e.name)
    } else {
        format!("; available: {}", e.settings.iter().map(|s| s.key).collect::<Vec<_>>().join(", "))
    }
}

/// Validate, add everything that is required, and order the add-ons so that each comes after what it requires.
pub fn resolve(choices: &[AddonChoice]) -> Result<Vec<Resolved>> {
    validate(choices)?;
    let mut all: Vec<Resolved> = vec![];
    // chosen ones first, then their requirements, until nothing is added
    for c in choices {
        all.push(Resolved { entry: entry(&c.id).expect("validated"), choice: c.clone(), auto: false, required_by: vec![] });
    }
    let mut i = 0;
    while i < all.len() {
        let (id, requires) = (all[i].entry.id, all[i].entry.requires);
        for r in requires {
            match all.iter().position(|a| a.entry.id == *r) {
                Some(p) => {
                    if !all[p].required_by.iter().any(|x| x == id) {
                        all[p].required_by.push(id.to_string());
                    }
                }
                None => all.push(Resolved {
                    entry: entry(r).unwrap_or_else(|| panic!("catalog: '{id}' requires unknown '{r}'")),
                    choice: AddonChoice::new(r),
                    auto: true,
                    required_by: vec![id.to_string()],
                }),
            }
        }
        i += 1;
    }
    // stable order: dependencies first, otherwise catalog order
    let mut ordered: Vec<Resolved> = vec![];
    let mut rest = all;
    rest.sort_by_key(|a| CATALOG.iter().position(|e| e.id == a.entry.id));
    while !rest.is_empty() {
        let Some(p) = rest.iter().position(|a| a.entry.requires.iter().all(|r| ordered.iter().any(|o| o.entry.id == *r))) else {
            bail!("circular add-on requirements");
        };
        ordered.push(rest.remove(p));
    }
    Ok(ordered)
}

/// The install order as ids (what the CLI and the confirm page list).
pub fn resolved_ids(choices: &[AddonChoice]) -> Result<Vec<&'static str>> {
    Ok(resolve(choices)?.iter().map(|r| r.entry.id).collect())
}

/// What the rest of the install plan tells us about the unit.
#[derive(Debug, Clone, Copy, Default)]
pub struct AddonContext {
    /// An offline bundle is the package source.
    pub offline: bool,
    /// Size of the root volume, MiB, when the plan is known.
    pub root_mib: Option<u64>,
    /// Does the plan create the data volume (`/srv/data`)? `None`: not known.
    pub has_data_volume: Option<bool>,
}

/// Lines for the plan / confirm page: what will be installed and when.
pub fn plan_lines(choices: &[AddonChoice]) -> Vec<String> {
    let Ok(items) = resolve(choices) else { return vec![] };
    if items.is_empty() {
        return vec![];
    }
    let list = items
        .iter()
        .map(|r| if r.auto { format!("{} (required by {})", r.entry.name, names(&r.required_by)) } else { r.entry.name.to_string() })
        .collect::<Vec<_>>()
        .join(", ");
    let mut v = vec![format!("Add-ons: {list}")];
    if items.iter().any(|r| r.entry.id == "docker") {
        v.push("Docker is installed with the system; the add-ons are installed on the first boot by the web management page.".into());
    } else {
        v.push("The add-ons are installed on the first boot by the web management page.".into());
    }
    if items.iter().any(|r| r.entry.needs_internet) {
        v.push("The first boot needs Internet access to pull the container images.".into());
    }
    v
}

fn names(ids: &[String]) -> String {
    ids.iter().map(|i| entry(i).map_or(i.as_str(), |e| e.name)).collect::<Vec<_>>().join(", ")
}

/// An add-on that asks for more than an ordinary container, with the plain sentence about what that means. The installer asks for
/// an explicit acknowledgement of exactly this list before it erases anything (the web interface asks the same at install).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Consent {
    pub id: &'static str,
    pub name: &'static str,
    pub privileges: &'static [&'static str],
    pub note: &'static str,
}

/// The privileged add-ons among what will be installed (Docker itself asks for none).
pub fn consent_items(choices: &[AddonChoice]) -> Vec<Consent> {
    resolve(choices)
        .unwrap_or_default()
        .iter()
        .filter(|r| !r.entry.privileges.is_empty())
        .map(|r| Consent { id: r.entry.id, name: r.entry.name, privileges: r.entry.privileges, note: privilege_note(r.entry.privileges) })
        .collect()
}

/// A short stable text that changes whenever the set of acknowledged privileges changes (`portainer:docker-socket;plex:host-network`);
/// empty when nothing privileged is chosen. A consent given for one signature is not a consent for another.
pub fn consent_signature(choices: &[AddonChoice]) -> String {
    consent_items(choices).iter().map(|c| format!("{}:{}", c.id, c.privileges.join("+"))).collect::<Vec<_>>().join(";")
}

/// Names of the chosen add-ons (dependencies included) that keep their data below `/srv/data` although the install creates no data volume.
pub fn needing_data_volume(choices: &[AddonChoice], has_data_volume: bool) -> Vec<&'static str> {
    if has_data_volume {
        return vec![];
    }
    resolve(choices).unwrap_or_default().iter().filter(|r| r.entry.needs_data_volume).map(|r| r.entry.name).collect()
}

/// Download speed the "about N minutes" estimate assumes: 20 Mbit/s, about 150 MiB a minute. It is an estimate for planning, not a
/// measurement; a slower line takes longer.
pub const ASSUMED_MIB_PER_MIN: u32 = 150;

/// Rough minutes the first boot spends on an add-on (at least 1): its download at [`ASSUMED_MIB_PER_MIN`].
pub fn approx_minutes(e: &CatalogEntry) -> u32 {
    e.approx_size_mib.div_ceil(ASSUMED_MIB_PER_MIN).max(1)
}

/// `https://192.168.1.20:9443` for an add-on with a web page (IPv6 hosts get brackets); `None` without one.
pub fn web_address(e: &CatalogEntry, host: &str) -> Option<String> {
    let w = e.web?;
    let h = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host.to_string() };
    Some(format!("{}://{h}:{}", w.scheme, w.port))
}

/// What the Done screen says about one add-on after the unit is installed. Nothing here is read back from the unit: it is what the
/// first boot is going to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AfterInstall {
    pub id: &'static str,
    pub name: &'static str,
    /// Where it answers once it has started (`None`: a system add-on without a page).
    pub address: Option<String>,
    pub minutes: u32,
    pub size_mib: u32,
    pub first_use: &'static str,
}

/// One entry per add-on that will be installed, in install order, for a unit at `host`.
pub fn after_install(choices: &[AddonChoice], host: &str) -> Vec<AfterInstall> {
    resolve(choices)
        .unwrap_or_default()
        .iter()
        .map(|r| AfterInstall {
            id: r.entry.id,
            name: r.entry.name,
            address: web_address(r.entry, host),
            minutes: approx_minutes(r.entry),
            size_mib: r.entry.approx_size_mib,
            first_use: r.entry.first_use,
        })
        .collect()
}

/// What kind of thing a [`warnings`] line is, so a screen can show the ones that matter in plain sight and fold the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    /// Images are pulled from the Internet at the first boot.
    Internet,
    /// The offline bundle has no Docker.
    Offline,
    /// An add-on asks for more than an ordinary container (who can log in controls the NAS).
    Privilege,
    /// How much room the add-ons take on the system volume.
    Size,
    /// The system volume is small for the add-ons.
    Tight,
    /// The plan creates no data volume although an add-on keeps data below `/srv/data`.
    NoDataVolume,
    /// What an add-on wants on the data volume.
    DataWants,
}

impl NoteKind {
    /// Lines a person must not miss before erasing: privileges, a tight system volume, a missing data volume, a missing Docker.
    pub fn is_loud(self) -> bool {
        matches!(self, NoteKind::Privilege | NoteKind::Tight | NoteKind::NoDataVolume | NoteKind::Offline)
    }
}

/// [`warnings`] with the kind of each line.
pub fn notes(choices: &[AddonChoice], ctx: &AddonContext) -> Vec<(NoteKind, String)> {
    let Ok(items) = resolve(choices) else { return vec![] };
    let mut w: Vec<(NoteKind, String)> = vec![];
    if items.is_empty() {
        return w;
    }
    let net: Vec<&str> = items.iter().filter(|r| r.entry.needs_internet).map(|r| r.entry.name).collect();
    if !net.is_empty() {
        w.push((NoteKind::Internet, format!(
            "{} will pull container images from the Internet when the unit first boots. Without Internet the add-on shows \"needs attention\" in the web page; the installation itself does not fail.",
            net.join(", ")
        )));
    }
    if ctx.offline {
        w.push((
            NoteKind::Offline,
            "The offline bundle does not contain Docker: its packages are downloaded from the Debian mirrors on the first boot, so the unit needs Internet for that too."
                .into(),
        ));
    }
    for r in &items {
        if !r.entry.privileges.is_empty() {
            w.push((
                NoteKind::Privilege,
                format!("{} asks for: {}. {}", r.entry.name, r.entry.privileges.join(", "), privilege_note(r.entry.privileges)),
            ));
        }
    }
    let size_mib: u64 = items.iter().map(|r| r.entry.approx_size_mib as u64).sum();
    w.push((
        NoteKind::Size,
        format!("The add-ons need about {:.1} GiB on the system volume (Docker keeps images under /var/lib/docker).", size_mib as f64 / 1024.0),
    ));
    if let Some(root) = ctx.root_mib {
        // leave room for the system itself, logs and updates
        if root < size_mib + 4096 {
            w.push((
                NoteKind::Tight,
                format!("The system volume is only {} GiB: that is tight for the add-ons. Raise the system size in the storage settings.", root / 1024),
            ));
        }
    }
    let missing = needing_data_volume(choices, ctx.has_data_volume != Some(false));
    if !missing.is_empty() {
        w.push((
            NoteKind::NoDataVolume,
            format!(
                "This plan creates no data volume (kept volumes are not mounted automatically), but these add-ons keep data below /srv/data: {}. Until /srv/data is a mount of its own they show \"needs attention\".",
                missing.join(", ")
            ),
        ));
    }
    for r in items.iter().filter(|r| r.entry.data_gib > 0 && !missing.contains(&r.entry.name)) {
        w.push((NoteKind::DataWants, format!("{} stores its data on the data volume and wants at least {} GiB free there.", r.entry.name, r.entry.data_gib)));
    }
    w
}

/// Things to know before starting: the Internet, privileges, disk use, offline limits.
pub fn warnings(choices: &[AddonChoice], ctx: &AddonContext) -> Vec<String> {
    notes(choices, ctx).into_iter().map(|(_, t)| t).collect()
}

fn privilege_note(p: &[&str]) -> &'static str {
    if p.contains(&"docker-socket") {
        "Whoever can log in to it controls Docker, which is as good as being administrator of the NAS."
    } else if p.contains(&"privileged") {
        "It runs with almost unrestricted access to the NAS."
    } else if p.contains(&"host-network") {
        "It shares the NAS's network."
    } else {
        "Shown here so that you know before it is installed."
    }
}

/// Settings of an add-on for display: the spec plus the value currently chosen (secrets are never returned).
pub fn setting_value<'a>(choice: &'a AddonChoice, spec: &SettingSpec) -> Option<&'a str> {
    if spec.is_secret() { None } else { choice.settings.get(spec.key).map(String::as_str) }
}

/// `/etc/synofurb/addons.d/pending/<id>.json`: the choice without secrets. `order` is the position in the install order (1-based).
pub fn pending_json(r: &Resolved, order: usize) -> String {
    let secrets: Vec<&String> = r.choice.secrets.keys().collect();
    let doc = serde_json::json!({
        "schema": 1,
        "id": r.entry.id,
        "order": order,
        "source": "installer",
        "auto": r.auto,
        "required_by": r.required_by,
        "requires": r.entry.requires,
        "settings": r.choice.settings,
        "secrets": secrets,
    });
    serde_json::to_string_pretty(&doc).expect("json") + "\n"
}

/// `/etc/synofurb/addons.d/pending-secrets/<id>.json` (0600), or `None` when the add-on has no secret.
pub fn pending_secrets_json(r: &Resolved) -> Option<String> {
    if r.choice.secrets.is_empty() {
        return None;
    }
    Some(serde_json::to_string_pretty(&r.choice.secrets).expect("json") + "\n")
}

/// The choice as one JSON document (plan JSON, dry run): who is installed and why, the lines and the warnings. Secret values never appear.
pub fn summary_json(choices: &[AddonChoice], ctx: &AddonContext) -> serde_json::Value {
    let items = resolve(choices).unwrap_or_default();
    let selected: Vec<serde_json::Value> = items
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.entry.id, "name": r.entry.name, "auto": r.auto, "required_by": r.required_by,
                "privileges": r.entry.privileges, "needs_internet": r.entry.needs_internet,
                "settings": r.choice.settings, "secrets": r.choice.secrets.keys().collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "selected": selected, "lines": plan_lines(choices), "warnings": warnings(choices, ctx) })
}

/// A secret value that stands in for the real one in dry runs and rendered artefacts.
pub const SECRET_PLACEHOLDER: &str = "DRYRUN-SECRET";

/// The same choices with every secret replaced by [`SECRET_PLACEHOLDER`] (after the prefix the setting requires, so the result still
/// validates). Dry runs use this.
pub fn redact_secrets(choices: &[AddonChoice]) -> Vec<AddonChoice> {
    choices
        .iter()
        .map(|c| {
            let prefix = |k: &str| match entry(&c.id).and_then(|e| e.settings.iter().find(|s| s.key == k)).map(|s| s.kind) {
                Some(SettingKind::Secret { prefix, .. }) => prefix,
                _ => "",
            };
            AddonChoice {
                secrets: c.secrets.keys().map(|k| (k.clone(), format!("{}{SECRET_PLACEHOLDER}", prefix(k)))).collect(),
                ..c.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(c: &[AddonChoice]) -> Vec<&'static str> {
        resolved_ids(c).unwrap()
    }

    fn choice(id: &str) -> AddonChoice {
        AddonChoice::new(id)
    }

    #[test]
    fn docker_is_added_when_anything_needs_it() {
        assert_eq!(ids(&[]), Vec::<&str>::new());
        assert_eq!(ids(&[choice("docker")]), ["docker"]);
        assert_eq!(ids(&[choice("portainer")]), ["docker", "portainer"]);
        assert_eq!(ids(&[choice("plex"), choice("frigate")]), ["docker", "frigate", "plex"], "dependencies first, then catalog order");
        let r = resolve(&[choice("frigate"), choice("plex")]).unwrap();
        assert!(r[0].auto && r[0].required_by == ["frigate", "plex"]);
        assert!(!r[1].auto && !r[2].auto);
        // explicit docker is not "auto"
        let r = resolve(&[choice("docker"), choice("portainer")]).unwrap();
        assert!(!r[0].auto && r[0].required_by == ["portainer"]);
    }

    #[test]
    fn every_requirement_is_in_the_catalog_and_the_graph_has_no_cycle() {
        for e in catalog() {
            for r in e.requires {
                assert!(entry(r).is_some(), "{} requires unknown {r}", e.id);
            }
            assert!(valid_id(e.id));
            assert!(e.summary.len() <= 100, "{}: keep summaries to one short line", e.id);
            for p in e.privileges {
                assert!(["docker-socket", "host-network", "privileged"].contains(p) || p.starts_with("cap:"), "{p}");
            }
        }
        let all: Vec<AddonChoice> = catalog().iter().map(|e| choice(e.id)).collect();
        assert_eq!(resolve(&all).unwrap().len(), catalog().len());
        for p in PRESETS {
            let c: Vec<AddonChoice> = p.addons.iter().map(|i| choice(i)).collect();
            validate(&c).unwrap();
        }
    }

    #[test]
    fn validation() {
        assert!(validate(&[choice("nope")]).unwrap_err().to_string().contains("available: docker, portainer, frigate, plex"));
        assert!(validate(&[choice("Docker")]).is_err());
        assert!(validate(&[choice("docker;rm")]).is_err());
        assert!(validate(&[choice("docker"), choice("docker")]).unwrap_err().to_string().contains("twice"));
        let mut f = choice("frigate");
        f.settings.insert("retain_days".into(), "14".into());
        validate(&[f.clone()]).unwrap();
        for bad in ["0", "366", "x", "", "7 days", "1\n2"] {
            f.settings.insert("retain_days".into(), bad.into());
            assert!(validate(&[f.clone()]).is_err(), "{bad:?}");
        }
        f.settings.clear();
        f.settings.insert("detector".into(), "coral".into());
        assert!(validate(&[f]).unwrap_err().to_string().contains("available: retain_days"));
        let mut d = choice("docker");
        d.settings.insert("x".into(), "y".into());
        assert!(validate(&[d]).unwrap_err().to_string().contains("has none"));
    }

    #[test]
    fn secrets_are_separate_hidden_and_checked() {
        let mut p = choice("plex");
        p.secrets.insert("plex_claim".into(), "claim-AbC123_xyz".into());
        validate(std::slice::from_ref(&p)).unwrap();
        // not serialized, redacted in Debug
        let j = serde_json::to_string(&p).unwrap();
        assert!(!j.contains("claim-AbC123") && !j.contains("plex_claim"), "{j}");
        assert!(!format!("{p:?}").contains("claim-AbC123") && format!("{p:?}").contains("plex_claim"));
        assert!(!format!("{:?}", resolve(std::slice::from_ref(&p)).unwrap()).contains("claim-AbC123"));
        // a secret may not be given as a plain setting, and vice versa
        let mut q = choice("plex");
        q.settings.insert("plex_claim".into(), "claim-x".into());
        assert!(validate(&[q]).unwrap_err().to_string().contains("secret"));
        let mut q = choice("frigate");
        q.secrets.insert("retain_days".into(), "7".into());
        assert!(validate(&[q]).is_err());
        for bad in [
            "",
            "claim-",
            "token-AbC123_xyz",
            "claim-a b c d e f",
            "claim-a'bcdefgh",
            "claim-\"abcdefgh",
            "claim-$abcdefgh",
            "claim-a;bcdefgh",
            "claim-`abcdefgh",
            "claim-abcdefgh\n",
        ] {
            let mut q = choice("plex");
            q.secrets.insert("plex_claim".into(), bad.into());
            assert!(validate(&[q]).is_err(), "{bad:?}");
        }
        // spec_string never shows the value
        assert_eq!(p.spec_string(), "plex:plex_claim=<secret>");
        let red = redact_secrets(&[p]);
        assert_eq!(red[0].secrets["plex_claim"], format!("claim-{SECRET_PLACEHOLDER}"));
        validate(&red).unwrap();
    }

    #[test]
    fn single_values_can_be_checked_as_they_are_typed() {
        check_setting("frigate", "retain_days", "").unwrap();
        check_setting("frigate", "retain_days", " 30 ").unwrap();
        assert!(check_setting("frigate", "retain_days", "0").is_err());
        check_setting("plex", "plex_claim", "claim-abcdefgh12345").unwrap();
        assert!(check_setting("plex", "plex_claim", "abc").is_err());
        assert!(check_setting("plex", "nope", "x").is_err() && check_setting("nope", "x", "y").is_err());
    }

    #[test]
    fn old_saved_configs_without_addons_still_load() {
        let c: AddonChoice = serde_json::from_str(r#"{"id":"docker"}"#).unwrap();
        assert_eq!(c, choice("docker"));
        let c: AddonChoice = serde_json::from_str(r#"{"id":"frigate","settings":{"retain_days":"3"}}"#).unwrap();
        assert_eq!(c.settings["retain_days"], "3");
    }

    #[test]
    fn plan_lines_and_warnings() {
        assert!(plan_lines(&[]).is_empty() && warnings(&[], &AddonContext::default()).is_empty());
        let l = plan_lines(&[choice("frigate"), choice("portainer")]);
        assert!(l[0].starts_with("Add-ons: Docker (required by Frigate NVR, Portainer), Portainer, Frigate NVR"), "{l:?}");
        assert!(l.iter().any(|x| x.contains("Internet")));
        let w = warnings(&[choice("portainer"), choice("frigate")], &AddonContext { offline: false, root_mib: Some(20 * 1024), has_data_volume: Some(true) });
        let all = w.join("\n");
        assert!(all.contains("pull container images from the Internet"), "{all}");
        assert!(all.contains("docker-socket") && all.contains("administrator of the NAS"), "{all}");
        assert!(all.contains("at least 50 GiB"), "{all}");
        assert!(!all.contains("offline bundle"), "{all}");
        assert!(!all.contains("tight"), "{all}");
        let w = warnings(&[choice("frigate")], &AddonContext { offline: true, root_mib: Some(6 * 1024), has_data_volume: None }).join("\n");
        assert!(w.contains("offline bundle does not contain Docker") && w.contains("tight"), "{w}");
        // a Docker-only choice needs no Internet for images
        assert!(!warnings(&[choice("docker")], &AddonContext::default()).join("\n").contains("pull container images"));
    }

    #[test]
    fn notes_say_which_lines_a_person_must_not_miss() {
        let ctx = AddonContext { offline: true, root_mib: Some(6 * 1024), has_data_volume: Some(false) };
        let n = notes(&[choice("portainer"), choice("frigate")], &ctx);
        let kinds: Vec<NoteKind> = n.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            kinds,
            [NoteKind::Internet, NoteKind::Offline, NoteKind::Privilege, NoteKind::Size, NoteKind::Tight, NoteKind::NoDataVolume]
        );
        let loud: Vec<NoteKind> = kinds.iter().copied().filter(|k| k.is_loud()).collect();
        assert_eq!(loud, [NoteKind::Offline, NoteKind::Privilege, NoteKind::Tight, NoteKind::NoDataVolume]);
        assert!(!NoteKind::Internet.is_loud() && !NoteKind::Size.is_loud() && !NoteKind::DataWants.is_loud());
        // the plain list is the same text
        assert_eq!(warnings(&[choice("portainer"), choice("frigate")], &ctx), n.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn summary_json_has_no_secret_values() {
        let mut p = choice("plex");
        p.secrets.insert("plex_claim".into(), "claim-SECRETVALUE".into());
        let j = summary_json(&[p, choice("portainer")], &AddonContext::default());
        assert_eq!(j["selected"].as_array().unwrap().len(), 3);
        assert_eq!(j["selected"][0]["id"], "docker");
        assert_eq!(j["selected"][0]["auto"], true);
        assert_eq!(j["selected"][2]["secrets"], serde_json::json!(["plex_claim"]));
        assert!(!j.to_string().contains("SECRETVALUE"));
        assert!(j["lines"].as_array().unwrap().len() >= 2 && !j["warnings"].as_array().unwrap().is_empty());
        assert_eq!(summary_json(&[], &AddonContext::default())["selected"], serde_json::json!([]));
    }

    #[test]
    fn pending_files() {
        let mut p = choice("plex");
        p.secrets.insert("plex_claim".into(), "claim-SECRETVALUE".into());
        let mut f = choice("frigate");
        f.settings.insert("retain_days".into(), "14".into());
        let r = resolve(&[p, f]).unwrap();
        let j: serde_json::Value = serde_json::from_str(&pending_json(&r[2], 3)).unwrap();
        assert_eq!(j["id"], "plex");
        assert_eq!((j["schema"].as_u64(), j["order"].as_u64(), j["auto"].as_bool()), (Some(1), Some(3), Some(false)));
        assert_eq!(j["secrets"][0], "plex_claim");
        assert!(!pending_json(&r[2], 3).contains("SECRETVALUE"), "secret values never go into the pending choice file");
        let s: serde_json::Value = serde_json::from_str(&pending_secrets_json(&r[2]).unwrap()).unwrap();
        assert_eq!(s["plex_claim"], "claim-SECRETVALUE");
        let j: serde_json::Value = serde_json::from_str(&pending_json(&r[1], 2)).unwrap();
        assert_eq!(j["settings"]["retain_days"], "14");
        assert!(pending_secrets_json(&r[1]).is_none());
        let j: serde_json::Value = serde_json::from_str(&pending_json(&r[0], 1)).unwrap();
        assert_eq!((j["id"].as_str(), j["auto"].as_bool()), (Some("docker"), Some(true)));
        assert_eq!(j["required_by"], serde_json::json!(["plex", "frigate"]));
    }

    /// The installer's list must say what the management plane's manifests say. Skipped while the manifests do not exist (the
    /// management plane is built in parallel); once they do, every difference fails here.
    #[test]
    fn catalog_matches_the_management_plane_manifests() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/mgmt-plane/addons");
        let mut checked = 0;
        for e in catalog() {
            let f = dir.join(e.id).join("addon.json");
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let m: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|x| panic!("{}: {x}", f.display()));
            assert_eq!(m["id"], e.id, "{}", f.display());
            assert_eq!(m["name"], e.name, "{}: name differs from the installer's list (crates/synofurb-core/src/addons.rs)", e.id);
            assert_eq!(m["summary"], e.summary, "{}: summary differs from the installer's list", e.id);
            let strings = |v: &serde_json::Value| -> Vec<String> {
                v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
            };
            assert_eq!(strings(&m["requires"]), e.requires, "{}: requires differ", e.id);
            assert_eq!(strings(&m["privileges"]), e.privileges, "{}: privileges differ", e.id);
            let kind = match e.kind {
                AddonKind::System => "system",
                AddonKind::Compose => "compose",
            };
            assert_eq!(m["kind"], kind, "{}: kind differs", e.id);
            // the page and the data volume need the installer tells the user about are the manifest's
            match e.web {
                Some(w) => {
                    assert_eq!((m["web"]["scheme"].as_str(), m["web"]["port"].as_u64()), (Some(w.scheme), Some(w.port as u64)), "{}: web differs", e.id)
                }
                None => assert!(m["web"].is_null(), "{}: the manifest has a web page the installer does not know", e.id),
            }
            let bulk = m["storage"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x["bulk"] == true && x["path"].as_str().is_some_and(|p| p.starts_with("/srv/data"))));
            assert_eq!(e.needs_data_volume, bulk, "{}: needs_data_volume differs from the manifest's bulk storage", e.id);
            for s in e.settings {
                let ms = m["settings"].as_array().and_then(|a| a.iter().find(|x| x["key"] == s.key));
                let ms = ms.unwrap_or_else(|| panic!("{}: the manifest has no setting '{}'", e.id, s.key));
                let t = match s.kind {
                    SettingKind::Int { .. } => "int",
                    SettingKind::Text { .. } => "string",
                    SettingKind::Secret { .. } => "secret",
                };
                assert_eq!(ms["type"], t, "{}.{}: setting type differs", e.id, s.key);
            }
            checked += 1;
        }
        eprintln!("catalog checked against {checked} manifest(s) in {}", dir.display());
    }

    #[test]
    fn privileged_add_ons_need_an_acknowledgement_that_names_them() {
        assert!(consent_items(&[]).is_empty() && consent_signature(&[]).is_empty());
        assert!(consent_items(&[choice("docker")]).is_empty(), "Docker alone asks for nothing");
        assert!(consent_items(&[choice("frigate")]).is_empty(), "Frigate has no privilege");
        let c = consent_items(&[choice("plex"), choice("portainer")]);
        assert_eq!(c.iter().map(|x| x.id).collect::<Vec<_>>(), ["portainer", "plex"]);
        assert!(c[0].note.contains("administrator of the NAS") && c[1].note.contains("network"), "{c:?}");
        let sig = consent_signature(&[choice("plex"), choice("portainer")]);
        assert_eq!(sig, "portainer:docker-socket;plex:host-network");
        // a consent for one set is not a consent for another
        assert_ne!(sig, consent_signature(&[choice("portainer")]));
        assert_eq!(consent_signature(&[choice("portainer")]), consent_signature(&[choice("docker"), choice("portainer")]));
        // every preset that carries Portainer carries the consent
        for p in PRESETS {
            let c: Vec<AddonChoice> = p.addons.iter().map(|i| choice(i)).collect();
            assert_eq!(!consent_items(&c).is_empty(), p.addons.contains(&"portainer"), "{}", p.id);
        }
    }

    #[test]
    fn presets_say_everything_they_install() {
        let names: Vec<String> = PRESETS.iter().map(preset_names).collect();
        assert_eq!(names[0], "Docker, Portainer");
        assert_eq!(names[2], "Docker, Portainer, Plex Media Server", "the Plex preset also installs Portainer, and says so");
        assert_eq!(names[3], "Docker, Portainer, Frigate NVR, Plex Media Server");
    }

    #[test]
    fn add_ons_that_keep_data_on_the_data_volume_are_flagged_without_one() {
        assert!(needing_data_volume(&[choice("frigate"), choice("plex")], true).is_empty());
        assert_eq!(needing_data_volume(&[choice("frigate"), choice("plex")], false), ["Frigate NVR", "Plex Media Server"]);
        assert!(needing_data_volume(&[choice("portainer")], false).is_empty(), "Portainer keeps nothing below /srv/data");
        let w = warnings(&[choice("frigate")], &AddonContext { has_data_volume: Some(false), ..Default::default() }).join("\n");
        assert!(w.contains("This plan creates no data volume") && w.contains("keep data below /srv/data: Frigate NVR."), "{w}");
        assert!(!w.contains("at least 50 GiB free there"), "the free-space line is not said for a volume that is not created: {w}");
        let w = warnings(&[choice("frigate")], &AddonContext { has_data_volume: Some(true), ..Default::default() }).join("\n");
        assert!(w.contains("at least 50 GiB") && !w.contains("creates no data volume"), "{w}");
        // unknown plan: no claim either way
        assert!(!warnings(&[choice("frigate")], &AddonContext::default()).join("\n").contains("creates no data volume"));
    }

    #[test]
    fn after_install_lines_give_the_address_the_time_and_the_first_use() {
        let a = after_install(&[choice("portainer"), choice("plex")], "192.168.1.20");
        assert_eq!(a.iter().map(|x| x.id).collect::<Vec<_>>(), ["docker", "portainer", "plex"]);
        assert_eq!(a[0].address, None, "Docker has no page");
        assert_eq!(a[1].address.as_deref(), Some("https://192.168.1.20:9443"));
        assert_eq!(a[2].address.as_deref(), Some("http://192.168.1.20:32400"));
        assert!(a[1].first_use.contains("5 minutes") && a[2].first_use.is_empty());
        assert_eq!(web_address(entry("portainer").unwrap(), "fe80::1").as_deref(), Some("https://[fe80::1]:9443"));
        // 3000 MiB at 150 MiB a minute: 20 minutes; a tiny one never reads "0 minutes"
        assert_eq!(approx_minutes(entry("frigate").unwrap()), 20);
        assert_eq!(approx_minutes(entry("portainer").unwrap()), 2);
        assert!(catalog().iter().all(|e| approx_minutes(e) >= 1));
        assert!(after_install(&[], "x").is_empty());
    }

    #[test]
    fn docker_package_names_are_plain_and_in_one_place() {
        assert!(DOCKER_PACKAGES.iter().all(|p| crate::config::valid_package_name(p)));
        assert!(DOCKER_PACKAGES.contains(&"docker.io") && DOCKER_PACKAGES.contains(&"docker-compose"));
    }
}
