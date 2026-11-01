//! Plain-text renderings (ASCII only, so a dumb terminal or serial console shows them correctly). Pure: they return lines.

use super::facts::{Assessment, resolve_auto_method};
use synofurb_core::model::*;
use synofurb_core::plan::Plan;
use synofurb_core::session::confirm_phrase;

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

pub fn hardware_lines(facts: &DeviceFacts, a: &Assessment<'_>) -> Vec<String> {
    let serial = facts.serial_console();
    let mut v = vec![
        format!("Model:           {}  (serial {})", facts.model, if facts.serial.is_empty() { "unknown" } else { &facts.serial }),
        format!("DSM:             {}  host name {}", facts.dsm_version, facts.hostname),
        format!("CPU:             {}  [{}]  {}", facts.cpu, facts.cpu_vendor.label(), facts.arch.label()),
        format!("Memory:          {} MiB", facts.mem_kib / 1024),
        format!("Firmware:        {}", facts.firmware.label()),
    ];
    if let Some(d) = &facts.dom {
        v.push(format!(
            "Boot flash:      {} MiB, partitions {} / {} MiB, Synology loader recognised: {}, editable: {}",
            d.sectors * SECTOR / MIB,
            d.p1_sectors * SECTOR / MIB,
            d.p2_sectors * SECTOR / MIB,
            yn(d.p1_magic && d.p2_magic),
            yn(d.is_editable())
        ));
    }
    v.push(format!("kexec in DSM:    {}", yn(facts.kexec)));
    v.push(format!("Serial console: {}  ({})", serial.kernel_arg(), serial.raw.as_deref().unwrap_or(&serial.tty)));
    if !facts.nics.is_empty() {
        v.push(format!("Network ports:   {}", facts.nics.iter().map(|n| format!("{} {}", n.name, n.mac)).collect::<Vec<_>>().join(", ")));
    }
    v.push(format!("SSH user is root: {}", yn(facts.is_root)));
    v.push(format!("Platform:        {} ({}) - {}: {}", a.platform.platform, a.platform.soc, a.platform.support.label(), a.platform.note));
    v.push(format!("Compatibility:   {}  [{}]", a.verdict.headline, a.verdict.status.label()));
    for d in &a.verdict.details {
        v.push(format!("                 {d}"));
    }
    if let Some(b) = a.blocking() {
        v.push(format!("BLOCKED:         {b}"));
    }
    v.push(format!("Install method:  automatic = {}", resolve_auto_method(facts)));
    v
}

pub fn storage_lines(facts: &DeviceFacts) -> Vec<String> {
    let mut v = vec!["Disks:".to_string()];
    for d in &facts.disks {
        let tag = if d.is_candidate() { "" } else { "   (not an install target)" };
        v.push(format!(
            "  {:10} {:>10}  {}  {}  {} partition(s){}{}",
            d.name,
            d.human_size(),
            d.model,
            if d.serial.is_empty() { "-" } else { &d.serial },
            d.parts.len(),
            if d.rotational { ", HDD" } else { ", SSD/flash" },
            tag
        ));
    }
    if !facts.arrays.is_empty() {
        v.push("RAID arrays:".into());
        for a in &facts.arrays {
            let role = match a.role {
                ArrayRole::DsmSystem => "DSM system",
                ArrayRole::DsmSwap => "DSM swap",
                ArrayRole::Data => "data",
            };
            v.push(format!(
                "  {:6} raid{:<3} {:>10}  {:8} {}  members {}{}",
                a.name,
                a.level.trim_start_matches("raid"),
                human_bytes(a.sectors * SECTOR),
                role,
                a.state,
                a.members.join(","),
                a.vg.as_ref().map(|g| format!("  LVM {g}")).unwrap_or_default()
            ));
        }
    }
    if !facts.lvs.is_empty() {
        v.push("Logical volumes:".into());
        for l in &facts.lvs {
            v.push(format!("  {}/{}  {}", l.vg, l.name, human_bytes(l.bytes)));
        }
    }
    if !facts.mounts.is_empty() {
        v.push("Volumes in use:".into());
        for m in &facts.mounts {
            v.push(format!(
                "  {:24} {:8} {:>10} used of {:>10}  {}",
                m.mountpoint,
                m.fstype,
                human_bytes(m.used_kib * 1024),
                human_bytes(m.size_kib * 1024),
                m.device
            ));
        }
    }
    v
}

pub fn inspect_lines(facts: &DeviceFacts, a: &Assessment<'_>) -> Vec<String> {
    let mut v = hardware_lines(facts, a);
    v.push(String::new());
    v.extend(storage_lines(facts));
    v.push(String::new());
    v.push(format!("Confirmation phrase for an install: \"{}\"", confirm_phrase(facts)));
    v
}

pub fn plan_lines(facts: &DeviceFacts, plan: &Plan) -> Vec<String> {
    let mut v = vec![format!("Plan ({:?}):", plan.mode)];
    for l in plan.summary() {
        v.push(format!("  {l}"));
    }
    v.push(format!(
        "  system size: {} GiB (min {}, recommended {}, max {})",
        plan.sizes.chosen_system_mib / 1024,
        plan.sizes.min_system_mib / 1024,
        plan.sizes.recommended_system_mib / 1024,
        plan.sizes.max_system_mib / 1024
    ));
    if !plan.destroyed.is_empty() {
        v.push(String::new());
        v.push("WILL BE DESTROYED:".into());
        v.extend(plan.destroyed.iter().map(|d| format!("  - {d}")));
    }
    if !plan.preserved.is_empty() {
        v.push(String::new());
        v.push("Will be preserved (no guarantee, see warnings):".into());
        v.extend(plan.preserved.iter().map(|d| format!("  - {}: {}", d.what, d.detail)));
    }
    if !plan.untouched.is_empty() {
        v.push(format!("Disks not touched at all: {}", plan.untouched.join(", ")));
    }
    for w in &plan.warnings {
        v.push(format!("warning: {w}"));
    }
    for e in &plan.errors {
        v.push(format!("ERROR: {e}"));
    }
    v.push(String::new());
    v.push(format!("Confirmation phrase for install: \"{}\"", confirm_phrase(facts)));
    v
}

/// `plan` output: the one-line unit header plus the plan.
pub fn plan_report(facts: &DeviceFacts, plan: &Plan, a: &Assessment<'_>) -> Vec<String> {
    let mut v = vec![
        format!(
            "{} | {} | {} | DSM {} | {} MiB RAM | firmware {}",
            facts.model,
            a.platform.soc,
            facts.arch.label(),
            facts.dsm_version,
            facts.mem_kib / 1024,
            facts.firmware.label()
        ),
        format!("platform support: {} - {}", a.platform.support.label(), a.platform.note),
        format!("compatibility: {} [{}]", a.verdict.headline, a.verdict.status.label()),
        String::new(),
        "Disks:".into(),
    ];
    for d in facts.candidate_disks() {
        v.push(format!("  {:8} {:>10}  {}  {}  ({} partitions)", d.name, d.human_size(), d.model, d.serial, d.parts.len()));
    }
    v.push(String::new());
    v.extend(plan_lines(facts, plan));
    v
}

/// The three-line fallback plan shown before the gate (worded after docs/RECOVERY.md; no promise of data recovery).
pub fn fallback_lines(fw: Firmware) -> [&'static str; 3] {
    const SAVED: &str = "Saved on this computer before the first write: the first and last MiB of every disk the plan changes (partition tables only, never your files). That is not a backup of your data.";
    match fw {
        Firmware::Dom => [
            "Saved on this computer before anything else: a full image of the unit's boot flash (checked against the NAS), then the first and last MiB of every disk the plan changes (partition tables only, never your files). Neither is a backup of your data.",
            "Before the point of no return the disks are untouched and DSM stays the default boot entry. If a step fails, the installer sets the default to DSM and reboots the unit after 60 s; a power cycle does the same.",
            "After the point of no return the default is set back to DSM and the unit reboots after 15 min (time to read the log). Only the DSM loader runs: DSM's operating system on the reclaimed partitions is gone. Reinstall DSM from find.synology.com with the .pat file for your model. Synology documents keeping data volumes on a reinstall, but synofurb cannot promise that any data survives.",
        ],
        Firmware::UBoot => [
            SAVED,
            "Before the point of no return the disks are untouched. Cancelling or power-cycling boots DSM as before: synofurb never writes the unit's U-Boot flash.",
            "After it, DSM's system partition is gone. The stock U-Boot still boots DSM from flash unless you saved a changed environment over the serial console; the install log prints the original bootcmd. Reinstall DSM from find.synology.com. No data recovery is promised.",
        ],
        _ => [
            SAVED,
            "Before the point of no return the disks are untouched. Cancelling or power-cycling boots DSM as before.",
            "After it, the installer stops instead of guessing and keeps its log and a rescue shell; DSM's operating system is gone. Reinstall DSM from find.synology.com with the .pat file for your model. No data recovery is promised.",
        ],
    }
}

/// Word-wrap `text` at `width` columns with `indent` spaces in front of every line.
pub fn wrap(text: &str, width: usize, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    let mut lines = vec![];
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width.saturating_sub(indent) {
            lines.push(format!("{pad}{cur}"));
            cur.clear();
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(format!("{pad}{cur}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::super::facts::{assess, demo_facts};
    use super::*;
    use synofurb_core::compat::CompatDb;
    use synofurb_core::plan::{PlanRequest, build_plan};

    #[test]
    fn reports_are_ascii_and_complete() {
        let (f, _) = demo_facts();
        let db = CompatDb::embedded();
        let a = assess(&db, &f);
        let plan = build_plan(&f, &PlanRequest::default());
        let all: Vec<String> = [inspect_lines(&f, &a), plan_report(&f, &plan, &a)].concat();
        let text = all.join("\n");
        assert!(text.is_ascii(), "dumb terminals: ASCII only");
        for needle in ["DS918+", "Disks:", "RAID arrays:", "WILL BE DESTROYED:", "ERASE DS918+", "Compatibility:", "Serial console"] {
            assert!(text.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn fallback_copy_never_promises_recovery() {
        for fw in [Firmware::Dom, Firmware::Uefi, Firmware::Bios, Firmware::UBoot] {
            let all = fallback_lines(fw).join(" ");
            assert!(all.contains("never your files"), "{fw:?}");
            assert!(!all.to_lowercase().contains("will be restored"), "{fw:?}");
        }
    }

    #[test]
    fn wrapping() {
        let l = wrap("one two three four five six", 14, 2);
        assert!(l.iter().all(|x| x.chars().count() <= 14 && x.starts_with("  ")), "{l:?}");
        assert_eq!(l.join(" ").split_whitespace().count(), 6);
    }
}
