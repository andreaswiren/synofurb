//! Stable process exit codes and the error type that carries them.

use std::fmt;

/// Exit codes of every command. Documented in docs/CLI.md; scripts may rely on them.
pub mod code {
    pub const OK: u8 = 0;
    /// Any other failure: network, SSH, I/O, bad file contents, ...
    pub const ERROR: u8 = 1;
    /// Command line misuse (clap uses the same code for parse errors).
    pub const USAGE: u8 = 2;
    /// The three-part data-loss gate (backup, acceptance, typed phrase) was not satisfied, or a confirmation was declined.
    pub const REFUSED: u8 = 3;
    /// The plan has errors; nothing was changed.
    pub const PLAN_ERRORS: u8 = 4;
    /// The hardware cannot run Debian 13 (or the compatibility list says so).
    pub const UNSUPPORTED: u8 = 5;
    /// The install failed after the point of no return: the disks were changed.
    pub const FAILED_AFTER_PONR: u8 = 6;
    /// The install failed or was cancelled before the point of no return: the disks are untouched.
    pub const FAILED_BEFORE_PONR: u8 = 7;
    /// `manage health`: the unit answered and its verdict is WARN (something needs attention).
    pub const HEALTH_WARN: u8 = 8;
    /// `manage health`: the unit answered and its verdict is CRIT (a degraded array, a failed disk, a full volume).
    pub const HEALTH_CRIT: u8 = 9;
    /// The unit was asked to reboot into the boot agent and the agent never answered. The unit's state is unknown: it may boot the
    /// agent again after every power cycle (docs/RECOVERY.md).
    pub const AGENT_TIMEOUT: u8 = 10;
    /// The SSH host key was refused (changed since it was pinned, does not match `--host-key`, or a first-time key was not trusted).
    /// Nothing was sent to the unit, not even the password.
    pub const HOST_KEY: u8 = 11;
}

#[derive(Debug)]
pub enum CliError {
    /// Wrong flags or flag combinations (exit 2).
    Usage(String),
    /// Safety gate not satisfied / confirmation declined (exit 3).
    Refused(String),
    /// The plan has errors (exit 4). `shown` = the details were already printed.
    PlanErrors(String),
    /// Unsupported hardware (exit 5).
    Unsupported(String),
    /// The install job failed. `after_ponr` selects exit 6 or 7.
    InstallFailed { after_ponr: bool, message: String },
    /// `manage health` found a problem (exit 8 for WARN, 9 for CRIT). The report was already printed.
    Health { critical: bool, message: String },
    /// The boot agent did not answer after the reboot request (exit 10). The message states both possible outcomes.
    AgentTimeout(String),
    /// The SSH host key was refused (exit 11); nothing was sent.
    HostKey(String),
    /// Everything else (exit 1).
    Other(anyhow::Error),
}

impl CliError {
    pub fn code(&self) -> u8 {
        match self {
            CliError::Usage(_) => code::USAGE,
            CliError::Refused(_) => code::REFUSED,
            CliError::PlanErrors(_) => code::PLAN_ERRORS,
            CliError::Unsupported(_) => code::UNSUPPORTED,
            CliError::InstallFailed { after_ponr: true, .. } => code::FAILED_AFTER_PONR,
            CliError::InstallFailed { after_ponr: false, .. } => code::FAILED_BEFORE_PONR,
            CliError::Health { critical: true, .. } => code::HEALTH_CRIT,
            CliError::Health { critical: false, .. } => code::HEALTH_WARN,
            CliError::AgentTimeout(_) => code::AGENT_TIMEOUT,
            CliError::HostKey(_) => code::HOST_KEY,
            CliError::Other(_) => code::ERROR,
        }
    }

    /// Machine-readable name of the failure class.
    pub fn kind(&self) -> &'static str {
        match self {
            CliError::Usage(_) => "usage",
            CliError::Refused(_) => "refused",
            CliError::PlanErrors(_) => "plan_errors",
            CliError::Unsupported(_) => "unsupported",
            CliError::InstallFailed { after_ponr: true, .. } => "failed_after_point_of_no_return",
            CliError::InstallFailed { after_ponr: false, .. } => "failed_before_point_of_no_return",
            CliError::Health { critical: true, .. } => "health_crit",
            CliError::Health { critical: false, .. } => "health_warn",
            CliError::AgentTimeout(_) => "agent_timeout",
            CliError::HostKey(_) => "host_key",
            CliError::Other(_) => "error",
        }
    }

    pub fn usage(m: impl Into<String>) -> CliError {
        CliError::Usage(m.into())
    }
    pub fn refused(m: impl Into<String>) -> CliError {
        CliError::Refused(m.into())
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage(m)
            | CliError::Refused(m)
            | CliError::PlanErrors(m)
            | CliError::Unsupported(m)
            | CliError::AgentTimeout(m)
            | CliError::HostKey(m) => f.write_str(m),
            CliError::InstallFailed { message, .. } | CliError::Health { message, .. } => f.write_str(message),
            CliError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for CliError {}

impl CliError {
    /// The typed engine failures get their own exit codes. `message` is what to print (it may carry more than the error's own text, such
    /// as a log path). `None` for every other error.
    pub fn typed(e: &anyhow::Error, message: &str) -> Option<CliError> {
        use synofurb_core::hostkey::HostKeyError;
        use synofurb_core::session::SessionError;
        if matches!(SessionError::find(e), Some(SessionError::AgentTimeout { .. })) {
            return Some(CliError::AgentTimeout(message.to_string()));
        }
        if matches!(e.downcast_ref::<HostKeyError>(), Some(h) if !matches!(h, HostKeyError::Store { .. })) {
            return Some(CliError::HostKey(message.to_string()));
        }
        None
    }
}

impl From<anyhow::Error> for CliError {
    fn from(e: anyhow::Error) -> Self {
        let m = format!("{e:#}");
        CliError::typed(&e, &m).unwrap_or(CliError::Other(e))
    }
}
impl From<synofurb_core::hostkey::HostKeyError> for CliError {
    fn from(e: synofurb_core::hostkey::HostKeyError) -> Self {
        CliError::from(anyhow::Error::new(e))
    }
}
impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Other(e.into())
    }
}
impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> Self {
        CliError::Other(e.into())
    }
}

pub type CliResult<T = ()> = Result<T, CliError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_engine_errors_get_their_own_codes_through_every_conversion() {
        use synofurb_core::hostkey::HostKeyError;
        use synofurb_core::session::SessionError;
        let t: CliError = anyhow::Error::new(SessionError::AgentTimeout { waited_min: 10, watchdog_min: 30 }).into();
        assert_eq!(t.code(), 10);
        assert!(t.to_string().contains("SYNOLOGY_2") && !t.to_string().to_lowercase().contains("boot dsm normally"), "{t}");
        // with context around it (the engine adds some) it is still found
        let wrapped = anyhow::Error::new(SessionError::AgentTimeout { waited_min: 10, watchdog_min: 30 }).context("while installing");
        assert_eq!(CliError::typed(&wrapped, "custom message").map(|e| e.code()), Some(10));
        let c: CliError = wrapped.into();
        assert_eq!(c.code(), 10);
        let h: CliError =
            anyhow::Error::new(HostKeyError::NotTrusted { host: "h".into(), port: 22, fingerprint: "SHA256:x".into() }).into();
        assert_eq!(h.code(), 11);
        let store: CliError = anyhow::Error::new(HostKeyError::Store { path: "p".into(), reason: "r".into() }).into();
        assert_eq!(store.code(), 1, "an unreadable known-hosts file is an ordinary error");
        assert_eq!(CliError::from(anyhow::anyhow!("boom")).code(), 1);
        assert_eq!(CliError::from(anyhow::Error::new(SessionError::FlashBackupFailed { reason: "r".into() })).code(), 1);
    }

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::usage("x").code(), 2);
        assert_eq!(CliError::refused("x").code(), 3);
        assert_eq!(CliError::PlanErrors("x".into()).code(), 4);
        assert_eq!(CliError::Unsupported("x".into()).code(), 5);
        assert_eq!(CliError::InstallFailed { after_ponr: true, message: "x".into() }.code(), 6);
        assert_eq!(CliError::InstallFailed { after_ponr: false, message: "x".into() }.code(), 7);
        assert_eq!(CliError::Other(anyhow::anyhow!("x")).code(), 1);
        assert_eq!(CliError::Health { critical: false, message: "x".into() }.code(), 8);
        assert_eq!(CliError::Health { critical: true, message: "x".into() }.code(), 9);
        assert_eq!(CliError::AgentTimeout("x".into()).code(), 10);
        assert_eq!(CliError::HostKey("x".into()).code(), 11);
        assert_eq!((CliError::AgentTimeout("x".into()).kind(), CliError::HostKey("x".into()).kind()), ("agent_timeout", "host_key"));
        assert_eq!(CliError::Health { critical: true, message: "x".into() }.kind(), "health_crit");
        assert_eq!(CliError::Health { critical: false, message: "x".into() }.kind(), "health_warn");
    }
}
