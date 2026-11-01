//! Knowledge about Synology hardware platforms.
//!
//! DSM identifies the board family in `/etc.defaults/synoinfo.conf` as
//! `unique="synology_<platform>_<model>"` (for example `synology_apollolake_918+`).
//! We map the platform token to an architecture and an honest support level.
//! This is deliberately conservative: nothing here has been verified on every
//! model, and the UI says so.

use crate::model::Arch;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Support {
    /// Standard PC-class hardware; the normal Debian installer path applies.
    Supported,
    /// Might work, needs serial console access and manual verification.
    Experimental,
    /// Debian 13 cannot run on this CPU/boot chain. The tool refuses to proceed.
    Unsupported,
}

impl Support {
    pub fn label(self) -> &'static str {
        match self {
            Support::Supported => "supported",
            Support::Experimental => "experimental",
            Support::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub arch: Arch,
    pub soc: &'static str,
    pub support: Support,
    pub note: &'static str,
}

struct Row {
    token: &'static str,
    arch: Arch,
    soc: &'static str,
    support: Support,
    note: &'static str,
}

const X86_NOTE: &str = "PC-class x86-64 board. Boots standard Debian; check BIOS boot order after install (the internal Synology flash module may take precedence).";
const X86_LEGACY: &str = "Older x86-64 Atom board, almost certainly legacy BIOS boot. Serial console strongly recommended.";

const TABLE: &[Row] = &[
    Row { token: "apollolake", arch: Arch::X86_64, soc: "Intel Apollo Lake", support: Support::Supported, note: X86_NOTE },
    Row { token: "geminilake", arch: Arch::X86_64, soc: "Intel Gemini Lake", support: Support::Supported, note: X86_NOTE },
    Row { token: "denverton", arch: Arch::X86_64, soc: "Intel Denverton", support: Support::Supported, note: X86_NOTE },
    Row { token: "v1000", arch: Arch::X86_64, soc: "AMD Ryzen V1000", support: Support::Supported, note: X86_NOTE },
    Row { token: "v1000nk", arch: Arch::X86_64, soc: "AMD Ryzen V1000", support: Support::Supported, note: X86_NOTE },
    Row { token: "r1000", arch: Arch::X86_64, soc: "AMD Ryzen R1000", support: Support::Supported, note: X86_NOTE },
    Row { token: "r1000nk", arch: Arch::X86_64, soc: "AMD Ryzen R1000", support: Support::Supported, note: X86_NOTE },
    Row { token: "epyc7002", arch: Arch::X86_64, soc: "AMD EPYC 7002", support: Support::Supported, note: X86_NOTE },
    Row { token: "broadwell", arch: Arch::X86_64, soc: "Intel Broadwell-DE", support: Support::Supported, note: X86_NOTE },
    Row { token: "broadwellnk", arch: Arch::X86_64, soc: "Intel Broadwell-DE", support: Support::Supported, note: X86_NOTE },
    Row { token: "broadwellnkv2", arch: Arch::X86_64, soc: "Intel Broadwell-DE", support: Support::Supported, note: X86_NOTE },
    Row { token: "broadwellntbap", arch: Arch::X86_64, soc: "Intel Broadwell-DE", support: Support::Supported, note: X86_NOTE },
    Row { token: "grantley", arch: Arch::X86_64, soc: "Intel Xeon E5 v3/v4", support: Support::Supported, note: X86_NOTE },
    Row { token: "purley", arch: Arch::X86_64, soc: "Intel Xeon Scalable", support: Support::Supported, note: X86_NOTE },
    Row { token: "kvmx64", arch: Arch::X86_64, soc: "virtual machine", support: Support::Supported, note: "Virtual DSM guest." },
    Row { token: "x64", arch: Arch::X86_64, soc: "x86-64", support: Support::Supported, note: X86_NOTE },
    Row { token: "bromolow", arch: Arch::X86_64, soc: "Intel Bromolow", support: Support::Supported, note: X86_LEGACY },
    Row { token: "cedarview", arch: Arch::X86_64, soc: "Intel Atom Cedarview", support: Support::Supported, note: X86_LEGACY },
    Row { token: "avoton", arch: Arch::X86_64, soc: "Intel Atom Avoton", support: Support::Supported, note: X86_LEGACY },
    Row { token: "braswell", arch: Arch::X86_64, soc: "Intel Braswell", support: Support::Supported, note: X86_LEGACY },
    // ARM64
    Row {
        token: "rtd1296",
        arch: Arch::Arm64,
        soc: "Realtek RTD1296",
        support: Support::Experimental,
        note: "Synology U-Boot, no UEFI. Needs a serial console and a U-Boot boot recipe; mainline support is partial.",
    },
    Row {
        token: "rtd1619b",
        arch: Arch::Arm64,
        soc: "Realtek RTD1619B",
        support: Support::Experimental,
        note: "Synology U-Boot, no UEFI. Needs a serial console and a U-Boot boot recipe; mainline support is partial.",
    },
    Row {
        token: "alpine4k",
        arch: Arch::Arm64,
        soc: "Annapurna Alpine v2",
        support: Support::Experimental,
        note: "Annapurna Alpine. Mainline Linux support is limited; expect missing drivers.",
    },
    Row {
        token: "armada37xx",
        arch: Arch::Arm64,
        soc: "Marvell Armada 37xx",
        support: Support::Experimental,
        note: "Armada 37xx has mainline support but Synology U-Boot must be steered to boot from disk.",
    },
    // ARMv7 (armhf)
    Row {
        token: "armada38x",
        arch: Arch::Armhf,
        soc: "Marvell Armada 38x",
        support: Support::Supported,
        note: "ARMv7 (armhf). Documented on the Debian wiki for DS218j and RS816: Debian boots from raw disk sectors via U-Boot, changed once over the serial console (115200 baud). Mainline has a DTB for DS116 only; other models need a custom DTB.",
    },
    Row {
        token: "armada370",
        arch: Arch::Armhf,
        soc: "Marvell Armada 370",
        support: Support::Supported,
        note: "ARMv7 (armhf). Mainline has a DTB for DS213j. Debian boots from raw disk sectors via U-Boot, changed once over the serial console (115200 baud).",
    },
    Row {
        token: "armada375",
        arch: Arch::Armhf,
        soc: "Marvell Armada 375",
        support: Support::Experimental,
        note: "Armada 375 support is partial. Synology U-Boot must be steered to boot from disk.",
    },
    Row {
        token: "armadaxp",
        arch: Arch::Armhf,
        soc: "Marvell Armada XP",
        support: Support::Supported,
        note: "ARMv7 (armhf). Mainline has a DTB for DS414. Debian boots from raw disk sectors via U-Boot, changed once over the serial console (115200 baud).",
    },
    Row {
        token: "alpine",
        arch: Arch::Armhf,
        soc: "Annapurna Alpine v1",
        support: Support::Experimental,
        note: "Annapurna Alpine. Mainline Linux support is limited; expect missing drivers.",
    },
    // Dead ends
    Row {
        token: "comcerto2k",
        arch: Arch::Unsupported,
        soc: "Mindspeed Comcerto 2000",
        support: Support::Unsupported,
        note: "No mainline Linux support for this SoC.",
    },
    Row {
        token: "hi3535",
        arch: Arch::Unsupported,
        soc: "HiSilicon Hi3535",
        support: Support::Unsupported,
        note: "No mainline Linux support for this SoC.",
    },
    Row {
        token: "monaco",
        arch: Arch::Unsupported,
        soc: "Synology Monaco",
        support: Support::Unsupported,
        note: "Not supported by Debian 13.",
    },
    Row {
        token: "qoriq",
        arch: Arch::Unsupported,
        soc: "Freescale QorIQ (PowerPC)",
        support: Support::Unsupported,
        note: "PowerPC. Debian 13 has no 32-bit PowerPC port.",
    },
    Row {
        token: "ppc853x",
        arch: Arch::Unsupported,
        soc: "Freescale MPC85xx (PowerPC)",
        support: Support::Unsupported,
        note: "PowerPC. Debian 13 has no 32-bit PowerPC port.",
    },
    Row {
        token: "ppc824x",
        arch: Arch::Unsupported,
        soc: "Freescale MPC824x (PowerPC)",
        support: Support::Unsupported,
        note: "PowerPC. Debian 13 has no 32-bit PowerPC port.",
    },
    Row {
        token: "evansport",
        arch: Arch::Unsupported,
        soc: "Intel CE5335 (32-bit x86)",
        support: Support::Unsupported,
        note: "32-bit x86. Debian 13 dropped the i386 installer.",
    },
    Row {
        token: "88f6281",
        arch: Arch::Unsupported,
        soc: "Marvell Kirkwood (ARMv5)",
        support: Support::Unsupported,
        note: "ARMv5 (armel). Debian 13 armel is end-of-life and not supported by this tool.",
    },
    Row {
        token: "88f6282",
        arch: Arch::Unsupported,
        soc: "Marvell Kirkwood (ARMv5)",
        support: Support::Unsupported,
        note: "ARMv5 (armel). Debian 13 armel is end-of-life and not supported by this tool.",
    },
    Row {
        token: "88f5281",
        arch: Arch::Unsupported,
        soc: "Marvell Orion (ARMv5)",
        support: Support::Unsupported,
        note: "ARMv5 (armel). Debian 13 armel is end-of-life and not supported by this tool.",
    },
];

/// Extract the platform token from DSM's `unique` value.
/// `synology_apollolake_918+` -> `apollolake`; returns `None` for malformed values.
pub fn platform_token(unique: &str) -> Option<&str> {
    let rest = unique.trim().trim_matches('"').strip_prefix("synology_")?;
    let tok = rest.split('_').next()?;
    if tok.is_empty() { None } else { Some(tok) }
}

/// Look up a platform. Falls back to the CPU architecture reported by `uname -m`
/// when DSM's platform token is unknown.
pub fn classify(unique: &str, uname_m: &str) -> PlatformInfo {
    if let Some(tok) = platform_token(unique) {
        if let Some(r) = TABLE.iter().find(|r| r.token == tok) {
            return PlatformInfo { platform: tok.to_string(), arch: r.arch, soc: r.soc, support: r.support, note: r.note };
        }
        let arch = Arch::from_uname(uname_m);
        return fallback(tok, arch);
    }
    fallback("unknown", Arch::from_uname(uname_m))
}

fn fallback(tok: &str, arch: Arch) -> PlatformInfo {
    let (support, note) = match arch {
        Arch::X86_64 => (Support::Supported, "Unknown Synology platform, but the CPU is x86-64."),
        Arch::Arm64 | Arch::Armhf => (Support::Experimental, "Unknown ARM platform. Needs a serial console and a U-Boot boot recipe."),
        Arch::Unsupported => (Support::Unsupported, "CPU architecture is not supported by Debian 13."),
    };
    PlatformInfo { platform: tok.to_string(), arch, soc: "unknown", support, note }
}

/// Device tree blobs for Synology boards that ship in Debian's kernel (`linux-image-armmp`).
/// Anything else needs a custom DTB supplied by the user.
pub fn mainline_dtb(model_or_unique: &str) -> Option<&'static str> {
    let m = model_or_unique.to_ascii_lowercase();
    let m = m.rsplit('_').next().unwrap_or(&m).trim().to_string();
    match m.as_str() {
        "ds213j" => Some("armada-370-synology-ds213j.dtb"),
        "ds116" => Some("armada-385-synology-ds116.dtb"),
        "ds414" => Some("armada-xp-synology-ds414.dtb"),
        _ => None,
    }
}

/// Model name to show when only `unique` is known (`synology_apollolake_918+` -> `918+`).
pub fn model_from_unique(unique: &str) -> Option<String> {
    let rest = unique.trim().trim_matches('"').strip_prefix("synology_")?;
    let mut it = rest.splitn(2, '_');
    it.next()?;
    it.next().map(|m| m.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_extraction() {
        assert_eq!(platform_token("synology_apollolake_918+"), Some("apollolake"));
        assert_eq!(platform_token("\"synology_rtd1296_ds118\""), Some("rtd1296"));
        assert_eq!(platform_token("garbage"), None);
        assert_eq!(model_from_unique("synology_apollolake_918+").as_deref(), Some("918+"));
    }

    #[test]
    fn classification() {
        let p = classify("synology_apollolake_918+", "x86_64");
        assert_eq!(p.arch, Arch::X86_64);
        assert_eq!(p.support, Support::Supported);

        let p = classify("synology_rtd1296_ds118", "aarch64");
        assert_eq!(p.arch, Arch::Arm64);
        assert_eq!(p.support, Support::Experimental);

        let p = classify("synology_qoriq_rs812", "ppc");
        assert_eq!(p.support, Support::Unsupported);

        // Unknown platform: trust uname
        assert_eq!(classify("synology_newthing_x", "x86_64").support, Support::Supported);
        assert_eq!(classify("synology_newthing_x", "armv7l").arch, Arch::Armhf);
        assert_eq!(classify("", "i686").support, Support::Unsupported);
    }

    #[test]
    fn armv7_boards_are_supported_and_dtbs_known() {
        let p = classify("synology_armada38x_ds218j", "armv7l");
        assert_eq!((p.arch, p.support), (Arch::Armhf, Support::Supported));
        assert_eq!(mainline_dtb("synology_armada38x_ds116"), Some("armada-385-synology-ds116.dtb"));
        assert_eq!(mainline_dtb("DS213j"), Some("armada-370-synology-ds213j.dtb"));
        assert_eq!(mainline_dtb("DS218j"), None);
    }

    #[test]
    fn table_has_no_duplicate_tokens() {
        let mut seen = std::collections::HashSet::new();
        for r in TABLE {
            assert!(seen.insert(r.token), "duplicate {}", r.token);
        }
    }
}
