//! RAID facts for the desktop app: what the install plan will give (usable space, how many disks may fail), the
//! hybrid slices as table rows, and a read-only view of a unit's arrays parsed from `/proc/mdstat`.
//!
//! The desktop app only READS arrays (docs/RAID.md, "Array management"): every change operation is handed to the NAS
//! web UI. The command preview for "Create array" is built from the planner's own output, so it never promises more
//! than `synofurb-core` computes.

use super::format::size_2dp;
use synofurb_core::plan::{BlockRef, DataRaid, Plan, ShrSlice};

/// Short picker labels (the closed list of the Data RAID select).
pub fn raid_label(r: DataRaid) -> &'static str {
    match r {
        DataRaid::Auto => "Auto",
        DataRaid::None => "None",
        DataRaid::Raid0 => "RAID 0",
        DataRaid::Raid1 => "RAID 1",
        DataRaid::Raid5 => "RAID 5",
        DataRaid::Raid6 => "RAID 6",
        DataRaid::Raid10 => "RAID 10",
        DataRaid::Shr1 => "Hybrid 1-disk",
        DataRaid::Shr2 => "Hybrid 2-disk",
    }
}

pub fn is_hybrid(r: DataRaid) -> bool {
    matches!(r, DataRaid::Shr1 | DataRaid::Shr2)
}

/// How many disks may fail without losing the data volume of `plan`: the minimum over the data arrays (hybrid: over
/// the usable slices). `None` when the plan has no data array.
pub fn survives(plan: &Plan) -> Option<u8> {
    let data_arrays: Vec<&str> = plan
        .vgs
        .iter()
        .filter(|v| v.name == "vg_data")
        .flat_map(|v| v.pvs.iter())
        .filter_map(|p| if let BlockRef::Md(n) = p { Some(n.as_str()) } else { None })
        .collect();
    let mut min: Option<u8> = None;
    if let Some(shr) = &plan.shr {
        for s in shr.layout.usable() {
            min = Some(min.map_or(s.fault_tolerance, |m| m.min(s.fault_tolerance)));
        }
        return min;
    }
    for a in plan.arrays.iter().filter(|a| data_arrays.contains(&a.name.as_str())) {
        let m = a.members.len() as u8;
        let t = match a.level.as_str() {
            "1" => m.saturating_sub(1),
            "5" => 1,
            "6" => 2,
            "10" => 1,
            _ => 0,
        };
        min = Some(min.map_or(t, |x| x.min(t)));
    }
    if min.is_none() && !plan.disks.is_empty() {
        // a single data disk without an array
        min = Some(0);
    }
    min
}

/// The level the plan will really build for the data volume ("RAID 6", "Hybrid 1-disk", "Single disk"), not the request
/// ("Auto"). The Confirm After line and the Quick setup choice show this.
pub fn resolved_label(plan: &Plan) -> String {
    if plan.shr.is_some() && is_hybrid(plan.request.data_raid) {
        return raid_label(plan.request.data_raid).to_string();
    }
    let data: Vec<&str> = plan
        .vgs
        .iter()
        .filter(|v| v.name == "vg_data")
        .flat_map(|v| v.pvs.iter())
        .filter_map(|p| if let BlockRef::Md(n) = p { Some(n.as_str()) } else { None })
        .collect();
    match plan.arrays.iter().find(|a| data.contains(&a.name.as_str())) {
        Some(a) => format!("RAID {}", a.level),
        None => "Single disk".to_string(),
    }
}

/// "1 disk", "2 disks", "none".
pub fn survives_text(n: u8) -> String {
    match n {
        0 => "none".into(),
        1 => "1 disk".into(),
        n => format!("{n} disks"),
    }
}

/// One row of the hybrid slices table: `(slice, disks, level, usable)`; an unusable slice says "Unused".
pub fn slice_row(s: &ShrSlice, disks: &[String]) -> (String, String, String, String) {
    let members = s.members.iter().filter_map(|i| disks.get(*i)).cloned().collect::<Vec<_>>().join(" ");
    if s.unusable {
        (format!("{}", s.index + 1), members, "-".into(), format!("Unused ({})", size_2dp(s.len_bytes)))
    } else {
        (format!("{}", s.index + 1), members, format!("RAID {}", s.level), size_2dp(s.usable_bytes))
    }
}

// ------------------------------------------------------------------------------------------------ arrays on a unit

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayState {
    Clean,
    Degraded,
    Rebuilding,
    Foreign,
    Inactive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ArrayRow {
    pub name: String,
    /// "RAID 5", "RAID 1", ...
    pub level: String,
    pub state: ArrayState,
    pub size_bytes: u64,
    /// Resync / recovery / reshape / check progress, 0..=1.
    pub progress: Option<f32>,
    pub members: Vec<String>,
    pub failed: Vec<String>,
}

fn member_name(tok: &str) -> Option<(String, bool)> {
    let (name, rest) = tok.split_once('[')?;
    if !rest.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some((name.to_string(), rest.contains("(F)")))
}

/// Parse `/proc/mdstat`. Arrays whose name is `md0` / `md1` on a DSM unit are DSM's own system and swap arrays; the
/// caller decides what is foreign (`foreign = true` marks every clean/degraded array as created by another system).
pub fn parse_mdstat(text: &str, foreign: bool) -> Vec<ArrayRow> {
    let mut rows: Vec<ArrayRow> = vec![];
    for line in text.lines() {
        let l = line.trim();
        if let Some((name, rest)) = l.split_once(" : ").filter(|(n, _)| n.starts_with("md") && n[2..].chars().all(|c| c.is_ascii_digit())) {
            let mut it = rest.split_whitespace();
            let active = it.next() == Some("active");
            let mut level = String::new();
            let mut members = vec![];
            let mut failed = vec![];
            for tok in it {
                if let Some(n) = tok.strip_prefix("raid") {
                    level = if n == "0" || n.chars().all(|c| c.is_ascii_digit()) { format!("RAID {n}") } else { tok.to_string() };
                } else if tok == "linear" {
                    level = "Linear".into();
                } else if let Some((m, f)) = member_name(tok) {
                    if f {
                        failed.push(m.clone());
                    }
                    members.push(m);
                }
            }
            rows.push(ArrayRow {
                name: name.to_string(),
                level,
                state: if !active {
                    ArrayState::Inactive
                } else if foreign {
                    ArrayState::Foreign
                } else {
                    ArrayState::Clean
                },
                size_bytes: 0,
                progress: None,
                members,
                failed,
            });
        } else if let Some(row) = rows.last_mut() {
            if let Some((blocks, _)) = l.split_once(" blocks") {
                row.size_bytes = blocks.trim().parse::<u64>().unwrap_or(0) * 1024;
            }
            // "[3/2] [UU_]": the first number is the member count the array wants, the second how many are working
            if let Some(counts) = l.split_whitespace().find(|t| t.starts_with('[') && t.contains('/') && t.ends_with(']')) {
                let inner = counts.trim_matches(|c| c == '[' || c == ']');
                if let Some((want, have)) = inner.split_once('/')
                    && let (Ok(w), Ok(h)) = (want.parse::<u32>(), have.parse::<u32>())
                    && h < w
                    && row.state != ArrayState::Inactive
                {
                    row.state = ArrayState::Degraded;
                }
            }
            for key in ["recovery", "resync", "reshape", "check"] {
                if let Some(pos) = l.find(key)
                    && let Some(pct) = l[pos..].split('=').nth(1).and_then(|r| r.split('%').next())
                    && let Ok(p) = pct.trim().parse::<f32>()
                {
                    row.progress = Some((p / 100.0).clamp(0.0, 1.0));
                    if key != "check" && row.state != ArrayState::Foreign {
                        row.state = ArrayState::Rebuilding;
                    }
                }
            }
        }
    }
    rows
}

/// The sample arrays of the demo (a 4+4+8 TB Hybrid volume with one array rebuilding after a disk swap, plus a foreign
/// DSM array), as `/proc/mdstat` text, so the demo exercises the real parser.
pub fn demo_mdstat() -> String {
    "Personalities : [raid1] [raid6] [raid5] [raid4]\n\
md3 : active raid1 sdb5[1] sdc6[2]\n      3906961984 blocks super 1.2 [2/2] [UU]\n\n\
md2 : active raid5 sda5[0] sdb4[1] sdc5[3]\n      7813923840 blocks super 1.2 level 5, 64k chunk, algorithm 2 [3/2] [UU_]\n      [=====>...............]  recovery = 24.3% (949000000/3906961920) finish=214.5min speed=229000K/sec\n\n\
md1 : active raid1 sda2[0] sdb2[1] sdc2[2]\n      2097088 blocks [3/3] [UUU]\n\n\
md0 : active raid1 sda1[0] sdb1[1] sdc1[2]\n      8388544 blocks [3/3] [UUU]\n\n\
unused devices: <none>\n"
        .to_string()
}

/// The commands "Create array" would run for `plan` (a read-only preview: the web UI runs them).
pub fn create_commands(plan: &Plan) -> Vec<String> {
    let mut v = vec![];
    for a in &plan.arrays {
        let devs: Vec<String> = a.members.iter().map(|(d, n)| format!("/dev/{d}{n}")).collect();
        let level = if a.level == "1" { "1".to_string() } else { a.level.clone() };
        v.push(format!(
            "mdadm --create /dev/md/{} --metadata=1.2 --level={} --raid-devices={} {}",
            a.name,
            level,
            devs.len(),
            devs.join(" ")
        ));
    }
    for g in &plan.vgs {
        if g.name != "vg_data" {
            continue;
        }
        let pvs: Vec<String> = g
            .pvs
            .iter()
            .map(|p| match p {
                BlockRef::Md(n) => format!("/dev/md/{n}"),
                BlockRef::Part(d, n) => format!("/dev/{d}{n}"),
            })
            .collect();
        v.push(format!("pvcreate {}", pvs.join(" ")));
        v.push(format!("vgcreate vg_data {}", pvs.join(" ")));
        for l in &g.lvs {
            v.push(format!(
                "lvcreate -n {} -l {} vg_data",
                l.name,
                l.vg_percent.map(|p| format!("{p}%VG")).unwrap_or_else(|| "100%FREE".into())
            ));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use synofurb_core::inspect::{fixtures, parse_inspect};
    use synofurb_core::plan::{PlanRequest, build_plan};

    #[test]
    fn mdstat_demo_parses_into_the_expected_states() {
        let rows = parse_mdstat(&demo_mdstat(), false);
        assert_eq!(rows.len(), 4);
        let by = |n: &str| rows.iter().find(|r| r.name == n).unwrap();
        assert_eq!(by("md2").state, ArrayState::Rebuilding);
        assert!((by("md2").progress.unwrap() - 0.243).abs() < 1e-3);
        assert_eq!(by("md2").level, "RAID 5");
        assert_eq!(by("md2").size_bytes, 7_813_923_840 * 1024);
        assert_eq!(by("md3").state, ArrayState::Clean);
        assert_eq!(by("md3").members, vec!["sdb5".to_string(), "sdc6".to_string()]);
        assert_eq!(by("md0").level, "RAID 1");
    }

    #[test]
    fn a_missing_member_is_degraded_and_foreign_marks_clean_arrays() {
        let t = "md5 : active raid5 sda1[0] sdb1[1] sdc1[2](F)\n      100 blocks super 1.2 level 5, 64k chunk [3/2] [UU_]\n";
        let r = parse_mdstat(t, false);
        assert_eq!(r[0].state, ArrayState::Degraded);
        assert_eq!(r[0].failed, vec!["sdc1".to_string()]);
        let f = parse_mdstat("md7 : active raid1 sda1[0] sdb1[1]\n      100 blocks [2/2] [UU]\n", true);
        assert_eq!(f[0].state, ArrayState::Foreign);
        assert_eq!(parse_mdstat("md8 : inactive sda1[0]\n      100 blocks\n", false)[0].state, ArrayState::Inactive);
        assert!(parse_mdstat("Personalities : [raid1]\nunused devices: <none>\n", false).is_empty());
    }

    #[test]
    fn survives_follows_the_plan() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&f, &PlanRequest { data_raid: DataRaid::Raid5, ..PlanRequest::default() });
        assert_eq!(survives(&plan), Some(1));
        let r6 = build_plan(&f, &PlanRequest { data_raid: DataRaid::Raid6, ..PlanRequest::default() });
        assert_eq!(survives(&r6), Some(2));
        let r0 = build_plan(&f, &PlanRequest { data_raid: DataRaid::Raid0, ..PlanRequest::default() });
        assert_eq!(survives(&r0), Some(0));
        let shr = build_plan(&f, &PlanRequest { data_raid: DataRaid::Shr1, ..PlanRequest::default() });
        assert_eq!(survives(&shr), Some(1));
        assert_eq!(survives_text(0), "none");
        assert_eq!(survives_text(2), "2 disks");
    }

    #[test]
    fn hybrid_rows_and_commands_come_from_the_planner() {
        let f = parse_inspect(&fixtures::ds_4bay()).unwrap();
        let plan = build_plan(&f, &PlanRequest { data_raid: DataRaid::Shr1, ..PlanRequest::default() });
        let shr = plan.shr.as_ref().expect("hybrid plan has slices");
        let rows: Vec<_> = shr.layout.slices.iter().map(|s| slice_row(s, &shr.disks)).collect();
        assert!(!rows.is_empty() && rows[0].2.starts_with("RAID"));
        let cmds = create_commands(&plan);
        assert!(cmds.iter().any(|c| c.starts_with("mdadm --create /dev/md/data1")), "{cmds:?}");
        assert!(cmds.iter().any(|c| c.starts_with("vgcreate vg_data")));
        assert!(is_hybrid(DataRaid::Shr2) && !is_hybrid(DataRaid::Raid5));
        assert_eq!(raid_label(DataRaid::Shr1), "Hybrid 1-disk");
    }
}
