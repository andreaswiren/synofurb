//! SSH host-key handling for the commands that connect to a unit: `--host-key`, `--known-hosts`, `--accept-new-host-key`,
//! `--insecure-host-key`, and `host-key list|forget`.
//!
//! The rules (the engine side is `synofurb_core::hostkey`):
//! * `--host-key SHA256:...` (or `@FILE`): the unit must present exactly that key. Nothing is remembered.
//! * Otherwise the key is checked against the known-hosts file (`known_hosts.json` in the data directory, or `--known-hosts FILE`).
//!   A **changed** key is a hard error (exit 11) that names both fingerprints; nothing is sent.
//! * A **first** key is shown with its fingerprint and needs an answer: a question on a terminal, `--accept-new-host-key` for scripts.
//!   Without a terminal and without the flag the connection is refused (exit 11) and the fingerprint is printed, so it can be checked and
//!   passed back with `--host-key`.
//! * `--insecure-host-key` switches the check off, says so on every run, and exists for lab networks only.
//!
//! The first-contact question is asked *before* the worker thread that connects starts (the key is read with a bare handshake that sends
//! nothing), then pinned; the real connection verifies the same key against the pin, so nothing can be swapped in between.

use super::args::{HostKeyOpts, HostKeysCmd};
use super::exit::{CliError, CliResult};
use super::out::Out;
use super::prompt::Prompter;
use std::sync::Arc;
use std::time::Duration;
use synofurb_core::hostkey::{self, HostKeyError, HostKeyPolicy, KnownHosts};

pub fn store_for(o: &HostKeyOpts) -> KnownHosts {
    match &o.known_hosts {
        Some(p) => KnownHosts::at(p),
        None => KnownHosts::app_default(),
    }
}

/// Decide the host-key policy for connections to `host:port` and install it as the process-wide default. May ask the user once.
pub fn install_policy(host: &str, port: u16, o: &HostKeyOpts, out: &Out, prompt: &mut dyn Prompter) -> CliResult<()> {
    if o.insecure_host_key {
        out.warn(&format!(
            "HOST KEY CHECKING IS OFF (--insecure-host-key) for {host}:{port}. Anyone between this computer and the unit can read the SSH password and the commands. Use this only on a network you control."
        ));
        hostkey::set_default_policy(HostKeyPolicy::Insecure);
        return Ok(());
    }
    if let Some(arg) = &o.host_key {
        let fp = hostkey::pin_from_arg(arg).map_err(|e| CliError::usage(format!("--host-key: {e:#}")))?;
        hostkey::set_default_policy(HostKeyPolicy::Pinned(fp));
        return Ok(());
    }
    let store = store_for(o);
    let accept = o.accept_new_host_key;
    // the connection pins nothing by itself unless --accept-new-host-key was given
    hostkey::set_default_policy(HostKeyPolicy::tofu_with(store.clone(), Arc::new(move |_| accept)));
    if accept || store.get(host, port)?.is_some() {
        return Ok(());
    }
    // first contact: read the key without authenticating, show it, ask
    let Ok(fp) = synofurb_core::remote::fetch_host_fingerprint(host, port, Duration::from_secs(8)) else {
        return Ok(()); // unreachable: the connection itself reports that
    };
    let refused = || {
        CliError::HostKey(
            HostKeyError::NotTrusted { host: host.to_string(), port, fingerprint: fp.clone() }.to_string()
                + " (or pass --accept-new-host-key to pin it without asking)",
        )
    };
    if !out.interactive {
        return Err(refused());
    }
    out.say(&format!("The SSH host key of {host}:{port} is {fp}"));
    out.say("This is the first connection to this NAS from this computer. Check the fingerprint on the unit if you can (DSM: Control Panel > Terminal; or `ssh-keygen -l -f /etc/ssh/ssh_host_ed25519_key.pub`).");
    if prompt.confirm("Trust this NAS and remember its key?", false)? {
        store.pin(host, port, &fp)?;
        out.say("Pinned. A different key later is refused.");
        Ok(())
    } else {
        Err(refused())
    }
}

/// `host-key list|forget`
pub fn host_key(out: &Out, action: HostKeysCmd) -> CliResult<()> {
    match action {
        HostKeysCmd::List { known_hosts, json } => {
            let store = store_for(&HostKeyOpts { known_hosts, ..Default::default() });
            let rows = store.list()?;
            if json {
                out.json_doc(&serde_json::json!({
                    "schema": 1,
                    "file": store.path().display().to_string(),
                    "hosts": rows.iter().map(|(k, v)| serde_json::json!({"host": k, "fingerprint": v.fingerprint, "first_seen": v.first_seen})).collect::<Vec<_>>(),
                }));
            } else if rows.is_empty() {
                out.data_lines(vec![format!("No pinned host keys ({}).", store.path().display())]);
            } else {
                let mut v = vec![format!("Pinned host keys in {}:", store.path().display())];
                v.extend(rows.iter().map(|(k, h)| format!("  {k:<28} {}  (first seen {})", h.fingerprint, h.first_seen)));
                out.data_lines(v);
            }
            Ok(())
        }
        HostKeysCmd::Forget { host, port, known_hosts } => {
            let store = store_for(&HostKeyOpts { known_hosts, ..Default::default() });
            if store.forget(&host, port)? {
                out.result_text(&[format!("Forgot the pinned key of {host}:{port}. The next connection asks again.")]);
            } else {
                out.result_text(&[format!("No pinned key for {host}:{port} in {}.", store.path().display())]);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_argument_must_be_a_fingerprint() {
        let o = HostKeyOpts { host_key: Some("not-a-fingerprint".into()), ..Default::default() };
        let out = Out::plain();
        let mut p = super::super::prompt::Scripted::new(Vec::<String>::new());
        let e = install_policy("h", 22, &o, &out, &mut p).unwrap_err();
        assert_eq!(e.code(), 2, "{e}");
        assert!(e.to_string().contains("--host-key"));
    }
}
