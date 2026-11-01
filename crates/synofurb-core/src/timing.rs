//! Every time limit of the boot-flash flows in one place. The GUI, the CLI texts and the docs use these numbers (not their own copies).
//!
//! What they mean:
//!
//! * The **watchdog** is inside the boot agent on the unit. It counts from the moment the *agent starts*. If nobody logs in or runs
//!   `sf-boot claim` before it expires, the agent sets the default boot entry to DSM and reboots. It cannot help when the agent
//!   never runs.
//! * The **app wait** is how long this computer waits for the agent to answer after it asked DSM to reboot. It is shorter than
//!   the watchdog on purpose: when the wait runs out the agent may still be starting.
//! * The **installer failure reboot** is a fixed sleep inside the generated installer script (boot-flash units only).

/// Watchdog armed in the agent for an install (`AgentOverlay::timeout_min`): time to claim it and start the Debian installer.
pub const INSTALL_WATCHDOG_MIN: u32 = 30;
/// Watchdog armed in the agent for "boot manager" use (you log in by hand, so longer).
pub const BOOT_MANAGER_WATCHDOG_MIN: u32 = 60;
/// The agent's own default when its overlay names none (`boot/agent/rootfs/usr/lib/synofurb/lib.sh`: `AGENT_TIMEOUT_MIN=20`).
pub const AGENT_DEFAULT_WATCHDOG_MIN: u32 = 20;
/// How long this computer waits for the agent to answer after the reboot request.
pub const AGENT_WAIT_MIN: u32 = 10;
/// How long this computer waits for the finished install's new system to come up.
pub const NEW_SYSTEM_WAIT_MIN: u32 = 20;
/// Default for "wait for DSM" after a recovery reboot.
pub const DSM_WAIT_MIN: u32 = 15;
/// Installer failure before the point of no return: the unit reboots after this many seconds (boot-flash units only).
pub const FAIL_REBOOT_BEFORE_PONR_SECS: u32 = 60;
/// Installer failure after the point of no return: the unit waits this long (time to read the log) before it reboots.
pub const FAIL_REBOOT_AFTER_PONR_SECS: u32 = 900;

/// Minutes of watchdog that can still be left when the app gives up waiting: the watchdog counts from agent start, the app's clock
/// from the reboot request, so at least this much (and up to the whole watchdog) may remain.
pub const fn watchdog_minutes_possibly_left(watchdog_min: u32, waited_min: u32) -> u32 {
    watchdog_min.saturating_sub(waited_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_are_the_documented_ones() {
        assert_eq!(
            (INSTALL_WATCHDOG_MIN, AGENT_WAIT_MIN, BOOT_MANAGER_WATCHDOG_MIN, AGENT_DEFAULT_WATCHDOG_MIN),
            (30, 10, 60, 20),
            "docs/BOOT.md, docs/RECOVERY.md and the GUI copy quote these"
        );
        assert_eq!((FAIL_REBOOT_BEFORE_PONR_SECS, FAIL_REBOOT_AFTER_PONR_SECS), (60, 900));
        assert!(AGENT_WAIT_MIN < INSTALL_WATCHDOG_MIN && INSTALL_WATCHDOG_MIN < BOOT_MANAGER_WATCHDOG_MIN);
        assert_eq!(watchdog_minutes_possibly_left(30, 10), 20);
        assert_eq!(watchdog_minutes_possibly_left(5, 10), 0);
    }

    /// The agent's built-in default lives in shell; keep the constant honest.
    #[test]
    fn agent_default_matches_the_agent_script() {
        let lib = include_str!("../../../boot/agent/rootfs/usr/lib/synofurb/lib.sh");
        assert!(lib.contains(&format!("AGENT_TIMEOUT_MIN={AGENT_DEFAULT_WATCHDOG_MIN}\n")), "lib.sh default changed");
    }
}
