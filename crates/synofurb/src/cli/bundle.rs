//! `bundle create|verify|info`: front end over `crate::bundle_cmd` (the offline-mode engine in `synofurb_core::offline`), plus the checks
//! `install` and the wizard run on a bundle before anything touches the NAS.

use super::args::{BundleCmd, BundleCreateArgs, KernelArg};
use super::exit::{CliError, CliResult};
use super::out::Out;
use crate::bundle_cmd as backend;
use std::path::Path;
use synofurb_core::config::KernelChannel;
use synofurb_core::model::Arch;
use synofurb_core::offline::Bundle;

fn validate_create(a: &BundleCreateArgs) -> CliResult<()> {
    for arch in &a.archs {
        if !["amd64", "arm64", "armhf", "all"].contains(&arch.trim().to_ascii_lowercase().as_str()) {
            return Err(CliError::usage(format!("unknown architecture '{arch}' (amd64, arm64, armhf, all)")));
        }
    }
    if a.suite.is_empty() || !a.suite.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(CliError::usage("invalid --suite"));
    }
    for p in &a.extra {
        if !synofurb_core::config::valid_package_name(p) {
            return Err(CliError::usage(format!("invalid package name '{p}'")));
        }
    }
    Ok(())
}

/// Does `bundle create` build for the Newest kernel? The default is yes, the same as the install default, so a plain `bundle create`
/// followed by a plain `install --bundle` fits. `--kernel stable` opts out; the old `--kernel-newest` still means newest.
pub fn wants_newest(a: &BundleCreateArgs) -> bool {
    a.kernel_newest || a.kernel == KernelArg::Newest
}

pub fn bundle(_out: &Out, action: BundleCmd) -> CliResult<()> {
    match action {
        BundleCmd::Create(a) => {
            validate_create(&a)?;
            let newest = wants_newest(&a);
            backend::create(a.archs, a.suite, newest, a.no_firmware, a.extra, a.out, a.no_agent)?;
        }
        BundleCmd::Verify { dir } => backend::verify(dir)?,
        BundleCmd::Info { dir, json } => backend::info(dir, json)?,
    }
    Ok(())
}

/// First look at `--bundle DIR`, before anything is contacted: it must be a finished bundle of a format this program understands.
/// (No hashing here: `bundle verify` does that, and the engine checks the installer files it serves.) A bad bundle is a usage error (exit 2).
pub fn open_bundle(dir: &Path) -> CliResult<Bundle> {
    Bundle::open(dir).map_err(|e| CliError::usage(format!("--bundle {}: {e:#}", dir.display())))
}

/// Does this bundle fit the install that is about to run (architecture, suite, kernel channel, extra packages, firmware)? Returns the
/// engine's warnings. A mismatch is a usage error (exit 2): the answer is another flag or another bundle, and nothing was changed.
pub fn check_for_install(
    dir: &Path,
    arch: Arch,
    suite: &str,
    kernel: KernelChannel,
    extra: &[String],
    want_firmware: bool,
) -> CliResult<Vec<String>> {
    let b = open_bundle(dir)?;
    b.check_for_install(arch, suite, kernel, extra, want_firmware)
        .map_err(|e| CliError::usage(format!("--bundle {}: {e:#}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(archs: &[&str], suite: &str) -> BundleCreateArgs {
        BundleCreateArgs {
            archs: archs.iter().map(|s| s.to_string()).collect(),
            suite: suite.into(),
            kernel: KernelArg::Newest,
            kernel_newest: false,
            no_firmware: false,
            extra: vec![],
            out: "o".into(),
            no_agent: false,
        }
    }

    #[test]
    fn a_plain_bundle_create_builds_what_a_plain_install_asks_for() {
        let mut a = args(&["amd64"], "trixie");
        assert!(wants_newest(&a), "default: newest, like `install --kernel newest`");
        a.kernel = KernelArg::Stable;
        assert!(!wants_newest(&a));
        a.kernel_newest = true; // the old spelling
        assert!(wants_newest(&a));
    }

    #[test]
    fn create_arguments_are_validated_before_the_engine_runs() {
        assert!(validate_create(&args(&["amd64", "arm64"], "trixie")).is_ok());
        assert!(validate_create(&args(&["all"], "trixie")).is_ok() && validate_create(&args(&[], "trixie")).is_ok());
        assert_eq!(validate_create(&args(&["riscv64"], "trixie")).err().unwrap().code(), 2);
        assert_eq!(validate_create(&args(&["amd64"], "Tri xie")).err().unwrap().code(), 2);
        let mut a = args(&["amd64"], "trixie");
        a.extra = vec!["x;rm".into()];
        assert_eq!(validate_create(&a).err().unwrap().code(), 2);
    }

    #[test]
    fn a_missing_or_foreign_directory_is_a_usage_error() {
        let e = open_bundle(Path::new("/definitely/not/a/bundle")).err().unwrap();
        assert_eq!(e.code(), 2);
        assert!(e.to_string().contains("--bundle"), "{e}");
        let tmp = std::env::temp_dir();
        assert_eq!(check_for_install(&tmp, Arch::X86_64, "trixie", KernelChannel::Newest, &[], true).err().unwrap().code(), 2);
    }
}
