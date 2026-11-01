//! Build metadata for `version` (target triple, profile, commit).
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SYNOFURB_GIT_DESCRIBE");

    println!("cargo:rustc-env=SYNOFURB_TARGET={}", std::env::var("TARGET").unwrap_or_default());
    println!("cargo:rustc-env=SYNOFURB_PROFILE={}", std::env::var("PROFILE").unwrap_or_default());

    // The commit the binary was built from. No `git` program is run: HEAD and the ref files are read directly (CI can set
    // SYNOFURB_GIT_DESCRIBE to the output of `git describe --tags --always --dirty` for something richer).
    let describe = std::env::var("SYNOFURB_GIT_DESCRIBE").ok().filter(|s| !s.is_empty()).or_else(|| short_commit(Path::new("../../.git")));
    println!("cargo:rustc-env=SYNOFURB_GIT_DESCRIBE_BUILD={}", describe.unwrap_or_default());
}

/// Short commit id from `.git/HEAD` (following a branch ref, loose or packed). `None` outside a checkout.
fn short_commit(git: &Path) -> Option<String> {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    let full = match head.strip_prefix("ref: ") {
        Some(r) => {
            println!("cargo:rerun-if-changed=../../.git/{r}");
            std::fs::read_to_string(git.join(r)).ok().map(|s| s.trim().to_string()).or_else(|| {
                let packed = std::fs::read_to_string(git.join("packed-refs")).ok()?;
                packed.lines().find_map(|l| l.strip_suffix(r).map(|h| h.trim().to_string()))
            })?
        }
        None => head.to_string(),
    };
    (full.len() >= 7 && full.chars().all(|c| c.is_ascii_hexdigit())).then(|| full[..7].to_string())
}
