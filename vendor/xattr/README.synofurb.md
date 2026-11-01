# Local patch of xattr 0.2.3

Why: GPUI (via `zed-async-tar`) depends on `xattr` 0.2.3, which refers to `libc::ENOATTR` on Linux. Recent `libc`
releases no longer define it there, so the crate fails to compile on Linux. The only change is in `src/sys/mod.rs`:
on Linux/Android `ENOATTR` is `ENODATA` (which is what it always was). Remove this patch (and the `[patch.crates-io]`
entry in the root `Cargo.toml`) as soon as upstream `zed-async-tar`/`xattr` builds on Linux without it.
Original license: MIT OR Apache-2.0 (see LICENSE files).
