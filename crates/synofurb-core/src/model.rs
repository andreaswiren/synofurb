//! Shared data model: what we know about a NAS before we touch it.
//!
//! All sector numbers are in 512-byte units (that is what `/sys/block/*/size`
//! and `/sys/block/*/*/start` report, independent of the logical sector size).

use serde::{Deserialize, Serialize};

pub const SECTOR: u64 = 512;
pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
pub const SECTORS_PER_MIB: u64 = MIB / SECTOR;

/// CPU architecture of the NAS, expressed as the Debian architecture we would install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    /// Debian `amd64`
    X86_64,
    /// Debian `arm64`
    Arm64,
    /// Debian `armhf` (ARMv7 hard-float)
    Armhf,
    /// Anything Debian 13 cannot be installed on (PowerPC, 32-bit x86, armel, ...)
    Unsupported,
}

impl Arch {
    pub fn from_uname(m: &str) -> Arch {
        match m.trim() {
            "x86_64" | "amd64" => Arch::X86_64,
            "aarch64" | "arm64" => Arch::Arm64,
            "armv7l" | "armv7" | "armhf" => Arch::Armhf,
            _ => Arch::Unsupported,
        }
    }
    pub fn debian(self) -> &'static str {
        match self {
            Arch::X86_64 => "amd64",
            Arch::Arm64 => "arm64",
            Arch::Armhf => "armhf",
            Arch::Unsupported => "unsupported",
        }
    }
    pub fn kernel_package(self) -> &'static str {
        match self {
            Arch::X86_64 => "linux-image-amd64",
            Arch::Arm64 => "linux-image-arm64",
            Arch::Armhf => "linux-image-armmp",
            Arch::Unsupported => "linux-image-unsupported",
        }
    }
    /// Debian package that installs GRUB for UEFI on this architecture.
    pub fn grub_efi_package(self) -> &'static str {
        match self {
            Arch::X86_64 => "grub-efi-amd64",
            Arch::Arm64 => "grub-efi-arm64",
            Arch::Armhf => "grub-efi-arm",
            Arch::Unsupported => "grub-efi-unsupported",
        }
    }
    /// `grub-install --target` for UEFI.
    pub fn grub_efi_target(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64-efi",
            Arch::Arm64 => "arm64-efi",
            Arch::Armhf => "arm-efi",
            Arch::Unsupported => "unsupported",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86-64 (amd64)",
            Arch::Arm64 => "ARM 64-bit (arm64)",
            Arch::Armhf => "ARM 32-bit (armhf)",
            Arch::Unsupported => "unsupported",
        }
    }
    pub fn is_arm(self) -> bool {
        matches!(self, Arch::Arm64 | Arch::Armhf)
    }
}

/// How the machine boots, which decides the bootloader and partition layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Firmware {
    /// UEFI: GRUB EFI on an ESP (mirrored across the system disks).
    Uefi,
    /// Legacy BIOS: GRUB on GPT with a `bios_grub` partition on every system disk.
    Bios,
    /// U-Boot style ARM boards without UEFI (flash-kernel). Experimental.
    UBoot,
    /// x86 Synology unit that always boots its internal DOM flash module: Debian is added as an extra
    /// boot entry there (kernel + initrd live on the DOM, everything else on the disks).
    Dom,
}

impl Firmware {
    pub fn label(self) -> &'static str {
        match self {
            Firmware::Uefi => "UEFI",
            Firmware::Bios => "legacy BIOS",
            Firmware::UBoot => "U-Boot (ARM)",
            Firmware::Dom => "internal boot flash (DOM)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuVendor {
    Intel,
    Amd,
    Arm,
    Other,
}

impl CpuVendor {
    /// From `/proc/cpuinfo` `vendor_id` (x86) or the architecture.
    pub fn detect(vendor_id: &str, arch: Arch) -> CpuVendor {
        match vendor_id.trim() {
            "GenuineIntel" => CpuVendor::Intel,
            "AuthenticAMD" | "HygonGenuine" => CpuVendor::Amd,
            _ if arch.is_arm() => CpuVendor::Arm,
            _ => CpuVendor::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CpuVendor::Intel => "Intel",
            CpuVendor::Amd => "AMD",
            CpuVendor::Arm => "ARM",
            CpuVendor::Other => "other",
        }
    }
    /// Debian package with CPU microcode updates (non-free-firmware), if any.
    pub fn microcode_package(self) -> Option<&'static str> {
        match self {
            CpuVendor::Intel => Some("intel-microcode"),
            CpuVendor::Amd => Some("amd64-microcode"),
            _ => None,
        }
    }
}

/// The internal boot flash module of x86 Synology units (see `boot/dom`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomFacts {
    pub sectors: u64,
    pub p1_sectors: u64,
    pub p2_sectors: u64,
    /// FAT volume id 10EE-589C found on partition 1
    pub p1_magic: bool,
    /// ext2 magic + Synology UUID found on partition 2
    pub p2_magic: bool,
}

impl DomFacts {
    /// Both partitions look like Synology's boot loader and are big enough to be edited safely.
    pub fn is_editable(&self) -> bool {
        self.p1_magic && self.p2_magic && self.p1_sectors >= 16 * 2048 && self.p2_sectors >= 64 * 2048
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PartTable {
    Gpt,
    Msdos,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part {
    /// Kernel name, e.g. `sda5` or `nvme0n1p2`.
    pub name: String,
    pub number: u32,
    pub start: u64,
    pub sectors: u64,
}

impl Part {
    pub fn end(&self) -> u64 {
        self.start + self.sectors
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    /// Kernel name in the *running* OS (DSM), e.g. `sata1`, `sda`, `nvme0n1`.
    /// The installer environment may enumerate disks differently, so the
    /// installer matches disks by serial/model/size, never by this name.
    pub name: String,
    pub model: String,
    pub serial: String,
    pub sectors: u64,
    pub logical_sector: u32,
    pub rotational: bool,
    pub removable: bool,
    pub table: PartTable,
    pub parts: Vec<Part>,
    /// How the disk is attached (from its sysfs device path). Old saved facts have no value: `Unknown`.
    #[serde(default)]
    pub transport: Transport,
    /// `readlink -f /sys/block/<name>/device` on the unit (diagnostics; empty when not collected).
    #[serde(default)]
    pub device_path: String,
}

/// How a disk is attached to the unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    #[default]
    Unknown,
    Sata,
    Sas,
    Nvme,
    Usb,
    Mmc,
    Virtual,
}

impl Transport {
    /// Classify a resolved sysfs device path such as `/sys/devices/pci0000:00/0000:00:12.0/ata1/host0/target0:0:0/0:0:0:0`.
    pub fn from_sysfs_path(path: &str) -> Transport {
        let p = path.to_ascii_lowercase();
        if p.contains("/usb") {
            Transport::Usb
        } else if p.contains("/mmc") {
            Transport::Mmc
        } else if p.contains("/nvme") {
            Transport::Nvme
        } else if p.contains("/virtio") || p.contains("/vmbus") || p.contains("/xen") {
            Transport::Virtual
        } else if p.contains("/ata") {
            Transport::Sata
        } else if p.contains("/port-") || p.contains("/sas") || p.contains("/end_device") {
            Transport::Sas
        } else {
            Transport::Unknown
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Transport::Unknown => "unknown",
            Transport::Sata => "SATA",
            Transport::Sas => "SAS",
            Transport::Nvme => "NVMe",
            Transport::Usb => "USB",
            Transport::Mmc => "memory card",
            Transport::Virtual => "virtual",
        }
    }
}

/// Why a disk is not an install target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exclusion {
    /// Synology's internal boot flash module (`synoboot`).
    BootFlash,
    /// Attached over USB (or a memory card).
    Usb,
    /// Reports itself as removable and its transport is unknown: not touched unless the transport proves it is internal.
    Removable,
    /// Smaller than 8 GiB.
    TooSmall,
}

/// Make the first word of a reason start in lower case for use inside a sentence, but leave acronyms and units alone:
/// `Boot flash` -> `boot flash`, `Under 8 GiB` -> `under 8 GiB`, `USB disk` -> `USB disk` (a plain `to_lowercase` gives `usb disk`).
pub fn lower_first(s: &str) -> String {
    let first = s.split(' ').next().unwrap_or("");
    let acronym = first.chars().count() > 1 && first.chars().skip(1).any(|c| c.is_uppercase());
    if acronym {
        return s.to_string();
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

impl Exclusion {
    /// [`Exclusion::reason`] ready to sit inside a sentence ("usb" stays "USB", "GiB" stays "GiB").
    pub fn reason_inline(&self) -> String {
        lower_first(self.reason())
    }

    pub fn reason(&self) -> &'static str {
        match self {
            Exclusion::BootFlash => "Boot flash",
            Exclusion::Usb => "USB disk",
            Exclusion::Removable => "Removable, unknown port",
            Exclusion::TooSmall => "Under 8 GiB",
        }
    }
}

impl Disk {
    pub fn size_bytes(&self) -> u64 {
        self.sectors * SECTOR
    }
    pub fn human_size(&self) -> String {
        human_bytes(self.size_bytes())
    }
    /// Why this disk is not an install target (None = it is one). Internal flash modules (Synology `synoboot`) and USB sticks
    /// never are. The kernel's `removable` flag alone does not exclude a disk: some Synology bays (for example the ones behind an
    /// add-in SATA controller on a DS918+) report `removable=1` although they hold ordinary internal disks, so a removable disk
    /// on an internal transport (SATA, SAS, NVMe) stays a target; only an unknown transport plus `removable` keeps it out.
    pub fn exclusion(&self) -> Option<Exclusion> {
        if self.name.starts_with("synoboot") {
            Some(Exclusion::BootFlash)
        } else if matches!(self.transport, Transport::Usb | Transport::Mmc) {
            Some(Exclusion::Usb)
        } else if self.removable && !matches!(self.transport, Transport::Sata | Transport::Sas | Transport::Nvme | Transport::Virtual) {
            Some(Exclusion::Removable)
        } else if self.sectors * SECTOR < 8 * GIB {
            Some(Exclusion::TooSmall)
        } else {
            None
        }
    }
    pub fn is_candidate(&self) -> bool {
        self.exclusion().is_none()
    }
    pub fn part(&self, number: u32) -> Option<&Part> {
        self.parts.iter().find(|p| p.number == number)
    }
    /// Partition device name for `number`, following the kernel convention
    /// (`sda` -> `sda1`, `nvme0n1` -> `nvme0n1p1`).
    pub fn part_name(&self, number: u32) -> String {
        part_device_name(&self.name, number)
    }
}

pub fn part_device_name(disk: &str, number: u32) -> String {
    if disk.chars().last().is_some_and(|c| c.is_ascii_digit()) { format!("{disk}p{number}") } else { format!("{disk}{number}") }
}

/// What a RAID array was used for by DSM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArrayRole {
    /// DSM system volume (RAID1 across partition 1 of every disk) - reclaimed by the install.
    DsmSystem,
    /// DSM swap (partition 2) - reclaimed.
    DsmSwap,
    /// Everything else: user data volumes, caches, ...
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdArray {
    pub name: String,
    pub level: String,
    pub state: String,
    pub sectors: u64,
    /// Member partition names (`sda5`, ...)
    pub members: Vec<String>,
    pub uuid: String,
    pub label: String,
    pub metadata: String,
    pub role: ArrayRole,
    /// LVM volume group living on this array, if any (e.g. `vg1000`).
    pub vg: Option<String>,
    /// `md/sync_action` (`idle`, `resync`, `recover`, `check`, `reshape`, ...); empty when not collected.
    #[serde(default)]
    pub sync_action: String,
    /// Progress of a running sync in tenths of a percent (0..=1000), when one is running.
    #[serde(default)]
    pub sync_permille: Option<u32>,
    /// `md/degraded`: number of missing members (0 = complete).
    #[serde(default)]
    pub degraded: u32,
}

impl MdArray {
    /// A sync, rebuild or reshape is running, or members are missing: the array is not fully redundant right now.
    pub fn is_busy_or_degraded(&self) -> bool {
        self.degraded > 0 || (!self.sync_action.is_empty() && self.sync_action != "idle")
    }
    /// Does what is running (or missing) mean the data is not fully protected right now? True for a resync, a rebuild, a
    /// reshape and missing members; false for a read-only `check`, where every copy is intact.
    pub fn reduces_redundancy(&self) -> bool {
        self.degraded > 0 || matches!(self.sync_action.as_str(), "resync" | "recover" | "reshape")
    }
    /// One-line status for the UI, e.g. `resyncing 90.8%` or `degraded (1 missing)`.
    pub fn health_note(&self) -> Option<String> {
        if self.degraded > 0 {
            return Some(format!("degraded ({} missing)", self.degraded));
        }
        if self.sync_action.is_empty() || self.sync_action == "idle" {
            return None;
        }
        let verb = match self.sync_action.as_str() {
            "resync" => "resyncing",
            "recover" => "rebuilding",
            "check" => "checking",
            "repair" => "repairing",
            "reshape" => "reshaping",
            other => other,
        };
        Some(match self.sync_permille {
            Some(p) => format!("{verb} {}.{}%", p / 10, p % 10),
            None => verb.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lv {
    pub vg: String,
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub device: String,
    pub mountpoint: String,
    pub fstype: String,
    pub size_kib: u64,
    pub used_kib: u64,
}

/// Serial console parameters, e.g. `ttyS0` at 115200 baud, or a raw `uart8250,mmio32,0xfedc9000,115200n8`
/// console (AMD Ryzen/EPYC Synology boards have no legacy I/O-port UART).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialConsole {
    /// Device name for getty (`ttyS0`). Empty for raw consoles: systemd's getty generator finds those.
    pub tty: String,
    pub speed: u32,
    /// Value after `console=` when it is not a plain tty name.
    #[serde(default)]
    pub raw: Option<String>,
}

impl Default for SerialConsole {
    fn default() -> Self {
        SerialConsole { tty: "ttyS0".into(), speed: 115_200, raw: None }
    }
}

fn console_speed(rest: &str) -> u32 {
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|s| (1200..=4_000_000).contains(s)).unwrap_or(115_200)
}

impl SerialConsole {
    /// Parse `console=` options out of a kernel command line. The *last* serial console wins (that is the
    /// kernel's `/dev/console`). Understands `ttyS0,115200n8` and `uart8250,mmio32,0xADDR,115200n8`.
    pub fn from_cmdline(cmdline: &str) -> Option<SerialConsole> {
        let mut found = None;
        for tok in cmdline.split_whitespace() {
            let Some(v) = tok.strip_prefix("console=") else { continue };
            let (name, rest) = v.split_once(',').unwrap_or((v, ""));
            if name.starts_with("uart8250") || name.starts_with("uart") || name.starts_with("earlycon") {
                let ok = v.chars().all(|c| c.is_ascii_alphanumeric() || ",._-:=".contains(c));
                if ok && v.contains("0x") {
                    let speed = v.rsplit(',').next().map(console_speed).unwrap_or(115_200);
                    found = Some(SerialConsole { tty: String::new(), speed, raw: Some(v.to_string()) });
                }
                continue;
            }
            if !(name.starts_with("ttyS")
                || name.starts_with("ttyAMA")
                || name.starts_with("ttyUSB")
                || name.starts_with("ttyMV")
                || name.starts_with("ttyPS"))
            {
                continue;
            }
            if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            found = Some(SerialConsole { tty: name.to_string(), speed: console_speed(rest), raw: None });
        }
        found
    }
    /// Kernel argument, e.g. `console=ttyS0,115200n8`.
    pub fn kernel_arg(&self) -> String {
        match &self.raw {
            Some(r) => format!("console={r}"),
            None => format!("console={},{}n8", self.tty, self.speed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nic {
    pub name: String,
    pub mac: String,
}

/// Everything learned from inspecting a running DSM over SSH.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFacts {
    pub model: String,
    pub unique: String,
    pub dsm_version: String,
    pub serial: String,
    pub hostname: String,
    pub uname_m: String,
    pub kernel: String,
    pub arch: Arch,
    pub mem_kib: u64,
    pub cpu: String,
    pub firmware: Firmware,
    pub kexec: bool,
    pub dt_model: String,
    pub cpu_vendor: CpuVendor,
    /// Internal boot flash module, when the unit has one.
    pub dom: Option<DomFacts>,
    /// Kernel command line of the running DSM (used to find the serial console).
    pub cmdline: String,
    pub is_root: bool,
    pub nics: Vec<Nic>,
    pub disks: Vec<Disk>,
    pub arrays: Vec<MdArray>,
    pub lvs: Vec<Lv>,
    pub mounts: Vec<Mount>,
}

/// A DSM storage volume as the user knows it (`volume1`), with what backs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DsmVolume {
    /// `volume1`
    pub name: String,
    pub mountpoint: String,
    pub fstype: String,
    pub size_kib: u64,
    pub used_kib: u64,
    /// LVM logical volume (`volume_1`) and its volume group (`vg1`), when the volume lives on LVM.
    pub lv: Option<String>,
    pub vg: Option<String>,
    pub lv_bytes: Option<u64>,
    /// Data RAID arrays that back it (`md2`, `md3`): several when the pool is Synology Hybrid RAID-like.
    pub arrays: Vec<String>,
    /// Disks that hold those arrays.
    pub disks: Vec<String>,
    /// Distinct RAID levels among the arrays (`raid5`, ...).
    pub levels: Vec<String>,
}

impl DsmVolume {
    pub fn size_bytes(&self) -> u64 {
        self.size_kib * 1024
    }
    pub fn used_bytes(&self) -> u64 {
        self.used_kib * 1024
    }
    /// Used share in whole percent (0 when the size is unknown).
    pub fn used_percent(&self) -> u32 {
        if self.size_kib == 0 { 0 } else { ((self.used_kib as f64 / self.size_kib as f64) * 100.0).round() as u32 }
    }
}

#[doc(hidden)]
pub fn split_dm_name_for_test(name: &str) -> Option<(String, String)> {
    split_dm_name(name)
}

/// `vg1-volume_1` -> (`vg1`, `volume_1`); LVM doubles the dashes that belong to a name (`my--vg-lv` is `my-vg` / `lv`).
fn split_dm_name(name: &str) -> Option<(String, String)> {
    let b = name.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'-' {
            if i + 1 < b.len() && b[i + 1] == b'-' {
                i += 2;
                continue;
            }
            let (vg, lv) = (&name[..i], &name[i + 1..]);
            return (!vg.is_empty() && !lv.is_empty()).then(|| (vg.replace("--", "-"), lv.replace("--", "-")));
        }
        i += 1;
    }
    None
}

impl DeviceFacts {
    /// Serial console to configure on the new system: whatever DSM uses, else ttyS0@115200.
    pub fn serial_console(&self) -> SerialConsole {
        SerialConsole::from_cmdline(&self.cmdline).unwrap_or_default()
    }
    pub fn candidate_disks(&self) -> Vec<&Disk> {
        self.disks.iter().filter(|d| d.is_candidate()).collect()
    }
    /// Disks that are NOT install targets, each with the reason (never silently dropped: the UI lists them).
    pub fn excluded_disks(&self) -> Vec<(&Disk, Exclusion)> {
        self.disks.iter().filter_map(|d| d.exclusion().map(|e| (d, e))).collect()
    }
    pub fn data_arrays(&self) -> Vec<&MdArray> {
        self.arrays.iter().filter(|a| a.role == ArrayRole::Data).collect()
    }
    /// Arrays that are resyncing, rebuilding, reshaping or missing members right now (an install should wait for them).
    pub fn busy_arrays(&self) -> Vec<&MdArray> {
        self.arrays.iter().filter(|a| a.is_busy_or_degraded()).collect()
    }
    /// DSM storage volumes (`/volume1`, `/volume2`, ...): the mounts that DSM exposes, joined with the LVM logical volume and the
    /// RAID arrays and disks behind them. Sub-mounts (`/volume2/@docker`, FUSE mounts) are not volumes and are skipped.
    pub fn volumes(&self) -> Vec<DsmVolume> {
        let mut out: Vec<DsmVolume> = vec![];
        for m in &self.mounts {
            let Some(num) = m.mountpoint.strip_prefix("/volume") else { continue };
            if num.is_empty() || !num.bytes().all(|c| c.is_ascii_digit()) || out.iter().any(|v| v.mountpoint == m.mountpoint) {
                continue;
            }
            let lv = self.lv_for_mount(&m.device, num);
            let arrays: Vec<&MdArray> = match lv {
                Some(l) => self.arrays.iter().filter(|a| a.role == ArrayRole::Data && a.vg.as_deref() == Some(l.vg.as_str())).collect(),
                None => m.device.strip_prefix("/dev/").and_then(|n| self.arrays.iter().find(|a| a.name == n)).into_iter().collect(),
            };
            let mut disks: Vec<String> =
                arrays.iter().flat_map(|a| a.members.iter()).filter_map(|mem| self.locate_part(mem).map(|(d, _)| d.name.clone())).collect();
            disks.sort();
            disks.dedup();
            let mut levels: Vec<String> = arrays.iter().map(|a| a.level.clone()).collect();
            levels.sort();
            levels.dedup();
            out.push(DsmVolume {
                name: format!("volume{num}"),
                mountpoint: m.mountpoint.clone(),
                fstype: m.fstype.clone(),
                size_kib: m.size_kib,
                used_kib: m.used_kib,
                lv: lv.map(|l| l.name.clone()),
                vg: lv.map(|l| l.vg.clone()),
                lv_bytes: lv.map(|l| l.bytes),
                arrays: arrays.iter().map(|a| a.name.clone()).collect(),
                disks,
                levels,
            });
        }
        out.sort_by(|a, b| a.mountpoint.len().cmp(&b.mountpoint.len()).then(a.mountpoint.cmp(&b.mountpoint)));
        out
    }

    /// The LVM logical volume behind a mount: from the device name when it says so (`/dev/mapper/vg1-volume_1`, `/dev/vg1/volume_1`),
    /// else by DSM's naming (`/volume1` is `volume_1`, also behind `cachedev_N` devices), else the only logical volume there is.
    fn lv_for_mount(&self, device: &str, num: &str) -> Option<&Lv> {
        if let Some(rest) = device.strip_prefix("/dev/mapper/") {
            if let Some((vg, lv)) = split_dm_name(rest)
                && let Some(l) = self.lvs.iter().find(|l| l.vg == vg && l.name == lv)
            {
                return Some(l);
            }
        } else if let Some(rest) = device.strip_prefix("/dev/")
            && let Some((vg, lv)) = rest.split_once('/')
            && let Some(l) = self.lvs.iter().find(|l| l.vg == vg && l.name == lv)
        {
            return Some(l);
        }
        let want = format!("volume_{num}");
        self.lvs.iter().find(|l| l.name == want).or_else(|| (self.lvs.len() == 1).then(|| &self.lvs[0]))
    }
    pub fn mem_gib(&self) -> f64 {
        self.mem_kib as f64 / (1024.0 * 1024.0)
    }
    pub fn disk(&self, name: &str) -> Option<&Disk> {
        self.disks.iter().find(|d| d.name == name)
    }
    /// Disk and partition number that own partition device `part` (`sda5` -> (`sda`, 5)).
    pub fn locate_part(&self, part: &str) -> Option<(&Disk, &Part)> {
        for d in &self.disks {
            if let Some(p) = d.parts.iter().find(|p| p.name == part) {
                return Some((d, p));
            }
        }
        None
    }
}

pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", UNITS[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_names_follow_kernel_convention() {
        assert_eq!(part_device_name("sda", 5), "sda5");
        assert_eq!(part_device_name("sata1", 5), "sata1p5");
        assert_eq!(part_device_name("nvme0n1", 2), "nvme0n1p2");
    }

    #[test]
    fn arch_mapping() {
        assert_eq!(Arch::from_uname("x86_64"), Arch::X86_64);
        assert_eq!(Arch::from_uname("aarch64"), Arch::Arm64);
        assert_eq!(Arch::from_uname("armv7l"), Arch::Armhf);
        assert_eq!(Arch::from_uname("ppc"), Arch::Unsupported);
        assert_eq!(Arch::from_uname("i686"), Arch::Unsupported);
    }

    #[test]
    fn serial_console_from_cmdline() {
        let c = SerialConsole::from_cmdline("root=/dev/md0 console=tty0 console=ttyS0,115200n8 quiet").unwrap();
        assert_eq!((c.tty.as_str(), c.speed), ("ttyS0", 115_200));
        assert_eq!(c.kernel_arg(), "console=ttyS0,115200n8");
        let c = SerialConsole::from_cmdline("console=ttyS1,9600").unwrap();
        assert_eq!((c.tty.as_str(), c.speed), ("ttyS1", 9600));
        assert!(SerialConsole::from_cmdline("console=tty0 quiet").is_none());
        assert!(SerialConsole::from_cmdline("console=ttyS0;rm,115200").is_none(), "no shell metacharacters");
        // AMD Ryzen/EPYC boards: MMIO UART, no ttyS name
        let amd = SerialConsole::from_cmdline("console=tty0 console=uart8250,mmio32,0xfedc9000,115200n8").unwrap();
        assert_eq!(amd.raw.as_deref(), Some("uart8250,mmio32,0xfedc9000,115200n8"));
        assert_eq!(amd.kernel_arg(), "console=uart8250,mmio32,0xfedc9000,115200n8");
        assert_eq!((amd.tty.as_str(), amd.speed), ("", 115_200));
        assert!(SerialConsole::from_cmdline("console=uart8250,mmio32,0xfe;rm -rf,9600").is_none());
        assert_eq!(SerialConsole::default().tty, "ttyS0");
    }

    #[test]
    fn vendors_and_microcode() {
        assert_eq!(CpuVendor::detect("AuthenticAMD", Arch::X86_64), CpuVendor::Amd);
        assert_eq!(CpuVendor::detect("GenuineIntel", Arch::X86_64), CpuVendor::Intel);
        assert_eq!(CpuVendor::detect("", Arch::Armhf), CpuVendor::Arm);
        assert_eq!(CpuVendor::Amd.microcode_package(), Some("amd64-microcode"));
        assert_eq!(CpuVendor::Arm.microcode_package(), None);
    }

    #[test]
    fn dom_editability() {
        let d = DomFacts { sectors: 262_144, p1_sectors: 40_000, p2_sectors: 200_000, p1_magic: true, p2_magic: true };
        assert!(d.is_editable());
        assert!(!DomFacts { p2_magic: false, ..d.clone() }.is_editable());
        assert!(!DomFacts { p2_sectors: 100_000, ..d }.is_editable(), "P2 below 64 MiB is refused");
    }

    #[test]
    fn human() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(4 * 1024 * GIB), "4.0 TiB");
    }
}
