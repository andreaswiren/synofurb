//! Remote execution on the NAS over SSH (russh), behind the [`RemoteShell`] trait so the install
//! flow can be tested with a mock. Every command and its output is written to the debug log.

use crate::hostkey::{HostKeyDecision, HostKeyError, HostKeyPolicy};
use crate::logs::{Level, Logger, Source};
use anyhow::{Context, Result, anyhow, bail};
use russh::client;
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect};
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ExecResult {
    pub code: u32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecResult {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

pub trait RemoteShell {
    /// Run a command; optional stdin is written then closed.
    fn exec(&mut self, cmd: &str, stdin: Option<&[u8]>) -> Result<ExecResult>;
    /// Run a command and stream its raw stdout (binary safe) into `out`; returns (exit code, stderr).
    fn exec_to(&mut self, cmd: &str, stdin: Option<&[u8]>, out: &mut dyn std::io::Write) -> Result<(u32, String)>;
    /// Stream `size` bytes to `remote_path` (mode applied afterwards).
    fn upload(&mut self, remote_path: &str, data: &mut dyn Read, size: u64, mode: u32, progress: &mut dyn FnMut(u64)) -> Result<()>;
    fn host_key_fingerprint(&self) -> Option<String>;
}

#[derive(Clone)]
pub enum Auth {
    Password(String),
    /// OpenSSH private key (PEM text)
    Key(String),
}

/// Client handler. `check_server_key` runs during the handshake, before authentication: when the policy refuses, `Ok(false)` makes
/// russh close the connection and the password / key signature is never produced.
struct Handler {
    fingerprint: Arc<Mutex<Option<String>>>,
    policy: HostKeyPolicy,
    host: String,
    port: u16,
    /// Why the key was refused (russh itself only says "unknown server key").
    refused: Arc<Mutex<Option<HostKeyError>>>,
    /// Set when this connection pinned a first-time key (for the log line).
    pinned: Arc<Mutex<bool>>,
}

impl Handler {
    fn new(host: &str, port: u16, policy: &HostKeyPolicy) -> Handler {
        Handler {
            fingerprint: Arc::new(Mutex::new(None)),
            policy: policy.clone(),
            host: host.to_string(),
            port,
            refused: Arc::new(Mutex::new(None)),
            pinned: Arc::new(Mutex::new(false)),
        }
    }
}

impl client::Handler for Handler {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
            PublicKeyOrCertificate::Certificate(_) => "certificate".to_string(),
        };
        *self.fingerprint.lock().unwrap() = Some(fp.clone());
        match self.policy.check(&self.host, self.port, &fp) {
            Ok(HostKeyDecision::Accept) => Ok(true),
            Ok(HostKeyDecision::AcceptedAndPinned) => {
                *self.pinned.lock().unwrap() = true;
                Ok(true)
            }
            Err(e) => {
                *self.refused.lock().unwrap() = Some(e);
                Ok(false)
            }
        }
    }
}

/// Turn a failed connect into the typed host-key error when that is what happened.
fn connect_error(refused: &Arc<Mutex<Option<HostKeyError>>>, e: anyhow::Error) -> anyhow::Error {
    match refused.lock().unwrap().take() {
        Some(h) => anyhow::Error::new(h),
        None => e,
    }
}

pub struct SshSession {
    rt: tokio::runtime::Runtime,
    handle: client::Handle<Handler>,
    fingerprint: Arc<Mutex<Option<String>>>,
    log: Logger,
    /// Redact the sudo password from logged commands.
    pub label: String,
}

/// Fetch just the SSH host key fingerprint (used after the install to show the user what to expect).
pub fn fetch_host_fingerprint(host: &str, port: u16, timeout: Duration) -> Result<String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    rt.block_on(async {
        // informational only: nothing is ever sent after this handshake, so no policy applies
        let handler = Handler::new(host, port, &HostKeyPolicy::Insecure);
        let fp = handler.fingerprint.clone();
        let cfg = Arc::new(client::Config { inactivity_timeout: Some(timeout), ..Default::default() });
        let h = tokio::time::timeout(timeout, client::connect(cfg, (host, port), handler)).await.map_err(|_| anyhow!("timed out"))??;
        let _ = h.disconnect(Disconnect::ByApplication, "", "en").await;
        let v = fp.lock().unwrap().clone();
        v.ok_or_else(|| anyhow!("server sent no host key"))
    })
}

impl SshSession {
    /// Connect with the process-wide host-key policy ([`crate::hostkey::default_policy`]: trust on first use, a changed key is an error).
    pub fn connect(host: &str, port: u16, user: &str, auth: &Auth, log: &Logger) -> Result<SshSession> {
        Self::connect_with(host, port, user, auth, log, &crate::hostkey::default_policy())
    }

    /// Connect, check the server's host key with `policy` **before** authenticating, then authenticate. A refused key is a
    /// [`HostKeyError`] inside the returned error (`err.downcast_ref::<HostKeyError>()`); nothing secret was sent in that case.
    pub fn connect_with(host: &str, port: u16, user: &str, auth: &Auth, log: &Logger, policy: &HostKeyPolicy) -> Result<SshSession> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let handler = Handler::new(host, port, policy);
        let (fingerprint, refused, pinned) = (handler.fingerprint.clone(), handler.refused.clone(), handler.pinned.clone());
        if matches!(policy, HostKeyPolicy::Insecure) {
            log.warn(format!("HOST KEY NOT CHECKED for {host}:{port} (insecure mode): anyone on the network path could read the password"));
        }
        let handle = rt
            .block_on(async {
                let cfg = Arc::new(client::Config {
                    inactivity_timeout: Some(Duration::from_secs(3600)),
                    keepalive_interval: Some(Duration::from_secs(20)),
                    ..Default::default()
                });
                let mut h = tokio::time::timeout(Duration::from_secs(20), client::connect(cfg, (host, port), handler))
                    .await
                    .map_err(|_| anyhow!("connection to {host}:{port} timed out"))?
                    .with_context(|| format!("connecting to {host}:{port}"))?;
                let ok = match auth {
                    Auth::Password(p) => h.authenticate_password(user, p.clone()).await?.success(),
                    Auth::Key(pem) => {
                        let key = russh::keys::PrivateKey::from_openssh(pem).context("invalid private key")?;
                        let hash = h.best_supported_rsa_hash().await?.flatten();
                        h.authenticate_publickey(user, russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?.success()
                    }
                };
                if !ok {
                    bail!("SSH authentication failed for {user}@{host}");
                }
                Ok::<_, anyhow::Error>(h)
            })
            // russh reports a refused key as a generic disconnect, possibly only when authentication is attempted
            .map_err(|e| connect_error(&refused, e))?;
        if *pinned.lock().unwrap() {
            log.warn(format!(
                "first contact with {host}:{port}: pinned its SSH host key {} (later connections must present the same key)",
                fingerprint.lock().unwrap().clone().unwrap_or_default()
            ));
        }
        let s = SshSession { rt, handle, fingerprint, log: log.clone(), label: format!("{user}@{host}") };
        log.info(format!("SSH connected to {} (host key {})", s.label, s.host_key_fingerprint().unwrap_or_default()));
        Ok(s)
    }

    pub fn close(self) {
        let _ = self.rt.block_on(self.handle.disconnect(Disconnect::ByApplication, "", "en"));
    }

    fn log_out(&self, level: Level, text: &str) {
        for l in text.lines() {
            self.log.log(level, Source::Remote, &format!("  | {l}"));
        }
    }
}

impl RemoteShell for SshSession {
    fn exec(&mut self, cmd: &str, stdin: Option<&[u8]>) -> Result<ExecResult> {
        self.log.log(Level::Debug, Source::Remote, &format!("$ {}", cmd.lines().next().unwrap_or("")));
        let res = self.rt.block_on(async {
            let mut ch = self.handle.channel_open_session().await?;
            ch.exec(true, cmd).await?;
            if let Some(data) = stdin {
                ch.data(data).await?;
            }
            ch.eof().await?;
            let (mut out, mut err, mut code) = (Vec::new(), Vec::new(), None);
            while let Some(msg) = ch.wait().await {
                match msg {
                    ChannelMsg::Data { data } => out.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, .. } => err.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>(ExecResult {
                code: code.unwrap_or(255),
                stdout: String::from_utf8_lossy(&out).into(),
                stderr: String::from_utf8_lossy(&err).into(),
            })
        })?;
        if res.code != 0 {
            self.log.log(Level::Warn, Source::Remote, &format!("exit status {}", res.code));
        }
        self.log_out(Level::Debug, &res.stdout);
        self.log_out(if res.code == 0 { Level::Debug } else { Level::Warn }, &res.stderr);
        Ok(res)
    }

    fn exec_to(&mut self, cmd: &str, stdin: Option<&[u8]>, out: &mut dyn std::io::Write) -> Result<(u32, String)> {
        self.log.log(Level::Debug, Source::Remote, &format!("$ {} (binary stream)", cmd.lines().next().unwrap_or("")));
        let (code, err) = self.rt.block_on(async {
            let mut ch = self.handle.channel_open_session().await?;
            ch.exec(true, cmd).await?;
            if let Some(data) = stdin {
                ch.data(data).await?;
            }
            ch.eof().await?;
            let (mut err, mut code) = (Vec::new(), None);
            while let Some(msg) = ch.wait().await {
                match msg {
                    ChannelMsg::Data { data } => out.write_all(&data)?,
                    ChannelMsg::ExtendedData { data, .. } => err.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>((code.unwrap_or(255), String::from_utf8_lossy(&err).to_string()))
        })?;
        out.flush()?;
        Ok((code, err))
    }

    fn upload(&mut self, remote_path: &str, data: &mut dyn Read, size: u64, mode: u32, progress: &mut dyn FnMut(u64)) -> Result<()> {
        if remote_path.contains(['\'', '\n']) {
            bail!("unsafe remote path");
        }
        self.log.info(format!("uploading {} bytes to {remote_path}", size));
        let cmd = format!("cat > '{remote_path}' && chmod {mode:o} '{remote_path}'");
        let res = self.rt.block_on(async {
            let mut ch = self.handle.channel_open_session().await?;
            ch.exec(true, cmd.as_str()).await?;
            let mut buf = vec![0u8; 256 * 1024];
            let mut sent = 0u64;
            loop {
                let n = data.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                ch.data(&buf[..n]).await?;
                sent += n as u64;
                progress(sent);
            }
            ch.eof().await?;
            let (mut err, mut code) = (Vec::new(), None);
            while let Some(msg) = ch.wait().await {
                match msg {
                    ChannelMsg::ExtendedData { data, .. } => err.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => code = Some(exit_status),
                    _ => {}
                }
            }
            Ok::<_, anyhow::Error>((code.unwrap_or(255), String::from_utf8_lossy(&err).to_string(), sent))
        })?;
        if res.0 != 0 {
            bail!("upload to {remote_path} failed (exit {}): {}", res.0, res.1);
        }
        if res.2 != size {
            bail!("short upload: sent {} of {} bytes", res.2, size);
        }
        Ok(())
    }

    fn host_key_fingerprint(&self) -> Option<String> {
        self.fingerprint.lock().unwrap().clone()
    }
}

/// Runs scripts with root privileges: directly if we are root, else via `sudo -S`.
pub struct Privileged<'a> {
    pub sh: &'a mut dyn RemoteShell,
    pub is_root: bool,
    pub sudo_password: Option<String>,
}

impl Privileged<'_> {
    /// Upload `script` to a private temp file and run it with `/bin/sh`. Returns the result.
    pub fn run_script(&mut self, script: &str) -> Result<ExecResult> {
        let name = format!("/tmp/.synofurb-{}.sh", crate::config::random_token(6));
        let mut cur = std::io::Cursor::new(script.as_bytes().to_vec());
        self.sh.upload(&name, &mut cur, script.len() as u64, 0o700, &mut |_| {})?;
        let r = self.run(&format!("/bin/sh '{name}'"));
        let _ = self.sh.exec(&format!("rm -f '{name}'"), None);
        r
    }

    /// Like [`run`](Self::run) but streams binary stdout into `out`.
    pub fn run_to(&mut self, cmd: &str, out: &mut dyn std::io::Write) -> Result<(u32, String)> {
        if self.is_root {
            return self.sh.exec_to(cmd, None, out);
        }
        let pw = self.sudo_password.clone().ok_or_else(|| anyhow!("not root and no sudo password available"))?;
        let wrapped = format!("sudo -S -p '' /bin/sh -c {}", crate::config::sh_quote(cmd));
        self.sh.exec_to(&wrapped, Some(format!("{pw}\n").as_bytes()), out)
    }

    /// Run one command line as root.
    pub fn run(&mut self, cmd: &str) -> Result<ExecResult> {
        if self.is_root {
            return self.sh.exec(cmd, None);
        }
        let pw = self.sudo_password.clone().ok_or_else(|| anyhow!("not root and no sudo password available"))?;
        let wrapped = format!("sudo -S -p '' /bin/sh -c {}", crate::config::sh_quote(cmd));
        self.sh.exec(&wrapped, Some(format!("{pw}\n").as_bytes()))
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted shell for tests: pops canned results and records what was run.
    #[derive(Default)]
    pub struct MockShell {
        pub commands: Vec<String>,
        pub stdins: Vec<Option<Vec<u8>>>,
        pub results: VecDeque<ExecResult>,
        pub uploads: Vec<(String, Vec<u8>, u32)>,
    }

    impl MockShell {
        pub fn push_ok(&mut self, out: &str) {
            self.results.push_back(ExecResult { code: 0, stdout: out.into(), stderr: String::new() });
        }
    }

    impl RemoteShell for MockShell {
        fn exec(&mut self, cmd: &str, stdin: Option<&[u8]>) -> Result<ExecResult> {
            self.commands.push(cmd.to_string());
            self.stdins.push(stdin.map(|s| s.to_vec()));
            Ok(self.results.pop_front().unwrap_or(ExecResult { code: 0, stdout: String::new(), stderr: String::new() }))
        }
        fn exec_to(&mut self, cmd: &str, stdin: Option<&[u8]>, out: &mut dyn std::io::Write) -> Result<(u32, String)> {
            self.commands.push(cmd.to_string());
            self.stdins.push(stdin.map(|s| s.to_vec()));
            let r = self.results.pop_front().unwrap_or(ExecResult { code: 0, stdout: String::new(), stderr: String::new() });
            out.write_all(r.stdout.as_bytes())?;
            Ok((r.code, r.stderr))
        }
        fn upload(&mut self, p: &str, d: &mut dyn Read, _size: u64, mode: u32, progress: &mut dyn FnMut(u64)) -> Result<()> {
            let mut v = vec![];
            d.read_to_end(&mut v)?;
            progress(v.len() as u64);
            self.uploads.push((p.to_string(), v, mode));
            Ok(())
        }
        fn host_key_fingerprint(&self) -> Option<String> {
            Some("SHA256:mock".into())
        }
    }

    #[test]
    fn privileged_uses_sudo_with_password_on_stdin_only() {
        let mut m = MockShell::default();
        {
            let mut p = Privileged { sh: &mut m, is_root: false, sudo_password: Some("s3cretpw".into()) };
            p.run("id -u").unwrap();
        }
        assert!(m.commands[0].starts_with("sudo -S -p ''"));
        assert!(!m.commands[0].contains("s3cretpw"), "password must never be on the command line");
        assert_eq!(m.stdins[0].as_deref(), Some(&b"s3cretpw\n"[..]));
    }

    #[test]
    fn root_runs_directly_and_scripts_are_cleaned_up() {
        let mut m = MockShell::default();
        {
            let mut p = Privileged { sh: &mut m, is_root: true, sudo_password: None };
            p.run_script("echo hi").unwrap();
        }
        assert_eq!(m.uploads.len(), 1);
        assert_eq!(m.uploads[0].2, 0o700);
        assert!(m.commands[0].starts_with("/bin/sh '/tmp/.synofurb-"));
        assert!(m.commands[1].starts_with("rm -f "));
    }
}

#[cfg(test)]
mod hostkey_tests {
    use super::*;
    use crate::hostkey::{HostKeyPolicy, KnownHosts};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A real SSH server (russh) that accepts any password and counts the password attempts it sees.
    struct Srv(Arc<AtomicUsize>);
    impl russh::server::Handler for Srv {
        type Error = russh::Error;
        async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<russh::server::Auth, Self::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(russh::server::Auth::Accept)
        }
    }

    /// Returns (port, host key fingerprint, password attempts seen).
    fn fake_sshd() -> (u16, String, Arc<AtomicUsize>) {
        let kp = crate::config::generate_ssh_keypair("fake-sshd").unwrap();
        let key = russh::keys::PrivateKey::from_openssh(&kp.private_openssh).unwrap();
        let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let attempts = Arc::new(AtomicUsize::new(0));
        let a2 = attempts.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
            rt.block_on(async move {
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(l.local_addr().unwrap().port()).unwrap();
                let cfg = Arc::new(russh::server::Config { keys: vec![key], ..Default::default() });
                loop {
                    let Ok((sock, _)) = l.accept().await else { break };
                    let (cfg, a) = (cfg.clone(), a2.clone());
                    tokio::spawn(async move {
                        if let Ok(session) = russh::server::run_stream(cfg, sock, Srv(a)).await {
                            let _ = session.await;
                        }
                    });
                }
            });
        });
        (rx.recv().unwrap(), fp, attempts)
    }

    fn store(tag: &str) -> KnownHosts {
        let p = std::env::temp_dir().join(format!("sf-sshkh-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        KnownHosts::at(p)
    }

    fn connect(port: u16, policy: &HostKeyPolicy) -> Result<SshSession> {
        SshSession::connect_with("127.0.0.1", port, "admin", &Auth::Password("hunter2".into()), &Logger::memory(None), policy)
    }

    #[test]
    fn a_refused_host_key_closes_the_connection_before_the_password_is_sent() {
        let (port, fp, attempts) = fake_sshd();
        let other = "SHA256:ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ".to_string();

        // wrong pin
        let Err(e) = connect(port, &HostKeyPolicy::Pinned(other.clone())) else { panic!("must refuse") };
        let h = e.downcast_ref::<HostKeyError>().unwrap_or_else(|| panic!("typed host key error, got: {e:#}"));
        assert!(matches!(h, HostKeyError::PinMismatch { expected, presented, .. } if *expected == other && *presented == fp), "{h}");
        // the user said no to a first contact
        let s = store("no");
        let Err(e) = connect(port, &HostKeyPolicy::tofu_with(s.clone(), Arc::new(|_| false))) else { panic!("must refuse") };
        assert!(matches!(e.downcast_ref::<HostKeyError>(), Some(HostKeyError::NotTrusted { fingerprint, .. }) if *fingerprint == fp));
        assert!(s.get("127.0.0.1", port).unwrap().is_none(), "a refused key is not pinned");
        // a pin on record that differs: both fingerprints are named
        let s2 = store("changed");
        s2.pin("127.0.0.1", port, &other).unwrap();
        let Err(e) = connect(port, &HostKeyPolicy::Tofu { store: s2.clone(), confirm: None }) else { panic!("must refuse") };
        let t = format!("{e:#}");
        assert!(
            matches!(e.downcast_ref::<HostKeyError>(), Some(HostKeyError::Changed { .. })) && t.contains(&other) && t.contains(&fp),
            "{t}"
        );

        // the heart of it: the server never saw a password in any of those attempts
        assert_eq!(attempts.load(Ordering::SeqCst), 0, "no password may be sent before the host key is accepted");

        // the right pin, a confirmed first contact, then the pinned key all work, and only now is the password used
        assert!(connect(port, &HostKeyPolicy::Pinned(fp.clone())).is_ok());
        let asked = Arc::new(AtomicUsize::new(0));
        let a = asked.clone();
        let pol = HostKeyPolicy::tofu_with(
            s.clone(),
            Arc::new(move |p| {
                a.fetch_add(1, Ordering::SeqCst);
                p.fingerprint.starts_with("SHA256:")
            }),
        );
        assert!(connect(port, &pol).is_ok() && connect(port, &pol).is_ok());
        assert_eq!(asked.load(Ordering::SeqCst), 1, "asked once; the second connection matches the pin");
        assert_eq!(s.get("127.0.0.1", port).unwrap().unwrap().fingerprint, fp);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        // insecure mode is a policy of its own and logs a loud warning
        let (tx, rx) = std::sync::mpsc::channel();
        let log = Logger::memory(Some(tx));
        assert!(SshSession::connect_with("127.0.0.1", port, "admin", &Auth::Password("p".into()), &log, &HostKeyPolicy::Insecure).is_ok());
        let warned = rx.try_iter().any(|e| matches!(e, crate::logs::Event::Log(l) if l.msg.contains("HOST KEY NOT CHECKED")));
        assert!(warned);
        let _ = std::fs::remove_file(s.path());
        let _ = std::fs::remove_file(s2.path());
    }
}
