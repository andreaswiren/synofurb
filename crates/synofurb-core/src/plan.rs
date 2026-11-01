//! Storage planner.
//!
//! Turns a [`DeviceFacts`] snapshot plus the user's choices into a fully
//! explicit [`Plan`]: which partitions are deleted, which are created (with exact
//! sector offsets), which md/LVM objects are built, and what is preserved or
//! destroyed. The plan is the single source of truth for both the confirmation
//! screen and the generated installer script, and [`Plan::validate`] enforces
//! the safety invariants (no overlap, never delete a preserved partition, ...).
//!
//! Layout (per system disk), in order:
//!
//! | BIOS               | UEFI / U-Boot      | purpose                                  |
//! |--------------------|--------------------|------------------------------------------|
//! | `bios_grub` 1 MiB  | ESP 512 MiB (UEFI) | bootloader (ESP mirrored with md 1.0)    |
//! | boot 1 GiB         | boot 1 GiB         | `/boot`, ext4, md RAID1 metadata 1.0     |
//! | system PV          | system PV          | md RAID1 -> LVM `vg_sys` (swap + root, xfs) |
//! | data (rest)        | data (rest)        | md RAID -> LVM `vg_data` (xfs), wiped disks only |

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MIN_SYSTEM_MIB: u64 = 8 * 1024;
pub const MAX_SYSTEM_MIB: u64 = 256 * 1024;
pub const BIOS_GRUB_MIB: u64 = 1;
pub const ESP_MIB: u64 = 512;
pub const BOOT_MIB: u64 = 1024;
const FIRST_USABLE: u64 = 2048;
/// U-Boot ARM boards (Debian wiki, InstallingDebianOn/Synology): kernel and initrd live in raw
/// sectors at 10 MiB / 20 MiB and the first 50 MiB of the disk stay out of the GPT.
pub const UBOOT_KERNEL_MIB: u64 = 10;
pub const UBOOT_INITRD_MIB: u64 = 20;
pub const UBOOT_RESERVED_MIB: u64 = 50;
const GPT_TAIL: u64 = 34;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageMode {
    /// Destroy everything on every disk and start with a clean layout.
    WipeAll,
    /// Reclaim only DSM's system/swap partitions (plus any array the user releases);
    /// preserved arrays and unknown partitions are never touched.
    KeepVolumes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataRaid {
    Auto,
    None,
    Raid0,
    Raid1,
    Raid5,
    Raid6,
    Raid10,
}

impl DataRaid {
    pub fn level(self) -> &'static str {
        match self {
            DataRaid::Auto | DataRaid::None => "none",
            DataRaid::Raid0 => "0",
            DataRaid::Raid1 => "1",
            DataRaid::Raid5 => "5",
            DataRaid::Raid6 => "6",
            DataRaid::Raid10 => "10",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DataRaid::Auto => "Automatic",
            DataRaid::None => "No RAID",
            DataRaid::Raid0 => "RAID 0 (stripe, no redundancy)",
            DataRaid::Raid1 => "RAID 1 (mirror)",
            DataRaid::Raid5 => "RAID 5",
            DataRaid::Raid6 => "RAID 6",
            DataRaid::Raid10 => "RAID 10",
        }
    }
    pub fn min_members(self) -> usize {
        match self {
            DataRaid::Auto | DataRaid::None => 1,
            DataRaid::Raid0 | DataRaid::Raid1 => 2,
            DataRaid::Raid5 => 3,
            DataRaid::Raid6 | DataRaid::Raid10 => 4,
        }
    }
    pub const ALL: [DataRaid; 7] =
        [DataRaid::Auto, DataRaid::None, DataRaid::Raid0, DataRaid::Raid1, DataRaid::Raid5, DataRaid::Raid6, DataRaid::Raid10];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub mode: StorageMode,
    /// Number of disks carrying a copy of the system. `None` = automatic.
    pub system_copies: Option<usize>,
    /// Size of the system LVM physical volume in MiB. `0` = recommended.
    pub system_size_mib: u64,
    pub data_raid: DataRaid,
    /// Percentage of `vg_data` left unallocated for later volumes/snapshots (0..=90).
    pub data_free_percent: u8,
    /// Data arrays to preserve in `KeepVolumes` mode. `None` = all of them.
    pub keep_arrays: Option<Vec<String>>,
}

impl Default for PlanRequest {
    fn default() -> Self {
        PlanRequest {
            mode: StorageMode::WipeAll,
            system_copies: None,
            system_size_mib: 0,
            data_raid: DataRaid::Auto,
            data_free_percent: 20,
            keep_arrays: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartRole {
    BiosGrub,
    Esp,
    Boot,
    SystemPv,
    Data,
}

impl PartRole {
    pub fn label(self) -> &'static str {
        match self {
            PartRole::BiosGrub => "bios_grub",
            PartRole::Esp => "esp",
            PartRole::Boot => "boot",
            PartRole::SystemPv => "system",
            PartRole::Data => "data",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPart {
    pub number: u32,
    pub start: u64,
    pub sectors: u64,
    pub role: PartRole,
}

impl NewPart {
    pub fn end(&self) -> u64 {
        self.start + self.sectors
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskIdent {
    pub model: String,
    pub serial: String,
    pub sectors: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskPlan {
    /// Name in the running DSM (informational; the installer matches by `ident`).
    pub disk: String,
    pub ident: DiskIdent,
    /// Create a brand-new GPT (destroys the whole table).
    pub new_table: bool,
    /// Existing partition numbers to delete (only when `new_table` is false).
    pub delete: Vec<u32>,
    pub create: Vec<NewPart>,
    /// Existing partition table that must match when the installer starts (safety check).
    pub expect: Vec<(u32, u64, u64)>,
}

/// A block device used as a building block: a partition or an md array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockRef {
    /// (DSM disk name, partition number)
    Part(String, u32),
    /// md array name from [`ArrayPlan::name`]
    Md(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayPlan {
    pub name: String,
    /// "0", "1", "5", "6", "10"
    pub level: String,
    pub metadata: String,
    pub members: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LvPlan {
    pub name: String,
    /// Size in MiB, or `None` for a percentage of the VG.
    pub mib: Option<u64>,
    pub vg_percent: Option<u8>,
    pub fs: String,
    pub mountpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VgPlan {
    pub name: String,
    pub pv: BlockRef,
    pub lvs: Vec<LvPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preserved {
    pub what: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Sizes {
    pub min_system_mib: u64,
    pub recommended_system_mib: u64,
    pub max_system_mib: u64,
    pub chosen_system_mib: u64,
    pub swap_mib: u64,
    pub root_mib: u64,
    pub vg_sys_free_mib: u64,
    pub data_raw_bytes: u64,
    pub data_usable_bytes: u64,
    pub data_lv_percent: u8,
}

/// Raw boot area for U-Boot boards, offsets in MiB from the start of each system disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawBoot {
    pub kernel_mib: u64,
    pub initrd_mib: u64,
    pub reserved_mib: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub mode: StorageMode,
    pub arch: Arch,
    pub firmware: Firmware,
    pub request: PlanRequest,
    pub system_disks: Vec<String>,
    pub disks: Vec<DiskPlan>,
    /// Disks that are not modified at all.
    pub untouched: Vec<String>,
    /// Arrays that must be stopped (and have their superblocks cleared) before partitioning.
    pub release_arrays: Vec<String>,
    pub arrays: Vec<ArrayPlan>,
    pub vgs: Vec<VgPlan>,
    /// `/boot` device. `None` on U-Boot boards (kernel and initrd are written raw).
    pub boot: Option<BlockRef>,
    pub esp: Option<BlockRef>,
    pub raw_boot: Option<RawBoot>,
    /// Debian boots through an entry on the unit's internal boot flash (DOM), see `boot/dom`.
    #[serde(default)]
    pub dom_boot: bool,
    pub preserved: Vec<Preserved>,
    /// Names of the preserved md arrays (DSM names, e.g. `md2`).
    pub kept_arrays: Vec<String>,
    pub destroyed: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub sizes: Sizes,
}

impl Plan {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

fn align_up(v: u64, a: u64) -> u64 {
    v.div_ceil(a) * a
}
fn align_down(v: u64, a: u64) -> u64 {
    (v / a) * a
}

pub fn swap_mib_for(mem_kib: u64) -> u64 {
    let ram_mib = mem_kib / 1024;
    ram_mib.clamp(1024, 4096)
}

/// The LV split of the system PV: swap, root, and a reserve left free in the VG.
pub fn split_system(pv_mib: u64, mem_kib: u64) -> (u64, u64, u64) {
    let swap = swap_mib_for(mem_kib);
    let reserve = (pv_mib / 10).max(512);
    let usable = pv_mib.saturating_sub(4); // LVM metadata
    let root = usable.saturating_sub(swap + reserve);
    (swap, root, usable.saturating_sub(swap + root))
}

#[derive(Debug, Clone)]
struct Extent {
    start: u64,
    end: u64,
}

struct DiskAnalysis<'a> {
    disk: &'a Disk,
    /// Partition numbers that will be deleted.
    reclaim: Vec<u32>,
    /// True when nothing on the disk is preserved.
    fully_wiped: bool,
    /// Free/reclaimed extents (sector ranges, MiB aligned), largest first.
    extents: Vec<Extent>,
    /// MBR disk with preserved partitions: cannot be modified.
    locked: bool,
}

fn array_of<'a>(facts: &'a DeviceFacts, part: &str) -> Option<&'a MdArray> {
    facts.arrays.iter().find(|a| a.members.iter().any(|m| m == part))
}

fn analyse<'a>(facts: &'a DeviceFacts, disk: &'a Disk, mode: StorageMode, kept: &BTreeSet<String>) -> DiskAnalysis<'a> {
    let reclaimable = |p: &Part| -> bool {
        match mode {
            StorageMode::WipeAll => true,
            StorageMode::KeepVolumes => match array_of(facts, &p.name) {
                Some(a) => a.role != ArrayRole::Data || !kept.contains(&a.name),
                None => false, // unknown partition: preserve
            },
        }
    };
    let reclaim: Vec<u32> = disk.parts.iter().filter(|p| reclaimable(p)).map(|p| p.number).collect();
    let fully_wiped = reclaim.len() == disk.parts.len();
    let locked = !fully_wiped && disk.table != PartTable::Gpt;
    let disk_end = align_down(disk.sectors.saturating_sub(GPT_TAIL), SECTORS_PER_MIB);

    let mut extents = vec![];
    if !locked {
        if fully_wiped {
            extents.push(Extent { start: FIRST_USABLE, end: disk_end });
        } else {
            let mut cursor = FIRST_USABLE;
            let mut parts: Vec<&Part> = disk.parts.iter().collect();
            parts.sort_by_key(|p| p.start);
            for p in parts {
                if !reclaimable(p) {
                    if p.start > cursor {
                        extents.push(Extent { start: cursor, end: p.start });
                    }
                    cursor = cursor.max(p.end());
                }
            }
            if disk_end > cursor {
                extents.push(Extent { start: cursor, end: disk_end });
            }
        }
    }
    for e in &mut extents {
        e.start = align_up(e.start, SECTORS_PER_MIB);
        e.end = align_down(e.end, SECTORS_PER_MIB);
    }
    extents.retain(|e| e.end > e.start);
    extents.sort_by_key(|e| std::cmp::Reverse(e.end - e.start));
    DiskAnalysis { disk, reclaim, fully_wiped, extents, locked }
}

fn is_sys_or_free(e: &Extent) -> bool {
    e.end > e.start
}

fn fw_overhead_mib(fw: Firmware) -> u64 {
    match fw {
        Firmware::Bios => BIOS_GRUB_MIB + BOOT_MIB,
        Firmware::Uefi => ESP_MIB + BOOT_MIB,
        // The reserved raw area is handled by `layout_start`, not as overhead.
        // DOM units keep kernel + initrd on the internal flash module, not on the disks.
        Firmware::UBoot | Firmware::Dom => 0,
    }
}

/// First sector of the system layout inside an extent.
fn layout_start(fw: Firmware, ext_start: u64) -> u64 {
    if fw == Firmware::UBoot { ext_start.max(UBOOT_RESERVED_MIB * SECTORS_PER_MIB) } else { ext_start }
}

/// On U-Boot boards the raw kernel/initrd area (10..50 MiB) must be inside the extent.
fn raw_area_free(fw: Firmware, e: &Extent) -> bool {
    fw != Firmware::UBoot || (e.start <= UBOOT_KERNEL_MIB * SECTORS_PER_MIB && e.end >= UBOOT_RESERVED_MIB * SECTORS_PER_MIB)
}

/// Recommended size of the system PV given the space available on the chosen disks.
pub fn recommended_system_mib(smallest_disk_bytes: u64, max_mib: u64) -> u64 {
    let want = if smallest_disk_bytes >= 300 * GIB {
        32 * 1024
    } else if smallest_disk_bytes >= 60 * GIB {
        16 * 1024
    } else {
        MIN_SYSTEM_MIB
    };
    want.min(max_mib).max(MIN_SYSTEM_MIB.min(max_mib))
}

fn data_usable(level: &str, members: usize, min_bytes: u64) -> u64 {
    let m = members as u64;
    match level {
        "none" | "1" => min_bytes,
        "0" => m * min_bytes,
        "5" => (m - 1) * min_bytes,
        "6" => (m - 2) * min_bytes,
        "10" => m * min_bytes / 2,
        _ => 0,
    }
}

fn auto_data_level(members: usize) -> &'static str {
    match members {
        0 | 1 => "none",
        2 => "1",
        3..=5 => "5",
        _ => "6",
    }
}

pub fn default_system_copies(candidates: usize, eligible: usize) -> usize {
    let n = eligible.min(candidates);
    match n {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 3,
    }
}

/// Build a plan. Never panics; problems are reported in `errors`/`warnings`.
pub fn build_plan(facts: &DeviceFacts, req: &PlanRequest) -> Plan {
    let fw = facts.firmware;
    let mut plan = Plan {
        mode: req.mode,
        arch: facts.arch,
        firmware: fw,
        request: req.clone(),
        system_disks: vec![],
        disks: vec![],
        untouched: vec![],
        release_arrays: vec![],
        arrays: vec![],
        vgs: vec![],
        boot: None,
        esp: None,
        raw_boot: None,
        dom_boot: false,
        preserved: vec![],
        kept_arrays: vec![],
        destroyed: vec![],
        warnings: vec![],
        errors: vec![],
        sizes: Sizes::default(),
    };

    if facts.arch == Arch::Unsupported {
        plan.errors.push(format!("CPU architecture '{}' is not supported by Debian 13.", facts.uname_m));
        return plan;
    }
    if req.data_free_percent > 90 {
        plan.errors.push("Free space in the data volume group must be between 0 and 90 percent.".into());
        return plan;
    }
    let candidates = facts.candidate_disks();
    if candidates.is_empty() {
        plan.errors.push("No internal disks of at least 8 GiB were found.".into());
        return plan;
    }
    if facts.mem_kib > 0 && facts.mem_kib < 480 * 1024 {
        plan.errors.push(format!("Only {} MiB of RAM: the Debian installer needs about 512 MiB.", facts.mem_kib / 1024));
    } else if facts.mem_kib < 900 * 1024 {
        plan.warnings
            .push("Less than 1 GiB of RAM: installation works but the system will be tight; keep the management services light.".into());
    }
    if fw == Firmware::UBoot {
        plan.warnings.push("This ARM board has no UEFI. Debian boots from raw disk sectors through U-Boot; the U-Boot environment must be changed once over the serial console (Debian wiki: InstallingDebianOn/Synology). Keep a TTL serial adapter ready (115200 baud).".into());
    }

    let kept: BTreeSet<String> = match (req.mode, &req.keep_arrays) {
        (StorageMode::WipeAll, _) => BTreeSet::new(),
        (StorageMode::KeepVolumes, Some(k)) => k.iter().cloned().collect(),
        (StorageMode::KeepVolumes, None) => facts.data_arrays().iter().map(|a| a.name.clone()).collect(),
    };
    for k in &kept {
        match facts.arrays.iter().find(|a| &a.name == k) {
            None => plan.errors.push(format!("Array {k} does not exist.")),
            Some(a) if a.role != ArrayRole::Data => {
                plan.errors.push(format!("Array {k} is a DSM system/swap array and cannot be preserved."))
            }
            _ => {}
        }
    }

    let analyses: Vec<DiskAnalysis> = candidates.iter().map(|d| analyse(facts, d, req.mode, &kept)).collect();

    // -- eligibility for the system
    let overhead = fw_overhead_mib(fw);
    let need_sectors_min = (overhead + MIN_SYSTEM_MIB) * SECTORS_PER_MIB;
    let eligible: Vec<&DiskAnalysis> = analyses
        .iter()
        .filter(|a| {
            !a.locked
                && a.extents
                    .first()
                    .is_some_and(|e| raw_area_free(fw, e) && e.end.saturating_sub(layout_start(fw, e.start)) >= need_sectors_min)
        })
        .collect();

    for a in analyses.iter().filter(|a| a.locked) {
        plan.warnings
            .push(format!("Disk {} uses an MBR partition table with preserved partitions; it is left completely untouched.", a.disk.name));
    }

    let copies = match req.system_copies {
        Some(c) => c,
        None => default_system_copies(candidates.len(), eligible.len()),
    };
    if copies == 0 || copies > eligible.len() {
        let msg = if eligible.is_empty() {
            format!(
                "No disk has {} MiB of reclaimable space for the system (needs boot area + at least {} GiB). \
                 In 'keep volumes' mode DSM's own system partitions are only about 2-10 GiB; \
                 choose 'wipe everything', release a data array, or use a dedicated SSD.",
                overhead + MIN_SYSTEM_MIB,
                MIN_SYSTEM_MIB / 1024
            )
        } else {
            format!("Requested {copies} system copies but only {} disks can hold the system.", eligible.len())
        };
        plan.errors.push(msg);
        return plan;
    }
    if candidates.len() > 2 && copies < 2 {
        plan.errors.push("With more than two disks the system partition must be redundant (at least 2 copies).".into());
        return plan;
    }
    if candidates.len() == 2 && copies == 1 {
        plan.warnings.push("System on a single disk: if that disk fails the NAS will not boot (data is not affected).".into());
    }
    if candidates.len() == 1 {
        plan.warnings.push("Only one disk: neither the system nor the data is redundant.".into());
    }

    let sys: Vec<&DiskAnalysis> = eligible.iter().take(copies).copied().collect();
    plan.system_disks = sys.iter().map(|a| a.disk.name.clone()).collect();

    // -- sizes
    let max_mib_raw =
        sys.iter().map(|a| (a.extents[0].end - layout_start(fw, a.extents[0].start)) / SECTORS_PER_MIB - overhead - 1).min().unwrap_or(0);
    let max_mib = max_mib_raw.min(MAX_SYSTEM_MIB);
    let smallest = sys.iter().map(|a| a.disk.size_bytes()).min().unwrap_or(0);
    let rec = recommended_system_mib(smallest, max_mib);
    let chosen = if req.system_size_mib == 0 { rec } else { req.system_size_mib };
    plan.sizes.min_system_mib = MIN_SYSTEM_MIB;
    plan.sizes.recommended_system_mib = rec;
    plan.sizes.max_system_mib = max_mib;
    plan.sizes.chosen_system_mib = chosen;
    if chosen < MIN_SYSTEM_MIB {
        plan.errors.push(format!("System size must be at least {} GiB.", MIN_SYSTEM_MIB / 1024));
    }
    if chosen > max_mib {
        plan.errors.push(format!("System size {} GiB exceeds the {} GiB available on the system disks.", chosen / 1024, max_mib / 1024));
    }
    if !plan.errors.is_empty() {
        return plan;
    }
    let (swap, root, free) = split_system(chosen, facts.mem_kib);
    plan.sizes.swap_mib = swap;
    plan.sizes.root_mib = root;
    plan.sizes.vg_sys_free_mib = free;

    // -- per-disk partition layout
    let mut sys_members: Vec<(String, u32)> = vec![];
    let mut boot_members: Vec<(String, u32)> = vec![];
    let mut esp_members: Vec<(String, u32)> = vec![];
    let mut data_members: Vec<(String, u32)> = vec![];
    let mut data_bytes: Vec<u64> = vec![];

    for a in &analyses {
        let d = a.disk;
        let is_sys = plan.system_disks.contains(&d.name);
        let touched = is_sys || (!a.locked && !a.reclaim.is_empty()) || (!a.locked && a.fully_wiped && !a.extents.is_empty());
        if !touched {
            plan.untouched.push(d.name.clone());
            continue;
        }
        let mut dp = DiskPlan {
            disk: d.name.clone(),
            ident: DiskIdent { model: d.model.clone(), serial: d.serial.clone(), sectors: d.sectors },
            new_table: a.fully_wiped,
            delete: if a.fully_wiped { vec![] } else { a.reclaim.clone() },
            create: vec![],
            expect: d.parts.iter().map(|p| (p.number, p.start, p.sectors)).collect(),
        };
        // free partition numbers after deletions
        let mut used: BTreeSet<u32> = if a.fully_wiped {
            BTreeSet::new()
        } else {
            d.parts.iter().filter(|p| !a.reclaim.contains(&p.number)).map(|p| p.number).collect()
        };
        let mut next_num = || {
            let n = (1..=128u32).find(|n| !used.contains(n)).unwrap_or(129);
            used.insert(n);
            n
        };
        let ext = a.extents.first().cloned().unwrap_or(Extent { start: 0, end: 0 });
        let mut cursor = if is_sys { layout_start(fw, ext.start) } else { ext.start };
        let mut add = |dp: &mut DiskPlan, sectors: u64, role: PartRole, cursor: &mut u64| -> u32 {
            let n = next_num();
            dp.create.push(NewPart { number: n, start: *cursor, sectors, role });
            *cursor += sectors;
            n
        };
        if is_sys {
            match fw {
                Firmware::Bios => {
                    add(&mut dp, BIOS_GRUB_MIB * SECTORS_PER_MIB, PartRole::BiosGrub, &mut cursor);
                }
                Firmware::Uefi => {
                    let n = add(&mut dp, ESP_MIB * SECTORS_PER_MIB, PartRole::Esp, &mut cursor);
                    esp_members.push((d.name.clone(), n));
                }
                Firmware::UBoot | Firmware::Dom => {}
            }
            if !matches!(fw, Firmware::UBoot | Firmware::Dom) {
                let n = add(&mut dp, BOOT_MIB * SECTORS_PER_MIB, PartRole::Boot, &mut cursor);
                boot_members.push((d.name.clone(), n));
            }
            let n = add(&mut dp, chosen * SECTORS_PER_MIB, PartRole::SystemPv, &mut cursor);
            sys_members.push((d.name.clone(), n));
        }
        if a.fully_wiped && is_sys_or_free(&ext) && ext.end > cursor + SECTORS_PER_MIB {
            let sectors = ext.end - cursor;
            let n = add(&mut dp, sectors, PartRole::Data, &mut cursor);
            data_members.push((d.name.clone(), n));
            data_bytes.push(sectors * SECTOR);
        } else if is_sys && !a.fully_wiped && ext.end > cursor {
            plan.warnings.push(format!(
                "Disk {}: {} of reclaimed space stays unallocated.",
                d.name,
                human_bytes((ext.end - cursor) * SECTOR)
            ));
        }
        plan.disks.push(dp);
    }

    // -- arrays
    let mdref = |members: &Vec<(String, u32)>, name: &str, level: &str, meta: &str, plan: &mut Plan| -> BlockRef {
        if members.len() == 1 {
            BlockRef::Part(members[0].0.clone(), members[0].1)
        } else {
            plan.arrays.push(ArrayPlan { name: name.into(), level: level.into(), metadata: meta.into(), members: members.clone() });
            BlockRef::Md(name.into())
        }
    };
    if fw == Firmware::Uefi {
        let r = mdref(&esp_members, "esp", "1", "1.0", &mut plan);
        plan.esp = Some(r);
    }
    if fw == Firmware::Dom {
        plan.dom_boot = true;
    } else if fw == Firmware::UBoot {
        plan.raw_boot = Some(RawBoot { kernel_mib: UBOOT_KERNEL_MIB, initrd_mib: UBOOT_INITRD_MIB, reserved_mib: UBOOT_RESERVED_MIB });
    } else {
        plan.boot = Some(mdref(&boot_members, "boot", "1", "1.0", &mut plan));
    }
    let sys_pv = mdref(&sys_members, "system", "1", "1.2", &mut plan);
    plan.vgs.push(VgPlan {
        name: "vg_sys".into(),
        pv: sys_pv,
        lvs: vec![
            LvPlan { name: "swap".into(), mib: Some(swap), vg_percent: None, fs: "swap".into(), mountpoint: "none".into() },
            LvPlan { name: "root".into(), mib: Some(root), vg_percent: None, fs: "xfs".into(), mountpoint: "/".into() },
        ],
    });

    // -- data
    if !data_members.is_empty() {
        let n = data_members.len();
        let level: String = match req.data_raid {
            DataRaid::Auto => auto_data_level(n).to_string(),
            other => other.level().to_string(),
        };
        let min_members = match level.as_str() {
            "none" => 1,
            "0" | "1" => 2,
            "5" => 3,
            _ => 4,
        };
        if req.data_raid != DataRaid::Auto && n < min_members {
            plan.errors.push(format!("{} needs at least {} disks for data but {} are available.", req.data_raid.label(), min_members, n));
            return plan;
        }
        if level == "none" && n > 1 {
            plan.warnings
                .push("Data disks without RAID are concatenated by LVM: losing any one disk loses the whole data volume group.".into());
        }
        if level == "0" {
            plan.warnings.push("RAID 0 has no redundancy: losing any disk loses all data.".into());
        }
        let min_bytes = *data_bytes.iter().min().unwrap();
        let max_bytes = *data_bytes.iter().max().unwrap();
        if max_bytes - min_bytes > 10 * GIB {
            plan.warnings.push(format!(
                "Data partitions differ by {}: RAID uses only the smallest size on every member.",
                human_bytes(max_bytes - min_bytes)
            ));
        }
        if facts.arch == Arch::Armhf && data_usable(&level, n, min_bytes) > 16 * 1024 * GIB {
            plan.warnings.push("32-bit ARM kernels can address at most 16 TiB per filesystem; the part of the data volume above 16 TiB cannot be used in a single xfs filesystem. Consider a smaller data LV.".into());
        }
        plan.sizes.data_raw_bytes = data_bytes.iter().sum();
        plan.sizes.data_usable_bytes = data_usable(&level, n, min_bytes);
        let pv = if level == "none" && n > 1 {
            // one VG spanning plain partitions: model as separate PVs via a special array name
            plan.arrays.push(ArrayPlan {
                name: "data".into(),
                level: "linear".into(),
                metadata: "1.2".into(),
                members: data_members.clone(),
            });
            BlockRef::Md("data".into())
        } else if n == 1 {
            BlockRef::Part(data_members[0].0.clone(), data_members[0].1)
        } else {
            plan.arrays.push(ArrayPlan {
                name: "data".into(),
                level: level.clone(),
                metadata: "1.2".into(),
                members: data_members.clone(),
            });
            BlockRef::Md("data".into())
        };
        let pct = 100 - req.data_free_percent;
        plan.sizes.data_lv_percent = pct;
        plan.vgs.push(VgPlan {
            name: "vg_data".into(),
            pv,
            lvs: if pct == 0 {
                vec![]
            } else {
                vec![LvPlan { name: "data".into(), mib: None, vg_percent: Some(pct), fs: "xfs".into(), mountpoint: "/srv/data".into() }]
            },
        });
    } else if req.mode == StorageMode::WipeAll {
        plan.errors.push("No disk space left for data after reserving the system area.".into());
    }

    // -- release / preserve / destroy bookkeeping
    for arr in &facts.arrays {
        let reclaimed = arr.role != ArrayRole::Data || !kept.contains(&arr.name) || req.mode == StorageMode::WipeAll;
        let size = human_bytes(arr.sectors * SECTOR);
        let vol_desc: Vec<String> = facts
            .mounts
            .iter()
            .filter(|m| arr.vg.as_ref().is_some_and(|vg| m.device.contains(vg)) || m.device == format!("/dev/{}", arr.name))
            .map(|m| {
                format!("{} ({}, {} used of {})", m.mountpoint, m.fstype, human_bytes(m.used_kib * 1024), human_bytes(m.size_kib * 1024))
            })
            .collect();
        let lv_desc: Vec<String> = arr
            .vg
            .as_ref()
            .map(|vg| facts.lvs.iter().filter(|l| &l.vg == vg).map(|l| format!("{}/{}", l.vg, l.name)).collect())
            .unwrap_or_default();
        let mut detail = format!("{} {} {}", arr.name, arr.level, size);
        if !lv_desc.is_empty() {
            detail.push_str(&format!(", LVM {}", lv_desc.join(", ")));
        }
        if !vol_desc.is_empty() {
            detail.push_str(&format!(", mounted as {}", vol_desc.join(", ")));
        }
        if reclaimed {
            plan.release_arrays.push(arr.name.clone());
            let what = match arr.role {
                ArrayRole::DsmSystem => "DSM system volume",
                ArrayRole::DsmSwap => "DSM swap",
                ArrayRole::Data => "Data volume",
            };
            plan.destroyed.push(format!("{what}: {detail}"));
        } else {
            plan.kept_arrays.push(arr.name.clone());
            plan.preserved.push(Preserved { what: "Data volume".into(), detail });
        }
    }
    for a in &analyses {
        for p in a.disk.parts.iter().filter(|p| a.reclaim.contains(&p.number)) {
            if array_of(facts, &p.name).is_none() {
                plan.destroyed.push(format!(
                    "Partition {} on {} ({}), contents unknown",
                    p.name,
                    a.disk.name,
                    human_bytes(p.sectors * SECTOR)
                ));
            }
        }
        if req.mode == StorageMode::KeepVolumes {
            for p in a.disk.parts.iter().filter(|p| !a.reclaim.contains(&p.number)) {
                if array_of(facts, &p.name).is_none() {
                    plan.preserved.push(Preserved {
                        what: "Partition".into(),
                        detail: format!("{} on {} ({}), contents unknown", p.name, a.disk.name, human_bytes(p.sectors * SECTOR)),
                    });
                }
            }
        }
    }
    // DSM data volumes (/volume1, ...) live on LVM/md devices we cannot always map back to a single
    // array (cachedev_N), so describe them once and attach them to whichever side holds the data arrays.
    let volumes: Vec<String> = facts
        .mounts
        .iter()
        .filter(|m| m.mountpoint.starts_with("/volume"))
        .map(|m| format!("{} ({}, {} used of {})", m.mountpoint, m.fstype, human_bytes(m.used_kib * 1024), human_bytes(m.size_kib * 1024)))
        .collect();
    if !volumes.is_empty() {
        if plan.kept_arrays.is_empty() {
            plan.destroyed.push(format!("DSM volumes with all their shares and files: {}", volumes.join("; ")));
        } else {
            plan.preserved.push(Preserved {
                what: "DSM volumes".into(),
                detail: format!(
                    "{} (kept intact on the preserved arrays; shares and DSM settings are not carried over)",
                    volumes.join("; ")
                ),
            });
        }
    }
    if req.mode == StorageMode::KeepVolumes && plan.preserved.is_empty() {
        plan.warnings.push("'Keep volumes' was selected but there is nothing to preserve; this is equivalent to wiping.".into());
    }
    if req.mode == StorageMode::KeepVolumes && !plan.preserved.is_empty() {
        plan.warnings.push("Preserved volumes are not mounted automatically. After the install they appear under 'Import volumes' in the management plane and mount read-only by default.".into());
    }

    if let Err(mut e) = plan.validate(facts) {
        plan.errors.append(&mut e);
    }
    plan
}

impl Plan {
    /// Re-check the safety invariants against the facts the plan was built from.
    pub fn validate(&self, facts: &DeviceFacts) -> Result<(), Vec<String>> {
        let mut errs = vec![];
        let mut names = BTreeSet::new();
        for a in &self.arrays {
            if !names.insert(a.name.clone()) {
                errs.push(format!("duplicate array name {}", a.name));
            }
        }
        for dp in &self.disks {
            let Some(d) = facts.disk(&dp.disk) else {
                errs.push(format!("disk {} not in facts", dp.disk));
                continue;
            };
            let end = align_down(d.sectors.saturating_sub(GPT_TAIL), SECTORS_PER_MIB);
            let mut seen: BTreeSet<u32> = BTreeSet::new();
            for c in &dp.create {
                if c.start % SECTORS_PER_MIB != 0 || c.sectors % SECTORS_PER_MIB != 0 {
                    errs.push(format!("{}: partition {} not MiB aligned", d.name, c.number));
                }
                if c.start < FIRST_USABLE || c.end() > end {
                    errs.push(format!("{}: partition {} outside the disk", d.name, c.number));
                }
                if !seen.insert(c.number) {
                    errs.push(format!("{}: partition number {} used twice", d.name, c.number));
                }
                for o in &dp.create {
                    if o.number != c.number && c.start < o.end() && o.start < c.end() {
                        errs.push(format!("{}: new partitions {} and {} overlap", d.name, c.number, o.number));
                    }
                }
                if !dp.new_table {
                    for p in &d.parts {
                        if dp.delete.contains(&p.number) {
                            continue;
                        }
                        if c.start < p.end() && p.start < c.end() {
                            errs.push(format!("{}: new partition {} overlaps preserved {}", d.name, c.number, p.name));
                        }
                        if p.number == c.number {
                            errs.push(format!("{}: new partition number {} collides with preserved {}", d.name, c.number, p.name));
                        }
                    }
                }
            }
            for n in &dp.delete {
                if let Some(p) = d.part(*n) {
                    if let Some(a) = array_of(facts, &p.name) {
                        if self.kept_arrays.contains(&a.name) {
                            errs.push(format!("{}: refusing to delete {} which belongs to preserved array {}", d.name, p.name, a.name));
                        }
                    } else if self.mode == StorageMode::KeepVolumes {
                        errs.push(format!("{}: refusing to delete unknown partition {} in keep mode", d.name, p.name));
                    }
                } else {
                    errs.push(format!("{}: partition {} to delete does not exist", d.name, n));
                }
            }
            if dp.new_table && self.mode == StorageMode::KeepVolumes {
                // allowed only if nothing is preserved on the disk
                for p in &d.parts {
                    if let Some(a) = array_of(facts, &p.name)
                        && self.kept_arrays.contains(&a.name)
                    {
                        errs.push(format!("{}: new partition table would destroy preserved array {}", d.name, a.name));
                    }
                }
            }
        }
        for v in &self.vgs {
            if let BlockRef::Md(n) = &v.pv
                && !self.arrays.iter().any(|a| &a.name == n)
            {
                errs.push(format!("VG {} references missing array {}", v.name, n));
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// Human readable one-liners for the UI.
    pub fn summary(&self) -> Vec<String> {
        let mut v = vec![];
        v.push(format!(
            "System: {} GiB LVM on RAID1 across {} disk(s) [{}] -> swap {} MiB, root {} GiB (xfs), {} MiB reserve",
            self.sizes.chosen_system_mib / 1024,
            self.system_disks.len(),
            self.system_disks.join(", "),
            self.sizes.swap_mib,
            self.sizes.root_mib / 1024,
            self.sizes.vg_sys_free_mib
        ));
        match self.firmware {
            Firmware::Bios => v.push(format!(
                "Boot: BIOS GRUB, ext4 /boot {} (GRUB installed on every system disk)",
                if matches!(self.boot, Some(BlockRef::Md(_))) { "on md RAID1" } else { "on a plain partition" }
            )),
            Firmware::Uefi => v.push(format!(
                "Boot: UEFI GRUB, {}ext4 /boot {}",
                if matches!(self.esp, Some(BlockRef::Md(_))) { "mirrored ESP + " } else { "ESP + " },
                if matches!(self.boot, Some(BlockRef::Md(_))) { "on md RAID1" } else { "on a plain partition" }
            )),
            Firmware::Dom => v.push(
                "Boot: Debian gets its own entry in the unit's internal boot flash (DOM); DSM's entry stays and is the fail-safe default for every failed or cancelled step".into(),
            ),
            Firmware::UBoot => v.push(format!(
                "Boot: U-Boot raw sectors (kernel @ {} MiB, initrd @ {} MiB, first {} MiB reserved) on every system disk - needs a serial console to point U-Boot at the disk",
                UBOOT_KERNEL_MIB, UBOOT_INITRD_MIB, UBOOT_RESERVED_MIB
            )),
        }
        for vg in self.vgs.iter().filter(|v| v.name == "vg_data") {
            if let Some(a) = self.arrays.iter().find(|a| a.name == "data") {
                v.push(format!(
                    "Data: RAID{} over {} disks, {} usable, LVM vg_data (xfs), {}% left free",
                    a.level,
                    a.members.len(),
                    human_bytes(self.sizes.data_usable_bytes),
                    100 - self.sizes.data_lv_percent
                ));
            } else {
                v.push(format!("Data: single disk, {} usable, LVM {} (xfs)", human_bytes(self.sizes.data_usable_bytes), vg.name));
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{fixtures, parse_inspect};

    fn facts() -> DeviceFacts {
        parse_inspect(&fixtures::ds_4bay()).unwrap()
    }

    #[test]
    fn wipe_all_four_disks_layout() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.is_ok(), "{:?}", p.errors);
        // 4 disks => redundant system on 3 copies, BIOS firmware => bios_grub + boot + system
        assert_eq!(p.system_disks.len(), 3);
        assert_eq!(p.firmware, Firmware::Bios);
        let d0 = &p.disks[0];
        assert!(d0.new_table);
        let roles: Vec<PartRole> = d0.create.iter().map(|c| c.role).collect();
        assert_eq!(roles, vec![PartRole::BiosGrub, PartRole::Boot, PartRole::SystemPv, PartRole::Data]);
        // the 4th disk only has data
        let d3 = p.disks.iter().find(|d| d.disk == "sdd").unwrap();
        assert_eq!(d3.create.len(), 1);
        assert_eq!(d3.create[0].role, PartRole::Data);
        // data RAID5 over 4 disks
        let data = p.arrays.iter().find(|a| a.name == "data").unwrap();
        assert_eq!(data.level, "5");
        assert_eq!(data.members.len(), 4);
        assert_eq!(p.sizes.chosen_system_mib, 32 * 1024);
        assert!(p.destroyed.iter().any(|d| d.contains("md2")));
        assert!(p.preserved.is_empty());
        p.validate(&f).unwrap();
    }

    #[test]
    fn more_than_two_disks_requires_redundant_system() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest { system_copies: Some(1), ..Default::default() });
        assert!(!p.is_ok());
        assert!(p.errors[0].contains("redundant"));
    }

    #[test]
    fn two_disks_single_copy_is_allowed_with_warning() {
        let mut f = facts();
        f.disks.retain(|d| d.name == "sda" || d.name == "sdb" || d.name == "synoboot");
        f.arrays.clear();
        let p = build_plan(&f, &PlanRequest { system_copies: Some(1), ..Default::default() });
        assert!(p.is_ok(), "{:?}", p.errors);
        assert!(p.warnings.iter().any(|w| w.contains("single disk")));
        // default is a mirror
        let p = build_plan(&f, &PlanRequest::default());
        assert_eq!(p.system_disks.len(), 2);
        assert_eq!(p.arrays.iter().find(|a| a.name == "data").unwrap().level, "1");
    }

    #[test]
    fn keep_volumes_preserves_data_partitions_and_reuses_dsm_system_area() {
        let f = facts();
        // DSM p1 (8 GiB) + p2 (2 GiB) + 3 MiB gap in front: ~10 GiB per disk
        let req = PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        for dp in &p.disks {
            assert!(!dp.new_table, "must not recreate the table on disk with preserved data");
            assert_eq!(dp.delete, vec![1, 2]);
            for c in &dp.create {
                assert!(c.end() <= 20979712, "new partitions must stay inside the reclaimed area, got {c:?}");
                assert!(dp.create.iter().all(|o| o.role == c.role || c.end() <= o.start || o.end() <= c.start));
            }
        }
        assert!(p.preserved.iter().any(|x| x.detail.starts_with("md2")));
        assert!(!p.release_arrays.contains(&"md2".to_string()));
        assert!(p.release_arrays.contains(&"md0".to_string()));
        assert!(p.release_arrays.contains(&"md1".to_string()));
        // no new data array: all space is preserved
        assert!(p.arrays.iter().all(|a| a.name != "data"));
        // system PV limited by the reclaimed area, not the recommended 32 GiB
        assert!(p.sizes.chosen_system_mib < 10 * 1024);
        assert!(p.sizes.chosen_system_mib >= MIN_SYSTEM_MIB);
        p.validate(&f).unwrap();
    }

    #[test]
    fn keep_volumes_rejects_oversized_system() {
        let f = facts();
        let req = PlanRequest { mode: StorageMode::KeepVolumes, system_size_mib: 32 * 1024, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(!p.is_ok());
        assert!(p.errors.iter().any(|e| e.contains("exceeds")));
    }

    #[test]
    fn keep_volumes_with_small_dsm6_system_area_explains_why() {
        let mut f = facts();
        for d in f.disks.iter_mut().filter(|d| d.name.starts_with("sd") && d.name != "synoboot") {
            // DSM 6: p1 2.4 GiB, p2 2 GiB
            d.parts[0].sectors = 4_980_736;
            d.parts[1].start = 4_988_928;
            d.parts[1].sectors = 4_194_304;
            // data volume starts right behind swap (nothing free in between)
            d.parts[2].start = 9_183_232;
        }
        let req = PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(!p.is_ok());
        assert!(p.errors[0].contains("reclaimable"));
    }

    #[test]
    fn releasing_the_data_array_makes_keep_equal_to_wipe_for_that_array() {
        let f = facts();
        let req = PlanRequest { mode: StorageMode::KeepVolumes, keep_arrays: Some(vec![]), ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert!(p.destroyed.iter().any(|d| d.contains("md2")));
        assert!(p.preserved.is_empty());
        assert!(p.disks.iter().all(|d| d.new_table));
    }

    #[test]
    fn explicit_raid_levels_are_validated() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest { data_raid: DataRaid::Raid6, ..Default::default() });
        assert!(p.is_ok());
        let min_data =
            p.disks.iter().flat_map(|d| d.create.iter()).filter(|c| c.role == PartRole::Data).map(|c| c.sectors * SECTOR).min().unwrap();
        assert_eq!(p.sizes.data_usable_bytes, 2 * min_data, "RAID6 over 4 members keeps two members' worth");
        let mut f2 = f.clone();
        f2.disks.retain(|d| d.name == "sda" || d.name == "sdb" || d.name == "synoboot");
        f2.arrays.clear();
        let p = build_plan(&f2, &PlanRequest { data_raid: DataRaid::Raid5, ..Default::default() });
        assert!(!p.is_ok());
        assert!(p.errors[0].contains("RAID 5"));
    }

    #[test]
    fn uefi_adds_mirrored_esp() {
        let mut f = facts();
        f.firmware = Firmware::Uefi;
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.is_ok(), "{:?}", p.errors);
        let esp = p.arrays.iter().find(|a| a.name == "esp").unwrap();
        assert_eq!(esp.metadata, "1.0");
        assert_eq!(esp.members.len(), 3);
        assert_eq!(p.disks[0].create[0].role, PartRole::Esp);
        assert_eq!(p.disks[0].create[0].sectors, 512 * SECTORS_PER_MIB);
    }

    #[test]
    fn single_disk_has_no_md_for_system() {
        let mut f = facts();
        f.disks.retain(|d| d.name == "sda" || d.name == "synoboot");
        f.arrays.clear();
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.is_ok(), "{:?}", p.errors);
        assert!(p.arrays.is_empty());
        assert!(matches!(p.vgs[0].pv, BlockRef::Part(_, 3)));
        assert!(matches!(p.boot, Some(BlockRef::Part(_, 2))));
    }

    #[test]
    fn unsupported_arch_and_low_ram() {
        let mut f = facts();
        f.arch = Arch::Unsupported;
        assert!(!build_plan(&f, &PlanRequest::default()).is_ok());
        let mut f = facts();
        f.mem_kib = 256 * 1024;
        assert!(!build_plan(&f, &PlanRequest::default()).is_ok());
    }

    #[test]
    fn mbr_disk_with_preserved_partitions_is_locked() {
        let mut f = facts();
        f.disks[0].table = PartTable::Msdos;
        let req = PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(p.untouched.contains(&"sda".to_string()));
        assert!(p.warnings.iter().any(|w| w.contains("MBR")));
        assert!(p.disks.iter().all(|d| d.disk != "sda"));
    }

    #[test]
    fn partitions_numbers_reuse_lowest_free_slots() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        let d = &p.disks[0];
        let nums: Vec<u32> = d.create.iter().map(|c| c.number).collect();
        assert_eq!(nums, vec![1, 2, 3]);
    }

    fn armv7_facts() -> DeviceFacts {
        let mut f = facts();
        f.uname_m = "armv7l".into();
        f.unique = "synology_armada38x_ds116".into();
        f.arch = Arch::Armhf;
        f.firmware = Firmware::UBoot;
        f.mem_kib = 512 * 1024;
        f.disks.retain(|d| d.name == "sda" || d.name == "sdb");
        for a in f.arrays.iter_mut() {
            a.members.retain(|m| m.starts_with("sda") || m.starts_with("sdb"));
            a.level = "1".into();
        }
        f
    }

    #[test]
    fn armv7_uboot_reserves_raw_area_and_has_no_boot_partition() {
        let f = armv7_facts();
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.is_ok(), "{:?}", p.errors);
        assert_eq!(p.arch, Arch::Armhf);
        assert!(p.boot.is_none() && p.esp.is_none());
        assert_eq!(p.raw_boot, Some(RawBoot { kernel_mib: 10, initrd_mib: 20, reserved_mib: 50 }));
        for dp in &p.disks {
            assert!(dp.create.iter().all(|c| c.start >= UBOOT_RESERVED_MIB * SECTORS_PER_MIB), "{dp:?}");
            assert!(dp.create.iter().all(|c| !matches!(c.role, PartRole::Boot | PartRole::Esp | PartRole::BiosGrub)));
        }
        assert!(p.warnings.iter().any(|w| w.contains("serial")));
        assert!(p.summary().iter().any(|l| l.contains("raw sectors")));
        assert_eq!(p.system_disks.len(), 2);
        p.validate(&f).unwrap();
    }

    #[test]
    fn armv7_keep_volumes_fits_in_dsm_system_area() {
        let f = armv7_facts();
        let p = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(p.is_ok(), "{:?}", p.errors);
        for dp in &p.disks {
            assert!(dp.create.iter().all(|c| c.start >= 102_400 && c.end() <= 20_979_712));
        }
        assert!(p.preserved.iter().any(|x| x.detail.starts_with("md2")));
    }

    #[test]
    fn armv7_raw_area_blocked_by_preserved_partition() {
        let mut f = armv7_facts();
        // a preserved partition sitting inside the first 50 MiB
        for d in f.disks.iter_mut() {
            d.parts[0].start = 34;
            d.parts[0].sectors = 2048;
        }
        f.arrays.retain(|a| a.role == ArrayRole::Data);
        let p = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        // partition 1 is now an unknown partition (array removed) and is preserved: raw area not free
        assert!(!p.is_ok(), "{:?}", p.warnings);
    }

    #[test]
    fn dsm_volumes_are_named_in_the_destroy_list() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.destroyed.iter().any(|d| d.contains("/volume1") && d.contains("btrfs")), "{:?}", p.destroyed);
        let k = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(k.preserved.iter().any(|d| d.what == "DSM volumes" && d.detail.contains("/volume1")));
        assert!(!k.destroyed.iter().any(|d| d.contains("/volume1")));
    }

    #[test]
    fn amd_dom_unit_plan_and_summary() {
        let mut raw = fixtures::ds_4bay();
        raw = raw
            .replace("KV|uefi|0\n", "KV|uefi|1\nKV|cpu_vendor|AuthenticAMD\n")
            .replace("NIC|eth0", "DOM|262144|65536|196608|1|1\nNIC|eth0");
        let f = parse_inspect(&raw).unwrap();
        assert_eq!((f.firmware, f.cpu_vendor), (Firmware::Dom, CpuVendor::Amd));
        let p = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(p.is_ok(), "{:?}", p.errors);
        // no boot partitions at all: the whole reclaimed area is available for the system PV
        assert!(p.sizes.max_system_mib > 9 * 1024, "{}", p.sizes.max_system_mib);
        assert!(p.summary().iter().any(|l| l.contains("boot flash")));
        p.validate(&f).unwrap();
    }

    #[test]
    fn swap_and_split_are_sane() {
        let (swap, root, free) = split_system(32 * 1024, 8 * 1024 * 1024);
        assert_eq!(swap, 4096);
        assert!(root > 20 * 1024);
        assert!(swap + root + free <= 32 * 1024);
        assert_eq!(swap_mib_for(256 * 1024), 1024);
    }
}
