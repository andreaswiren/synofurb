use super::*;
use crate::remote::mock::MockShell;

// ---------------------------------------------------------------------------------------------- fixtures

/// What `health --json` prints on a healthy unit (shape of apps/mgmt-plane/synofurb_mgmt.py).
const HEALTHY: &str = r#"{"schema":1,"version":"0.4.0","hostname":"nas","uptime_s":273600,"load":[0.12,0.1,0.05],
"mem":{"total":8000000000,"avail":6000000000},"os":"Debian GNU/Linux 13 (trixie)","kernel":"6.12.41+deb13-amd64",
"root_fs":{"used_pct":23},
"arrays":[{"name":"md2","level":"raid5","state":"clean","degraded":false,"sync_pct":null,"sync_action":null}],
"disks":[{"dev":"/dev/sda","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":34},
         {"dev":"/dev/sdb","model":"WDC WD40EFRX","size":4000787030016,"smart":"passed","temp_c":35}],
"volumes":[{"mount":"/srv/data","used_pct":41,"fstype":"ext4"}],
"services":[{"name":"synofurb-mgmt","active":true},{"name":"ssh","active":true}],
"boot":{"default":null,"flash_present":false},"updates":{"apt_pending":4,"security_pending":0},
"overall":"ok","warnings":[]}"#;

const DEGRADED: &str = r#"{"schema":1,"version":"0.4.0","hostname":"nas","uptime_s":100,"load":[1,1,1],
"mem":{"total":8000000000,"avail":6000000000},"root_fs":{"used_pct":30},
"arrays":[{"name":"md2","level":"raid5","state":"degraded","degraded":true,"sync_pct":42.5,"sync_action":"recovery"},
          {"name":"md0","level":"raid1","state":"clean","degraded":false,"sync_pct":null,"sync_action":null}],
"disks":[],"volumes":[],"services":[{"name":"synofurb-mgmt","active":true}],
"overall":"crit","warnings":["md2 degraded"]}"#;

const SMART_FAILED: &str = r#"{"schema":1,"version":"0.4.0","overall":"crit","warnings":["sdb SMART failed"],
"disks":[{"dev":"/dev/sda","model":"A","size":1,"smart":"passed","temp_c":30},
         {"dev":"/dev/sdb","model":"B","size":1,"smart":"failed","temp_c":71},
         {"dev":"/dev/sdc","model":"C","size":1,"smart":"unknown","temp_c":null}]}"#;

const SERVICES_DOWN: &str = r#"{"schema":1,"version":"0.4.0","overall":"warn","warnings":["ssh not running","smbd not running"],
"services":[{"name":"synofurb-mgmt","active":true},{"name":"ssh","active":false},{"name":"smbd","active":"inactive"},
            {"name":"nfs","active":"failed"},{"name":"avahi","active":false}]}"#;

fn health(json: &str) -> Health {
    parse_health(json).unwrap_or_else(|e| panic!("{e}\n{json}"))
}

fn json_with(version: &str, overall: &str) -> String {
    format!(r#"{{"schema":1,"version":"{version}","hostname":"nas","overall":"{overall}","warnings":[]}}"#)
}

// ---------------------------------------------------------------------------------------------- parsing

#[test]
fn a_healthy_report_parses_completely() {
    let h = health(HEALTHY);
    assert_eq!((h.schema, h.version.as_str(), h.overall), (1, "0.4.0", Overall::Ok));
    assert_eq!((h.hostname.as_str(), h.uptime_s, h.load.clone()), ("nas", Some(273600), vec![0.12, 0.1, 0.05]));
    assert_eq!(h.mem.total, Some(8_000_000_000));
    assert_eq!(h.mem.used_pct().map(|p| p.round() as i64), Some(25));
    assert_eq!(h.os, "Debian GNU/Linux 13 (trixie)");
    assert_eq!(h.root_fs.used_pct, Some(23.0));
    assert_eq!((h.arrays.len(), h.arrays[0].name.as_str(), h.arrays[0].sync_pct, h.arrays[0].sync_action.as_str()), (1, "md2", None, ""));
    assert_eq!(
        (h.disks.len(), h.disks[0].smart, h.disks[0].temp_c, h.disks[0].size),
        (2, Smart::Passed, Some(34.0), Some(4_000_787_030_016))
    );
    assert_eq!((h.volumes[0].mount.as_str(), h.volumes[0].used_pct, h.volumes[0].fstype.as_str()), ("/srv/data", Some(41.0), "ext4"));
    assert_eq!(h.services.len(), 2);
    assert_eq!((h.boot.default.as_str(), h.boot.flash_present), ("", Some(false)));
    assert_eq!((h.updates.apt_pending, h.updates.security_pending), (Some(4), Some(0)));
    assert!(h.parse_notes.is_empty(), "{:?}", h.parse_notes);
    // and round-trips through serde (the desktop app may cache it)
    let again: Health = serde_json::from_value(serde_json::to_value(&h).unwrap()).unwrap();
    assert_eq!(again, h);
}

#[test]
fn only_schema_version_and_overall_are_required() {
    let h = health(r#"{"schema":1,"version":"0.4.0","overall":"ok"}"#);
    assert!(h.arrays.is_empty() && h.disks.is_empty() && h.services.is_empty() && h.warnings.is_empty());
    assert_eq!((h.uptime_s, h.mem.total, h.root_fs.used_pct, h.boot.flash_present, h.updates.apt_pending), (None, None, None, None, None));
    for (doc, what) in [
        (r#"{"version":"0.4.0","overall":"ok"}"#, "schema"),
        (r#"{"schema":0,"version":"0.4.0","overall":"ok"}"#, "schema"),
        (r#"{"schema":1,"overall":"ok"}"#, "version"),
        (r#"{"schema":1,"version":"","overall":"ok"}"#, "version"),
        (r#"{"schema":1,"version":"0.4.0"}"#, "overall"),
        (r#"{"schema":1,"version":"0.4.0","overall":7}"#, "overall"),
    ] {
        match parse_health(doc) {
            Err(ManageError::BadOutput(m)) => assert!(m.contains(what), "{doc}: {m}"),
            other => panic!("{doc}: {other:?}"),
        }
    }
    assert!(matches!(parse_health("[1,2]"), Err(ManageError::BadOutput(_))));
    assert!(matches!(parse_health("not json at all"), Err(ManageError::BadOutput(_))));
    assert!(matches!(parse_health(""), Err(ManageError::BadOutput(_))));
}

#[test]
fn nulls_wrong_types_and_unknown_fields_are_tolerated() {
    // what the real script prints when a part failed: nulls for the section, an "x unavailable" warning
    let h = health(
        r#"{"schema":1,"version":"0.4.0","overall":"warn","hostname":"nas","uptime_s":null,"load":null,"mem":null,"os":null,"kernel":null,
        "root_fs":null,"arrays":null,"disks":null,"volumes":null,"services":null,"boot":null,"updates":null,
        "warnings":["updates unavailable"],"something_new":{"a":1}}"#,
    );
    assert_eq!((h.overall, h.warnings.clone()), (Overall::Warn, vec!["updates unavailable".to_string()]));
    assert!(h.parse_notes.is_empty());

    let h = health(
        r#"{"schema":1,"version":"0.4.0","overall":"ok","mem":"lots","arrays":[{"name":"md0","degraded":"no-idea"},"junk",{"name":7,"sync_pct":"12.5"}],
        "disks":{"not":"a list"},"uptime_s":"90","load":[1,"2",null],"services":[{"name":"ssh","active":"running"},{"name":"x"}],
        "root_fs":{"used_pct":"87%"},"updates":{"apt_pending":3.0,"security_pending":"2"}}"#,
    );
    assert_eq!(h.mem, Memory::default(), "a section of the wrong type is dropped");
    assert_eq!(h.arrays.len(), 2, "a junk element is skipped, the others kept");
    assert_eq!((h.arrays[0].name.as_str(), h.arrays[0].degraded), ("md0", false));
    assert_eq!((h.arrays[1].name.as_str(), h.arrays[1].sync_pct), ("7", Some(12.5)));
    assert!(h.disks.is_empty());
    assert_eq!((h.uptime_s, h.load.clone()), (Some(90), vec![1.0, 2.0]));
    assert_eq!((h.services[0].active, h.services[1].active), (Some(true), None));
    assert_eq!(h.root_fs.used_pct, Some(87.0));
    assert_eq!((h.updates.apt_pending, h.updates.security_pending), (Some(3), Some(2)));
    let notes = h.parse_notes.join("|");
    assert!(notes.contains("mem") && notes.contains("arrays") && notes.contains("disks"), "{notes}");
}

#[test]
fn verdict_words_and_future_schemas_are_handled() {
    for (w, want) in [
        ("ok", Overall::Ok),
        ("OK", Overall::Ok),
        ("warn", Overall::Warn),
        ("warning", Overall::Warn),
        ("crit", Overall::Crit),
        ("critical", Overall::Crit),
        ("banana", Overall::Unknown),
    ] {
        assert_eq!(Overall::parse(w), want, "{w}");
    }
    assert!(Overall::Crit > Overall::Warn && Overall::Warn > Overall::Ok && Overall::Ok > Overall::Unknown);
    let h = health(r#"{"schema":2,"version":"1.0.0","overall":"ok","brand_new":[1]}"#);
    assert_eq!(h.schema, 2);
    assert!(h.parse_notes.iter().any(|n| n.contains("newer")), "{:?}", h.parse_notes);
    assert_eq!(serde_json::to_value(Overall::Crit).unwrap(), "crit");
    assert_eq!(serde_json::to_value(Smart::Failed).unwrap(), "failed");
}

#[test]
fn noise_around_the_document_is_ignored() {
    let noisy = format!("Warning: something harmless\n{}\n", HEALTHY.replace('\n', ""));
    assert_eq!(health(&noisy).version, "0.4.0");
    let compact = serde_json::to_string(&serde_json::from_str::<Value>(HEALTHY).unwrap()).unwrap();
    assert_eq!(health(&format!("Banner\n{compact}")).hostname, "nas");
}

#[test]
fn health_results_are_classified() {
    let r = |code, stdout: &str, stderr: &str| ExecResult { code, stdout: stdout.into(), stderr: stderr.into() };
    assert_eq!(interpret_health_result(&r(0, HEALTHY, "")).unwrap().overall, Overall::Ok);
    // a non-zero exit with a valid document still counts (a future "exit 1 on warn")
    assert_eq!(interpret_health_result(&r(1, HEALTHY, "")).unwrap().version, "0.4.0");
    assert!(matches!(
        interpret_health_result(&r(
            2,
            "",
            "python3: can't open file '/opt/synofurb-mgmt/synofurb_mgmt.py': [Errno 2] No such file or directory"
        )),
        Err(ManageError::NotInstalled)
    ));
    assert!(matches!(interpret_health_result(&r(127, "", "sh: 1: python3: not found")), Err(ManageError::Command { code: 127, .. })));
    assert!(matches!(
        interpret_health_result(&r(
            2,
            "",
            "usage: synofurb_mgmt.py [-h] ...\nsynofurb_mgmt.py: error: unrecognized arguments: health --json"
        )),
        Err(ManageError::HealthUnsupported { .. })
    ));
    assert!(matches!(
        interpret_health_result(&r(2, "", "synofurb_mgmt.py: error: argument command: invalid choice: 'health'")),
        Err(ManageError::HealthUnsupported { .. })
    ));
    assert!(matches!(interpret_health_result(&r(1, "", "Traceback ...\nValueError")), Err(ManageError::Command { code: 1, .. })));
    assert!(matches!(interpret_health_result(&r(0, "<html>", "")), Err(ManageError::BadOutput(_))));
    let msg = ManageError::HealthUnsupported { installed_version: Some("0.2.0".into()) }.to_string();
    assert!(msg.contains("0.2.0") && msg.contains("manage update"), "{msg}");
}

#[test]
fn fetch_health_with_the_mock_shell_runs_privileged_and_without_secrets() {
    let mut m = MockShell::default();
    m.push_ok(HEALTHY);
    {
        let mut p = Privileged { sh: &mut m, is_root: false, sudo_password: Some("s3cretpw".into()) };
        let h = fetch_health(&mut p).unwrap();
        assert_eq!(h.hostname, "nas");
    }
    assert!(m.commands[0].starts_with("sudo -S -p ''"), "{}", m.commands[0]);
    assert!(m.commands[0].contains("synofurb_mgmt.py health --json"));
    assert!(!m.commands[0].contains("s3cretpw"));
    assert_eq!(m.stdins[0].as_deref(), Some(&b"s3cretpw\n"[..]));
    // root: no sudo
    let mut m = MockShell::default();
    m.push_ok(HEALTHY);
    fetch_health(&mut Privileged { sh: &mut m, is_root: true, sudo_password: None }).unwrap();
    assert_eq!(m.commands[0], format!("python3 {MGMT_SCRIPT} health --json"));
    // not root and no password: the unprivileged part of the report, never a failure about sudo
    let mut m = MockShell::default();
    m.push_ok(HEALTHY);
    fetch_health(&mut Privileged { sh: &mut m, is_root: false, sudo_password: None }).unwrap();
    assert_eq!(m.commands[0], format!("python3 {MGMT_SCRIPT} health --json"));
    assert_eq!(m.stdins[0], None);
}

// ---------------------------------------------------------------------------------------------- summary

#[test]
fn a_healthy_unit_has_an_empty_summary() {
    let s = health(HEALTHY).summary();
    assert_eq!((s.overall, s.headline.as_str(), s.problems), (Overall::Ok, "Healthy", 0));
    assert!(s.bullets.is_empty());
    assert_eq!((s.degraded_arrays, s.failed_disks, s.services_down, s.security_updates, s.pending_updates), (0, 0, 0, 0, 4));
}

#[test]
fn a_degraded_array_is_critical_and_says_so_briefly() {
    let s = health(DEGRADED).summary();
    assert_eq!((s.overall, s.headline.as_str()), (Overall::Crit, "Problem"));
    assert_eq!((s.degraded_arrays, s.rebuilding_arrays, s.problems), (1, 1, 1));
    assert_eq!(s.bullets.len(), 1, "the NAS's own warning repeats this and must not be listed twice: {:?}", s.bullets);
    assert_eq!(s.bullets[0], Bullet { level: Overall::Crit, text: "md2 is degraded, recovery 43%.".into() });
}

#[test]
fn a_failed_smart_check_is_critical_and_a_hot_disk_follows_the_nas_thresholds() {
    let s = health(SMART_FAILED).summary();
    assert_eq!((s.overall, s.failed_disks, s.problems), (Overall::Crit, 1, 1));
    assert_eq!(s.bullets[0].text, "sdb failed its SMART check; replace it.");
    // 62 C is a warning, 71 C on a disk whose SMART passed is critical
    let hot = health(r#"{"schema":1,"version":"1","overall":"ok","disks":[{"dev":"/dev/sda","smart":"passed","temp_c":62},{"dev":"/dev/sdb","smart":"passed","temp_c":71}]}"#).summary();
    assert_eq!(hot.overall, Overall::Crit);
    assert_eq!(
        hot.bullets.iter().map(|b| (b.level, b.text.as_str())).collect::<Vec<_>>(),
        vec![(Overall::Crit, "sdb is hot (71 °C)."), (Overall::Warn, "sda is hot (62 °C).")]
    );
}

#[test]
fn services_that_are_down_are_counted_and_grouped() {
    let s = health(SERVICES_DOWN).summary();
    assert_eq!((s.overall, s.services_down, s.problems), (Overall::Warn, 4, 1));
    assert_eq!(s.bullets[0].text, "4 services are not running: ssh, smbd, nfs and 1 more.");
    let one =
        health(r#"{"schema":1,"version":"1","overall":"warn","services":[{"name":"ssh","active":false},{"name":"x","active":true}]}"#)
            .summary();
    assert_eq!((one.services_down, one.bullets[0].text.as_str()), (1, "Service ssh is not running."));
}

#[test]
fn the_derived_verdict_can_raise_but_never_lower_the_nas_verdict() {
    // the NAS says ok but a volume is 96 % full
    let s = health(r#"{"schema":1,"version":"1","overall":"ok","volumes":[{"mount":"/srv/data","used_pct":96}]}"#).summary();
    assert_eq!((s.overall, s.reported, s.volumes_nearly_full), (Overall::Crit, Overall::Ok, 1));
    assert_eq!(s.bullets[0].text, "/srv/data is 96% full.");
    // the NAS says crit about something the numbers do not show: its sentences are shown
    let s = health(r#"{"schema":1,"version":"1","overall":"crit","warnings":["raid controller offline","fan stopped"]}"#).summary();
    assert_eq!(s.overall, Overall::Crit);
    assert_eq!(s.bullets.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(), vec!["raid controller offline", "fan stopped"]);
    // crit with no explanation at all still gets one line
    let s = health(r#"{"schema":1,"version":"1","overall":"crit"}"#).summary();
    assert_eq!(s.bullets.len(), 1);
    assert!(s.bullets[0].text.contains("crit"));
    // 85 % is a warning, 84 % nothing; the system disk is named
    let s = health(r#"{"schema":1,"version":"1","overall":"ok","root_fs":{"used_pct":85},"volumes":[{"mount":"/mnt/a","used_pct":84}]}"#)
        .summary();
    assert_eq!((s.overall, s.bullets.len(), s.bullets[0].text.as_str()), (Overall::Warn, 1, "The system disk is 85% full."));
    // an unknown verdict word is surfaced, not trusted
    let s = health(r#"{"schema":1,"version":"1","overall":"purple"}"#).summary();
    assert_eq!((s.overall, s.reported), (Overall::Warn, Overall::Unknown));
    // security updates warn, ordinary ones only count
    let s = health(r#"{"schema":1,"version":"1","overall":"ok","updates":{"apt_pending":12,"security_pending":3}}"#).summary();
    assert_eq!(
        (s.overall, s.security_updates, s.pending_updates, s.bullets[0].text.as_str()),
        (Overall::Warn, 3, 12, "3 security updates are waiting.")
    );
    // a scheduled check is not a problem the numbers show; the NAS flagged it, so its sentence appears
    let s = health(r#"{"schema":1,"version":"1","overall":"warn","warnings":["md0 check 12.0%"],"arrays":[{"name":"md0","state":"resyncing","degraded":false,"sync_pct":12.0,"sync_action":"check"}]}"#).summary();
    assert_eq!((s.overall, s.rebuilding_arrays, s.bullets[0].text.as_str()), (Overall::Warn, 0, "md0 check 12.0%"));
}

#[test]
fn long_problem_lists_are_cut_and_every_line_is_short() {
    let mut j = serde_json::from_str::<Value>(DEGRADED).unwrap();
    j["services"] = json!((0..4).map(|i| json!({"name": format!("svc{i}"), "active": false})).collect::<Vec<_>>());
    j["volumes"] = json!((0..4).map(|i| json!({"mount": format!("/srv/vol{i}"), "used_pct": 90 + i})).collect::<Vec<_>>());
    j["warnings"] = json!(["x".repeat(400)]);
    j["updates"] = json!({"apt_pending": 1, "security_pending": 1});
    let s = parse_health(&j.to_string()).unwrap().summary();
    assert!(s.problems > MAX_BULLETS, "{}", s.problems);
    assert_eq!(s.bullets.len(), MAX_BULLETS);
    assert!(s.bullets.last().unwrap().text.starts_with("And "), "{:?}", s.bullets.last());
    // worst first
    let levels: Vec<Overall> = s.bullets.iter().map(|b| b.level).collect();
    assert!(levels.windows(2).all(|w| w[0] >= w[1]), "{levels:?}");
    assert_eq!(levels[0], Overall::Crit);
    assert!(s.bullets.iter().all(|b| b.text.chars().count() <= MAX_BULLET_CHARS), "{:?}", s.bullets);
    assert!(s.bullets.iter().all(|b| !b.text.contains('\n')));
    // a long NAS warning is truncated with an ellipsis
    let long = health(&json_with("1", "warn").replace(r#""warnings":[]"#, &format!(r#""warnings":["{}"]"#, "word ".repeat(80)))).summary();
    assert!(long.bullets[0].text.ends_with('…') && long.bullets[0].text.chars().count() <= MAX_BULLET_CHARS);
}

// ---------------------------------------------------------------------------------------------- versions and plans

#[test]
fn versions_compare_by_semver() {
    use VersionRelation::*;
    for (nas, bundled, want) in [
        ("0.2.0", "0.2.0", Same),
        ("0.2.0", "0.4.0", NasOlder),
        ("0.10.0", "0.9.0", NasNewer),
        ("0.4.0", "0.4.0-dev", NasNewer),
        ("0.4.0-dev", "0.4.0", NasOlder),
        ("v0.4.0", "0.4.0", Same),
        ("0.4", "0.4.0", Same),
        ("0.3", "0.4.0", NasOlder),
        ("1.0.0+build5", "1.0.0", Same),
        ("garbage", "0.4.0", Unknown),
        ("garbage", "garbage", Same),
        ("", "0.4.0", Unknown),
    ] {
        assert_eq!(compare(nas, bundled), want, "{nas} vs {bundled}");
    }
}

fn installed(version: Option<&str>, sha: Option<&str>) -> InstalledInfo {
    InstalledInfo { present: true, version: version.map(Into::into), script_sha256: sha.map(Into::into) }
}

fn bundle_v(version: &str) -> MgmtBundle {
    let mut b = MgmtBundle::embedded();
    b.version = version.into();
    for f in &mut b.files {
        if f.name == "VERSION" {
            f.data = format!("{version}\n").into_bytes();
        }
    }
    b
}

#[test]
fn update_plans_cover_every_relation() {
    let b = bundle_v("0.5.0");
    let same_sha = b.script_sha256().unwrap();
    use UpdateDecision::*;
    let plan = |i: &InstalledInfo, force| plan_update(i, &b, force);
    assert_eq!(plan(&installed(Some("0.4.0"), Some("aa")), false).decision, Upgrade);
    assert_eq!(plan(&installed(Some("0.5.0"), Some(&same_sha)), false).decision, UpToDate);
    assert_eq!(plan(&installed(Some("0.5.0"), None), false).decision, UpToDate, "no hash to compare: trust the version");
    assert_eq!(plan(&installed(Some("0.5.0"), Some("different")), false).decision, SameVersionChanged);
    assert_eq!(plan(&installed(Some("0.5.0"), Some(&same_sha)), true).decision, Reinstall);
    assert_eq!(plan(&installed(Some("0.6.0"), Some("aa")), false).decision, DowngradeNeedsForce);
    assert_eq!(plan(&installed(Some("0.6.0"), Some("aa")), true).decision, Downgrade);
    assert_eq!(plan(&installed(None, Some("aa")), false).decision, Install, "unreadable version");
    assert_eq!(plan(&InstalledInfo::default(), false).decision, Install, "not installed");
    for d in [Upgrade, SameVersionChanged, Reinstall, Downgrade, Install] {
        assert!(d.applies());
    }
    assert!(!UpToDate.applies() && !DowngradeNeedsForce.applies());
    let up = plan(&installed(Some("0.4.0"), None), false);
    assert!(up.restarts_web_ui && up.message.contains("0.4.0") && up.message.contains("0.5.0"), "{}", up.message);
    assert!(!plan(&installed(Some("0.5.0"), Some(&same_sha)), false).restarts_web_ui);
    assert_eq!(up.files.len(), b.files.len());
    assert!(up.files.iter().all(|f| f.sha256.len() == 64 && f.bytes > 0));
}

// ---------------------------------------------------------------------------------------------- the files we ship

#[test]
fn the_embedded_management_plane_is_complete_and_consistent() {
    let b = MgmtBundle::embedded();
    assert_eq!(b.version, bundled_mgmt_version());
    assert!(parse_semver(&b.version).is_some(), "VERSION {:?} is not a version", b.version);
    for (name, required, _) in MGMT_FILES {
        assert!(b.files.iter().any(|f| f.name == *name && !f.data.is_empty()), "{name} (required: {required}) missing");
    }
    // the script's built-in default is what a unit without a VERSION file reports; it must not drift from VERSION
    let script = std::str::from_utf8(&b.files.iter().find(|f| f.name == "synofurb_mgmt.py").unwrap().data).unwrap().to_string();
    assert!(
        script.contains(&format!("DEFAULT_VERSION = \"{}\"", b.version)),
        "DEFAULT_VERSION in synofurb_mgmt.py differs from apps/mgmt-plane/VERSION"
    );
    assert!(b.files.iter().find(|f| f.name == "synofurb_mgmt.py").unwrap().executable);
    // the NAS-side contract: every name matches [A-Za-z0-9._-]{1,64}, only the files its updater accepts
    for f in &b.files {
        assert!(f.name.len() <= 64 && f.name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')), "{}", f.name);
        assert!(
            ["synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service", "synofurb_addons.py", "addons.pack.json"]
                .contains(&f.name.as_str())
        );
    }
    b.check_manifest(&b.manifest()).unwrap();
}

#[test]
fn the_manifest_is_in_sha256sum_format_and_the_tarball_carries_it() {
    let b = MgmtBundle::embedded();
    let m = b.manifest();
    let lines: Vec<&str> = m.lines().collect();
    assert_eq!(lines.len(), b.files.len());
    let re_ok = |l: &&str| {
        let (h, n) = l.split_once("  ").unwrap_or(("", ""));
        h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) && !n.is_empty()
    };
    assert!(lines.iter().all(re_ok), "{m}");
    let mut sorted = lines.clone();
    sorted.sort_by_key(|l| l.split("  ").nth(1).unwrap().to_string());
    assert_eq!(sorted, lines, "sorted by file name");
    // unpack and verify like the unit does
    let tar = b.tarball().unwrap();
    assert_eq!(tar, b.tarball().unwrap(), "deterministic");
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&tar[..]));
    let mut seen = std::collections::BTreeMap::new();
    for e in ar.entries().unwrap() {
        let mut e = e.unwrap();
        let name = e.path().unwrap().to_string_lossy().to_string();
        assert!(!name.contains('/') && !name.starts_with('.'), "flat names only: {name}");
        let mode = e.header().mode().unwrap();
        let mut data = vec![];
        std::io::Read::read_to_end(&mut e, &mut data).unwrap();
        seen.insert(name, (mode, data));
    }
    assert_eq!(seen.len(), b.files.len() + 1);
    assert_eq!(seen[MANIFEST_NAME].1, m.as_bytes());
    assert_eq!(seen["synofurb_mgmt.py"].0, 0o755);
    assert_eq!(seen["VERSION"].0, 0o644);
    for f in &b.files {
        assert_eq!(seen[&f.name].1, f.data);
    }
}

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(tag: &str) -> TmpDir {
        let d = std::env::temp_dir().join(format!("sf-manage-{tag}-{}-{}", std::process::id(), random_token(3)));
        std::fs::create_dir_all(&d).unwrap();
        TmpDir(d)
    }
    fn write(&self, name: &str, data: &str) {
        std::fs::write(self.0.join(name), data).unwrap();
    }
}
impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_bundle(dir: &Path, version: &str, with_manifest: bool) {
    for (n, c) in
        [("synofurb_mgmt.py", "#!/usr/bin/env python3\nprint('hi')\n"), ("README.md", "# readme\n"), ("synofurb-mgmt.service", "[Unit]\n")]
    {
        std::fs::write(dir.join(n), c).unwrap();
    }
    std::fs::write(dir.join("VERSION"), format!("{version}\n")).unwrap();
    std::fs::write(dir.join("notes.txt"), "not shipped").unwrap();
    if with_manifest {
        let b = MgmtBundle::from_dir(dir).unwrap();
        std::fs::write(dir.join(MANIFEST_NAME), b.manifest()).unwrap();
    }
}

#[test]
fn a_bundle_directory_is_loaded_checked_and_filtered() {
    let t = TmpDir::new("dir");
    write_bundle(&t.0, "0.9.1", true);
    let b = MgmtBundle::from_dir(&t.0).unwrap();
    assert_eq!(b.version, "0.9.1");
    assert_eq!(
        b.files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["synofurb_mgmt.py", "VERSION", "README.md", "synofurb-mgmt.service"]
    );
    assert!(matches!(b.source, BundleSource::Dir(_)));
    // an offline bundle keeps the files in mgmt/
    let outer = TmpDir::new("outer");
    std::fs::create_dir_all(outer.0.join("mgmt")).unwrap();
    write_bundle(&outer.0.join("mgmt"), "0.9.2", false);
    assert_eq!(MgmtBundle::from_dir(&outer.0).unwrap().version, "0.9.2");
    // the unit file is optional, README.md and VERSION are not
    std::fs::remove_file(t.0.join("synofurb-mgmt.service")).unwrap();
    std::fs::remove_file(t.0.join(MANIFEST_NAME)).unwrap();
    assert_eq!(MgmtBundle::from_dir(&t.0).unwrap().files.len(), 3);
    std::fs::remove_file(t.0.join("README.md")).unwrap();
    assert!(MgmtBundle::from_dir(&t.0).unwrap_err().to_string().contains("README.md"));
}

#[test]
fn a_bad_bundle_directory_is_refused() {
    let t = TmpDir::new("bad");
    write_bundle(&t.0, "0.9.1", true);
    // changed after the manifest was written
    t.write("synofurb_mgmt.py", "print('tampered')\n");
    let e = format!("{:#}", MgmtBundle::from_dir(&t.0).unwrap_err());
    assert!(e.contains("MANIFEST.sha256") && e.contains("different checksum"), "{e}");
    // invalid version
    let v = TmpDir::new("ver");
    write_bundle(&v.0, "1.0; rm -rf /", false);
    assert!(MgmtBundle::from_dir(&v.0).unwrap_err().to_string().contains("VERSION"));
    // not a folder / no files
    assert!(MgmtBundle::from_dir(&t.0.join("nope")).is_err());
    let empty = TmpDir::new("empty");
    assert!(MgmtBundle::from_dir(&empty.0).unwrap_err().to_string().contains("not a management plane folder"));
    // a manifest that omits a shipped file
    let m = TmpDir::new("omit");
    write_bundle(&m.0, "0.9.1", false);
    m.write(MANIFEST_NAME, "");
    assert!(format!("{:#}", MgmtBundle::from_dir(&m.0).unwrap_err()).contains("is not listed"));
}

#[cfg(unix)]
#[test]
fn symbolic_links_in_a_bundle_directory_are_refused() {
    let t = TmpDir::new("link");
    write_bundle(&t.0, "0.9.1", false);
    std::fs::remove_file(t.0.join("synofurb_mgmt.py")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", t.0.join("synofurb_mgmt.py")).unwrap();
    assert!(MgmtBundle::from_dir(&t.0).unwrap_err().to_string().contains("not a regular file"));
}

// ---------------------------------------------------------------------------------------------- events

#[test]
fn remote_lines_become_events() {
    let ev = parse_remote_line(r#"{"step":"swap","status":"ok","message":"previous version kept"}"#).unwrap();
    let ManageEvent::Remote(r) = &ev else { panic!("{ev:?}") };
    assert_eq!((r.step.as_str(), r.status, r.message.as_str()), ("swap", StepStatus::Ok, "previous version kept"));
    assert_eq!(r.line(), "swap: ok - previous version kept");
    for (word, want) in [
        ("start", StepStatus::Running),
        ("ok", StepStatus::Ok),
        ("fail", StepStatus::Failed),
        ("warn", StepStatus::Warn),
        ("skipped", StepStatus::Skipped),
        ("rolled_back", StepStatus::Rollback),
        ("???", StepStatus::Info),
    ] {
        assert_eq!(StepStatus::parse(word), want, "{word}");
    }
    // other key names, extra keys, plain text, blank lines
    let ev = parse_remote_line(r#"{"name":"health","state":"failed","msg":"port 8443 is not listening","pct":50}"#).unwrap();
    assert!(matches!(&ev, ManageEvent::Remote(r) if r.status == StepStatus::Failed && r.step == "health" && r.message.contains("8443")));
    assert!(matches!(parse_remote_line("plain progress text"), Some(ManageEvent::Log { level: Level::Info, .. })));
    assert!(matches!(parse_remote_line("{\"unrelated\":1}"), Some(ManageEvent::Log { level: Level::Debug, .. })));
    assert!(matches!(parse_remote_line("{broken"), Some(ManageEvent::Log { .. })));
    assert!(parse_remote_line("   ").is_none());
}

#[test]
fn the_line_sink_works_on_arbitrary_chunks() {
    let mut got: Vec<ManageEvent> = vec![];
    let mut cb = |e: ManageEvent| got.push(e);
    let mut sink = LineSink::new(&mut cb);
    let text = "{\"step\":\"verify\",\"status\":\"start\",\"message\":\"a\"}\n\n{\"step\":\"verify\",\"status\":\"ok\",\"message\":\"b\"}\ntrailing no newline";
    for chunk in text.as_bytes().chunks(7) {
        sink.write_all(chunk).unwrap();
    }
    assert_eq!(sink.steps.len(), 2, "two json lines are complete, the last text is still pending");
    sink.finish();
    let steps = sink.steps.clone();
    drop(sink);
    assert_eq!(steps.len(), 2);
    assert_eq!(got.len(), 3, "two steps and the trailing text: {got:?}");
    assert!(matches!(&got[2], ManageEvent::Log { msg, .. } if msg == "trailing no newline"));
}

#[test]
fn events_map_onto_the_cli_vocabulary() {
    use std::sync::mpsc::channel;
    let step = ManageEvent::Step { n: 2, total: 5, name: "Uploading".into() };
    assert!(matches!(step.to_core(), Event::Step { n: 2, total: 5, .. }));
    assert_eq!(step.to_json(), json!({"event":"step","n":2,"total":5,"name":"Uploading"}));
    let pr = ManageEvent::Progress { what: "update files".into(), fraction: 0.12345 };
    assert!(matches!(pr.to_core(), Event::Progress { .. }));
    assert_eq!(pr.to_json()["fraction"], 0.123);
    let nas = ManageEvent::Remote(RemoteStep {
        step: "swap".into(),
        status: StepStatus::Failed,
        raw_status: "fail".into(),
        message: "disk full".into(),
    });
    assert_eq!(nas.to_json(), json!({"event":"nas_step","step":"swap","status":"failed","raw_status":"fail","message":"disk full"}));
    let Event::Log(l) = nas.to_core() else { panic!() };
    assert_eq!((l.level, l.source, l.msg.as_str()), (Level::Error, Source::App, "NAS swap: fail - disk full"));
    assert_eq!(
        ManageEvent::Log { level: Level::Warn, msg: "x".into() }.to_json(),
        json!({"event":"log","level":"warn","source":"app","msg":"x"})
    );
    // through a logger: steps arrive as events, lines as log lines with secrets redacted
    let (tx, rx) = channel();
    let lg = Logger::memory(Some(tx));
    lg.add_secret("hunter2hunter2");
    step.emit(&lg);
    ManageEvent::Log { level: Level::Info, msg: "login hunter2hunter2".into() }.emit(&lg);
    nas.emit(&lg);
    let evs: Vec<Event> = rx.try_iter().collect();
    assert!(matches!(evs[0], Event::Step { n: 2, .. }));
    assert!(matches!(&evs[1], Event::Log(l) if l.msg == "login ***"));
    assert!(matches!(&evs[2], Event::Log(l) if l.level == Level::Error));
}

// ---------------------------------------------------------------------------------------------- a scripted NAS

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// The updater succeeds and the new version runs.
    Ok,
    /// Exit 3: failed health check, rolled back by the unit.
    RolledBack,
    /// Exit 4: failed and the rollback failed, the unit does not answer.
    RollbackFailed,
    /// Exit 2: refused before changing anything.
    Refused,
    /// Exit 0, but the unit then reports the old version.
    SucceedsButStillOld,
    /// The `update` subcommand does not exist (argparse error), `health` does.
    NoUpdateSubcommand,
}

struct FakeNas {
    version: Option<String>,
    sha: String,
    has_health: bool,
    mode: Mode,
    new_version: String,
    answering: bool,
    fail_upload: bool,
    health_json: String,
    commands: Vec<String>,
    stdins: Vec<Option<Vec<u8>>>,
    uploads: Vec<(String, Vec<u8>, u32)>,
    health_calls: u32,
    prev: Option<String>,
}

impl FakeNas {
    fn new(version: Option<&str>, new_version: &str, mode: Mode) -> FakeNas {
        FakeNas {
            version: version.map(Into::into),
            sha: "ab".repeat(32),
            has_health: true,
            mode,
            new_version: new_version.into(),
            answering: true,
            fail_upload: false,
            health_json: HEALTHY.to_string(),
            commands: vec![],
            stdins: vec![],
            uploads: vec![],
            health_calls: 0,
            prev: None,
        }
    }
    fn health(&mut self) -> ExecResult {
        self.health_calls += 1;
        let ok = |s: String| ExecResult { code: 0, stdout: s, stderr: String::new() };
        if !self.has_health {
            return ExecResult {
                code: 2,
                stdout: String::new(),
                stderr: "usage: synofurb_mgmt.py [-h] [--port PORT]\nsynofurb_mgmt.py: error: unrecognized arguments: health --json\n"
                    .into(),
            };
        }
        if !self.answering {
            return ExecResult { code: 1, stdout: String::new(), stderr: "Traceback (most recent call last):\n".into() };
        }
        let mut v: Value = serde_json::from_str(&self.health_json).unwrap();
        v["version"] = json!(self.version.clone().unwrap_or_default());
        ok(v.to_string())
    }
    fn lines(steps: &[(&str, &str, &str)]) -> String {
        steps.iter().map(|(s, st, m)| format!("{}\n", json!({"step": s, "status": st, "message": m}))).collect()
    }
    fn updater(&mut self, cmd: &str, out: &mut dyn std::io::Write) -> (u32, String) {
        if cmd.contains("update --rollback") {
            if let Some(p) = self.prev.take() {
                self.prev = self.version.take();
                self.version = Some(p);
                self.answering = true;
                out.write_all(
                    Self::lines(&[
                        ("rollback", "start", "restoring"),
                        ("rollback", "ok", "previous version is running"),
                        ("result", "ok", "rolled back"),
                    ])
                    .as_bytes(),
                )
                .unwrap();
                return (0, String::new());
            }
            out.write_all(
                Self::lines(&[("rollback", "start", "restoring"), ("rollback", "fail", "no previous version is kept")]).as_bytes(),
            )
            .unwrap();
            return (2, String::new());
        }
        if cmd.contains("update --from") && self.mode == Mode::NoUpdateSubcommand {
            return (2, "usage: synofurb_mgmt.py [-h]\nsynofurb_mgmt.py: error: unrecognized arguments: update --from x --json\n".into());
        }
        // `update --from` (the unit's updater) and the uploaded bootstrap script (run as /bin/sh) both end up here
        let head = [("verify", "start", "checking"), ("verify", "ok", "manifest verified")];
        match self.mode {
            Mode::Ok | Mode::NoUpdateSubcommand => {
                let mut s = Self::lines(&head);
                s.push_str(&Self::lines(&[
                    ("stage", "ok", "5 files staged"),
                    ("swap", "ok", "previous kept"),
                    ("restart", "ok", "restarted"),
                    ("health", "ok", "version answers"),
                    ("result", "ok", "updated"),
                ]));
                out.write_all(s.as_bytes()).unwrap();
                self.prev = self.version.take();
                self.version = Some(self.new_version.clone());
                self.has_health = true; // the new script has `health`, whatever the old one had
                (0, String::new())
            }
            Mode::RolledBack => {
                let mut s = Self::lines(&head);
                s.push_str(&Self::lines(&[
                    ("swap", "ok", "swapped"),
                    ("health", "fail", "port 8443 is not listening"),
                    ("rollback", "start", "restoring the previous version"),
                    ("rollback", "ok", "previous version 0.2.0 is running again"),
                    ("result", "fail", "update to 0.5.0 failed; rolled back"),
                ]));
                out.write_all(s.as_bytes()).unwrap();
                (3, String::new())
            }
            Mode::RollbackFailed => {
                out.write_all(
                    Self::lines(&[
                        ("swap", "ok", "swapped"),
                        ("health", "fail", "service is not active"),
                        ("rollback", "fail", "previous version does not answer"),
                        ("result", "fail", "rollback failed"),
                    ])
                    .as_bytes(),
                )
                .unwrap();
                self.answering = false;
                (4, "boom\n".into())
            }
            Mode::Refused => {
                out.write_all(
                    Self::lines(&[("verify", "start", "checking"), ("verify", "fail", "checksum mismatch for synofurb_mgmt.py")])
                        .as_bytes(),
                )
                .unwrap();
                (2, String::new())
            }
            Mode::SucceedsButStillOld => {
                out.write_all(
                    Self::lines(&[("verify", "ok", "ok"), ("swap", "ok", "ok"), ("health", "ok", "answers"), ("result", "ok", "updated")])
                        .as_bytes(),
                )
                .unwrap();
                self.prev = self.version.clone();
                (0, String::new())
            }
        }
    }
}

impl RemoteShell for FakeNas {
    fn exec(&mut self, cmd: &str, stdin: Option<&[u8]>) -> Result<ExecResult> {
        self.commands.push(cmd.to_string());
        self.stdins.push(stdin.map(|s| s.to_vec()));
        let ok = |s: &str| Ok(ExecResult { code: 0, stdout: s.into(), stderr: String::new() });
        if cmd.contains("DEFAULT_VERSION") {
            return match &self.version {
                Some(v) => ok(&format!("present=1\nversion={v}\nsha256={}\n", self.sha)),
                None => ok("present=0\n"),
            };
        }
        if cmd.contains("health --json") {
            return Ok(self.health());
        }
        ok("")
    }
    fn exec_to(&mut self, cmd: &str, stdin: Option<&[u8]>, out: &mut dyn std::io::Write) -> Result<(u32, String)> {
        self.commands.push(cmd.to_string());
        self.stdins.push(stdin.map(|s| s.to_vec()));
        Ok(self.updater(cmd, out))
    }
    fn upload(&mut self, p: &str, d: &mut dyn std::io::Read, _size: u64, mode: u32, progress: &mut dyn FnMut(u64)) -> Result<()> {
        if self.fail_upload {
            bail!("connection reset");
        }
        let mut v = vec![];
        let mut buf = [0u8; 4096];
        loop {
            let n = d.read(&mut buf)?;
            if n == 0 {
                break;
            }
            v.extend_from_slice(&buf[..n]);
            progress(v.len() as u64);
        }
        self.uploads.push((p.to_string(), v, mode));
        Ok(())
    }
    fn host_key_fingerprint(&self) -> Option<String> {
        None
    }
}

fn quick() -> UpdateOptions {
    UpdateOptions { verify_attempts: 2, verify_delay: Duration::ZERO, ..Default::default() }
}

fn run_update(nas: &mut FakeNas, bundle: &MgmtBundle, opts: &UpdateOptions) -> (UpdateReport, Vec<ManageEvent>) {
    let mut events = vec![];
    let r = {
        let mut p = Privileged { sh: nas, is_root: true, sudo_password: None };
        push_update(&mut p, bundle, opts, &mut |e| events.push(e)).unwrap()
    };
    (r, events)
}

fn steps_of(events: &[ManageEvent]) -> Vec<(u32, String)> {
    events.iter().filter_map(|e| if let ManageEvent::Step { n, name, .. } = e { Some((*n, name.clone())) } else { None }).collect()
}

/// The commands and scripts one flow ran must stay inside /opt/synofurb-mgmt and /tmp: no disks, arrays, partitions or boot flash.
fn assert_harmless(nas: &FakeNas) {
    let mut all = nas.commands.join("\n");
    for (_, data, _) in &nas.uploads {
        if let Ok(t) = std::str::from_utf8(data) {
            all.push('\n');
            all.push_str(t);
        }
    }
    let all = all.replace("/dev/null", "");
    for bad in [
        "/dev/",
        "mkfs",
        "mdadm",
        "parted",
        "sgdisk",
        "wipefs",
        "fdisk",
        "synofurb-dom",
        "dd ",
        "cryptsetup",
        "lvm",
        "mount ",
        "reboot",
        "poweroff",
        "shutdown",
        "kexec",
    ] {
        assert!(!all.contains(bad), "a management command mentions {bad:?}:\n{all}");
    }
}

#[test]
fn a_successful_update_streams_progress_and_verifies() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Updated, "{:?}", r.failure);
    assert_eq!(r.operation, Operation::Update);
    assert_eq!((r.from_version.as_deref(), r.to_version.as_deref(), r.method), (Some("0.2.0"), Some("0.5.0"), Some(UpdateMethod::Unit)));
    assert_eq!(r.rollback, RollbackState::NotApplicable);
    assert_eq!(r.health_after.as_ref().map(|h| h.version.as_str()), Some("0.5.0"));
    assert_eq!(r.summary_after.as_ref().map(|s| s.overall), Some(Overall::Ok));
    assert_eq!(r.remote_steps.len(), 7);
    assert!(r.headline().contains("0.2.0 -> 0.5.0"), "{}", r.headline());
    // host-side stages come in order, the NAS lines arrive between "apply" and "verify"
    let st = steps_of(&ev);
    assert_eq!(st.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
    let apply = ev.iter().position(|e| matches!(e, ManageEvent::Step { n: 3, .. })).unwrap();
    let verify = ev.iter().position(|e| matches!(e, ManageEvent::Step { n: 4, .. })).unwrap();
    let nas_idx: Vec<usize> = ev.iter().enumerate().filter(|(_, e)| matches!(e, ManageEvent::Remote(_))).map(|(i, _)| i).collect();
    assert_eq!(nas_idx.len(), 7);
    assert!(nas_idx.iter().all(|i| *i > apply && *i < verify));
    // upload progress reached 100 %
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Progress { fraction, .. } if *fraction >= 1.0)));
    // what was shipped: a tarball whose hash the staging script checks, then the unit's own updater on the staged copy
    assert_eq!(nas.uploads.len(), 2, "tarball + staging script");
    let tar = nas.uploads.iter().find(|u| u.0.ends_with(".tar.gz")).unwrap();
    assert_eq!(tar.2, 0o600);
    assert_eq!(tar.1, b.tarball().unwrap());
    let stage_script_text = String::from_utf8(nas.uploads.iter().find(|u| u.0.ends_with(".sh")).unwrap().1.clone()).unwrap();
    assert!(stage_script_text.contains(&hex::encode(Sha256::digest(&tar.1))), "the staging script pins the archive hash");
    assert!(stage_script_text.contains("sha256sum -c MANIFEST.sha256"));
    let upd = nas.commands.iter().find(|c| c.contains("update --from")).unwrap();
    assert!(
        upd.starts_with(&format!("python3 {MGMT_SCRIPT} update --from '/tmp/.synofurb-mgmt-stage-")) && upd.ends_with("--json"),
        "{upd}"
    );
    assert!(!upd.contains("--force"));
    // staging is removed again, also on success
    assert!(nas.commands.last().unwrap().starts_with("rm -rf '/tmp/.synofurb-mgmt-stage-"), "{:?}", nas.commands.last());
    assert_harmless(&nas);
}

#[test]
fn a_non_root_user_uses_sudo_and_no_secret_reaches_a_command_line_or_an_upload() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    let r = {
        let mut p = Privileged { sh: &mut nas, is_root: false, sudo_password: Some("Sup3rS3cretPw".into()) };
        push_update(&mut p, &b, &quick(), &mut |_| {}).unwrap()
    };
    assert_eq!(r.outcome, UpdateOutcome::Updated);
    for c in &nas.commands {
        assert!(!c.contains("Sup3rS3cretPw"), "password on a command line: {c}");
    }
    for (path, data, _) in &nas.uploads {
        assert!(!String::from_utf8_lossy(data).contains("Sup3rS3cretPw"), "password in {path}");
    }
    // the privileged ones carry it on stdin only
    let with_stdin: Vec<&String> = nas.commands.iter().zip(&nas.stdins).filter(|(_, s)| s.is_some()).map(|(c, _)| c).collect();
    assert!(with_stdin.len() >= 5, "{with_stdin:?}");
    assert!(with_stdin.iter().all(|c| c.starts_with("sudo -S -p ''")));
    assert!(nas.stdins.iter().flatten().all(|s| s == b"Sup3rS3cretPw\n"));
    // the version probe is read-only and runs without sudo
    assert!(!nas.commands[0].starts_with("sudo"));
}

#[test]
fn an_up_to_date_unit_is_left_alone() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.5.0"), "0.5.0", Mode::Ok);
    nas.sha = b.script_sha256().unwrap();
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::UpToDate);
    assert!(nas.uploads.is_empty());
    assert!(!nas.commands.iter().any(|c| c.contains("update --from") || c.starts_with("rm")), "{:?}", nas.commands);
    assert_eq!(steps_of(&ev).len(), 1);
    assert!(r.headline().contains("up to date"));
    // --force reinstalls identical files
    let (r, _) = run_update(&mut nas, &b, &UpdateOptions { force: true, ..quick() });
    assert_eq!(r.outcome, UpdateOutcome::Updated);
    assert_eq!(r.plan.as_ref().unwrap().decision, UpdateDecision::Reinstall);
}

#[test]
fn a_downgrade_needs_force_and_passes_it_on() {
    let b = bundle_v("0.3.0");
    let mut nas = FakeNas::new(Some("0.5.0"), "0.3.0", Mode::Ok);
    let (r, _) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Refused);
    assert!(r.failure.as_deref().unwrap().contains("--force"));
    assert!(nas.uploads.is_empty());
    let (r, _) = run_update(&mut nas, &b, &UpdateOptions { force: true, ..quick() });
    assert_eq!(r.outcome, UpdateOutcome::Updated, "{:?}", r.failure);
    assert!(nas.commands.iter().any(|c| c.contains("update --from") && c.contains("--force")));
}

#[test]
fn a_failed_update_that_the_unit_rolled_back_is_reported_as_restored() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::RolledBack);
    // the unit rolls back by itself: its version is still 0.2.0 when we look
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Failed);
    assert_eq!(r.rollback, RollbackState::Restored);
    assert!(r.rollback_reported, "the unit printed rollback steps");
    assert_eq!(r.failure.as_deref(), Some("port 8443 is not listening (health)"), "the first real failure, not the closing line");
    assert_eq!(r.health_after.as_ref().unwrap().version, "0.2.0");
    let h = r.headline();
    assert!(h.contains("Update failed") && h.contains("port 8443") && h.contains("back on version 0.2.0"), "{h}");
    assert!(!r.ok());
    // the failure was announced to the event stream and staging was cleaned up
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Remote(s) if s.status == StepStatus::Failed)));
    assert!(nas.commands.iter().any(|c| c.starts_with("rm -rf '/tmp/.synofurb-mgmt-stage-")));
    assert_harmless(&nas);
}

#[test]
fn a_failed_rollback_is_reported_as_unverified_and_never_as_success() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::RollbackFailed);
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Failed);
    assert_eq!(r.rollback, RollbackState::Unverified);
    assert!(r.health_after.is_none());
    assert!(r.rollback_reported);
    assert!(r.headline().contains("did not complete; check the unit"), "{}", r.headline());
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Log { level: Level::Error, msg } if msg.contains("does not answer"))));
}

#[test]
fn a_unit_that_refuses_the_files_was_not_changed() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Refused);
    let (r, _) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Failed);
    assert_eq!(r.rollback, RollbackState::NotApplicable, "exit 2 means nothing was touched");
    assert!(r.failure.as_deref().unwrap().contains("checksum mismatch"));
    assert!(!r.rollback_reported);
    assert!(r.headline().contains("Nothing was changed"), "{}", r.headline());
}

#[test]
fn success_without_the_new_version_answering_triggers_a_rollback_request() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::SucceedsButStillOld);
    // the updater says ok, but the unit keeps reporting 0.2.0
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Failed);
    assert!(r.failure.as_deref().unwrap().contains("expected 0.5.0"), "{:?}", r.failure);
    assert!(nas.commands.iter().any(|c| c.contains("update --rollback")), "the host asks the unit to roll back");
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Log { msg, .. } if msg.contains("roll back"))));
    assert!(r.rollback_reported);
    assert_eq!(r.rollback, RollbackState::Restored);
}

#[test]
fn a_unit_without_a_health_report_is_updated_with_the_builtin_swap() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    nas.has_health = false;
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.outcome, UpdateOutcome::Updated, "{:?}", r.failure);
    assert_eq!(r.method, Some(UpdateMethod::Bootstrap));
    assert!(nas.commands.iter().all(|c| !c.contains("update --from")), "the old script is never asked to update");
    let script = nas.uploads.iter().filter(|u| u.0.ends_with(".sh")).map(|u| String::from_utf8_lossy(&u.1).to_string()).collect::<Vec<_>>();
    assert!(script.iter().any(|s| s.contains("PREV=") && s.contains("MANIFEST.sha256")), "{script:?}");
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Log { msg, .. } if msg.contains("too old"))));
    assert_harmless(&nas);
    // and when the swap fails and the new script still does not answer, the report says "unverified", not "ok"
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::RolledBack);
    nas.has_health = false;
    let (r, _) = run_update(&mut nas, &b, &quick());
    assert_eq!((r.outcome, r.rollback, r.method), (UpdateOutcome::Failed, RollbackState::Unverified, Some(UpdateMethod::Bootstrap)));
}

#[test]
fn a_health_capable_unit_without_update_falls_back_to_the_builtin_swap() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.3.0"), "0.5.0", Mode::NoUpdateSubcommand);
    let (r, ev) = run_update(&mut nas, &b, &quick());
    assert_eq!(r.method, Some(UpdateMethod::Bootstrap));
    assert_eq!(r.outcome, UpdateOutcome::Updated, "{:?}", r.failure);
    assert!(ev.iter().any(|e| matches!(e, ManageEvent::Log { msg, .. } if msg.contains("no updater"))));
}

#[test]
fn a_failed_upload_is_an_error_and_leaves_nothing_behind() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    nas.fail_upload = true;
    let e = {
        let mut p = Privileged { sh: &mut nas, is_root: true, sudo_password: None };
        push_update(&mut p, &b, &quick(), &mut |_| {}).unwrap_err()
    };
    assert!(format!("{e:#}").contains("connection reset"), "{e:#}");
    assert!(nas.commands.iter().any(|c| c.starts_with("rm -rf '/tmp/.synofurb-mgmt-stage-")), "cleanup still ran");
    assert!(!nas.commands.iter().any(|c| c.contains("update --from")));
}

#[test]
fn cancelling_before_the_update_starts_changes_nothing() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    let flag = Arc::new(AtomicBool::new(true));
    let (r, _) = run_update(&mut nas, &b, &UpdateOptions { cancel: Some(flag), ..quick() });
    assert_eq!(r.outcome, UpdateOutcome::Refused);
    assert!(nas.uploads.is_empty());
    assert!(r.failure.as_deref().unwrap().contains("Cancelled"));
}

#[test]
fn an_unreachable_unit_is_an_error_not_a_report() {
    let b = bundle_v("0.5.0");
    let mut m = MockShell::default();
    let r = push_update(&mut Privileged { sh: &mut m, is_root: false, sudo_password: None }, &b, &quick(), &mut |_| {});
    let e = r.unwrap_err();
    assert!(format!("{e:#}").contains("no sudo password"), "{e:#}");
    assert!(m.commands.is_empty(), "nothing is run before the privilege check");
    let e = push_rollback(&mut Privileged { sh: &mut m, is_root: false, sudo_password: None }, &quick(), &mut |_| {}).unwrap_err();
    assert!(format!("{e:#}").contains("needs root"), "{e:#}");
}

#[test]
fn the_mock_shell_drives_a_minimal_update_end_to_end() {
    // MockShell answers in order: version probe, health, (upload), stage script run, rm, updater, health, cleanup
    let b = bundle_v("0.5.0");
    let mut m = MockShell::default();
    m.push_ok("present=1\nversion=0.2.0\nsha256=");
    m.push_ok(&json_with("0.2.0", "ok"));
    m.push_ok(""); // stage script
    m.push_ok(""); // rm of the script
    m.push_ok(&format!("{}\n", json!({"step":"result","status":"ok","message":"updated"}))); // updater
    m.push_ok(&json_with("0.5.0", "ok")); // verify
    let mut events = vec![];
    let r = push_update(&mut Privileged { sh: &mut m, is_root: true, sudo_password: None }, &b, &quick(), &mut |e| events.push(e)).unwrap();
    assert_eq!(r.outcome, UpdateOutcome::Updated, "{:?}", r.failure);
    assert!(events.iter().any(|e| matches!(e, ManageEvent::Remote(s) if s.step == "result")));
}

#[test]
fn rollback_goes_through_the_unit_and_is_verified() {
    let mut nas = FakeNas::new(Some("0.5.0"), "0.5.0", Mode::Ok);
    nas.prev = Some("0.2.0".into());
    let mut events = vec![];
    let r = push_rollback(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &quick(), &mut |e| events.push(e)).unwrap();
    assert_eq!(r.outcome, UpdateOutcome::RolledBack, "{:?}", r.failure);
    assert_eq!((r.from_version.as_deref(), r.to_version.as_deref()), (Some("0.5.0"), Some("0.2.0")));
    assert!(r.ok() && r.headline().contains("0.2.0"));
    assert!(nas.commands.iter().any(|c| c.ends_with("update --rollback --json")));
    assert!(nas.uploads.is_empty(), "a rollback uploads nothing");
    // no previous version kept: the unit refuses (exit 2) and says why
    let mut nas = FakeNas::new(Some("0.5.0"), "0.5.0", Mode::Ok);
    let r = push_rollback(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &quick(), &mut |_| {}).unwrap();
    assert_eq!(r.outcome, UpdateOutcome::Failed);
    assert_eq!(r.failure.as_deref(), Some("no previous version is kept (rollback)"));
    assert!(r.headline().starts_with("Rollback failed: no previous version is kept"), "{}", r.headline());
    assert_eq!(r.operation, Operation::Rollback);
    // an old unit cannot roll back remotely
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    nas.has_health = false;
    let e = push_rollback(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &quick(), &mut |_| {}).unwrap_err();
    assert!(e.to_string().contains("cannot roll back remotely"), "{e}");
    // not installed at all
    let mut nas = FakeNas::new(None, "0.5.0", Mode::Ok);
    assert!(push_rollback(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &quick(), &mut |_| {}).is_err());
}

#[test]
fn status_reports_installed_health_and_what_an_update_would_do() {
    let b = bundle_v("0.5.0");
    let mut nas = FakeNas::new(Some("0.2.0"), "0.5.0", Mode::Ok);
    let st = status(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &b).unwrap();
    assert_eq!(
        (st.installed.version.as_deref(), st.bundled_version.as_str(), st.relation),
        (Some("0.2.0"), "0.5.0", Some(VersionRelation::NasOlder))
    );
    assert_eq!(st.plan.decision, UpdateDecision::Upgrade);
    assert!(st.health.is_some() && st.summary.is_some() && st.health_error.is_none());
    // too old to report health: the version still comes from the files
    nas.has_health = false;
    let st = status(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &b).unwrap();
    assert!(st.health.is_none());
    assert!(st.health_error.as_deref().unwrap().contains("too old"), "{:?}", st.health_error);
    assert_eq!(st.installed.version.as_deref(), Some("0.2.0"));
    // not installed
    let mut nas = FakeNas::new(None, "0.5.0", Mode::Ok);
    let st = status(&mut Privileged { sh: &mut nas, is_root: true, sudo_password: None }, &b).unwrap();
    assert!(!st.installed.present && st.relation.is_none());
    assert_eq!(st.plan.decision, UpdateDecision::Install);
    assert!(st.health_error.as_deref().unwrap().contains("not installed"));
    // serialises for the UI / CLI
    assert!(serde_json::to_value(&st).is_ok());
}

#[test]
fn installed_info_is_parsed_defensively() {
    let i = parse_installed(&format!("present=1\nversion=0.4.0\nsha256={}\n", "A".repeat(64)));
    assert_eq!((i.present, i.version.as_deref(), i.script_sha256.as_deref()), (true, Some("0.4.0"), Some("a".repeat(64).as_str())));
    let i = parse_installed("present=1\nversion=$(rm -rf /)\nsha256=zz\n");
    assert_eq!((i.version, i.script_sha256), (None, None), "anything odd is dropped, never trusted");
    assert!(!parse_installed("present=0\n").present);
    assert!(!parse_installed("").present);
    // the probe command only reads
    for bad in ["rm ", ">", "mv ", "systemctl"] {
        assert!(!READ_INSTALLED.replace(">/dev/null", "").replace("2>", "").contains(bad), "{bad}");
    }
}

// ---------------------------------------------------------------------------------------------- the shell scripts, run for real

fn sh_available() -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg("command -v sha256sum >/dev/null && command -v tar >/dev/null")
        .status()
        .is_ok_and(|s| s.success())
}

fn run_sh(script: &str) -> (i32, String, String) {
    let o = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

#[test]
fn the_generated_scripts_are_valid_posix_sh() {
    if !sh_available() {
        return;
    }
    let t = TmpDir::new("syntax");
    for (name, text) in [
        ("stage", stage_script("/tmp/x.tar.gz", "/tmp/stage", &"0".repeat(64))),
        ("bootstrap", bootstrap_script("", "/tmp/stage")),
        ("probe", READ_INSTALLED.to_string()),
    ] {
        let f = t.0.join(name);
        std::fs::write(&f, text).unwrap();
        let (code, _, err) = run_sh(&format!("sh -n '{}'", f.display()));
        assert_eq!(code, 0, "{name}: {err}");
    }
}

#[test]
fn the_staging_script_unpacks_and_verifies_a_real_archive() {
    if !sh_available() {
        return;
    }
    let t = TmpDir::new("stagerun");
    let b = bundle_v("0.5.0");
    let tar = b.tarball().unwrap();
    let tar_path = t.0.join("up.tar.gz");
    std::fs::write(&tar_path, &tar).unwrap();
    let stage = t.0.join("stage");
    let sha = hex::encode(Sha256::digest(&tar));
    let script = stage_script(tar_path.to_str().unwrap(), stage.to_str().unwrap(), &sha);
    let (code, _, err) = run_sh(&script);
    assert_eq!(code, 0, "{err}");
    assert!(!tar_path.exists(), "the archive is removed after unpacking");
    for f in &b.files {
        assert_eq!(std::fs::read(stage.join(&f.name)).unwrap(), f.data, "{}", f.name);
    }
    assert!(stage.join(MANIFEST_NAME).is_file());
    // wrong hash: refused before anything is unpacked
    std::fs::write(&tar_path, &tar).unwrap();
    let stage2 = t.0.join("stage2");
    let (code, _, err) = run_sh(&stage_script(tar_path.to_str().unwrap(), stage2.to_str().unwrap(), &"0".repeat(64)));
    assert_eq!(code, 3, "{err}");
    assert!(err.contains("does not match"), "{err}");
    assert!(!stage2.exists());
}

fn make_stage(dir: &Path, b: &MgmtBundle) {
    std::fs::create_dir_all(dir).unwrap();
    for f in &b.files {
        std::fs::write(dir.join(&f.name), &f.data).unwrap();
    }
    std::fs::write(dir.join(MANIFEST_NAME), b.manifest()).unwrap();
}

fn parse_lines(out: &str) -> Vec<RemoteStep> {
    out.lines()
        .filter_map(|l| match parse_remote_line(l) {
            Some(ManageEvent::Remote(r)) => Some(r),
            _ => None,
        })
        .collect()
}

#[test]
fn the_bootstrap_swap_replaces_the_files_keeps_a_rollback_copy_and_speaks_ndjson() {
    if !sh_available() {
        return;
    }
    let t = TmpDir::new("boot");
    let root = t.0.join("root");
    let opt = root.join("opt/synofurb-mgmt");
    std::fs::create_dir_all(&opt).unwrap();
    std::fs::write(opt.join("synofurb_mgmt.py"), "old script\n").unwrap();
    let unit_dir = root.join("etc/systemd/system");
    std::fs::create_dir_all(&unit_dir).unwrap();
    std::fs::write(unit_dir.join("synofurb-mgmt.service"), "old unit\n").unwrap();
    let b = bundle_v("0.5.0");
    let stage = t.0.join("stage");
    make_stage(&stage, &b);
    let (code, out, err) = run_sh(&bootstrap_script(root.to_str().unwrap(), stage.to_str().unwrap()));
    assert_eq!(code, 0, "{out}\n{err}");
    let steps = parse_lines(&out);
    assert_eq!(steps.len(), out.lines().count(), "every line is NDJSON: {out}");
    assert_eq!(steps.last().map(|s| (s.step.as_str(), s.status)), Some(("result", StepStatus::Ok)));
    for f in &b.files {
        assert_eq!(std::fs::read(opt.join(&f.name)).unwrap(), f.data, "{}", f.name);
    }
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(opt.join("synofurb_mgmt.py")).unwrap().permissions().mode() & 0o777, 0o755);
    // the old install is kept where the unit's own `update --rollback` looks for it, with the old unit file
    let prev = root.join("opt/synofurb-mgmt.prev");
    assert_eq!(std::fs::read_to_string(prev.join("synofurb_mgmt.py")).unwrap(), "old script\n");
    assert_eq!(std::fs::read_to_string(prev.join("unit.backup")).unwrap(), "old unit\n");
    assert_eq!(
        std::fs::read_to_string(unit_dir.join("synofurb-mgmt.service")).unwrap().as_bytes(),
        b.files.iter().find(|f| f.name == "synofurb-mgmt.service").unwrap().data.as_slice()
    );
    assert!(!root.join("opt/synofurb-mgmt.new").exists() && !root.join("opt/synofurb-mgmt.old").exists());
    // nothing outside the sandbox root was written
    assert!(std::fs::read_dir(&t.0).unwrap().count() == 2);
}

#[test]
fn the_bootstrap_swap_installs_into_an_empty_unit_and_refuses_corrupt_files() {
    if !sh_available() {
        return;
    }
    let t = TmpDir::new("boot2");
    let root = t.0.join("root");
    let b = bundle_v("0.5.0");
    let stage = t.0.join("stage");
    make_stage(&stage, &b);
    let (code, out, err) = run_sh(&bootstrap_script(root.to_str().unwrap(), stage.to_str().unwrap()));
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(root.join("opt/synofurb-mgmt/synofurb_mgmt.py").is_file());
    assert!(!root.join("opt/synofurb-mgmt.prev").exists(), "nothing to keep on a first install");

    // a staged file that does not match its checksum: refused, the install is untouched
    let before = std::fs::read(root.join("opt/synofurb-mgmt/VERSION")).unwrap();
    std::fs::write(stage.join("VERSION"), "9.9.9\n").unwrap();
    let (code, out, _) = run_sh(&bootstrap_script(root.to_str().unwrap(), stage.to_str().unwrap()));
    assert_eq!(code, 1);
    let steps = parse_lines(&out);
    let last = steps.last().unwrap();
    assert_eq!((last.step.as_str(), last.status), ("verify", StepStatus::Failed), "{out}");
    assert_eq!(std::fs::read(root.join("opt/synofurb-mgmt/VERSION")).unwrap(), before);
    // a missing manifest too
    std::fs::remove_file(stage.join(MANIFEST_NAME)).unwrap();
    let (code, out, _) = run_sh(&bootstrap_script(root.to_str().unwrap(), stage.to_str().unwrap()));
    assert_eq!(code, 1);
    assert!(out.contains("MANIFEST.sha256 is missing"));
}

#[test]
fn the_bootstrap_script_quotes_what_it_embeds() {
    let s = bootstrap_script("", "/tmp/it's here");
    assert!(s.contains(r"STAGE='/tmp/it'\''s here'"), "{}", s.lines().find(|l| l.starts_with("STAGE")).unwrap_or(""));
    let s = stage_script("/tmp/a b.tar.gz", "/tmp/s t", "ab");
    assert!(s.contains("T='/tmp/a b.tar.gz'") && s.contains("S='/tmp/s t'"));
}

#[test]
fn web_urls_handle_names_and_ipv6() {
    assert_eq!(web_url("192.168.1.20"), "https://192.168.1.20:8443");
    assert_eq!(web_url(" nas.lan "), "https://nas.lan:8443");
    assert_eq!(web_url("fe80::1"), "https://[fe80::1]:8443");
    assert_eq!(web_url("[fe80::1]"), "https://[fe80::1]:8443");
}

// ---------------------------------------------------------------------------------------------- against the real script
//
// These run apps/mgmt-plane/synofurb_mgmt.py with `python3 --root <sandbox>` (skipped when python3 is missing). `update` runs with
// --dry-run, so systemctl is not really called: the swap happens inside the sandbox, the health wait then fails (no service), which
// exercises the failure path against the real NDJSON and exit codes.

const REAL_SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/mgmt-plane/synofurb_mgmt.py");

fn python_available() -> bool {
    std::process::Command::new("python3")
        .arg("-c")
        .arg("import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)")
        .status()
        .is_ok_and(|s| s.success())
}

fn run_real(root: &Path, args: &[&str]) -> ExecResult {
    let o =
        std::process::Command::new("python3").arg("-I").arg(REAL_SCRIPT).arg("--root").arg(root).args(args).output().expect("run python3");
    ExecResult {
        code: o.status.code().unwrap_or(255) as u32,
        stdout: String::from_utf8_lossy(&o.stdout).into(),
        stderr: String::from_utf8_lossy(&o.stderr).into(),
    }
}

#[test]
fn the_real_health_command_output_parses() {
    if !python_available() {
        return;
    }
    let t = TmpDir::new("real-health");
    let r = run_real(&t.0, &["health", "--json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let h = interpret_health_result(&r).unwrap_or_else(|e| panic!("{e}\n{}", r.stdout));
    assert_eq!(h.schema, HEALTH_SCHEMA);
    assert_eq!(h.version, bundled_mgmt_version(), "the script and apps/mgmt-plane/VERSION agree");
    assert_ne!(h.overall, Overall::Unknown);
    assert!(h.parse_notes.is_empty(), "the script's own output needed repairs: {:?}", h.parse_notes);
    assert!(!h.kernel.is_empty() && h.uptime_s.is_some() && h.mem.total.is_some());
    let s = h.summary();
    assert!(s.overall >= h.overall.min(Overall::Warn));
    assert!(s.bullets.iter().all(|b| b.text.chars().count() <= MAX_BULLET_CHARS));
}

#[test]
fn the_real_updater_accepts_our_staged_files_and_its_ndjson_and_exit_codes_are_understood() {
    if !python_available() || !sh_available() {
        return;
    }
    let t = TmpDir::new("real-update");
    let root = t.0.join("root");
    let opt = root.join("opt/synofurb-mgmt");
    std::fs::create_dir_all(&opt).unwrap();
    std::fs::write(opt.join("synofurb_mgmt.py"), "# old\n").unwrap();
    std::fs::write(opt.join("VERSION"), "0.1.0\n").unwrap();
    // stage exactly like push_update does: unpack our tarball
    let b = MgmtBundle::embedded();
    let stage = t.0.join("stage");
    std::fs::create_dir_all(&stage).unwrap();
    tar::Archive::new(flate2::read::GzDecoder::new(&b.tarball().unwrap()[..])).unpack(&stage).unwrap();
    let r = run_real(&root, &["--dry-run", "update", "--from", stage.to_str().unwrap(), "--json", "--health-timeout", "0.2"]);
    let steps: Vec<RemoteStep> =
        r.stdout.lines().filter_map(|l| if let Some(ManageEvent::Remote(s)) = parse_remote_line(l) { Some(s) } else { None }).collect();
    let find = |step: &str, st: StepStatus| steps.iter().any(|s| s.step == step && s.status == st);
    assert!(find("verify", StepStatus::Ok), "the real verifier rejected our files:\n{}", r.stdout);
    assert!(find("stage", StepStatus::Ok) && find("swap", StepStatus::Ok), "{}", r.stdout);
    assert!(find("health", StepStatus::Failed), "no systemd in the sandbox, so the health wait fails: {}", r.stdout);
    assert!(steps.iter().any(RemoteStep::is_rollback));
    assert_eq!(steps.len(), r.stdout.lines().count(), "every line parsed as a step");
    // exit 3 = rolled back, 4 = rollback failed; either way our failure text names the health problem, not the closing line
    assert!(matches!(r.code, 3 | 4), "exit {}", r.code);
    assert_eq!(last_failure(&steps, &r.stderr, r.code), "service is not active (health)");
    // the previous version was kept where `update --rollback` looks
    assert!(root.join("opt/synofurb-mgmt.prev/synofurb_mgmt.py").is_file());

    // a refused bundle (changed after the manifest was made) exits 2 and nothing is touched
    std::fs::write(stage.join("VERSION"), "0.0.1\n").unwrap();
    let r = run_real(&t.0.join("root2"), &["--dry-run", "update", "--from", stage.to_str().unwrap(), "--json"]);
    assert_eq!(r.code, 2, "{}", r.stdout);
    let steps = parse_lines(&r.stdout);
    assert_eq!(last_failure(&steps, &r.stderr, r.code), "checksum mismatch for VERSION (verify)");
    assert!(!t.0.join("root2/opt").exists(), "a refused update creates nothing");
}
