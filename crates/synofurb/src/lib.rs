//! Everything synofurb's two programs share: the command line (`cli`), the self-updater front end and the offline-bundle commands (`bundle_cmd`).
//!
//! The GPUI desktop UI is NOT part of this library: it lives in `src/ui` and is compiled into the `synofurb` binary only
//! (cargo feature `gui`). That is what lets `synofurb-cli` run on a headless server without any GUI system library.

pub mod cli;
pub mod update_cmd;

pub mod bundle_cmd;
