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
}

/// A `health --json` document as the unit's management plane would print it (sample data).
pub fn demo_health_json(kind: DemoHealth) -> String {
    let (overall, md2_state, md2_degraded, sync, sdc_smart, svc_down, warnings) = match kind {
        DemoHealth::Ok | DemoHealth::Unreachable => ("ok", "clean", false, "null", "passed", false, "[]"),
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
}
