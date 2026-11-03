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
    /// Synology-Hybrid-RAID-like: mdadm slices + one LVM volume group, tolerates 1 failed disk.
    Shr1,
    /// Like [`DataRaid::Shr1`] but RAID 6 slices (2 failed disks, needs at least 4 disks).
    Shr2,
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
            DataRaid::Shr1 => "shr1",
            DataRaid::Shr2 => "shr2",
        }
    }
    /// Disks of protection requested by a hybrid (SHR-like) choice; `None` for the plain levels.
    pub fn shr_redundancy(self) -> Option<u8> {
        match self {
            DataRaid::Shr1 => Some(1),
            DataRaid::Shr2 => Some(2),
            _ => None,
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
            DataRaid::Shr1 => "Hybrid RAID, 1-disk protection (SHR-like)",
            DataRaid::Shr2 => "Hybrid RAID, 2-disk protection (SHR-like)",
        }
    }
    pub fn min_members(self) -> usize {
        match self {
            DataRaid::Auto | DataRaid::None => 1,
            DataRaid::Raid0 | DataRaid::Raid1 | DataRaid::Shr1 => 2,
            DataRaid::Raid5 => 3,
            DataRaid::Raid6 | DataRaid::Raid10 | DataRaid::Shr2 => 4,
        }
    }
    pub const ALL: [DataRaid; 9] = [
        DataRaid::Auto,
        DataRaid::None,
        DataRaid::Raid0,
        DataRaid::Raid1,
        DataRaid::Raid5,
        DataRaid::Raid6,
        DataRaid::Raid10,
        DataRaid::Shr1,
        DataRaid::Shr2,
    ];
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
    /// Physical volumes of the group, in creation order. The data group of an SHR-like layout has one
    /// PV per slice array; every other group has exactly one. Older serialised plans carry a single
    /// `pv` and still load.
    #[serde(alias = "pv", deserialize_with = "one_or_many")]
    pub pvs: Vec<BlockRef>,
    pub lvs: Vec<LvPlan>,
}

impl VgPlan {
    /// The first (for single-PV groups: the only) physical volume.
    pub fn first_pv(&self) -> Option<&BlockRef> {
        self.pvs.first()
    }
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<BlockRef>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<BlockRef>),
        One(BlockRef),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::Many(v) => v,
        OneOrMany::One(b) => vec![b],
    })
}

// ---------------------------------------------------------------------------------------------- SHR slices

/// One slice of a hybrid layout: the same-sized stretch of the data space on every disk that reaches it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShrSlice {
    /// 0-based tier number (slice 1 in the docs is index 0).
    pub index: usize,
    /// Offset of the slice inside every member's data space.
    pub start_bytes: u64,
    pub len_bytes: u64,
    /// Indices into the `sizes` slice given to [`shr_slices`].
    pub members: Vec<usize>,
    /// md level: "1", "5" or "6"; empty for an unusable slice.
    pub level: String,
    pub usable_bytes: u64,
    /// Exists on a single disk only: cannot be protected, never used.
    pub unusable: bool,
    /// How many member disks may fail without losing this slice.
    pub fault_tolerance: u8,
    /// md array name (`data1`, `data2`, ... in slice order); `None` for an unusable slice.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShrLayout {
    /// 1 (SHR-1) or 2 (SHR-2).
    pub redundancy: u8,
    pub slices: Vec<ShrSlice>,
    pub usable_bytes: u64,
    /// Capacity left unused because it exists on one disk only.
    pub unusable_bytes: u64,
    pub warnings: Vec<String>,
}

impl ShrLayout {
    /// The slices that become md arrays.
    pub fn usable(&self) -> impl Iterator<Item = &ShrSlice> {
        self.slices.iter().filter(|s| !s.unusable)
    }
}

/// Synology-Hybrid-RAID-like slicing (see `docs/RAID.md`). Pure: `sizes` is the data space of every
/// data disk in bytes, `redundancy` 1 (SHR-1) or 2 (SHR-2; anything larger is treated as 2).
///
/// Sizes are rounded down to whole GiB, which also merges tiers closer than 1 GiB into the lower one.
/// Disks that round to 0 take no part. Too-few-disks is the caller's decision: this function
/// just reports what the sizes allow (e.g. a single disk yields one unusable slice).
pub fn shr_slices(sizes: &[u64], redundancy: u8) -> ShrLayout {
    let red: u8 = if redundancy >= 2 { 2 } else { 1 };
    let gib: Vec<u64> = sizes.iter().map(|s| s / GIB).collect();
    let tiers: BTreeSet<u64> = gib.iter().copied().filter(|g| *g > 0).collect();
    let mut layout = ShrLayout { redundancy: red, slices: vec![], usable_bytes: 0, unusable_bytes: 0, warnings: vec![] };
    let mut prev = 0u64;
    let mut next_name = 1usize;
    for (index, t) in tiers.into_iter().enumerate() {
        let members: Vec<usize> = gib.iter().enumerate().filter(|(_, g)| **g >= t).map(|(i, _)| i).collect();
        let m = members.len() as u64;
        let len = (t - prev) * GIB;
        let (level, usable, tol, unusable) = match (red, members.len()) {
            (_, 0 | 1) => ("", 0, 0u8, true),
            (1, 2) => ("1", len, 1, false),
            (1, _) => ("5", (m - 1) * len, 1, false),
            (_, 2) => ("1", len, 1, false),
            (_, 3) => ("1", len, 2, false),
            (_, _) => ("6", (m - 2) * len, 2, false),
        };
        let name = (!unusable).then(|| {
            let n = format!("data{next_name}");
            next_name += 1;
            n
        });
        let label = format!("Slice {} ({} on {} disk(s))", index + 1, human_bytes(len), members.len());
        if unusable {
            layout.unusable_bytes += len;
            layout.warnings.push(format!(
                "{label} exists on one disk only: it cannot be protected, so {} of that disk stay unused (add a second disk of that size to use it).",
                human_bytes(len)
            ));
        } else if red == 2 && members.len() < 4 {
            layout.warnings.push(if members.len() == 2 {
                format!("{label} is mirrored (RAID 1) because RAID 6 needs 4 members: it survives 1 failed disk, not the 2 you asked for.")
            } else {
                format!("{label} is a 3-way RAID 1 mirror because RAID 6 needs 4 members: it survives 2 failed disks but only one disk's worth of it is usable.")
            });
        }
        layout.usable_bytes += usable;
        layout.slices.push(ShrSlice {
            index,
            start_bytes: prev * GIB,
            len_bytes: len,
            members,
            level: level.into(),
            usable_bytes: usable,
            unusable,
            fault_tolerance: tol,
            name,
        });
        prev = t;
    }
    layout
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

/// The hybrid layout plus the names of the disks its `members` indices refer to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShrPlan {
    pub disks: Vec<String>,
    pub layout: ShrLayout,
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
    /// Slice layout of the data volume when a hybrid (SHR-like) level was chosen.
    #[serde(default)]
    pub shr: Option<ShrPlan>,
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
    /// DSM data arrays that have members on an erased disk and on a disk the install leaves alone (lost as a whole).
    #[serde(default)]
    pub straddling: Vec<Straddle>,
}

impl Plan {
    /// Does the install create the data volume that is mounted at `/srv/data`? (Not in Keep mode when nothing new is built.)
    pub fn has_data_volume(&self) -> bool {
        self.vgs.iter().any(|v| v.name == "vg_data" && !v.lvs.is_empty())
    }

    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// A disk that takes part in an SHR-like data layout, before its partitions are cut.
struct ShrDisk {
    /// Index into `Plan::disks`.
    dp_index: usize,
    disk: String,
    /// First free sector after the system partitions.
    start: u64,
    /// Free sectors from `start` to the end of the usable area.
    sectors: u64,
    /// Partition numbers already taken on the disk.
    used: BTreeSet<u32>,
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

/// A DSM data array whose members sit on a disk the install erases and on a disk it does not touch: it cannot survive, and
/// the untouched disk keeps only a useless piece of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Straddle {
    pub array: String,
    /// Member disks that are erased.
    pub erased: Vec<String>,
    /// Member disks that are left alone.
    pub left: Vec<String>,
}

/// One warning sentence per group of arrays that span the same disks: `md2 and md3 span sda, sdc, sdd (erased) and sdb (not touched):
/// they are lost as a whole, including their part on that disk.`
pub fn straddle_warnings(all: &[Straddle]) -> Vec<String> {
    let mut groups: Vec<(Vec<&str>, &Straddle)> = vec![];
    for s in all {
        match groups.iter_mut().find(|(_, g)| g.erased == s.erased && g.left == s.left) {
            Some((names, _)) => names.push(s.array.as_str()),
            None => groups.push((vec![s.array.as_str()], s)),
        }
    }
    groups
        .into_iter()
        .map(|(names, s)| {
            let many = names.len() > 1;
            format!(
                "{} {} {} (erased) and {} (not touched): {} lost as a whole, including {} part on {}.",
                names.join(" and "),
                if many { "span" } else { "spans" },
                s.erased.join(", "),
                s.left.join(", "),
                if many { "they are" } else { "it is" },
                if many { "their" } else { "its" },
                if s.left.len() == 1 { "that disk" } else { "those disks" }
            )
        })
        .collect()
}

/// The disks that hold members of `a` (sorted, without duplicates).
fn array_disks(facts: &DeviceFacts, a: &MdArray) -> Vec<String> {
    let mut v: Vec<String> = a.members.iter().filter_map(|m| facts.locate_part(m).map(|(d, _)| d.name.clone())).collect();
    v.sort();
    v.dedup();
    v
}

/// DSM data arrays (not the preserved ones in `kept`) that have members on both an install target and a disk that is left alone
/// (USB, removable with an unknown port, too small). The boot flash never holds data members.
pub fn straddling_arrays(facts: &DeviceFacts, kept: &BTreeSet<String>) -> Vec<Straddle> {
    let mut out = vec![];
    for a in facts.arrays.iter().filter(|a| a.role == ArrayRole::Data && !kept.contains(&a.name)) {
        let (mut erased, mut left) = (vec![], vec![]);
        for d in array_disks(facts, a) {
            match facts.disk(&d).map(|x| x.exclusion()) {
                Some(None) => erased.push(d),
                Some(Some(crate::model::Exclusion::BootFlash)) | None => {}
                Some(Some(_)) => left.push(d),
            }
        }
        if !erased.is_empty() && !left.is_empty() {
            out.push(Straddle { array: a.name.clone(), erased, left });
        }
    }
    out
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
        shr: None,
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
        straddling: vec![],
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
    // Arrays that are still resyncing or missing members: an install now would start from a pool that is not redundant.
    for a in facts.busy_arrays() {
        if let Some(note) = a.health_note() {
            plan.warnings.push(format!("DSM array {} is {note}. {}", a.name, a.advice()));
        }
    }
    // Disks the plan will not touch, other than the boot flash: never leave them out silently.
    let left: Vec<(String, String)> = facts
        .excluded_disks()
        .into_iter()
        .filter(|(_, e)| !matches!(e, crate::model::Exclusion::BootFlash))
        .map(|(d, e)| (d.name.clone(), format!("{} ({})", d.name, e.reason_inline())))
        .collect();
    // A DSM data array with members on an erased disk and on a disk that is left alone is destroyed as a whole: say so instead
    // of promising that the untouched disk keeps its DSM data.
    let kept_for_straddle: BTreeSet<String> = match (req.mode, &req.keep_arrays) {
        (StorageMode::WipeAll, _) => BTreeSet::new(),
        (StorageMode::KeepVolumes, Some(k)) => k.iter().cloned().collect(),
        (StorageMode::KeepVolumes, None) => facts.data_arrays().iter().map(|a| a.name.clone()).collect(),
    };
    plan.straddling = straddling_arrays(facts, &kept_for_straddle);
    if !left.is_empty() {
        let mut msg = format!(
            "Not used and not touched: {}. synofurb does not write to them.",
            left.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join(", ")
        );
        // "DSM data stays as it is" is only said for a disk that holds DSM data and none of it belongs to an array that is erased
        let safe: Vec<&str> = left
            .iter()
            .map(|l| l.0.as_str())
            .filter(|n| {
                let holds = facts.arrays.iter().any(|a| a.role == ArrayRole::Data && array_disks(facts, a).iter().any(|d| d == n));
                holds && !plan.straddling.iter().any(|s| s.left.iter().any(|d| d == n))
            })
            .collect();
        if !safe.is_empty() {
            msg.push_str(&format!(" The DSM data on {} stays as it is.", safe.join(", ")));
        }
        plan.warnings.push(msg);
    }
    plan.warnings.extend(straddle_warnings(&plan.straddling));
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

    let shr_red = req.data_raid.shr_redundancy();
    if shr_red.is_some() && !kept.is_empty() {
        plan.errors.push(format!(
            "{} builds new data arrays on whole, wiped disks and cannot be combined with preserved arrays ({}). \
             Release the preserved arrays, choose 'wipe everything', or pick another data level.",
            req.data_raid.label(),
            kept.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
        return plan;
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
    // SHR: disks with room for data. Their partitions are cut once every disk's size is known.
    let mut shr_disks: Vec<ShrDisk> = vec![];

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
        if a.fully_wiped && is_sys_or_free(&ext) && ext.end > cursor + SECTORS_PER_MIB && shr_red.is_some() {
            shr_disks.push(ShrDisk {
                dp_index: plan.disks.len(),
                disk: d.name.clone(),
                start: cursor,
                sectors: ext.end - cursor,
                used: used.clone(),
            });
        } else if a.fully_wiped && is_sys_or_free(&ext) && ext.end > cursor + SECTORS_PER_MIB {
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
        pvs: vec![sys_pv],
        lvs: vec![
            LvPlan { name: "swap".into(), mib: Some(swap), vg_percent: None, fs: "swap".into(), mountpoint: "none".into() },
            LvPlan { name: "root".into(), mib: Some(root), vg_percent: None, fs: "xfs".into(), mountpoint: "/".into() },
        ],
    });

    // -- data
    if let Some(red) = shr_red {
        let skipped: Vec<String> = analyses.iter().filter(|a| !a.fully_wiped || a.locked).map(|a| a.disk.name.clone()).collect();
        plan_shr_data(&mut plan, facts, req, red, shr_disks, &skipped);
    } else if !data_members.is_empty() {
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
            pvs: vec![pv],
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
    // `volumes()` skips sub-mounts (`/volume2/@docker`) and FUSE mounts, which are not volumes and would be listed twice.
    let volumes: Vec<String> = facts
        .volumes()
        .iter()
        .map(|v| format!("{} ({}, {} used of {})", v.mountpoint, v.fstype, human_bytes(v.used_kib * 1024), human_bytes(v.size_kib * 1024)))
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

/// Cut the data partitions of an SHR-like layout, one md array per usable slice, all PVs of `vg_data`.
fn plan_shr_data(plan: &mut Plan, facts: &DeviceFacts, req: &PlanRequest, red: u8, mut disks: Vec<ShrDisk>, skipped: &[String]) {
    let n = disks.len();
    let min = req.data_raid.min_members();
    if n < min {
        let mut msg = format!("{} needs at least {} wiped data disks but {} are available.", req.data_raid.label(), min, n);
        if !skipped.is_empty() {
            msg.push_str(&format!(
                " Disks that keep partitions or arrays cannot take part ({} left out); use 'wipe everything' or release them.",
                skipped.join(", ")
            ));
        }
        plan.errors.push(msg);
        return;
    }
    if !skipped.is_empty() {
        plan.warnings.push(format!("Disks {} keep their partitions and take no part in the hybrid RAID group.", skipped.join(", ")));
    }
    let sizes: Vec<u64> = disks.iter().map(|d| d.sectors * SECTOR).collect();
    let layout = shr_slices(&sizes, red);
    if layout.usable_bytes == 0 {
        plan.errors.push(format!(
            "{}: the disks leave no slice that exists on at least two disks, so nothing can be protected.",
            req.data_raid.label()
        ));
        return;
    }
    let names: Vec<String> = disks.iter().map(|d| d.disk.clone()).collect();
    let mut cursors: Vec<u64> = disks.iter().map(|d| d.start).collect();
    let mut raw = 0u64;
    let mut pvs = vec![];
    for slice in layout.usable() {
        let name = slice.name.clone().unwrap_or_default();
        let mut members = vec![];
        for &i in &slice.members {
            let d = &mut disks[i];
            let num = (1..=128u32).find(|x| !d.used.contains(x)).unwrap_or(129);
            d.used.insert(num);
            let sectors = slice.len_bytes / SECTOR;
            plan.disks[d.dp_index].create.push(NewPart { number: num, start: cursors[i], sectors, role: PartRole::Data });
            cursors[i] += sectors;
            raw += slice.len_bytes;
            members.push((d.disk.clone(), num));
        }
        plan.arrays.push(ArrayPlan { name: name.clone(), level: slice.level.clone(), metadata: "1.2".into(), members });
        pvs.push(BlockRef::Md(name));
    }
    plan.warnings.extend(layout.warnings.iter().cloned());
    if layout.slices.iter().any(|s| s.unusable && s.members.iter().any(|&i| !plan.system_disks.contains(&names[i]))) {
        plan.warnings.push(
            "The disks that carry no copy of the system have more data space than the others. Raising the number of system copies to the number of disks evens this out.".into(),
        );
    }
    if facts.arch == Arch::Armhf && layout.usable_bytes > 16 * 1024 * GIB {
        plan.warnings.push("32-bit ARM kernels can address at most 16 TiB per filesystem; the part of the data volume above 16 TiB cannot be used in a single xfs filesystem. Consider a smaller data LV.".into());
    }
    plan.sizes.data_raw_bytes = raw;
    plan.sizes.data_usable_bytes = layout.usable_bytes;
    let pct = 100 - req.data_free_percent;
    plan.sizes.data_lv_percent = pct;
    plan.vgs.push(VgPlan {
        name: "vg_data".into(),
        pvs,
        lvs: if pct == 0 {
            vec![]
        } else {
            vec![LvPlan { name: "data".into(), mib: None, vg_percent: Some(pct), fs: "xfs".into(), mountpoint: "/srv/data".into() }]
        },
    });
    plan.shr = Some(ShrPlan { disks: names, layout });
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
            if v.pvs.is_empty() {
                errs.push(format!("VG {} has no physical volume", v.name));
            }
            for pv in &v.pvs {
                if let BlockRef::Md(n) = pv
                    && !self.arrays.iter().any(|a| &a.name == n)
                {
                    errs.push(format!("VG {} references missing array {}", v.name, n));
                }
            }
        }
        if let Some(shr) = &self.shr {
            // every slice array must have exactly its slice's member count, one partition per member disk
            for sl in shr.layout.usable() {
                let name = sl.name.clone().unwrap_or_default();
                match self.arrays.iter().find(|a| a.name == name) {
                    None => errs.push(format!("SHR slice {} has no array", sl.index + 1)),
                    Some(a) if a.members.len() != sl.members.len() => {
                        errs.push(format!("SHR array {name} has {} members, slice needs {}", a.members.len(), sl.members.len()))
                    }
                    Some(_) => {}
                }
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
            if let Some(shr) = &self.shr {
                v.push(format!(
                    "Data: hybrid RAID (SHR-like, {}-disk protection) over {} disks, {} usable, {} md array(s) joined in LVM vg_data (xfs), {}% left free",
                    shr.layout.redundancy,
                    shr.disks.len(),
                    human_bytes(self.sizes.data_usable_bytes),
                    vg.pvs.len(),
                    100 - self.sizes.data_lv_percent
                ));
                for sl in shr.layout.slices.iter() {
                    let members: Vec<&str> = sl.members.iter().map(|&i| shr.disks[i].as_str()).collect();
                    v.push(match &sl.name {
                        Some(n) => format!(
                            "  {n}: RAID{} {} x {} on {} -> {} usable",
                            sl.level,
                            members.len(),
                            human_bytes(sl.len_bytes),
                            members.join(","),
                            human_bytes(sl.usable_bytes)
                        ),
                        None => format!(
                            "  slice {}: {} on {} left unused (single disk)",
                            sl.index + 1,
                            human_bytes(sl.len_bytes),
                            members.join(",")
                        ),
                    });
                }
            } else if let Some(a) = self.arrays.iter().find(|a| a.name == "data") {
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
        assert!(matches!(p.vgs[0].pvs[..], [BlockRef::Part(_, 3)]));
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

    // ------------------------------------------------------------------------------------ SHR

    const TB: u64 = 1024 * GIB;

    fn lv(layout: &ShrLayout) -> Vec<(u64, usize, &str, u64, bool)> {
        layout.slices.iter().map(|s| (s.len_bytes / GIB, s.members.len(), s.level.as_str(), s.usable_bytes / GIB, s.unusable)).collect()
    }

    #[test]
    fn shr_worked_example_4_4_8() {
        let l = shr_slices(&[4 * TB, 4 * TB, 8 * TB], 1);
        assert_eq!(lv(&l), vec![(4096, 3, "5", 8192, false), (4096, 1, "", 0, true)]);
        assert_eq!(l.usable_bytes, 8 * TB, "what DSM reports");
        assert_eq!(l.unusable_bytes, 4 * TB);
        assert_eq!(l.slices[0].name.as_deref(), Some("data1"));
        assert_eq!(l.slices[1].name, None);
        assert_eq!(l.slices[0].members, vec![0, 1, 2]);
        assert_eq!(l.slices[1].members, vec![2]);
        assert!(l.warnings.iter().any(|w| w.contains("one disk only")));
    }

    #[test]
    fn shr_worked_example_2_4_4_8() {
        let l = shr_slices(&[2 * TB, 4 * TB, 4 * TB, 8 * TB], 1);
        assert_eq!(lv(&l), vec![(2048, 4, "5", 6144, false), (2048, 3, "5", 4096, false), (4096, 1, "", 0, true)]);
        assert_eq!(l.usable_bytes, 10 * TB);
        assert_eq!(l.slices[1].start_bytes, 2 * TB);
        assert_eq!(l.slices[1].name.as_deref(), Some("data2"));
    }

    #[test]
    fn shr_worked_example_1_2_2_3() {
        let l = shr_slices(&[3 * TB, TB, 2 * TB, 2 * TB], 1);
        assert_eq!(lv(&l), vec![(1024, 4, "5", 3072, false), (1024, 3, "5", 2048, false), (1024, 1, "", 0, true)]);
        assert_eq!(l.usable_bytes, 5 * TB);
        // member indices follow the order of the input, not the order of the sizes
        assert_eq!(l.slices[1].members, vec![0, 2, 3]);
    }

    #[test]
    fn shr2_with_four_equal_disks_is_one_raid6_slice() {
        let l = shr_slices(&[4 * TB; 4], 2);
        assert_eq!(lv(&l), vec![(4096, 4, "6", 8192, false)]);
        assert_eq!(l.slices[0].fault_tolerance, 2);
        assert!(l.warnings.is_empty());
    }

    #[test]
    fn shr2_falls_back_to_mirrors_for_narrow_slices_and_warns() {
        let l = shr_slices(&[4 * TB, 4 * TB, 8 * TB, 8 * TB], 2);
        assert_eq!(lv(&l), vec![(4096, 4, "6", 8192, false), (4096, 2, "1", 4096, false)]);
        assert_eq!(l.slices[1].fault_tolerance, 1);
        assert!(l.warnings.iter().any(|w| w.contains("Slice 2") && w.contains("not the 2 you asked for")), "{:?}", l.warnings);
        let l = shr_slices(&[4 * TB, 8 * TB, 8 * TB, 8 * TB], 2);
        assert_eq!(lv(&l), vec![(4096, 4, "6", 8192, false), (4096, 3, "1", 4096, false)]);
        assert_eq!(l.slices[1].fault_tolerance, 2);
        assert!(l.warnings.iter().any(|w| w.contains("3-way")), "{:?}", l.warnings);
    }

    #[test]
    fn shr1_with_two_disks_is_raid1_and_equal_disks_are_one_slice() {
        let l = shr_slices(&[4 * TB, 4 * TB], 1);
        assert_eq!(lv(&l), vec![(4096, 2, "1", 4096, false)]);
        let l = shr_slices(&[4 * TB; 5], 1);
        assert_eq!(lv(&l), vec![(4096, 5, "5", 16384, false)]);
        assert!(l.warnings.is_empty());
        // two disks of different size: the mirror is limited by the smaller one
        let l = shr_slices(&[4 * TB, 8 * TB], 1);
        assert_eq!(l.usable_bytes, 4 * TB);
        assert_eq!(l.slices.len(), 2);
    }

    #[test]
    fn shr_rounds_down_to_gib_and_merges_close_tiers() {
        // 10 GiB + 900 MiB and 10 GiB are the same tier; 20 GiB + 1 MiB is the 20 GiB tier
        let l = shr_slices(&[10 * GIB + 900 * MIB, 10 * GIB, 20 * GIB + MIB], 1);
        assert_eq!(lv(&l), vec![(10, 3, "5", 20, false), (10, 1, "", 0, true)]);
        // exactly 1 GiB apart stays a (1 GiB) slice
        let l = shr_slices(&[10 * GIB, 11 * GIB, 11 * GIB], 1);
        assert_eq!(lv(&l), vec![(10, 3, "5", 20, false), (1, 2, "1", 1, false)]);
        // below 1 GiB takes no part; a single disk has nothing usable
        let l = shr_slices(&[GIB / 2, 5 * GIB], 1);
        assert_eq!((l.slices.len(), l.usable_bytes), (1, 0));
        assert_eq!(shr_slices(&[], 1).slices.len(), 0);
    }

    #[test]
    fn shr_sizes_are_in_input_order_and_layout_survives_json() {
        let l = shr_slices(&[8 * TB, 4 * TB, 4 * TB], 1);
        assert_eq!(l.slices[1].members, vec![0]);
        let back: ShrLayout = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
    }

    /// The language-neutral vectors shared with the NAS-side Python implementation
    /// (`apps/mgmt-plane/tests/shr_vectors.json`): both must produce the same slices.
    #[test]
    fn shr_slices_agree_with_the_shared_vectors() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/mgmt-plane/tests/shr_vectors.json");
        let Ok(text) = std::fs::read_to_string(&path) else {
            eprintln!("skipping: {} not present", path.display());
            return;
        };
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        let vectors = doc["vectors"].as_array().expect("vectors");
        assert!(!vectors.is_empty());
        for v in vectors {
            let name = v["name"].as_str().unwrap();
            assert_eq!(v["unit"], "MiB", "{name}");
            let disks: Vec<(&str, u64)> = v["disks"].as_object().unwrap().iter().map(|(k, x)| (k.as_str(), x.as_u64().unwrap())).collect();
            let sizes: Vec<u64> = disks.iter().map(|d| d.1 * MIB).collect();
            let red = if v["mode"] == "shr2" { 2 } else { 1 };
            let l = shr_slices(&sizes, red);
            assert_eq!(l.usable_bytes / MIB, v["usable"].as_u64().unwrap(), "{name}: usable");
            let want = v["slices"].as_array().unwrap();
            assert_eq!(l.slices.len(), want.len(), "{name}: slice count");
            for (got, w) in l.slices.iter().zip(want) {
                let members: Vec<&str> = got.members.iter().map(|&i| disks[i].0).collect();
                let wm: Vec<&str> = w["members"].as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
                assert_eq!(members, wm, "{name}: members");
                assert_eq!(got.len_bytes / MIB, w["length"].as_u64().unwrap(), "{name}: length");
                assert_eq!(got.usable_bytes / MIB, w["usable"].as_u64().unwrap(), "{name}: slice usable");
                match w["level"].as_str() {
                    Some(l) => assert_eq!(format!("raid{}", got.level), l, "{name}: level"),
                    None => assert!(got.unusable, "{name}: unusable"),
                }
            }
        }
    }

    /// `n` plain disks with the given usable DATA sizes (GiB), system mirrored on all of them, so the data
    /// partitions are exactly those sizes: 1 MiB start + bios_grub 1 MiB + boot 1 GiB + 8 GiB system.
    fn shr_facts(data_gib: &[u64]) -> (DeviceFacts, PlanRequest) {
        let mut f = facts();
        let names = ["sda", "sdb", "sdc", "sdd"];
        f.disks.retain(|d| d.name == "synoboot" || names[..data_gib.len()].contains(&d.name.as_str()));
        f.arrays.clear();
        for d in f.disks.iter_mut().filter(|d| d.name != "synoboot") {
            let i = names.iter().position(|n| *n == d.name).unwrap();
            d.sectors = (1 + 1 + 1024 + 8192 + data_gib[i] * 1024) * SECTORS_PER_MIB + 34;
        }
        let req =
            PlanRequest { data_raid: DataRaid::Shr1, system_copies: Some(data_gib.len()), system_size_mib: 8192, ..Default::default() };
        (f, req)
    }

    fn data_parts<'a>(p: &'a Plan, disk: &str) -> Vec<&'a NewPart> {
        p.disks.iter().find(|d| d.disk == disk).unwrap().create.iter().filter(|c| c.role == PartRole::Data).collect()
    }

    #[test]
    fn shr_plan_for_4_4_8_builds_slice_partitions_arrays_and_one_vg() {
        let (f, req) = shr_facts(&[64, 64, 128]);
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        p.validate(&f).unwrap();
        // sda/sdb carry one data partition, sdc (the 8) too: its upper slice is unusable and stays unallocated
        for d in ["sda", "sdb", "sdc"] {
            let parts = data_parts(&p, d);
            assert_eq!(parts.len(), 1, "{d}");
            assert_eq!(parts[0].sectors * SECTOR, 64 * GIB);
        }
        let data: Vec<&ArrayPlan> = p.arrays.iter().filter(|a| a.name.starts_with("data")).collect();
        assert_eq!(data.len(), 1);
        assert_eq!((data[0].name.as_str(), data[0].level.as_str(), data[0].metadata.as_str()), ("data1", "5", "1.2"));
        assert_eq!(data[0].members.len(), 3);
        let vg = p.vgs.iter().find(|v| v.name == "vg_data").unwrap();
        assert_eq!(vg.pvs, vec![BlockRef::Md("data1".into())]);
        assert_eq!(p.sizes.data_usable_bytes, 128 * GIB);
        assert_eq!(p.sizes.data_raw_bytes, 3 * 64 * GIB);
        assert!(p.warnings.iter().any(|w| w.contains("one disk only") && w.contains("unused")), "{:?}", p.warnings);
        assert!(p.summary().iter().any(|l| l.contains("hybrid RAID") && l.contains("128.0 GiB")), "{:?}", p.summary());
        // the unallocated tail of the big disk is really free
        let sdc = p.disks.iter().find(|d| d.disk == "sdc").unwrap();
        let last = sdc.create.iter().map(|c| c.end()).max().unwrap();
        assert!(last + 64 * 1024 * SECTORS_PER_MIB <= f.disk("sdc").unwrap().sectors);
    }

    #[test]
    fn shr_plan_for_2_4_4_8_has_consecutive_partitions_per_disk() {
        let (f, req) = shr_facts(&[32, 64, 64, 128]);
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        let names: Vec<&str> = p.arrays.iter().filter(|a| a.name.starts_with("data")).map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["data1", "data2"]);
        let level_members: Vec<(String, usize)> =
            p.arrays.iter().filter(|a| a.name.starts_with("data")).map(|a| (a.level.clone(), a.members.len())).collect();
        assert_eq!(level_members, vec![("5".to_string(), 4), ("5".to_string(), 3)]);
        assert_eq!(p.sizes.data_usable_bytes, (3 * 32 + 2 * 32) * GIB);
        let vg = p.vgs.iter().find(|v| v.name == "vg_data").unwrap();
        assert_eq!(vg.pvs, vec![BlockRef::Md("data1".into()), BlockRef::Md("data2".into())]);
        // sda has slice 1 only; sdb/sdc slice 1 and 2 back to back; sdd the same (slice 3 is unusable)
        assert_eq!(data_parts(&p, "sda").len(), 1);
        for d in ["sdb", "sdc", "sdd"] {
            let ps = data_parts(&p, d);
            assert_eq!(ps.len(), 2, "{d}");
            assert_eq!(ps[0].end(), ps[1].start, "{d}: slice partitions are consecutive");
            assert_eq!(ps[1].sectors * SECTOR, 32 * GIB);
        }
        // every array member exists as a created partition with the right size
        for a in p.arrays.iter().filter(|a| a.name.starts_with("data")) {
            for (disk, num) in &a.members {
                let c = p.disks.iter().find(|d| &d.disk == disk).unwrap().create.iter().find(|c| c.number == *num).unwrap();
                assert_eq!(c.role, PartRole::Data);
            }
        }
        p.validate(&f).unwrap();
    }

    #[test]
    fn shr_plan_for_1_2_2_3() {
        let (f, req) = shr_facts(&[16, 32, 32, 48]);
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert_eq!(p.sizes.data_usable_bytes, (3 * 16 + 2 * 16) * GIB);
        assert_eq!(p.shr.as_ref().unwrap().layout.slices.len(), 3);
    }

    #[test]
    fn shr2_with_four_equal_disks_is_one_raid6_array() {
        let (f, mut req) = shr_facts(&[64, 64, 64, 64]);
        req.data_raid = DataRaid::Shr2;
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        let data: Vec<&ArrayPlan> = p.arrays.iter().filter(|a| a.name.starts_with("data")).collect();
        assert_eq!((data.len(), data[0].level.as_str(), data[0].members.len()), (1, "6", 4));
        assert_eq!(p.sizes.data_usable_bytes, 128 * GIB);
        assert!(p.warnings.iter().all(|w| !w.contains("RAID 1")), "{:?}", p.warnings);
    }

    #[test]
    fn shr1_with_two_equal_disks_is_a_raid1_array() {
        let (f, req) = shr_facts(&[64, 64]);
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        let a = p.arrays.iter().find(|a| a.name == "data1").unwrap();
        assert_eq!((a.level.as_str(), a.members.len()), ("1", 2));
        assert_eq!(p.sizes.data_usable_bytes, 64 * GIB);
        assert!(p.arrays.iter().all(|a| a.name != "data"));
    }

    #[test]
    fn shr_with_too_few_disks_is_an_error() {
        let (f, mut req) = shr_facts(&[64]);
        let p = build_plan(&f, &req);
        assert!(!p.is_ok());
        assert!(p.errors.iter().any(|e| e.contains("at least 2")), "{:?}", p.errors);
        let (f, r) = shr_facts(&[64, 64, 64]);
        req = PlanRequest { data_raid: DataRaid::Shr2, ..r };
        let p = build_plan(&f, &req);
        assert!(!p.is_ok());
        assert!(
            p.errors.iter().any(|e| e.contains("Hybrid RAID, 2-disk") && e.contains("at least 4") && e.contains("3 are available")),
            "{:?}",
            p.errors
        );
        // two disks of very different size: still valid SHR-1, the big disk's extra space is reported unused
        let (f, req) = shr_facts(&[32, 96]);
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert_eq!(p.sizes.data_usable_bytes, 32 * GIB);
    }

    #[test]
    fn shr_tiers_closer_than_one_gib_merge_in_the_plan() {
        let (mut f, req) = shr_facts(&[64, 64, 64]);
        f.disks.iter_mut().find(|d| d.name == "sdc").unwrap().sectors += 700 * SECTORS_PER_MIB;
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert_eq!(p.shr.as_ref().unwrap().layout.slices.len(), 1);
        assert_eq!(p.sizes.data_usable_bytes, 128 * GIB);
    }

    #[test]
    fn shr_cannot_be_combined_with_preserved_arrays() {
        let f = facts();
        for raid in [DataRaid::Shr1, DataRaid::Shr2] {
            // keep mode, md2 preserved (the default): nothing wiped to build on
            let p = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, data_raid: raid, ..Default::default() });
            assert!(!p.is_ok());
            assert!(p.errors.iter().any(|e| e.contains("cannot be combined with preserved arrays") && e.contains("md2")), "{:?}", p.errors);
            assert!(p.arrays.is_empty() && p.disks.is_empty(), "no half-built plan");
        }
        // releasing md2 turns every disk into a wiped disk: SHR works again
        let req =
            PlanRequest { mode: StorageMode::KeepVolumes, keep_arrays: Some(vec![]), data_raid: DataRaid::Shr1, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert!(p.arrays.iter().any(|a| a.name == "data1"));
        p.validate(&f).unwrap();
    }

    #[test]
    fn shr_leaves_out_disks_that_keep_partitions_and_says_so() {
        let mut f = facts();
        // sdd5 is no longer part of an array: an unknown partition, which keep mode preserves
        f.arrays.iter_mut().find(|a| a.name == "md2").unwrap().members.retain(|m| m != "sdd5");
        let req =
            PlanRequest { mode: StorageMode::KeepVolumes, keep_arrays: Some(vec![]), data_raid: DataRaid::Shr1, ..Default::default() };
        let p = build_plan(&f, &req);
        assert!(p.is_ok(), "{:?}", p.errors);
        assert_eq!(p.shr.as_ref().unwrap().disks.len(), 3);
        assert!(p.warnings.iter().any(|w| w.contains("sdd") && w.contains("keep their partitions")), "{:?}", p.warnings);
        let p = build_plan(&f, &PlanRequest { data_raid: DataRaid::Shr2, ..req });
        assert!(!p.is_ok());
        assert!(p.errors.iter().any(|e| e.contains("sdd") && e.contains("cannot take part")), "{:?}", p.errors);
    }

    #[test]
    fn shr_default_system_copies_report_the_uneven_disk() {
        // 4 equal disks, system on 3: the 4th has ~9+ GiB more data space which cannot be protected
        let mut f = facts();
        for d in f.disks.iter_mut().filter(|d| d.name != "synoboot") {
            d.sectors = 200 * GIB / SECTOR;
        }
        let p = build_plan(&f, &PlanRequest { data_raid: DataRaid::Shr1, system_size_mib: 8192, ..Default::default() });
        assert!(p.is_ok(), "{:?}", p.errors);
        assert!(p.warnings.iter().any(|w| w.contains("no copy of the system")), "{:?}", p.warnings);
    }

    #[test]
    fn plain_levels_are_unchanged_by_shr_support() {
        let f = facts();
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.shr.is_none());
        assert_eq!(p.vgs.iter().find(|v| v.name == "vg_data").unwrap().pvs, vec![BlockRef::Md("data".into())]);
        assert_eq!(DataRaid::ALL.len(), 9);
        assert_eq!(DataRaid::Shr1.level(), "shr1");
        assert_eq!(DataRaid::Shr2.level(), "shr2");
        assert_eq!((DataRaid::Shr1.min_members(), DataRaid::Shr2.min_members()), (2, 4));
        assert_eq!(DataRaid::Shr1.label(), "Hybrid RAID, 1-disk protection (SHR-like)");
        assert_eq!(DataRaid::Shr2.label(), "Hybrid RAID, 2-disk protection (SHR-like)");
        assert_eq!(DataRaid::Raid5.shr_redundancy(), None);
    }

    #[test]
    fn vg_plan_reads_the_old_single_pv_form() {
        let old = r#"{"name":"vg_sys","pv":{"Md":"system"},"lvs":[]}"#;
        let v: VgPlan = serde_json::from_str(old).unwrap();
        assert_eq!(v.pvs, vec![BlockRef::Md("system".into())]);
        let new: VgPlan = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
        assert_eq!(new.pvs, v.pvs);
        let (f, req) = shr_facts(&[32, 64, 64, 128]);
        let p = build_plan(&f, &req);
        let back: Plan = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back.shr, p.shr);
        assert_eq!(back.vgs.last().unwrap().pvs.len(), 2);
    }

    #[test]
    fn swap_and_split_are_sane() {
        let (swap, root, free) = split_system(32 * 1024, 8 * 1024 * 1024);
        assert_eq!(swap, 4096);
        assert!(root > 20 * 1024);
        assert!(swap + root + free <= 32 * 1024);
        assert_eq!(swap_mib_for(256 * 1024), 1024);
    }

    #[test]
    fn the_real_ds918_plan_uses_all_four_disks_and_warns_about_the_running_resync() {
        let f = crate::inspect::parse_inspect(&crate::inspect::fixtures::ds918_two_volumes()).unwrap();
        let plan = build_plan(&f, &PlanRequest::default());
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        assert!(plan.warnings.iter().any(|w| w.contains("md2") && w.contains("resyncing 90.8%")), "{:?}", plan.warnings);
        assert!(!plan.warnings.iter().any(|w| w.contains("Not used and not touched")), "{:?}", plan.warnings);
        // the unit boots from its internal flash: no GRUB, no /boot or ESP on the disks, and all four disks (sda/sdb say removable=1) take part
        assert!(plan.dom_boot && plan.boot.is_none() && plan.esp.is_none() && plan.firmware == Firmware::Dom);
        assert_eq!(plan.disks.len(), 4);
        // the destroyed-volumes line names the two volumes once, not their sub-mounts or the FUSE mount
        let vol = plan.destroyed.iter().find(|d| d.starts_with("DSM volumes")).expect("volumes line");
        assert!(
            vol.contains("/volume1 (ext4") && vol.contains("/volume2 (btrfs") && !vol.contains("@docker") && !vol.contains("synodedup"),
            "{vol}"
        );
        // SHR over four equal disks with the system mirrored on all four: one data array, no unused slice, no uneven-space warning
        let shr = build_plan(&f, &PlanRequest { data_raid: DataRaid::Shr1, system_copies: Some(4), ..Default::default() });
        assert!(shr.is_ok(), "{:?}", shr.errors);
        assert_eq!(
            shr.arrays.iter().filter(|a| a.name.starts_with("data")).count(),
            1,
            "{:?}",
            shr.arrays.iter().map(|a| &a.name).collect::<Vec<_>>()
        );
        assert!(!shr.warnings.iter().any(|w| w.contains("one disk only") || w.contains("more data space")), "{:?}", shr.warnings);
        // a USB stick that is plugged in is named, not silently ignored
        let mut f2 = f.clone();
        f2.disks.push(crate::model::Disk {
            name: "sde".into(),
            model: "SanDisk".into(),
            serial: "X".into(),
            sectors: 62_500_000,
            logical_sector: 512,
            rotational: false,
            removable: true,
            table: crate::model::PartTable::Gpt,
            parts: vec![],
            transport: crate::model::Transport::Usb,
            device_path: String::new(),
        });
        let plan2 = build_plan(&f2, &PlanRequest::default());
        assert!(plan2.warnings.iter().any(|w| w.contains("Not used and not touched") && w.contains("sde")), "{:?}", plan2.warnings);
    }

    fn real_ds918() -> DeviceFacts {
        crate::inspect::parse_inspect(&crate::inspect::fixtures::ds918_two_volumes()).unwrap()
    }

    #[test]
    fn an_array_with_members_on_an_erased_and_an_untouched_disk_is_called_out() {
        // sdb reports removable and its port is unknown (an old saved report): it is left alone while sda, sdc and sdd are erased
        let mut f = real_ds918();
        f.disks.iter_mut().find(|d| d.name == "sdb").unwrap().transport = Transport::Unknown;
        assert_eq!(f.excluded_disks().len(), 2, "the boot flash and sdb");
        let p = build_plan(&f, &PlanRequest::default());
        let names: Vec<&str> = p.straddling.iter().map(|s| s.array.as_str()).collect();
        assert_eq!(names, ["md2", "md3"], "{:?}", p.straddling);
        assert_eq!(
            (p.straddling[0].erased.clone(), p.straddling[0].left.clone()),
            (vec!["sda".into(), "sdc".into(), "sdd".into()], vec!["sdb".to_string()])
        );
        let all = p.warnings.join("\n");
        assert!(
            all.contains("md2 and md3 span sda, sdc, sdd (erased) and sdb (not touched): they are lost as a whole, including their part on that disk."),
            "two arrays on the same disks make one warning: {all}"
        );
        assert_eq!(
            straddle_warnings(&p.straddling[..1]),
            ["md2 spans sda, sdc, sdd (erased) and sdb (not touched): it is lost as a whole, including its part on that disk."]
        );
        assert!(all.contains("Not used and not touched: sdb (removable, unknown port). synofurb does not write to them."), "{all}");
        assert!(!all.contains("stays as it is"), "the reassurance is false for sdb and must not be printed: {all}");
        // keeping the arrays removes the straddle: they are not erased
        let keep = build_plan(&f, &PlanRequest { mode: StorageMode::KeepVolumes, ..Default::default() });
        assert!(keep.straddling.is_empty(), "{:?}", keep.straddling);
    }

    #[test]
    fn dsm_data_stays_as_it_is_is_only_said_when_it_is_true() {
        let mut f = real_ds918();
        // a USB stick without DSM data: nothing to promise about DSM
        let usb = |name: &str, parts: Vec<Part>| Disk {
            name: name.into(),
            model: "SanDisk".into(),
            serial: "X".into(),
            sectors: 62_500_000,
            logical_sector: 512,
            rotational: false,
            removable: true,
            table: PartTable::Gpt,
            parts,
            transport: Transport::Usb,
            device_path: String::new(),
        };
        f.disks.push(usb("sde", vec![]));
        let w = build_plan(&f, &PlanRequest::default()).warnings.join("\n");
        assert!(w.contains("sde (USB disk). synofurb does not write to them."), "{w}");
        assert!(!w.contains("stays as it is"), "{w}");
        // a USB disk that holds a whole DSM data array of its own: that one really stays
        f.disks.push(usb("sdf", vec![Part { name: "sdf1".into(), number: 1, start: 2048, sectors: 1_000_000 }]));
        let mut md = f.arrays.iter().find(|a| a.name == "md2").unwrap().clone();
        (md.name, md.members, md.vg) = ("md9".into(), vec!["sdf1".into()], None);
        f.arrays.push(md);
        let p = build_plan(&f, &PlanRequest::default());
        assert!(p.straddling.is_empty());
        assert!(p.warnings.join("\n").contains("The DSM data on sdf stays as it is."), "{:?}", p.warnings);
    }

    #[test]
    fn only_a_resync_or_a_missing_member_says_the_data_is_not_protected() {
        let mut f = real_ds918();
        let w = build_plan(&f, &PlanRequest::default()).warnings.join("\n");
        assert!(w.contains("md2 is resyncing 90.8%. Let it finish before installing; until then your data is not fully protected."), "{w}");
        for a in &mut f.arrays {
            if a.name == "md2" {
                a.sync_action = "check".into();
            }
        }
        let w = build_plan(&f, &PlanRequest::default()).warnings.join("\n");
        assert!(w.contains("md2 is checking") && !w.contains("not fully protected"), "a check leaves every copy intact: {w}");
        // a missing member needs a repair, not waiting
        for a in &mut f.arrays {
            if a.name == "md3" {
                a.degraded = 1;
            }
        }
        let w = build_plan(&f, &PlanRequest::default()).warnings.join("\n");
        assert!(w.contains("md3 is degraded (1 missing). Repair it before installing"), "{w}");
    }
}
