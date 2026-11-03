//! The Add-ons tab of the Setup step: what is ticked, the typed install-time settings, and the lines other screens show
//! about them. Pure and tested; the screen itself is `screens/addons.rs`.
//!
//! The model is *picks*: what the person ticked. What those need (Docker) is added by the engine (`addons::resolve`),
//! so the Docker row is "on and required" while something ticked needs it, and goes away again when nothing does.
//! Secret settings (a Plex claim token) live only in their `Input` entities while the form is open; they are read when
//! the job is built, never stored in these picks, never in the command that "Copy command" shows (it names an
//! environment variable instead).

use std::collections::{BTreeMap, BTreeSet};
use synofurb_core::addons::{self, AddonChoice, AddonContext, Resolved, SettingSpec};

/// `(add-on id, setting key)`.
pub type Key = (&'static str, &'static str);

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct AddonPicks {
    /// Ticked by the person. Docker can be on without being in here: it is required by something else.
    pub picked: BTreeSet<&'static str>,
    /// Typed values of settings that are not secret. Empty or missing = the add-on's default.
    pub values: BTreeMap<Key, String>,
}

/// The catalog id behind a string (`&'static` so the maps need no allocation).
fn static_id(id: &str) -> Option<&'static str> {
    addons::entry(id).map(|e| e.id)
}

impl AddonPicks {
    pub fn is_picked(&self, id: &str) -> bool {
        self.picked.contains(id)
    }

    /// Tick or untick. Unticking drops the typed values of that add-on.
    pub fn set(&mut self, id: &str, on: bool) {
        let Some(id) = static_id(id) else { return };
        if on {
            self.picked.insert(id);
        } else {
            self.picked.remove(id);
            self.values.retain(|(a, _), _| *a != id);
        }
    }

    /// Replace the picks by a quick-setup preset (settings typed so far are kept for add-ons that stay).
    pub fn apply_preset(&mut self, preset: &addons::Preset) {
        self.picked = preset.addons.iter().filter_map(|i| static_id(i)).collect();
        let keep = self.picked.clone();
        self.values.retain(|(a, _), _| keep.contains(a));
    }

    pub fn set_value(&mut self, key: Key, v: &str) {
        if v.trim().is_empty() {
            self.values.remove(&key);
        } else {
            self.values.insert(key, v.to_string());
        }
    }

    /// The choices as the engine takes them: picked add-ons in catalog order, typed values, secrets read from `secrets`.
    pub fn choices(&self, secrets: &BTreeMap<Key, String>) -> Vec<AddonChoice> {
        addons::catalog()
            .iter()
            .filter(|e| self.picked.contains(e.id))
            .map(|e| {
                let mut c = AddonChoice::new(e.id);
                for s in e.settings {
                    let k: Key = (e.id, s.key);
                    if s.is_secret() {
                        if let Some(v) = secrets.get(&k).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                            c.secrets.insert(s.key.to_string(), v.to_string());
                        }
                    } else if let Some(v) = self.values.get(&k).map(|v| v.trim()).filter(|v| !v.is_empty()) {
                        c.settings.insert(s.key.to_string(), v.to_string());
                    }
                }
                c
            })
            .collect()
    }

    /// Everything that will be installed, in install order, with the reason Docker is there.
    pub fn effective(&self) -> Vec<Resolved> {
        addons::resolve(&self.choices(&BTreeMap::new())).unwrap_or_default()
    }

    pub fn is_on(&self, id: &str) -> bool {
        self.effective().iter().any(|r| r.entry.id == id)
    }

    /// Names of the ticked add-ons that need `id` (empty when nothing does).
    pub fn required_by(&self, id: &str) -> Vec<&'static str> {
        self.effective()
            .iter()
            .find(|r| r.entry.id == id)
            .map(|r| r.required_by.iter().filter_map(|i| addons::entry(i).map(|e| e.name)).collect())
            .unwrap_or_default()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    /// Problems with typed values, per field: `((add-on, key), message)`. Only picked add-ons are checked.
    pub fn field_errors(&self, secrets: &BTreeMap<Key, String>) -> Vec<(Key, String)> {
        let mut v = vec![];
        for e in addons::catalog().iter().filter(|e| self.picked.contains(e.id)) {
            for s in e.settings {
                let k: Key = (e.id, s.key);
                let typed = if s.is_secret() { secrets.get(&k) } else { self.values.get(&k) };
                if let Some(t) = typed
                    && let Err(err) = addons::check_setting(e.id, s.key, t)
                {
                    v.push((k, err.to_string()));
                }
            }
        }
        v
    }

    /// "Will be installed after first boot: Docker, Portainer, Frigate NVR" (None when nothing is ticked).
    pub fn confirm_line(&self) -> Option<String> {
        let names: Vec<&str> = self.effective().iter().map(|r| r.entry.name).collect();
        (!names.is_empty()).then(|| format!("{} {}", crate::ui::copy::CONF_ADDONS.short, names.join(", ")))
    }

    /// Does the first boot need the Internet for something chosen?
    pub fn needs_internet(&self) -> bool {
        self.effective().iter().any(|r| r.entry.needs_internet)
    }

    /// The `--addon` values of "Copy command": ticked add-ons only (the CLI adds Docker itself), typed values, and a secret as the
    /// name of an environment variable that must hold it. The secret itself is never written.
    pub fn cli_specs(&self, secrets: &BTreeMap<Key, String>) -> Vec<String> {
        self.choices(secrets).iter().map(cli_spec).collect()
    }

    /// Short lines for Quick setup: the preset the picks equal, if any.
    pub fn matching_preset(&self) -> Option<usize> {
        addons::PRESETS.iter().position(|p| p.addons.iter().copied().collect::<BTreeSet<_>>() == self.picked.iter().copied().collect())
    }
}

/// `plex:plex_claim=@env:PLEX_CLAIM`, `frigate:retain_days=14`, `docker`.
pub fn cli_spec(c: &AddonChoice) -> String {
    let mut parts: Vec<String> = c.settings.iter().map(|(k, v)| format!("{k}={v}")).collect();
    parts.extend(c.secrets.keys().map(|k| format!("{k}=@env:{}", k.to_ascii_uppercase())));
    if parts.is_empty() { c.id.clone() } else { format!("{}:{}", c.id, parts.join(",")) }
}

/// The one-line summary of an add-on as the tab shows it. The catalog says "the redundant data volume" for Frigate; that is only
/// true when the plan builds a redundant data array, so without one the word is dropped ("RAID 0" and "None" survive no failure).
pub fn display_summary(e: &addons::CatalogEntry, redundant: bool) -> String {
    if redundant { e.summary.to_string() } else { e.summary.replace("redundant data volume", "data volume") }
}

/// What the one info button of an add-on says, a line each: what it needs, what it is allowed to do, how big it is and how long the
/// first boot spends on it. Everything the tab used to spread over five tags and three buttons.
pub fn about_lines(e: &addons::CatalogEntry) -> Vec<String> {
    let mut v = vec![];
    let docker_names: Vec<&str> = e.requires.iter().filter_map(|r| addons::entry(r).map(|x| x.name)).collect();
    if !docker_names.is_empty() {
        v.push(format!("Needs {}, which is installed with it.", docker_names.join(" and ")));
    }
    if e.needs_internet {
        v.push(crate::ui::copy::ADDON_NEEDS_NET.tip.to_string());
    }
    for p in e.privileges {
        let it = crate::ui::screens::addons::privilege_item(p);
        v.push(format!("{}: {}", it.short, it.tip));
    }
    let size = if e.approx_size_mib >= 1000 { format!("{:.1} GB", e.approx_size_mib as f64 / 1000.0) } else { format!("{} MB", e.approx_size_mib) };
    v.push(format!("Download about {size}, about {} min on a 20 Mbit/s line.", addons::approx_minutes(e)));
    if e.data_gib > 0 {
        v.push(format!("Wants at least {} GiB free on the data volume.", e.data_gib));
    } else if e.needs_data_volume {
        v.push("Keeps its temporary files on the data volume.".to_string());
    }
    v
}

/// Is the data volume of the plan redundant (a disk may fail and the data is still there)? No plan, or no data array: no.
pub fn data_is_redundant(plan: Option<&synofurb_core::plan::Plan>) -> bool {
    plan.and_then(crate::ui::raid::survives).is_some_and(|n| n > 0)
}

/// Why an add-on cannot be ticked with this plan (None = it can). Frigate and Plex keep their data below `/srv/data`, which only
/// exists when the plan creates a data volume; Keep mode does not.
pub fn unavailable_reason(e: &addons::CatalogEntry, plan: Option<&synofurb_core::plan::Plan>) -> Option<String> {
    let has = plan.is_none_or(|p| p.has_data_volume());
    (e.needs_data_volume && !has).then(|| crate::ui::copy::ADDON_NEEDS_DATA.short.to_string())
}

/// The sentence for the Next button when something ticked cannot work with the plan: `Needs a data volume: Frigate NVR, Plex Media
/// Server. Keep mode does not create one.`
pub fn keep_mode_blocker(picks: &AddonPicks, plan: Option<&synofurb_core::plan::Plan>) -> Option<String> {
    let has = plan.is_none_or(|p| p.has_data_volume());
    let names = addons::needing_data_volume(&picks.choices(&BTreeMap::new()), has);
    (!names.is_empty()).then(|| crate::ui::copy::ADDON_KEEP_BLOCK.fmt(&[&names.join(", ")]))
}

/// Quick setup's radio label: the preset and everything it installs, so nothing arrives unnamed: `Plex: Docker, Portainer, Plex Media Server`.
pub fn preset_label(p: &addons::Preset) -> String {
    format!("{}: {}", p.label, addons::preset_names(p))
}

/// The line under the Quick setup add-ons choice when the chosen preset carries privileges: `Portainer controls Docker.`
pub fn preset_privilege_line(p: &addons::Preset) -> Option<String> {
    let c: Vec<AddonChoice> = p.addons.iter().map(|i| AddonChoice::new(i)).collect();
    privilege_line(&addons::consent_items(&c))
}

/// `Portainer controls Docker \u{b7} Plex Media Server uses NAS network`: the privileges in the words of the Add-ons tab.
pub fn privilege_line(items: &[addons::Consent]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    Some(
        items
            .iter()
            .map(|c| {
                let what: Vec<String> = c
                    .privileges
                    .iter()
                    .map(|p| synofurb_core::model::lower_first(crate::ui::screens::addons::privilege_item(p).short))
                    .collect();
                format!("{} {}", c.name, what.join(", "))
            })
            .collect::<Vec<_>>()
            .join(" \u{b7} "),
    )
}

/// Has the person accepted exactly the privileges that are on screen now? `acked` is the signature that was ticked.
pub fn consent_given(choices: &[AddonChoice], acked: Option<&str>) -> bool {
    let need = addons::consent_signature(choices);
    need.is_empty() || acked == Some(need.as_str())
}

/// The "what the Add-ons choice means" part of Confirm, built from the engine's own text so Quick setup and the Add-ons tab say
/// the same: names, the privileges to acknowledge, and the engine's warnings (Internet, disk use, data volume).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfirmModel {
    pub names: Vec<&'static str>,
    pub privileges: Vec<addons::Consent>,
    /// The lines in plain sight: privileges, a tight system volume, no data volume, no Docker in the offline bundle.
    pub loud: Vec<String>,
    /// The rest (Internet, sizes), one click away.
    pub details: Vec<String>,
}

pub fn confirm_model(choices: &[AddonChoice], ctx: &AddonContext) -> ConfirmModel {
    let Ok(all) = addons::resolve(choices) else { return ConfirmModel::default() };
    let (loud, details): (Vec<_>, Vec<_>) = addons::notes(choices, ctx).into_iter().partition(|(k, _)| k.is_loud());
    ConfirmModel {
        names: all.iter().map(|r| r.entry.name).collect(),
        privileges: addons::consent_items(choices),
        loud: loud.into_iter().map(|(_, t)| t).collect(),
        details: details.into_iter().map(|(_, t)| t).collect(),
    }
}

/// The per-add-on line of the Done screen: `about 2 min \u{b7} 300 MB`.
pub fn done_meta(a: &addons::AfterInstall) -> String {
    let size = if a.size_mib >= 1000 { format!("{:.1} GB", a.size_mib as f64 / 1000.0) } else { format!("{} MB", a.size_mib) };
    format!("about {} min \u{b7} {size}", a.minutes)
}

/// The catalog setting behind a key, if it exists.
pub fn setting(id: &str, key: &str) -> Option<&'static SettingSpec> {
    addons::entry(id)?.settings.iter().find(|s| s.key == key)
}

// ------------------------------------------------------------------------------------------------ the form (needs GPUI)

use super::app::App;
use gpui_kit::component::input::InputState;
use gpui_kit::{AppContext, Context, Entity, Subscription, Window};

/// What the Add-ons tab holds: the picks, and one `Input` per setting of a ticked add-on (made when first needed, because an
/// input needs a window). The typed text stays in the inputs; [`App::addon_state`] reads it when something needs the choices.
#[derive(Default)]
pub struct AddonsForm {
    pub picks: AddonPicks,
    pub inputs: BTreeMap<Key, Entity<InputState>>,
    subs: Vec<Subscription>,
}

impl App {
    /// Make the inputs of the settings of every ticked add-on (no-op for those that exist).
    pub fn ensure_addon_inputs(&mut self, window: &mut Window, cx: &mut Context<App>) {
        let want: Vec<(Key, &'static SettingSpec)> = addons::catalog()
            .iter()
            .filter(|e| self.addons.picks.is_picked(e.id))
            .flat_map(|e| e.settings.iter().map(move |s| ((e.id, s.key), s)))
            .collect();
        for (k, spec) in want {
            if self.addons.inputs.contains_key(&k) {
                continue;
            }
            let hint = if spec.is_secret() || spec.default.is_empty() { String::new() } else { spec.default.to_string() };
            let secret = spec.is_secret();
            let st = cx.new(|cx| {
                let s = InputState::new(window, cx).placeholder(hint);
                if secret { s.masked(true) } else { s }
            });
            self.addons.subs.push(cx.observe(&st, |_, _, cx| cx.notify()));
            self.addons.inputs.insert(k, st);
        }
    }

    /// The picks with the typed values filled in, and the secret values (kept apart so they cannot reach a command line).
    pub fn addon_state(&self, cx: &gpui_kit::App) -> (AddonPicks, BTreeMap<Key, String>) {
        let mut picks = self.addons.picks.clone();
        let mut secrets = BTreeMap::new();
        for (k, st) in &self.addons.inputs {
            let v = self.input_value(st, cx);
            if setting(k.0, k.1).is_some_and(|s| s.is_secret()) {
                secrets.insert(*k, v);
            } else {
                picks.set_value(*k, &v);
            }
        }
        (picks, secrets)
    }

    /// The choices for the job and the command line.
    pub fn addon_choices(&self, cx: &gpui_kit::App) -> Vec<AddonChoice> {
        let (picks, secrets) = self.addon_state(cx);
        picks.choices(&secrets)
    }

    /// Why the Add-ons tab cannot go on (None = fine).
    pub fn addons_blocker(&self, cx: &gpui_kit::App) -> Option<String> {
        let (picks, secrets) = self.addon_state(cx);
        if picks.is_empty() {
            return None;
        }
        if !self.sys.enable_mgmt {
            return Some(crate::ui::copy::ADDON_NEEDS_WEB.short.to_string());
        }
        // a pick that cannot work with the plan (Frigate or Plex in Keep mode) is a reason too, said before a field's error
        keep_mode_blocker(&picks, self.form.plan.as_ref()).or_else(|| picks.field_errors(&secrets).into_iter().next().map(|(_, m)| m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picks(ids: &[&str]) -> AddonPicks {
        let mut p = AddonPicks::default();
        for i in ids {
            p.set(i, true);
        }
        p
    }

    #[test]
    fn docker_is_on_and_required_while_something_needs_it() {
        let mut p = picks(&["portainer", "frigate"]);
        assert!(p.is_on("docker") && !p.is_picked("docker"));
        assert_eq!(p.required_by("docker"), ["Portainer", "Frigate NVR"]);
        p.set("portainer", false);
        assert_eq!(p.required_by("docker"), ["Frigate NVR"]);
        p.set("frigate", false);
        assert!(!p.is_on("docker") && p.is_empty(), "nothing needs Docker any more");
        // ticked on its own it stays, and nothing requires it
        p.set("docker", true);
        assert!(p.is_on("docker") && p.required_by("docker").is_empty());
        p.set("nope", true);
        assert_eq!(p.picked.len(), 1, "unknown ids are ignored");
    }

    #[test]
    fn choices_follow_the_catalog_order_and_carry_typed_values_only() {
        let mut p = picks(&["plex", "frigate"]);
        p.set_value(("frigate", "retain_days"), " 14 ");
        p.set_value(("portainer", "x"), "ignored");
        let mut secrets = BTreeMap::new();
        secrets.insert(("plex", "plex_claim"), " claim-abcdefgh12345 ".to_string());
        secrets.insert(("frigate", "retain_days"), "not a secret".to_string());
        let c = p.choices(&secrets);
        assert_eq!(c.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["frigate", "plex"]);
        assert_eq!(c[0].settings["retain_days"], "14");
        assert_eq!(c[1].secrets["plex_claim"], "claim-abcdefgh12345");
        assert!(c[1].settings.is_empty() && c[0].secrets.is_empty());
        addons::validate(&c).unwrap();
        // empty means default
        p.set_value(("frigate", "retain_days"), "  ");
        assert!(p.choices(&BTreeMap::new())[0].settings.is_empty());
    }

    #[test]
    fn unticking_forgets_that_add_ons_values() {
        let mut p = picks(&["frigate"]);
        p.set_value(("frigate", "retain_days"), "30");
        p.set("frigate", false);
        p.set("frigate", true);
        assert!(p.choices(&BTreeMap::new())[0].settings.is_empty());
    }

    #[test]
    fn typed_values_are_checked_per_field_and_only_for_ticked_add_ons() {
        let mut p = picks(&["frigate", "plex"]);
        assert!(p.field_errors(&BTreeMap::new()).is_empty());
        p.set_value(("frigate", "retain_days"), "400");
        let mut s = BTreeMap::new();
        s.insert(("plex", "plex_claim"), "oops".to_string());
        let e = p.field_errors(&s);
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e[0].0, ("frigate", "retain_days"));
        assert!(e[1].1.contains("claim-"), "{e:?}");
        p.set("frigate", false);
        assert_eq!(p.field_errors(&s).len(), 1, "an unticked add-on's leftovers are not checked");
    }

    #[test]
    fn presets_replace_the_picks() {
        let mut p = picks(&["plex"]);
        p.apply_preset(&addons::PRESETS[0]);
        assert_eq!(p.picked.iter().copied().collect::<Vec<_>>(), ["docker", "portainer"]);
        assert_eq!(p.matching_preset(), Some(0));
        p.apply_preset(&addons::PRESETS[1]);
        assert!(p.is_on("frigate") && p.matching_preset() == Some(1));
        p.set("plex", true);
        assert_eq!(p.matching_preset(), Some(3), "Frigate + Plex is a preset of its own");
        p.set("docker", false);
        assert_eq!(p.matching_preset(), None);
        for pr in addons::PRESETS {
            let mut q = AddonPicks::default();
            q.apply_preset(pr);
            addons::validate(&q.choices(&BTreeMap::new())).unwrap();
        }
    }

    #[test]
    fn the_copy_command_names_an_environment_variable_never_the_secret() {
        let mut p = picks(&["portainer", "frigate", "plex"]);
        p.set_value(("frigate", "retain_days"), "14");
        let mut s = BTreeMap::new();
        s.insert(("plex", "plex_claim"), "claim-SECRETVALUE1234".to_string());
        let specs = p.cli_specs(&s);
        assert_eq!(specs, ["portainer", "frigate:retain_days=14", "plex:plex_claim=@env:PLEX_CLAIM"]);
        assert!(!specs.join(" ").contains("SECRETVALUE"));
        // and the CLI parser accepts exactly what the GUI writes
        for sp in &specs {
            let parsed = crate::cli::addons_cmd::parse_addon_spec(sp).unwrap();
            assert!(addons::entry(&parsed.id).is_some());
        }
        assert!(picks(&[]).cli_specs(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn confirm_lines() {
        assert_eq!(picks(&[]).confirm_line(), None);
        let p = picks(&["portainer"]);
        let line = p.confirm_line().unwrap();
        assert!(line.starts_with("Will be installed after first boot:") && line.ends_with("Docker, Portainer"), "{line}");
        assert!(p.needs_internet());
        assert!(!picks(&["docker"]).needs_internet());
    }

    #[test]
    fn confirm_shows_the_engines_own_warnings_for_every_way_of_ticking() {
        // the same text whether Quick setup or the Add-ons tab ticked it: it is built from the choices, not from the screen
        let tab = picks(&["portainer", "plex"]).choices(&BTreeMap::new());
        let mut q = AddonPicks::default();
        q.apply_preset(&addons::PRESETS[2]);
        let quick = q.choices(&BTreeMap::new());
        let ctx = AddonContext { has_data_volume: Some(true), root_mib: Some(20 * 1024), ..Default::default() };
        let (a, b) = (confirm_model(&tab, &ctx), confirm_model(&quick, &ctx));
        assert_eq!(a, b);
        assert_eq!(a.names, ["Docker", "Portainer", "Plex Media Server"]);
        assert!(a.loud.join("\n").contains("administrator of the NAS"), "{:?}", a.loud);
        let rest = a.details.join("\n");
        assert!(rest.contains("Internet") && rest.contains("GiB on the system volume"), "{rest}");
        assert_eq!(a.privileges.iter().map(|c| c.id).collect::<Vec<_>>(), ["portainer", "plex"]);
        assert_eq!(
            privilege_line(&a.privileges).as_deref(),
            Some("Portainer controls Docker \u{b7} Plex Media Server uses NAS network"),
            "{:?}",
            privilege_line(&a.privileges)
        );
        // the offline source adds its own line, and a plan without a data volume adds the data-volume line
        let off = confirm_model(&quick, &AddonContext { offline: true, has_data_volume: Some(false), ..Default::default() });
        assert!(off.loud.iter().any(|w| w.contains("offline bundle")), "{:?}", off.loud);
        let keep = confirm_model(&picks(&["frigate"]).choices(&BTreeMap::new()), &AddonContext { has_data_volume: Some(false), ..Default::default() });
        assert!(keep.loud.iter().any(|w| w.contains("creates no data volume")), "{:?}", keep.loud);
        assert!(confirm_model(&[], &ctx).names.is_empty() && confirm_model(&[], &ctx).loud.is_empty());
    }

    #[test]
    fn quick_setup_names_what_each_preset_installs() {
        let labels: Vec<String> = addons::PRESETS.iter().map(preset_label).collect();
        assert_eq!(labels[0], "Docker + Portainer: Docker, Portainer");
        assert_eq!(labels[2], "Plex: Docker, Portainer, Plex Media Server", "Plex also installs Portainer, and the label says so");
        assert!(labels.iter().all(|l| l.contains("Portainer")), "every preset carries Portainer today, and none hides it");
        // the privilege line is there exactly for the presets that carry a privileged add-on
        for (p, l) in addons::PRESETS.iter().zip(&labels) {
            assert_eq!(preset_privilege_line(p).is_some(), l.contains("Portainer") || p.addons.contains(&"plex"), "{l}");
        }
        assert!(preset_privilege_line(&addons::PRESETS[2]).unwrap().contains("Portainer"));
    }

    #[test]
    fn the_redundant_wording_is_dropped_when_the_plan_is_not_redundant() {
        let frigate = addons::entry("frigate").unwrap();
        assert!(frigate.summary.contains("redundant data volume"));
        assert!(display_summary(frigate, true).contains("redundant data volume"));
        let plain = display_summary(frigate, false);
        assert!(!plain.contains("redundant") && plain.ends_with("go to the data volume."), "{plain}");
        // from a real plan: RAID 5 is redundant, RAID 0 and None are not, and no plan claims nothing
        use synofurb_core::plan::{DataRaid, PlanRequest, build_plan};
        let f = synofurb_core::inspect::parse_inspect(&synofurb_core::inspect::fixtures::ds918_two_volumes()).unwrap();
        let plan = |r: DataRaid| build_plan(&f, &PlanRequest { data_raid: r, ..Default::default() });
        assert!(data_is_redundant(Some(&plan(DataRaid::Raid5))));
        assert!(!data_is_redundant(Some(&plan(DataRaid::Raid0))), "RAID 0 survives no failure");
        assert!(!data_is_redundant(Some(&plan(DataRaid::None))));
        assert!(!data_is_redundant(None));
    }

    #[test]
    fn data_hungry_add_ons_are_blocked_in_keep_mode_with_a_visible_reason() {
        use synofurb_core::plan::{PlanRequest, StorageMode, build_plan};
        let f = synofurb_core::inspect::parse_inspect(&synofurb_core::inspect::fixtures::ds918_two_volumes()).unwrap();
        let wipe = build_plan(&f, &PlanRequest::default());
        let keep = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(wipe.has_data_volume() && !keep.has_data_volume(), "the rule reads the plan, as the installer does");
        let (frigate, plex, portainer) =
            (addons::entry("frigate").unwrap(), addons::entry("plex").unwrap(), addons::entry("portainer").unwrap());
        assert!(unavailable_reason(frigate, Some(&keep)).is_some() && unavailable_reason(plex, Some(&keep)).is_some());
        assert!(unavailable_reason(portainer, Some(&keep)).is_none(), "Portainer keeps nothing below /srv/data");
        assert!(unavailable_reason(frigate, Some(&wipe)).is_none() && unavailable_reason(frigate, None).is_none());
        // Next explains itself when such an add-on is already ticked
        let p = picks(&["frigate", "plex", "portainer"]);
        let why = keep_mode_blocker(&p, Some(&keep)).unwrap();
        assert_eq!(why, "Needs a data volume: Frigate NVR, Plex Media Server. Keep mode does not create one.");
        assert!(keep_mode_blocker(&p, Some(&wipe)).is_none());
        assert!(keep_mode_blocker(&picks(&["portainer"]), Some(&keep)).is_none());
    }

    #[test]
    fn the_info_button_carries_everything_the_tags_used_to() {
        let l = about_lines(addons::entry("portainer").unwrap()).join("\n");
        assert!(l.contains("Needs Docker") && l.contains("Internet") && l.contains("Controls Docker"), "{l}");
        assert!(l.contains("Download about 300 MB, about 2 min"), "{l}");
        let f = about_lines(addons::entry("frigate").unwrap()).join("\n");
        assert!(f.contains("3.0 GB") && f.contains("at least 50 GiB free on the data volume"), "{f}");
        assert!(about_lines(addons::entry("docker").unwrap()).iter().all(|x| !x.contains("Internet")));
    }

    #[test]
    fn the_done_line_says_how_long_and_how_big() {
        let a = addons::after_install(&[AddonChoice::new("frigate")], "192.168.1.20");
        let frigate = a.iter().find(|x| x.id == "frigate").unwrap();
        assert_eq!(done_meta(frigate), "about 20 min \u{b7} 3.0 GB");
        let p = addons::after_install(&[AddonChoice::new("portainer")], "192.168.1.20");
        assert_eq!(done_meta(p.iter().find(|x| x.id == "portainer").unwrap()), "about 2 min \u{b7} 300 MB");
    }

    #[test]
    fn every_catalog_setting_has_a_gui_description() {
        for e in addons::catalog() {
            for s in e.settings {
                assert!(crate::ui::screens::addons::setting_item(e.id, s.key).is_some(), "{}.{} has no copy item", e.id, s.key);
            }
        }
    }
}
