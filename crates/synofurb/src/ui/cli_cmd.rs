//! "Copy command": the `synofurb-cli` command line that does what the current screen choices do (UX spec F4).
//!
//! Pure and tested. Secrets never appear: the SSH password is read by the CLI from `SYNOFURB_PASSWORD`, the new
//! system's password is not on the line at all (the CLI generates one, or `--prompt-password` asks). The three
//! confirmation flags are added only when the caller passes the phrase, i.e. after the confirmation screen is satisfied.

use super::state::BootChoice;
use synofurb_core::config::KernelChannel;
use synofurb_core::plan::DataRaid;

/// Everything the command line needs; filled from the form state.
#[derive(Clone, Debug)]
pub struct CliCommand {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_file: Option<String>,
    pub keep_mode: bool,
    pub keep_arrays: Vec<String>,
    pub system_gib: u64,
    pub copies: Option<usize>,
    pub raid: DataRaid,
    pub free_percent: u8,
    pub kernel: KernelChannel,
    pub hostname: String,
    pub username: String,
    /// `generate`, `none`, or the path of a public key file.
    pub authorize_key: String,
    pub timezone: String,
    pub mirror: String,
    pub serial: String,
    pub serial_speed: u32,
    pub dtb: Option<String>,
    pub method: BootChoice,
    pub kit_dir: Option<String>,
    pub kexec: Option<String>,
    pub bundle: Option<String>,
    pub no_firmware: bool,
    pub no_verbose: bool,
}

/// Quote one shell word (POSIX): bare when it only has safe characters, else single-quoted.
pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:=,@+%".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

fn raid_arg(r: DataRaid) -> &'static str {
    match r {
        DataRaid::Auto => "auto",
        DataRaid::None => "none",
        DataRaid::Raid0 => "0",
        DataRaid::Raid1 => "1",
        DataRaid::Raid5 => "5",
        DataRaid::Raid6 => "6",
        DataRaid::Raid10 => "10",
        DataRaid::Shr1 => "shr1",
        DataRaid::Shr2 => "shr2",
    }
}

impl CliCommand {
    fn plan_opts(&self, out: &mut Vec<String>) {
        out.push(format!("--mode {}", if self.keep_mode { "keep" } else { "wipe" }));
        if self.keep_mode && !self.keep_arrays.is_empty() {
            out.push(format!("--keep {}", self.keep_arrays.join(",")));
        }
        if self.system_gib > 0 {
            out.push(format!("--system-gib {}", self.system_gib));
        }
        if let Some(c) = self.copies {
            out.push(format!("--copies {c}"));
        }
        out.push(format!("--raid {}", raid_arg(self.raid)));
        out.push(format!("--free-percent {}", self.free_percent));
    }

    fn conn(&self, out: &mut Vec<String>) {
        out.push(format!("--host {}", quote(&self.host)));
        if self.port != 22 {
            out.push(format!("--port {}", self.port));
        }
        out.push(format!("--user {}", quote(&self.user)));
        if let Some(k) = &self.key_file {
            out.push(format!("--key {}", quote(k)));
        }
    }

    fn kernel_arg(&self) -> &'static str {
        match self.kernel {
            KernelChannel::Stable => "stable",
            KernelChannel::Newest => "newest",
        }
    }

    /// `synofurb-cli plan ...`: read-only.
    pub fn plan_command(&self) -> String {
        let mut a = vec!["synofurb-cli plan".to_string()];
        self.conn(&mut a);
        self.plan_opts(&mut a);
        a.push(format!("--kernel {}", self.kernel_arg()));
        a.join(" ")
    }

    /// `synofurb-cli install ...` with the current choices. `confirm` = the typed phrase once the confirmation screen is
    /// satisfied: only then the three confirmation flags are added.
    pub fn install_command(&self, confirm: Option<&str>) -> String {
        let mut a = vec!["synofurb-cli install".to_string()];
        self.conn(&mut a);
        self.plan_opts(&mut a);
        a.push(format!("--kernel {}", self.kernel_arg()));
        a.push(format!("--hostname {}", quote(&self.hostname)));
        a.push(format!("--username {}", quote(&self.username)));
        a.push(format!("--authorize-key {}", quote(&self.authorize_key)));
        a.push(format!("--timezone {}", quote(&self.timezone)));
        a.push(format!("--mirror {}", quote(&self.mirror)));
        if !self.serial.is_empty() {
            a.push(format!("--serial {}", quote(&self.serial)));
            a.push(format!("--serial-speed {}", self.serial_speed));
        }
        if let Some(d) = &self.dtb {
            a.push(format!("--dtb {}", quote(d)));
        }
        let method = match self.method {
            BootChoice::Auto => "auto",
            BootChoice::Dom => "dom",
            BootChoice::Kexec => "kexec",
            BootChoice::Kit => "kit",
        };
        a.push(format!("--method {method}"));
        if self.method == BootChoice::Kit
            && let Some(d) = &self.kit_dir
        {
            a.push(format!("--kit-dir {}", quote(d)));
        }
        if self.method == BootChoice::Kexec
            && let Some(k) = &self.kexec
        {
            a.push(format!("--kexec {}", quote(k)));
        }
        match &self.bundle {
            Some(b) => {
                a.push("--source offline".to_string());
                a.push(format!("--bundle {}", quote(b)));
            }
            None => a.push("--source online".to_string()),
        }
        if self.no_firmware {
            a.push("--no-firmware".into());
        }
        if self.no_verbose {
            a.push("--no-verbose-installer".into());
        }
        a.push("--prompt-password".into());
        if let Some(phrase) = confirm {
            a.push("--have-backup --accept-data-loss".into());
            a.push(format!("--confirm {}", quote(phrase)));
        }
        a.join(" ")
    }
}

/// Split a command line into words, honouring single and double quotes (POSIX-ish; enough for what we generate).
pub fn shell_words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if started || !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `(program words, flag -> value)` of a command line; a flag without a value maps to an empty string.
pub fn parse_flags(s: &str) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
    let words = shell_words(s);
    let mut head = vec![];
    let mut flags = std::collections::BTreeMap::new();
    let mut i = 0;
    while i < words.len() {
        if let Some(name) = words[i].strip_prefix("--") {
            let val = match words.get(i + 1) {
                Some(v) if !v.starts_with("--") => {
                    i += 1;
                    v.clone()
                }
                _ => String::new(),
            };
            flags.insert(name.to_string(), val);
        } else if flags.is_empty() {
            head.push(words[i].clone());
        }
        i += 1;
    }
    (head, flags)
}

impl CliCommand {
    /// Does `text` (what the user sees or edited in the Copy command dialog) say exactly what the plan says? The tag
    /// "Matches plan" in the dialog is this function, computed from the text, not a constant.
    pub fn matches(&self, text: &str, confirm: Option<&str>) -> bool {
        parse_flags(text) == parse_flags(&self.install_command(confirm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CliCommand {
        CliCommand {
            host: "192.168.1.40".into(),
            port: 22,
            user: "admin".into(),
            key_file: None,
            keep_mode: false,
            keep_arrays: vec![],
            system_gib: 32,
            copies: None,
            raid: DataRaid::Auto,
            free_percent: 20,
            kernel: KernelChannel::Stable,
            hostname: "nas".into(),
            username: "nasadmin".into(),
            authorize_key: "generate".into(),
            timezone: "UTC".into(),
            mirror: "deb.debian.org".into(),
            serial: "ttyS0".into(),
            serial_speed: 115_200,
            dtb: None,
            method: BootChoice::Dom,
            kit_dir: None,
            kexec: None,
            bundle: None,
            no_firmware: false,
            no_verbose: false,
        }
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("nas.local"), "nas.local");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("ERASE DS918+"), "'ERASE DS918+'");
    }

    #[test]
    fn plan_form_is_read_only_and_has_no_gate() {
        let c = sample().plan_command();
        assert!(c.starts_with("synofurb-cli plan --host 192.168.1.40 --user admin"), "{c}");
        assert!(c.contains("--mode wipe") && c.contains("--kernel stable") && c.contains("--system-gib 32"));
        assert!(!c.contains("--confirm") && !c.contains("install"));
    }

    #[test]
    fn install_form_has_the_gate_flags_only_after_confirmation() {
        let c = sample();
        let before = c.install_command(None);
        assert!(before.starts_with("synofurb-cli install --host 192.168.1.40"), "{before}");
        assert!(!before.contains("--have-backup") && !before.contains("--accept-data-loss") && !before.contains("--confirm"));
        let after = c.install_command(Some("ERASE DS918+"));
        assert!(after.ends_with("--have-backup --accept-data-loss --confirm 'ERASE DS918+'"), "{after}");
    }

    #[test]
    fn no_secret_is_ever_on_the_line() {
        let mut c = sample();
        c.key_file = Some("/home/me/.ssh/id ed25519".into());
        let s = c.install_command(Some("ERASE DS918+"));
        assert!(s.contains("--key '/home/me/.ssh/id ed25519'") && s.contains("--prompt-password"));
        assert!(!s.contains("password-env") && !s.to_lowercase().contains("passw0rd"));
    }

    #[test]
    fn words_and_flags_round_trip() {
        assert_eq!(shell_words("a 'b c' \"d e\" f"), vec!["a", "b c", "d e", "f"]);
        assert_eq!(shell_words("--x ''"), vec!["--x", ""]);
        let (head, flags) = parse_flags("synofurb-cli install --host 1.2.3.4 --have-backup --confirm 'ERASE X 3456'");
        assert_eq!(head, vec!["synofurb-cli", "install"]);
        assert_eq!(flags["host"], "1.2.3.4");
        assert_eq!(flags["have-backup"], "");
        assert_eq!(flags["confirm"], "ERASE X 3456");
    }

    #[test]
    fn matches_plan_is_computed_from_the_text() {
        let c = sample();
        let text = c.install_command(None);
        assert!(c.matches(&text, None));
        assert!(c.matches(&text.replace("--mode wipe", "--mode   wipe"), None), "spacing does not matter");
        assert!(!c.matches(&text.replace("--mode wipe", "--mode keep"), None), "a changed choice no longer matches");
        assert!(!c.matches(&text.replace("--kernel stable", "--kernel newest"), None));
        assert!(!c.matches(&format!("{text} --have-backup"), None), "an extra flag does not match");
        let confirmed = c.install_command(Some("ERASE DS918+ 3456"));
        assert!(c.matches(&confirmed, Some("ERASE DS918+ 3456")));
        assert!(!c.matches(&confirmed, None), "the confirmation flags belong to the confirmed form only");
        // every plan choice is in the command: dropping any one flag breaks the match
        for (flag, _) in parse_flags(&text).1 {
            let stripped: Vec<String> = {
                let w = shell_words(&text);
                let mut out = vec![];
                let mut i = 0;
                while i < w.len() {
                    if w[i] == format!("--{flag}") {
                        if w.get(i + 1).is_some_and(|v| !v.starts_with("--")) {
                            i += 1;
                        }
                    } else {
                        out.push(quote(&w[i]));
                    }
                    i += 1;
                }
                out
            };
            assert!(!c.matches(&stripped.join(" "), None), "dropping --{flag} must break the match");
        }
    }

    #[test]
    fn offline_keep_and_kit_variants() {
        let mut c = sample();
        c.keep_mode = true;
        c.keep_arrays = vec!["md2".into(), "md3".into()];
        c.copies = Some(3);
        c.kernel = KernelChannel::Newest;
        c.method = BootChoice::Kit;
        c.kit_dir = Some("/tmp/kit dir".into());
        c.bundle = Some("/data/bundle".into());
        c.port = 2222;
        let s = c.install_command(None);
        for want in [
            "--mode keep",
            "--keep md2,md3",
            "--copies 3",
            "--kernel newest",
            "--method kit",
            "--kit-dir '/tmp/kit dir'",
            "--source offline",
            "--bundle /data/bundle",
            "--port 2222",
        ] {
            assert!(s.contains(want), "{want} in {s}");
        }
        assert!(!s.contains("--source online"));
    }
}
