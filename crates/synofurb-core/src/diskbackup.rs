//! Backup of the disk metadata that an install can destroy: the first and last MiB of every disk the plan touches
//! (MBR / primary and backup GPT, and everything stored in the first MiB), streamed to the user's computer.
//!
//! This is what makes a failed *partitioning* step recoverable by hand: writing these files back with `dd`
//! restores the partition tables. It does **not** back up file data or RAID members' contents, and in a wipe
//! install it cannot bring back a formatted filesystem. The README written next to the files says so.

use crate::logs::Logger;
use crate::remote::{Privileged, RemoteShell};
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SECTORS_PER_BLOCK: u64 = 2048; // 1 MiB of 512-byte sectors

/// Back up `disks` (kernel name, size in 512-byte sectors) into a new folder below `dest_root`.
pub fn backup_disk_metadata(
    sh: &mut dyn RemoteShell,
    is_root: bool,
    sudo_pw: Option<String>,
    disks: &[(String, u64)],
    dest_root: &Path,
    unit: &str,
    log: &Logger,
) -> Result<PathBuf> {
    if disks.is_empty() {
        bail!("no disks to back up");
    }
    let safe: String = unit.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    let dir = dest_root.join(format!("disk-metadata-{safe}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    std::fs::create_dir_all(&dir)?;
    let mut sums = String::new();
    let mut restore = String::from(
        "Disk metadata backup made by synofurb before the install.\n\
         It contains the FIRST and LAST 1 MiB of each disk (MBR / GPT headers and tables). It does NOT contain your files.\n\
         If an install stopped while partitioning, boot any Linux that sees the disks and write the matching files back:\n\n",
    );
    let mut p = Privileged { sh, is_root, sudo_password: sudo_pw };
    for (name, sectors) in disks {
        if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
            bail!("unsafe disk name '{name}'");
        }
        if *sectors < SECTORS_PER_BLOCK * 4 {
            bail!("disk {name} is too small to back up its ends");
        }
        for (label, skip) in [("head", 0u64), ("tail", sectors - SECTORS_PER_BLOCK)] {
            let path = dir.join(format!("{name}.{label}.bin"));
            let mut w = HashingFile { f: std::fs::File::create(&path)?, h: Sha256::new(), n: 0 };
            let cmd = format!("dd if=/dev/{name} bs=512 skip={skip} count={SECTORS_PER_BLOCK} 2>/dev/null");
            let (code, err) = p.run_to(&cmd, &mut w)?;
            w.flush()?;
            if code != 0 || w.n != SECTORS_PER_BLOCK * 512 {
                bail!("backing up the {label} of {name} failed (exit {code}, {} bytes): {}", w.n, err.trim());
            }
            sums.push_str(&format!("{}  {name}.{label}.bin\n", hex::encode(w.h.finalize())));
            restore.push_str(&format!(
                "  dd if={name}.{label}.bin of=/dev/{name} bs=512 seek={skip} conv=notrunc    # check the target disk first!\n"
            ));
        }
        log.info(format!("backed up the first and last MiB of {name}"));
    }
    restore.push_str("\nAfterwards run `partprobe` or reboot. Verify checksums with: sha256sum -c SHA256SUMS\n");
    std::fs::write(dir.join("SHA256SUMS"), sums)?;
    std::fs::write(dir.join("README.txt"), restore)?;
    log.info(format!("disk metadata backup written to {}", dir.display()));
    Ok(dir)
}

struct HashingFile {
    f: std::fs::File,
    h: Sha256,
    n: u64,
}
impl Write for HashingFile {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.f.write(b)?;
        self.h.update(&b[..n]);
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.f.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::mock::MockShell;

    #[test]
    fn writes_head_and_tail_with_checksums_and_an_honest_readme() {
        let root = std::env::temp_dir().join(format!("sf-meta-{}", std::process::id()));
        let mut m = MockShell::default();
        let block = "x".repeat(1024 * 1024);
        for _ in 0..4 {
            m.push_ok(&block);
        }
        let dir = backup_disk_metadata(
            &mut m,
            true,
            None,
            &[("sda".into(), 7_814_037_168), ("sdb".into(), 7_814_037_168)],
            &root,
            "DS918+",
            &Logger::memory(None),
        )
        .unwrap();
        for f in ["sda.head.bin", "sda.tail.bin", "sdb.head.bin", "sdb.tail.bin", "SHA256SUMS", "README.txt"] {
            assert!(dir.join(f).exists(), "{f}");
        }
        assert_eq!(std::fs::metadata(dir.join("sda.tail.bin")).unwrap().len(), 1024 * 1024);
        let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap();
        assert!(readme.contains("does NOT contain your files") && readme.contains("seek=7814035120"));
        assert!(m.commands[1].contains("skip=7814035120"), "tail starts 2048 sectors before the end");
    }

    #[test]
    fn short_read_or_bad_names_abort_the_install() {
        let root = std::env::temp_dir().join(format!("sf-meta2-{}", std::process::id()));
        let mut m = MockShell::default();
        m.push_ok("too short");
        assert!(backup_disk_metadata(&mut m, true, None, &[("sda".into(), 1 << 30)], &root, "x", &Logger::memory(None)).is_err());
        assert!(
            backup_disk_metadata(&mut MockShell::default(), true, None, &[("sda;rm".into(), 1 << 30)], &root, "x", &Logger::memory(None))
                .is_err()
        );
        assert!(backup_disk_metadata(&mut MockShell::default(), true, None, &[], &root, "x", &Logger::memory(None)).is_err());
    }
}
