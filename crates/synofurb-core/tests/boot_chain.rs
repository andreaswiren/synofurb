//! The boot chain of a DOM unit as far as it can be checked without the unit: which kernel modules the system disks need (found through a
//! stand-in sysfs shaped like the owner's DS918+), whether the initramfs check adds what is missing, and what the kernel hook does when
//! the boot flash refuses a kernel. Real shell, stub tools; nothing here needs root or Linux-specific devices.

use std::path::{Path, PathBuf};
use std::process::Command;

const CHROOT_SETUP: &str = include_str!("../../../payload/chroot-setup.sh");
const DOM_UPDATE: &str = include_str!("../../../payload/files/usr/local/sbin/synofurb-dom-update");

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("sf-bootchain-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn put(path: &Path, body: &str, exec: bool) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    if exec {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
fn link(target: &str, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    if at.symlink_metadata().is_err() {
        std::os::unix::fs::symlink(target, at).unwrap();
    }
}

/// The functions of chroot-setup.sh that deal with the initramfs, as a sourceable snippet (everything from `SYSFS=` to the end of
/// `ensure_initrd_storage`), with `say` defined.
fn initrd_functions() -> String {
    let start = CHROOT_SETUP.find("# (SYSFS, BOOT_DIR and IRFS_MODULES").expect("helper block present");
    let end_marker =
        "  say \"initramfs of $_kv carries the storage drivers of the system disks:${_need:- (all built into the kernel)}\"\n}\n";
    let end = CHROOT_SETUP[start..].find(end_marker).expect("end of ensure_initrd_storage") + start + end_marker.len();
    format!("say() {{ echo \"[chroot] $*\"; }}\n{}", &CHROOT_SETUP[start..end])
}

/// A sysfs shaped like the DS918+ (hardware facts from the unit): sda/sdb on the add-in AHCI controller 0000:01:00.0 behind the root port
/// 0000:00:13.0 (ata3/ata4), sdc/sdd on the SoC AHCI controller 0000:00:12.0 (ata1/ata2), the DOM as a USB mass-storage device behind
/// the xHCI controller 0000:00:15.0. Drivers are linked the way the kernel does it: `driver` -> bus/<bus>/drivers/<name>, whose `module`
/// -> module/<module>. The root port driver (`pcieport`) is built in and has no module link.
#[cfg(unix)]
fn ds918_sysfs(root: &Path) {
    let dev = root.join("devices/pci0000:00");
    let drivers = |bus: &str, name: &str, module: Option<&str>| {
        std::fs::create_dir_all(root.join(format!("bus/{bus}/drivers/{name}"))).unwrap();
        if let Some(m) = module {
            std::fs::create_dir_all(root.join(format!("module/{m}"))).unwrap();
            link(&format!("../../../../module/{m}"), &root.join(format!("bus/{bus}/drivers/{name}/module")));
        }
    };
    drivers("pci", "ahci", Some("ahci"));
    drivers("pci", "pcieport", None);
    drivers("pci", "xhci_hcd", Some("xhci_pci"));
    drivers("scsi", "sd", Some("sd_mod"));
    drivers("usb", "usb-storage", Some("usb_storage"));
    let to = |dir: &Path, bus: &str, name: &str| {
        let depth = dir.strip_prefix(root).unwrap().components().count();
        link(&format!("{}bus/{bus}/drivers/{name}", "../".repeat(depth)), &dir.join("driver"));
    };
    // (disk, controller path below pci0000:00, ata port, scsi address)
    let disks = [
        ("sda", "0000:00:13.0/0000:01:00.0", "ata3", "2:0:0:0"),
        ("sdb", "0000:00:13.0/0000:01:00.0", "ata4", "3:0:0:0"),
        ("sdc", "0000:00:12.0", "ata1", "0:0:0:0"),
        ("sdd", "0000:00:12.0", "ata2", "1:0:0:0"),
    ];
    for (name, ctrl, ata, scsi) in disks {
        let host = scsi.split(':').next().unwrap();
        let ctrl_dir = dev.join(ctrl);
        let scsi_dir = ctrl_dir.join(format!("{ata}/host{host}/target{host}:0:0/{scsi}"));
        std::fs::create_dir_all(&scsi_dir).unwrap();
        to(&scsi_dir, "scsi", "sd");
        to(&ctrl_dir, "pci", "ahci");
        if ctrl.contains('/') {
            to(&dev.join("0000:00:13.0"), "pci", "pcieport");
        }
        link(
            &format!("../../devices/pci0000:00/{ctrl}/{ata}/host{host}/target{host}:0:0/{scsi}"),
            &root.join(format!("block/{name}/device")),
        );
    }
    // the DOM: usb1/1-1/1-1:1.0/host4/target4:0:0/4:0:0:0 behind 0000:00:15.0 (xhci)
    let xhci = dev.join("0000:00:15.0");
    let scsi_dir = xhci.join("usb1/1-1/1-1:1.0/host4/target4:0:0/4:0:0:0");
    std::fs::create_dir_all(&scsi_dir).unwrap();
    to(&scsi_dir, "scsi", "sd");
    to(&xhci, "pci", "xhci_hcd");
    to(&xhci.join("usb1/1-1/1-1:1.0"), "usb", "usb-storage");
    link("../../devices/pci0000:00/0000:00:15.0/usb1/1-1/1-1:1.0/host4/target4:0:0/4:0:0:0", &root.join("block/synoboot/device"));
}

fn run_sh(dir: &Path, script: &str, env: &[(&str, &str)]) -> (i32, String) {
    let f = dir.join("run.sh");
    put(&f, script, false);
    let mut c = Command::new("sh");
    c.arg(&f).env("PATH", format!("{}:/usr/bin:/bin", dir.join("stubs").display())).env("DIR", dir);
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().unwrap();
    (out.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[cfg(unix)]
#[test]
fn the_system_disks_of_the_ds918_resolve_to_ahci_and_sd_mod_whichever_controller_they_hang_off() {
    let d = tmp("sysfs");
    let sys = d.join("sys");
    ds918_sysfs(&sys);
    let script = format!(
        "{}\nfor x in sda sdb sdc sdd /dev/sda synoboot; do echo \"$x: $(disk_driver_modules $x | tr '\\n' ' ')\"; done\n",
        initrd_functions()
    );
    let (code, out) = run_sh(&d, &script, &[("SYSFS", sys.to_str().unwrap())]);
    assert_eq!(code, 0, "{out}");
    // sda/sdb: the add-in controller behind the 00:13.0 root port; sdc/sdd: the SoC controller. Same driver, and the built-in pcieport adds nothing.
    for disk in ["sda", "sdb", "sdc", "sdd", "/dev/sda"] {
        assert!(out.contains(&format!("{disk}: sd_mod ahci \n")), "{disk}:\n{out}");
    }
    // the DOM itself is not a system disk, but the same walk finds the USB path (useful in the logs)
    assert!(out.contains("synoboot: sd_mod usb_storage xhci_pci \n"), "{out}");
}

/// Stub `modinfo`/`lsinitramfs`/`update-initramfs` around a fake /boot. `have` is the module list the first initramfs contains.
#[cfg(unix)]
fn initrd_env(tag: &str, have: &[&str], builtin: &[&str], rebuild_adds: bool) -> PathBuf {
    let d = tmp(tag);
    ds918_sysfs(&d.join("sys"));
    put(&d.join("boot/vmlinuz-6.12.107+deb13-amd64"), "k", false);
    put(&d.join("boot/vmlinuz-6.1.0-old"), "k", false);
    put(&d.join("boot/initrd.img-6.12.107+deb13-amd64"), "i", false);
    put(&d.join("irfs-modules"), "", false);
    let listing: String = have.iter().map(|m| format!("usr/lib/modules/6.12.107+deb13-amd64/kernel/drivers/x/{m}.ko.xz\n")).collect();
    put(&d.join("initrd.list"), &listing, false);
    put(&d.join("builtin"), &builtin.join("\n"), false);
    // modinfo -k KV -F filename MOD : a path, or (builtin) for built-in ones
    put(
        &d.join("stubs/modinfo"),
        "#!/bin/sh\nm=$5\nif grep -qx \"$m\" \"$DIR/builtin\"; then echo '(builtin)'; else echo \"/lib/modules/$2/kernel/$m.ko.xz\"; fi\n",
        true,
    );
    put(&d.join("stubs/lsinitramfs"), "#!/bin/sh\ncat \"$DIR/initrd.list\"\n", true);
    // update-initramfs -u -k KV : the "rebuilt" initramfs lists whatever was added to the modules file
    let add = if rebuild_adds {
        "while read -r m; do [ -n \"$m\" ] && echo \"usr/lib/modules/x/kernel/drivers/x/$m.ko.xz\" >> \"$DIR/initrd.list\"; done < \"$DIR/irfs-modules\"\n"
    } else {
        ":\n"
    };
    put(&d.join("stubs/update-initramfs"), &format!("#!/bin/sh\necho \"update-initramfs $*\" >> \"$DIR/calls\"\n{add}"), true);
    d
}

#[cfg(unix)]
fn run_ensure(d: &Path) -> (i32, String) {
    let script = format!("set -eu\nSYSTEM_DISKS=\"/dev/sda /dev/sdb /dev/sdc /dev/sdd\"\n{}\nensure_initrd_storage\n", initrd_functions());
    run_sh(
        d,
        &script,
        &[
            ("SYSFS", d.join("sys").to_str().unwrap()),
            ("BOOT_DIR", d.join("boot").to_str().unwrap()),
            ("IRFS_MODULES", d.join("irfs-modules").to_str().unwrap()),
        ],
    )
}

#[cfg(unix)]
#[test]
fn an_initramfs_with_the_controller_driver_passes_and_is_not_rebuilt() {
    let d = initrd_env("ok", &["ahci", "libahci", "libata", "sd_mod", "raid1"], &[], false);
    let (code, out) = run_ensure(&d);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("carries the storage drivers of the system disks: sd_mod ahci"), "{out}");
    assert!(!d.join("calls").exists(), "no rebuild");
}

#[cfg(unix)]
#[test]
fn a_missing_controller_driver_is_added_and_the_initramfs_rebuilt_for_the_newest_kernel_only() {
    let d = initrd_env("add", &["sd_mod", "raid1", "xfs"], &[], true);
    let (code, out) = run_ensure(&d);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("lacks driver(s) of the system disks: ahci"), "{out}");
    assert_eq!(std::fs::read_to_string(d.join("irfs-modules")).unwrap().trim(), "ahci");
    assert_eq!(
        std::fs::read_to_string(d.join("calls")).unwrap().trim(),
        "update-initramfs -u -k 6.12.107+deb13-amd64",
        "newest kernel by version sort"
    );
}

#[cfg(unix)]
#[test]
fn a_driver_that_cannot_be_put_into_the_initramfs_stops_the_install_instead_of_shipping_an_unbootable_system() {
    let d = initrd_env("fail", &["sd_mod"], &[], false);
    let (code, out) = run_ensure(&d);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("ERROR: the initramfs of 6.12.107+deb13-amd64 still lacks the storage driver(s) ahci"), "{out}");
}

#[cfg(unix)]
#[test]
fn drivers_built_into_the_kernel_are_not_demanded_from_the_initramfs() {
    let d = initrd_env("builtin", &["raid1"], &["ahci", "sd_mod"], false);
    let (code, out) = run_ensure(&d);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("all built into the kernel"), "{out}");
}

#[test]
fn the_initramfs_policy_for_boot_flash_units_is_small_and_complete() {
    // the static list names the add-in controller families and the USB path; xz must be installed before the first kernel
    for m in ["ahci", "libahci", "sata_mv", "sd_mod", "xhci_pci", "usb_storage", "raid1", "raid456", "dm_mod", "xfs"] {
        assert!(CHROOT_SETUP.split_whitespace().any(|w| w == m || w == format!("{m};")), "module {m} is in the list");
    }
    assert!(CHROOT_SETUP.contains("MODULES=list\\nCOMPRESS=xz"));
    let xz =
        CHROOT_SETUP.find("apt_install initramfs-tools xz-utils busybox").expect("xz-utils and busybox installed with initramfs-tools");
    assert!(xz < CHROOT_SETUP.find("installing the kernel (").unwrap(), "before any kernel is installed");
    // the sanity check runs after every initramfs build that matters
    assert_eq!(CHROOT_SETUP.matches("\n    ensure_initrd_storage\n").count(), 3, "BIOS, UEFI and boot flash branches");
}

// ----------------------------------------------------------------------------------------------------- the kernel hook

#[cfg(unix)]
fn hook_env(tag: &str, tool_body: &str) -> PathBuf {
    let d = tmp(tag);
    put(
        &d.join("dom.conf"),
        "ROOT_UUID=\"11111111-2222-3333-4444-555555555555\"\nSERIAL_ARG=\"console=ttyS2,115200n8\"\nEXTRA_ARGS=\"ro rootdelay=5 panic=60\"\n",
        false,
    );
    put(&d.join("boot/vmlinuz-7.2.11+deb13-amd64"), "k", false);
    put(&d.join("boot/initrd.img-7.2.11+deb13-amd64"), "i", false);
    put(&d.join("tool"), &format!("#!/bin/sh\necho \"$@\" >> \"$DIR/calls\"\n{tool_body}\n"), true);
    put(&d.join("dom-update"), DOM_UPDATE, true);
    d
}

#[cfg(unix)]
fn run_hook(d: &Path, args: &str) -> (i32, String) {
    let script = format!("sh \"$DIR/dom-update\" {args}\n");
    run_sh(
        d,
        &script,
        &[
            ("SF_DOM_CONF", d.join("dom.conf").to_str().unwrap()),
            ("SF_BOOT_DIR", d.join("boot").to_str().unwrap()),
            ("SF_DOM_TOOL", d.join("tool").to_str().unwrap()),
            ("SF_DOM_STATE", d.join("state/failed").to_str().unwrap()),
        ],
    )
}

#[cfg(unix)]
#[test]
fn the_kernel_hook_registers_the_newest_kernel_with_root_uuid_and_the_serial_console_last() {
    let d = hook_env("hook-ok", "exit 0");
    std::fs::write(d.join("state-marker"), "").unwrap();
    let (code, out) = run_hook(&d, "--hook");
    assert_eq!(code, 0, "{out}");
    let calls = std::fs::read_to_string(d.join("calls")).unwrap();
    assert!(calls.starts_with("install-debian "), "{calls}");
    assert!(calls.contains("vmlinuz-7.2.11+deb13-amd64") && calls.contains("initrd.img-7.2.11+deb13-amd64"), "{calls}");
    assert!(
        calls.contains(
            "--cmdline root=UUID=11111111-2222-3333-4444-555555555555 ro rootdelay=5 panic=60 console=tty0 console=ttyS2,115200n8 --yes"
        ),
        "the serial console is the last console= (the kernel's /dev/console): {calls}"
    );
    assert!(!d.join("state/failed").exists());
}

#[cfg(unix)]
#[test]
fn a_refused_kernel_is_reported_loudly_but_does_not_fail_the_package_hook() {
    let d = hook_env("hook-refused", "echo 'not enough space on partition 2: need 29 MiB, have 3 MiB. Nothing was written.'; exit 1");
    let (code, out) = run_hook(&d, "--hook");
    assert_eq!(code, 0, "a failing hook would leave linux-image half-configured: {out}");
    assert!(out.contains("was NOT registered on the boot flash") && out.contains("not enough space on partition 2"), "{out}");
    let state = std::fs::read_to_string(d.join("state/failed")).unwrap();
    assert!(state.contains("NOT registered") && state.contains("have 3 MiB"), "{state}");
    // by hand the same refusal is an error
    let (code, _) = run_hook(&d, "");
    assert_eq!(code, 1);
    // a later success clears the marker
    put(&d.join("tool"), "#!/bin/sh\nexit 0\n", true);
    let (code, _) = run_hook(&d, "--hook");
    assert_eq!(code, 0);
    assert!(!d.join("state/failed").exists(), "marker cleared once the flash accepts the kernel");
}

// ------------------------------------------------------------------------------------------------- what the system needs to boot

#[test]
fn the_installed_system_gets_an_init_because_debootstraps_minbase_has_none() {
    // `debootstrap --variant=minbase --include=ca-certificates,locales,dbus trixie` installs init-system-helpers, libsystemd0, sysvinit-utils
    // ... but no `systemd`, no `systemd-sysv` and no `udev` (checked with `debootstrap --print-debs`, 209 packages). /sbin/init comes
    // from systemd-sysv: without it the kernel starts nothing and the initramfs ends in "run-init: can't execute '/sbin/init'".
    let tools = CHROOT_SETUP.split("apt_install $BOOTPKG").nth(1).expect("the big package install");
    let list = &tools[..tools.find("$EXTRA $EXTRA_PACKAGES").expect("end of the list")];
    for pkg in ["systemd-sysv", "libpam-systemd", "mdadm", "lvm2", "xfsprogs", "openssh-server", "iproute2", "systemd-timesyncd"] {
        assert!(list.split_whitespace().any(|w| w == pkg), "{pkg} is installed");
    }
    // and the offline bundle fetches exactly what the script installs
    for pkg in ["systemd-sysv", "libpam-systemd"] {
        assert!(synofurb_core::offline::CHROOT_TOOLS.contains(&pkg), "{pkg} is in the offline bundle's package seed");
    }
    for pkg in ["xz-utils", "busybox"] {
        assert!(synofurb_core::offline::CHROOT_INITRAMFS.contains(&pkg), "{pkg} is in the offline bundle's package seed");
    }
}

#[test]
fn the_cli_smoke_test_feeds_the_same_ds918_facts_as_the_fixture() {
    // tests/cli/ds918-real.inspect.txt is `cargo run -p synofurb-core --example dump_fixture`; the smoke test (and a human with
    // `synofurb plan --facts tests/cli/ds918-real.inspect.txt`) plans for exactly the facts the unit tests use
    let file = include_str!("../../../tests/cli/ds918-real.inspect.txt");
    assert_eq!(file, synofurb_core::inspect::fixtures::ds918_two_volumes(), "regenerate the file with the dump_fixture example");
    let f = synofurb_core::inspect::parse_inspect(file).unwrap();
    assert_eq!(f.firmware, synofurb_core::model::Firmware::Dom, "the real unit boots through its internal flash");
    assert_eq!(f.serial_console().kernel_arg(), "console=ttyS2,115200n8");
    assert_eq!(f.nics.len(), 2, "two Intel i211 ports");
    assert_eq!(f.candidate_disks().len(), 4);
}
