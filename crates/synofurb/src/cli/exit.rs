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
            CliError::Usage(m) | CliError::Refused(m) | CliError::PlanErrors(m) | CliError::Unsupported(m) => f.write_str(m),
            CliError::InstallFailed { message, .. } => f.write_str(message),
            CliError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

impl std::error::Error for CliError {}

impl From<anyhow::Error> for CliError {
    fn from(e: anyhow::Error) -> Self {
        CliError::Other(e)
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
    fn exit_codes_are_stable() {
        assert_eq!(CliError::usage("x").code(), 2);
        assert_eq!(CliError::refused("x").code(), 3);
        assert_eq!(CliError::PlanErrors("x".into()).code(), 4);
        assert_eq!(CliError::Unsupported("x".into()).code(), 5);
        assert_eq!(CliError::InstallFailed { after_ponr: true, message: "x".into() }.code(), 6);
        assert_eq!(CliError::InstallFailed { after_ponr: false, message: "x".into() }.code(), 7);
        assert_eq!(CliError::Other(anyhow::anyhow!("x")).code(), 1);
    }
}
