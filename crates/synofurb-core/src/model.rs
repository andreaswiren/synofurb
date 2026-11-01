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
}

impl Firmware {
    pub fn label(self) -> &'static str {
        match self {
            Firmware::Uefi => "UEFI",
            Firmware::Bios => "legacy BIOS",
            Firmware::UBoot => "U-Boot (ARM)",
        }
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
}

impl Disk {
    pub fn size_bytes(&self) -> u64 {
        self.sectors * SECTOR
    }
    pub fn human_size(&self) -> String {
        human_bytes(self.size_bytes())
    }
    /// Internal flash modules (Synology `synoboot`) and USB sticks are never install targets.
    pub fn is_candidate(&self) -> bool {
        !self.removable && self.sectors * SECTOR >= 8 * GIB && !self.name.starts_with("synoboot")
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
    if disk.chars().last().is_some_and(|c| c.is_ascii_digit()) {
        format!("{disk}p{number}")
    } else {
        format!("{disk}{number}")
    }
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

/// Serial console parameters, e.g. `ttyS0` at 115200 baud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialConsole {
    pub tty: String,
    pub speed: u32,
}

impl Default for SerialConsole {
    fn default() -> Self {
        SerialConsole { tty: "ttyS0".into(), speed: 115_200 }
    }
}

impl SerialConsole {
    /// Parse `console=ttyS0,115200n8` style options out of a kernel command line.
    /// The *last* serial console wins (that is the kernel's `/dev/console`).
    pub fn from_cmdline(cmdline: &str) -> Option<SerialConsole> {
        let mut found = None;
        for tok in cmdline.split_whitespace() {
            let Some(v) = tok.strip_prefix("console=") else { continue };
            let (tty, rest) = v.split_once(',').unwrap_or((v, ""));
            if !(tty.starts_with("ttyS") || tty.starts_with("ttyAMA") || tty.starts_with("ttyUSB") || tty.starts_with("ttyMV") || tty.starts_with("ttyPS")) {
                continue;
            }
            if !tty.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let speed = digits.parse().ok().filter(|s| (1200..=4_000_000).contains(s)).unwrap_or(115_200);
            found = Some(SerialConsole { tty: tty.to_string(), speed });
        }
        found
    }
    /// Kernel argument, e.g. `console=ttyS0,115200n8`.
    pub fn kernel_arg(&self) -> String {
        format!("console={},{}n8", self.tty, self.speed)
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
    /// Kernel command line of the running DSM (used to find the serial console).
    pub cmdline: String,
    pub is_root: bool,
    pub nics: Vec<Nic>,
    pub disks: Vec<Disk>,
    pub arrays: Vec<MdArray>,
    pub lvs: Vec<Lv>,
    pub mounts: Vec<Mount>,
}

impl DeviceFacts {
    /// Serial console to configure on the new system: whatever DSM uses, else ttyS0@115200.
    pub fn serial_console(&self) -> SerialConsole {
        SerialConsole::from_cmdline(&self.cmdline).unwrap_or_default()
    }
    pub fn candidate_disks(&self) -> Vec<&Disk> {
        self.disks.iter().filter(|d| d.is_candidate()).collect()
    }
    pub fn data_arrays(&self) -> Vec<&MdArray> {
        self.arrays.iter().filter(|a| a.role == ArrayRole::Data).collect()
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
        assert_eq!(SerialConsole::default().tty, "ttyS0");
    }

    #[test]
    fn human() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(4 * 1024 * GIB), "4.0 TiB");
    }
}
