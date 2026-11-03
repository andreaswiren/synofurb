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
    pub settings: &'static [SettingSpec],
}

const CLAIM_HELP: &str = "A Plex claim token ties the server to your account. Get one at plex.tv/claim. It is valid for only 4 minutes and the \
first boot happens much later, so normally leave this empty and claim the server later in its web page.";

static CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "docker",
        name: "Docker",
        summary: "Container engine and Compose, the base for the other add-ons.",
        kind: AddonKind::System,
        requires: &[],
        privileges: &[],
        needs_internet: false,
        approx_size_mib: 500,
        data_gib: 0,
        settings: &[],
    },
    CatalogEntry {
        id: "portainer",
        name: "Portainer",
        summary: "Web page to manage your containers, images and volumes.",
        kind: AddonKind::Compose,
        requires: &["docker"],
        privileges: &["docker-socket"],
        needs_internet: true,
        approx_size_mib: 300,
        data_gib: 0,
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
        settings: &[SettingSpec {
            key: "plex_claim",
            label: "Claim token (optional)",
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
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub addons: &'static [&'static str],
}

pub const PRESETS: &[Preset] = &[
    Preset { id: "docker-portainer", label: "Docker + Portainer", addons: &["docker", "portainer"] },
    Preset { id: "frigate", label: "Frigate NVR", addons: &["docker", "portainer", "frigate"] },
    Preset { id: "plex", label: "Plex", addons: &["docker", "portainer", "plex"] },
];

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

/// Things to know before starting: the Internet, privileges, disk use, offline limits.
pub fn warnings(choices: &[AddonChoice], ctx: &AddonContext) -> Vec<String> {
    let Ok(items) = resolve(choices) else { return vec![] };
    let mut w = vec![];
    if items.is_empty() {
        return w;
    }
    let net: Vec<&str> = items.iter().filter(|r| r.entry.needs_internet).map(|r| r.entry.name).collect();
    if !net.is_empty() {
        w.push(format!(
            "{} will pull container images from the Internet when the unit first boots. Without Internet the add-on shows \"needs attention\" in the web page; the installation itself does not fail.",
            net.join(", ")
        ));
    }
    if ctx.offline {
        w.push(
            "The offline bundle does not contain Docker: its packages are downloaded from the Debian mirrors on the first boot, so the unit needs Internet for that too."
                .into(),
        );
    }
    for r in &items {
        if !r.entry.privileges.is_empty() {
            w.push(format!("{} asks for: {}. {}", r.entry.name, r.entry.privileges.join(", "), privilege_note(r.entry.privileges)));
        }
    }
    let size_mib: u64 = items.iter().map(|r| r.entry.approx_size_mib as u64).sum();
    w.push(format!(
        "The add-ons need about {:.1} GiB on the system volume (Docker keeps images under /var/lib/docker).",
        size_mib as f64 / 1024.0
    ));
    if let Some(root) = ctx.root_mib {
        // leave room for the system itself, logs and updates
        if root < size_mib + 4096 {
            w.push(format!(
                "The system volume is only {} GiB: that is tight for the add-ons. Raise the system size in the storage settings.",
                root / 1024
            ));
        }
    }
    for r in items.iter().filter(|r| r.entry.data_gib > 0) {
        w.push(format!("{} stores its data on the data volume and wants at least {} GiB free there.", r.entry.name, r.entry.data_gib));
    }
    w
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

/// A secret value that stands in for the real one in dry runs and rendered artefacts.
pub const SECRET_PLACEHOLDER: &str = "DRYRUN-SECRET";

/// The same choices with every secret replaced by [`SECRET_PLACEHOLDER`] (dry runs).
pub fn redact_secrets(choices: &[AddonChoice]) -> Vec<AddonChoice> {
    choices
        .iter()
        .map(|c| AddonChoice { secrets: c.secrets.keys().map(|k| (k.clone(), SECRET_PLACEHOLDER.to_string())).collect(), ..c.clone() })
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
        assert_eq!(red[0].secrets["plex_claim"], SECRET_PLACEHOLDER);
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
        let w = warnings(&[choice("portainer"), choice("frigate")], &AddonContext { offline: false, root_mib: Some(20 * 1024) });
        let all = w.join("\n");
        assert!(all.contains("pull container images from the Internet"), "{all}");
        assert!(all.contains("docker-socket") && all.contains("administrator of the NAS"), "{all}");
        assert!(all.contains("at least 50 GiB"), "{all}");
        assert!(!all.contains("offline bundle"), "{all}");
        assert!(!all.contains("tight"), "{all}");
        let w = warnings(&[choice("frigate")], &AddonContext { offline: true, root_mib: Some(6 * 1024) }).join("\n");
        assert!(w.contains("offline bundle does not contain Docker") && w.contains("tight"), "{w}");
        // a Docker-only choice needs no Internet for images
        assert!(!warnings(&[choice("docker")], &AddonContext::default()).join("\n").contains("pull container images"));
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
    fn docker_package_names_are_plain_and_in_one_place() {
        assert!(DOCKER_PACKAGES.iter().all(|p| crate::config::valid_package_name(p)));
        assert!(DOCKER_PACKAGES.contains(&"docker.io") && DOCKER_PACKAGES.contains(&"docker-compose"));
    }
}
