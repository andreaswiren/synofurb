//! Manage a converted unit: demo health reports (parsed by the real `synofurb_core::manage::parse_health`) and the
//! mapping from a report to the risk Tag's health state. Pure, tested.

use super::facts::Health as RiskHealth;
use synofurb_core::manage::{Health, Overall, parse_health};

/// Which demo report to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DemoHealth {
    #[default]
    Ok,
    Rebuild,
    FailedDisk,
    ServicesDown,
    Unreachable,
    /// The page has no sign-in for this unit yet (nothing was read).
    NeedsSignIn,
}

/// A `health --json` document as the unit's management plane would print it (sample data).
pub fn demo_health_json(kind: DemoHealth) -> String {
    let (overall, md2_state, md2_degraded, sync, sdc_smart, svc_down, warnings) = match kind {
        DemoHealth::Ok | DemoHealth::Unreachable | DemoHealth::NeedsSignIn => ("ok", "clean", false, "null", "passed", false, "[]"),
        DemoHealth::Rebuild => ("warn", "active", false, "24.0", "passed", false, r#"["md2 rebuilding 24 %"]"#),
        DemoHealth::FailedDisk => ("crit", "degraded", true, "null", "failed", false, r#"["sdc failed","md2 degraded"]"#),
        DemoHealth::ServicesDown => ("warn", "clean", false, "null", "passed", true, r#"["2 services down"]"#),
    };
    let action = if sync == "null" { "\"\"" } else { "\"recovery\"" };
    let svc = |n: &str, down: bool| format!(r#"{{"name":"{n}","active":{}}}"#, !down);
    format!(
        r#"{{"schema":1,"version":"0.9.1","hostname":"attic","uptime_s":8040,"load":[0.12,0.10,0.05],
"mem":{{"total":8000000000,"avail":6000000000}},"os":"Debian GNU/Linux 13 (trixie)","kernel":"6.12.57+deb13-amd64","root_fs":{{"used_pct":41}},
"arrays":[{{"name":"md2","level":"raid5","state":"{md2_state}","degraded":{md2_degraded},"sync_pct":{sync},"sync_action":{action}}},
          {{"name":"md3","level":"raid1","state":"clean","degraded":false,"sync_pct":null,"sync_action":""}}],
"disks":[{{"dev":"/dev/sda","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":37}},
         {{"dev":"/dev/sdb","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":36}},
         {{"dev":"/dev/sdc","model":"WDC WD40EFRX","size":4000787030016,"smart":"{sdc_smart}","temp_c":38}},
         {{"dev":"/dev/sdd","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":37}}],
"volumes":[{{"mount":"/","used_pct":41,"fstype":"ext4"}},{{"mount":"/srv/data","used_pct":16,"fstype":"ext4"}}],
"services":[{},{},{},{},{}],
"boot":{{"default":"debian","flash_present":true}},"updates":{{"apt_pending":12,"security_pending":3}},
"overall":"{overall}","warnings":{warnings}}}"#,
        svc("ssh", false),
        svc("synofurb-mgmt", svc_down),
        svc("smartd", false),
        svc("mdadm", svc_down),
        svc("unattended-upgrades", false),
    )
}

pub fn demo_health(kind: DemoHealth) -> Health {
    parse_health(&demo_health_json(kind)).expect("demo health parses")
}

/// The risk Tag's health state for a report (`None` = the unit did not answer).
pub fn risk_health(h: Option<&Health>, services_fixed: bool) -> RiskHealth {
    let Some(h) = h else { return RiskHealth::Unreachable };
    let s = h.summary();
    // the unit's own verdict decides the tag; pending security updates alone never turn it into "Needs attention"
    match h.overall {
        Overall::Crit => RiskHealth::FailedDisk,
        Overall::Warn if s.rebuilding_arrays > 0 => RiskHealth::Rebuild,
        Overall::Warn if s.services_down > 0 && !services_fixed => RiskHealth::ServiceDown,
        Overall::Warn if s.services_down > 0 => RiskHealth::Ok,
        Overall::Warn => RiskHealth::Rebuild,
        _ => RiskHealth::Ok,
    }
}

/// The management update: allowed with a warning while an array is degraded (owner decision), blocked while updating
/// or when the unit does not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateGate {
    Allowed,
    AllowedWithWarning(&'static str),
    Blocked(&'static str),
}

pub fn update_gate(h: Option<&Health>, updating: bool, up_to_date: bool) -> UpdateGate {
    if h.is_none() {
        return UpdateGate::Blocked("Unit unreachable.");
    }
    if updating {
        return UpdateGate::Blocked("Updating now.");
    }
    if up_to_date {
        return UpdateGate::Blocked("Already up to date.");
    }
    match h.map(|h| h.summary()) {
        Some(s) if s.degraded_arrays > 0 || s.failed_disks > 0 => UpdateGate::AllowedWithWarning(
            "An array is degraded. The update restarts only the web UI, but a second failure now could lose data.",
        ),
        _ => UpdateGate::Allowed,
    }
}

// ------------------------------------------------------------------------------------------------ which unit Manage talks to

/// The credential typed on the Manage page for ONE unit (never the Connect form's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManageSecret {
    Password(String),
    /// The text of an OpenSSH private key.
    Key(String),
}

/// The SSH target of the unit shown on the Manage page: its own address and the sign-in typed for it. The Connect form is never
/// an input here, so Health, Update and Roll back can only reach the unit on screen.
pub fn manage_target_for(
    addr: Option<std::net::IpAddr>,
    user: &str,
    secret: &ManageSecret,
) -> Result<synofurb_core::session::SshTarget, String> {
    use synofurb_core::remote::Auth;
    let Some(addr) = addr else { return Err("This unit has no address; scan again.".into()) };
    let user = user.trim();
    if user.is_empty() {
        return Err("Enter the user of this NAS.".into());
    }
    let (auth, sudo) = match secret {
        ManageSecret::Password(p) if p.is_empty() => return Err("Enter the password of this NAS.".into()),
        ManageSecret::Password(p) => (Auth::Password(p.clone()), Some(p.clone())),
        ManageSecret::Key(k) if k.trim().is_empty() => return Err("The key file is empty.".into()),
        ManageSecret::Key(k) => (Auth::Key(k.clone()), None),
    };
    Ok(synofurb_core::session::SshTarget { host: addr.to_string(), port: 22, user: user.to_string(), auth, sudo_password: sudo })
}

/// Which unit the Manage page is bound to. Binding another unit forgets the sign-in, the report and every result of the
/// previous one: nothing carries over.
#[derive(Default, Debug)]
pub struct ManageBinding {
    unit: Option<String>,
}

impl ManageBinding {
    /// `true` when `unit` is not the bound unit (so the caller must reset the page state); binds it.
    pub fn bind(&mut self, unit: &str) -> bool {
        if self.unit.as_deref() == Some(unit) {
            return false;
        }
        self.unit = Some(unit.to_string());
        true
    }
}

/// The key a unit is bound by: its serial number, else its address.
pub fn unit_key(serial: &str, addr: Option<std::net::IpAddr>) -> String {
    if serial.is_empty() { addr.map(|a| a.to_string()).unwrap_or_default() } else { serial.to_string() }
}

/// Does a worker result still belong on the page? `msg_gen` is the unit context the work was started for, `now_gen` the current
/// one (bumped whenever another unit is opened or selected or the session starts over); a Manage result also needs Manage to be
/// the page (or Recovery opened from it). A result that fails this is dropped: it must never change another unit's page.
pub fn accepts_result(msg_gen: u64, now_gen: u64, manage_result: bool, manage_page: bool) -> bool {
    msg_gen == now_gen && (!manage_result || manage_page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_reports_parse_and_map_to_the_risk_states() {
        let ok = demo_health(DemoHealth::Ok);
        assert_eq!(ok.overall, Overall::Ok);
        assert_eq!(risk_health(Some(&ok), false), RiskHealth::Ok);
        let rb = demo_health(DemoHealth::Rebuild);
        assert!(rb.arrays.iter().any(|a| a.rebuilding()));
        assert_eq!(risk_health(Some(&rb), false), RiskHealth::Rebuild);
        let fd = demo_health(DemoHealth::FailedDisk);
        assert_eq!(risk_health(Some(&fd), false), RiskHealth::FailedDisk);
        let sv = demo_health(DemoHealth::ServicesDown);
        assert_eq!(risk_health(Some(&sv), false), RiskHealth::ServiceDown);
        assert_eq!(risk_health(Some(&sv), true), RiskHealth::Ok, "restarted services clear the warning");
        assert_eq!(risk_health(None, false), RiskHealth::Unreachable);
    }

    #[test]
    fn update_is_allowed_with_a_warning_when_degraded() {
        let ok = demo_health(DemoHealth::Ok);
        assert_eq!(update_gate(Some(&ok), false, false), UpdateGate::Allowed);
        assert!(matches!(update_gate(Some(&demo_health(DemoHealth::FailedDisk)), false, false), UpdateGate::AllowedWithWarning(_)));
        assert!(matches!(update_gate(Some(&ok), true, false), UpdateGate::Blocked("Updating now.")));
        assert!(matches!(update_gate(Some(&ok), false, true), UpdateGate::Blocked("Already up to date.")));
        assert!(matches!(update_gate(None, false, false), UpdateGate::Blocked("Unit unreachable.")));
    }

    #[test]
    fn manage_actions_target_the_unit_on_screen_never_the_connect_form() {
        let a: std::net::IpAddr = "192.168.1.60".parse().unwrap();
        let b: std::net::IpAddr = "192.168.1.61".parse().unwrap();
        let pw = ManageSecret::Password("pw".into());
        let ta = manage_target_for(Some(a), "nasadmin", &pw).unwrap();
        let tb = manage_target_for(Some(b), "nasadmin", &pw).unwrap();
        assert_eq!((ta.host.as_str(), tb.host.as_str()), ("192.168.1.60", "192.168.1.61"));
        assert_eq!(ta.user, "nasadmin");
        // a missing credential is an error to show ("Sign in"), not a target
        assert!(manage_target_for(Some(a), "", &pw).is_err());
        assert!(manage_target_for(Some(a), "u", &ManageSecret::Password(String::new())).is_err());
        assert!(manage_target_for(None, "u", &pw).is_err());
        assert!(manage_target_for(Some(a), "u", &ManageSecret::Key("  ".into())).is_err());
    }

    #[test]
    fn binding_another_unit_forgets_everything_of_the_first() {
        let mut b = ManageBinding::default();
        assert!(b.bind("A"), "first bind resets");
        assert!(!b.bind("A"), "the same unit keeps its state");
        assert!(b.bind("B"), "another unit starts clean");
        assert!(b.bind("A"), "and so does going back: no credential survives a switch");
        assert_eq!(unit_key("SN1", None), "SN1");
        assert_eq!(unit_key("", "10.0.0.2".parse().ok()), "10.0.0.2");
    }

    #[test]
    fn a_result_of_unit_a_never_lands_on_unit_b() {
        // start on A (epoch 1), a read is under way; the user opens B (epoch 2); A's answer arrives
        let a_gen = 1;
        let b_gen = 2;
        assert!(accepts_result(a_gen, a_gen, true, true), "A's answer while A is on screen");
        assert!(!accepts_result(a_gen, b_gen, true, true), "A's answer after B was opened is dropped");
        assert!(!accepts_result(a_gen, b_gen, false, false));
        // the user left Manage: a Manage result is dropped even for the same unit
        assert!(!accepts_result(b_gen, b_gen, true, false));
        // a recovery result needs no Manage page
        assert!(accepts_result(b_gen, b_gen, false, false));
    }
}
