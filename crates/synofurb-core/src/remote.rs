//! Remote execution on the NAS over SSH (russh), behind the [`RemoteShell`] trait so the install
//! flow can be tested with a mock. Every command and its output is written to the debug log.

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

struct Handler {
    fingerprint: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        // Trust-on-first-use: the fingerprint is shown to the user and written to the log.
        let fp = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
            PublicKeyOrCertificate::Certificate(_) => "certificate".to_string(),
        };
        *self.fingerprint.lock().unwrap() = Some(fp);
        Ok(true)
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
        let fp = Arc::new(Mutex::new(None));
        let cfg = Arc::new(client::Config { inactivity_timeout: Some(timeout), ..Default::default() });
        let h = tokio::time::timeout(timeout, client::connect(cfg, (host, port), Handler { fingerprint: fp.clone() }))
            .await
            .map_err(|_| anyhow!("timed out"))??;
        let _ = h.disconnect(Disconnect::ByApplication, "", "en").await;
        let v = fp.lock().unwrap().clone();
        v.ok_or_else(|| anyhow!("server sent no host key"))
    })
}

impl SshSession {
    pub fn connect(host: &str, port: u16, user: &str, auth: &Auth, log: &Logger) -> Result<SshSession> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let fingerprint = Arc::new(Mutex::new(None));
        let fp2 = fingerprint.clone();
        let handle = rt.block_on(async {
            let cfg = Arc::new(client::Config {
                inactivity_timeout: Some(Duration::from_secs(3600)),
                keepalive_interval: Some(Duration::from_secs(20)),
                ..Default::default()
            });
            let mut h = tokio::time::timeout(Duration::from_secs(20), client::connect(cfg, (host, port), Handler { fingerprint: fp2 }))
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
        })?;
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
