//! Install configuration: user choices that end up on the NAS, with validation.

use crate::model::SerialConsole;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("OS random number generator unavailable");
    b
}

/// URL-safe random token (hex).
pub fn random_token(bytes: usize) -> String {
    hex::encode(random_bytes(bytes))
}

/// Strong random password without ambiguous characters.
pub fn generate_password(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut out = String::with_capacity(len);
    // rejection sampling to avoid modulo bias
    let limit = 256 - (256 % ALPHABET.len());
    while out.len() < len {
        for b in random_bytes(len * 2) {
            if (b as usize) < limit && out.len() < len {
                out.push(ALPHABET[b as usize % ALPHABET.len()] as char);
            }
        }
    }
    out
}

/// SHA-512 crypt (`$6$...`) hash accepted by `chpasswd -e` on Debian.
pub fn hash_password(password: &str) -> Result<String> {
    use sha_crypt::{PasswordHasher, ShaCrypt};
    let h = ShaCrypt::default().hash_password(password.as_bytes()).map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?;
    canonical_sha512_crypt(h.as_ref())
}

/// glibc's crypt(3) only uses 16 salt characters and re-emits them, so a stored hash with a
/// longer salt (or an explicit default `rounds=5000`) never compares equal and logins fail.
/// Normalise to the canonical `$6$<salt16>$<digest>` form.
pub fn canonical_sha512_crypt(h: &str) -> Result<String> {
    let parts: Vec<&str> = h.split('$').collect();
    // ["", "6", (rounds=N)?, salt, digest]
    let (salt, digest) = match parts.as_slice() {
        ["", "6", salt, digest] => (*salt, *digest),
        ["", "6", r, salt, digest] if *r == "rounds=5000" => (*salt, *digest),
        _ => bail!("unexpected password hash format"),
    };
    let salt: String = salt.chars().take(16).collect();
    if salt.is_empty() || digest.len() != 86 {
        bail!("unexpected password hash length");
    }
    Ok(format!("$6${salt}${digest}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshKeypair {
    pub private_openssh: String,
    pub public_openssh: String,
    pub fingerprint: String,
}

pub fn generate_ssh_keypair(comment: &str) -> Result<SshKeypair> {
    use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey, rand_core::OsRng};
    let mut k = PrivateKey::random(&mut OsRng, Algorithm::Ed25519)?;
    k.set_comment(comment);
    Ok(SshKeypair {
        private_openssh: k.to_openssh(LineEnding::LF)?.to_string(),
        public_openssh: k.public_key().to_openssh()?,
        fingerprint: k.fingerprint(HashAlg::Sha256).to_string(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallConfig {
    pub hostname: String,
    pub username: String,
    /// `$6$...` crypt hash; never the plaintext.
    pub password_hash: String,
    pub ssh_authorized_keys: Vec<String>,
    pub timezone: String,
    pub locale: String,
    pub keymap: String,
    pub mirror_host: String,
    pub mirror_dir: String,
    pub suite: String,
    pub serial: SerialConsole,
    pub enable_mgmt: bool,
    /// Extra Debian packages to install.
    pub extra_packages: Vec<String>,
    /// U-Boot boards: DTB file name to append to the kernel (e.g. `armada-385-synology-ds116.dtb`).
    pub dtb: Option<String>,
}

impl InstallConfig {
    pub fn new(hostname: &str, username: &str, password_hash: String) -> Self {
        InstallConfig {
            hostname: hostname.into(),
            username: username.into(),
            password_hash,
            ssh_authorized_keys: vec![],
            timezone: "UTC".into(),
            locale: "en_US.UTF-8".into(),
            keymap: "us".into(),
            mirror_host: "deb.debian.org".into(),
            mirror_dir: "/debian".into(),
            suite: "trixie".into(),
            serial: SerialConsole::default(),
            enable_mgmt: true,
            extra_packages: vec![],
            dtb: None,
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_hostname(&self.hostname)?;
        validate_username(&self.username)?;
        if !self.password_hash.starts_with("$6$") || self.password_hash.contains(['\n', '\'', ' ']) {
            bail!("password hash must be a SHA-512 crypt string");
        }
        for k in &self.ssh_authorized_keys {
            validate_ssh_public_key(k)?;
        }
        if !self.timezone.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c)) || self.timezone.is_empty() {
            bail!("invalid time zone");
        }
        if !self.locale.chars().all(|c| c.is_ascii_alphanumeric() || "._@-".contains(c)) || self.locale.is_empty() {
            bail!("invalid locale");
        }
        if !self.keymap.chars().all(|c| c.is_ascii_alphanumeric() || "_-".contains(c)) || self.keymap.is_empty() {
            bail!("invalid keymap");
        }
        if !self.mirror_host.chars().all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c)) || self.mirror_host.is_empty() {
            bail!("invalid mirror host");
        }
        if !self.mirror_dir.starts_with('/') || !self.mirror_dir.chars().all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c)) {
            bail!("invalid mirror directory");
        }
        if !self.suite.chars().all(|c| c.is_ascii_lowercase() || c == '-') || self.suite.is_empty() {
            bail!("invalid suite");
        }
        if !self.serial.tty.chars().all(|c| c.is_ascii_alphanumeric()) || self.serial.tty.is_empty() {
            bail!("invalid serial console");
        }
        for p in &self.extra_packages {
            if !valid_package_name(p) {
                bail!("invalid package name '{p}'");
            }
        }
        if let Some(d) = &self.dtb
            && (!d.ends_with(".dtb") || !d.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
        {
            bail!("invalid DTB file name");
        }
        Ok(())
    }
}

fn valid_package_name(p: &str) -> bool {
    !p.is_empty()
        && p.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+-.".contains(c))
}

pub fn validate_hostname(h: &str) -> Result<()> {
    let ok = !h.is_empty()
        && h.len() <= 63
        && !h.starts_with('-')
        && !h.ends_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !ok {
        bail!("hostname must be 1-63 letters, digits or hyphens (no leading/trailing hyphen)");
    }
    Ok(())
}

pub fn validate_username(u: &str) -> Result<()> {
    let reserved = [
        "root", "daemon", "bin", "sys", "sync", "games", "man", "lp", "mail", "news", "uucp", "proxy", "www-data", "backup", "list", "irc",
        "nobody", "admin_",
    ];
    let ok = !u.is_empty()
        && u.len() <= 32
        && u.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !ok {
        bail!("user name must start with a lowercase letter and use only a-z, 0-9, '_' or '-' (max 32)");
    }
    if reserved.contains(&u) {
        bail!("'{u}' is a reserved system account name");
    }
    Ok(())
}

pub fn validate_ssh_public_key(k: &str) -> Result<()> {
    let k = k.trim();
    if k.contains('\n') || k.contains('\'') {
        bail!("SSH public key must be a single line");
    }
    let mut it = k.split_whitespace();
    let kind = it.next().unwrap_or("");
    let blob = it.next().unwrap_or("");
    let known = [
        "ssh-ed25519",
        "ssh-rsa",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];
    if !known.contains(&kind) || blob.len() < 20 || !blob.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)) {
        bail!("not a valid OpenSSH public key");
    }
    Ok(())
}

/// Quote a value for POSIX sh single-quoted context.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_generation_and_hash() {
        let p = generate_password(20);
        assert_eq!(p.len(), 20);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(p, generate_password(20));
        let h = hash_password("correct horse").unwrap();
        assert!(h.starts_with("$6$"), "{h}");
        assert!(!h.contains('\n'));
        assert_eq!(h.split('$').nth(2).unwrap().len(), 16, "glibc uses 16 salt chars; longer salts break login");
        assert!(!h.contains("rounds="));
        // Cross-check against openssl when available.
        let salt = h.split('$').nth(2).unwrap();
        if let Ok(out) = std::process::Command::new("openssl").args(["passwd", "-6", "-salt", salt, "correct horse"]).output()
            && out.status.success()
        {
            assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), h, "sha512-crypt must match openssl");
        }
    }

    #[test]
    fn ssh_keypair_roundtrip() {
        let k = generate_ssh_keypair("synofurb-test").unwrap();
        assert!(k.public_openssh.starts_with("ssh-ed25519 "));
        assert!(k.private_openssh.contains("BEGIN OPENSSH PRIVATE KEY"));
        assert!(k.fingerprint.starts_with("SHA256:"));
        validate_ssh_public_key(&k.public_openssh).unwrap();
    }

    #[test]
    fn validators_reject_injection() {
        assert!(validate_hostname("nas-01").is_ok());
        assert!(validate_hostname("nas;rm").is_err());
        assert!(validate_hostname("-nas").is_err());
        assert!(validate_username("nasadmin").is_ok());
        assert!(validate_username("root").is_err());
        assert!(validate_username("a b").is_err());
        assert!(validate_username("$(id)").is_err());
        assert!(validate_ssh_public_key("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIabcdefghijklmnopqrstuvwxyz0123456789 me@x").is_ok());
        assert!(validate_ssh_public_key("ssh-ed25519 AAAA'; rm -rf / #").is_err());
        assert!(validate_ssh_public_key("nonsense").is_err());
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn config_validation() {
        let mut c = InstallConfig::new("nas", "nasadmin", "$6$salt$abcdef".into());
        c.validate().unwrap();
        c.extra_packages = vec!["htop".into(), "x;y".into()];
        assert!(c.validate().is_err());
        c.extra_packages.clear();
        c.dtb = Some("../evil".into());
        assert!(c.validate().is_err());
    }
}
