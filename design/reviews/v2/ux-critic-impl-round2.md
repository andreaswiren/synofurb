# UX critic: implementation fidelity of the desktop app, round 2

Implementation review only. No gate verdict, nothing written to `design/approvals/`, no code edited. This re-checks the 6 blocking and 7 major findings of `ux-critic-impl-round1.md` against HEAD (`1996dce`), reads the new engine code (`recovery.rs`, `hostkey.rs`, `timing.rs`, the failure handling in `script.rs`, `session.rs`, `remote.rs`) and the GUI (`ui/trust.rs`, `worker.rs`, `screens/{failed,recover,manage,bootmgr}.rs`, `overlays.rs`, `facts.rs`, `copy.rs`), and looks at the new screenshots (`failed-agent`, `dlg-quick`, `dlg-manual`, others among the 55).

I did **not** run the test suite or the app: my earlier `cargo test` build was still compiling and was stopped, and the disk is now full, so a rebuild was not possible. "Tested" below means "a test exists and I read it".

## Summary

Of my 13 round-1 findings, 12 are fixed in code and one (the host-key pin) is fixed with a defect. The engine work is good: the recovery actions read the unit back and report "not confirmed" as a warning rather than a success; the host-key check sits before authentication; the timing numbers have one home; the failure copy follows `script::failure_handling`.

Reading the new code more closely found **3 blocking findings** that were not visible from the screenshots or from round 1, 6 major and 6 minor. B1 (Manage targets the wrong host) is in code that already existed in round 1, and I missed it then; B2 (restore fallback) and B3 (host-key timing) come with the new engine and GUI batch.

| | Count |
|---|---|
| Blocking, still open | 3 (B1, B2, B3) |
| Major | 6 (M1 to M6) |
| Minor | 6 |

---

## 1. Round-1 findings: status

| Round 1 | Status | Evidence |
|---|---|---|
| **B1** agent-never-answered text | **Fixed** | `failed.rs`: for `agent_unknown` the banner is `FAIL_AGENT_TEXT` ("No answer within 10 minutes. Nothing was reset after the wait."), not the engine text; `scrub_agent_promise` strips the old sentence from any engine or log line; `next_item` returns `DLG_NEXT_AGENT` ("If it never started, the NAS may keep booting the agent after every power cycle ... choose SYNOLOGY_2 ..."); tags "Default: not confirmed" and "DSM may return in 20 min or more" are on the page; subtitle "Phase 4 of 7"; screenshot `v2-app-failed-agent-light.png` agrees. The engine now returns a typed `SessionError::AgentTimeout` (no substring match; `boot_default_unconfirmed()`), and `FailureKind` travels to the UI. |
| **B2** Manual recovery wording | **Fixed** | `DLG_MANUAL`: "Both ways need physical access", "choose SYNOLOGY_2 in the boot menu ... run the first command", "or put the boot flash in another Linux computer ... check twice", with `sudo synofurb-dom default dsm --yes` and `sudo SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes <saved boot-flash image>`. It matches `boot/dom/README.md`. See M3 for what route 2 still lacks. |
| **B3** fake Manage update | **Fixed in the engine path** | `spawn_mgmt_update` calls `push_update(&MgmtBundle::embedded())` / `push_rollback`; the unit's version comes from `h.version`, the app's from `bundled_mgmt_version()`; constants are demo-only; "Version unknown" when either is empty. But see B1 below: it targets the wrong host. |
| **B4** Recovery facts from memory | **Fixed** | Opening Recovery calls `read_unit_state` (`go()`); `render.rs::unit_facts` prefers the unit's own `UnitBootState` (`our_entries`, `default_entry`, `running`) and sets `agent_unknown: false` once the menu was read; copy says "Read from the unit." only when `ReadOutcome::Read`, otherwise "From this session." and "Nothing known to recover. This session changed nothing; the app cannot read earlier changes." `agent_unknown` is reset in `start_over`. Remaining: the "Disks" chip still comes from session memory (m2). |
| **B5** success after nothing | **Fixed** | Recovery actions call `rec::boot_dsm_on`, `restore_flash_on`, `remove_our_entries_on`, `enter_agent`. Remove and Boot DSM return `RecoveryError::Unverified` unless the menu was read back and shows the expected default (`recovery.rs`: `(Read, true, true)` only). `RecLevel::Done` means read back; `Warn` means not confirmed; `Info` is a command with Copy ("Show command"), never success. Boot DSM has an explicit "Set default" vs "Set and reboot" dialog; the reboot is sent only after the default was verified, and the message says "This only means the command was sent". Return to DSM on the Boot manager goes through the same dialog and shows "Back in DSM" only when a `Done` result exists. |
| **B6** Keep gate wording | **Fixed** | `confirm.rs:68`: `if keep { CONF_KEEP_RISK } else { CONF_ERASED }`. |
| **M1** Quick setup | **Fixed** | `QuickView.sel: Option<usize>`, nothing preselected for mixed disks; options show the resolved level and survival ("RAID 5 · survives 1 disk · 10.82 TiB", "Hybrid 1-disk · survives 1 disk · 14.46 TiB"); "Hybrid is built by synofurb; DSM may not adopt it."; "Use these" is a button behind `with_reason` ("choose a layout") and its handler returns early with no choice; the summary says "Newest kernel". Screenshot `dlg-quick-light.png`. One unverified risk: m4. |
| **M2** kernel default | **Resolved by decision** | Newest is the product requirement ("kernel 7+ preferred"); UX-SPEC F3 was corrected; Quick setup, System, Command and Confirm all say Newest, and the Quick summary states it. I withdraw the finding. Note for the real hardware test: Newest on an Untested model is still an unverified choice; the copy does say so ("Untested"). |
| **M3** rescue shell / script text | **Fixed** | No "rescue shell" claim remains in the GUI (`facts.rs` tests assert it); `DLG_NEXT_*` say "there is no rescue shell"; clocks come from `script::failure_handling(&Plan)` (boot-flash: 60 s and 15 min; others: "will NOT reboot by itself" with `DLG_NEXT_*_HALT`); `script.rs` no longer says "reinstall DSM keeps data volumes"; the new line says "synofurb cannot promise that any data survives". See section 3 for what remains without a rescue shell. |
| **M4** Wait for DSM | **Fixed** | `spawn_wait_dsm` → `recovery::wait_for_dsm` with `probe_unit` (DsmBack, LoaderOnly, AgentStillUp, Unreachable), `DSM_WAIT_MIN` 15, elapsed seconds in the UI; the Failed page shows back / loader / agent-up / not-back states. |
| **M5** host-key checking | **Fixed, with a defect** | See B2 and M1 below. |
| **M6** "Flash changed" after a failed backup | **Fixed** | `FlashBackupFailed` → `flash_untouched` → `session_flash` is false. |
| **M7** 30 vs 60 minutes | **Fixed** | `GOAL_BOOTMGR` and the Help topic say 60; `timing.rs` holds the numbers; `copy.rs` has tests that tie texts to the constants (`has(&DLG_NEXT_AGENT, AGENT_WAIT_MIN, "minutes")`). |

---

## 2. Your specific questions

**Host-key bridge (`trust.rs`, `hostkey.rs`, `remote.rs`).**
- *Password never sent before the answer:* yes. `Handler::check_server_key` calls `policy.check` during key exchange; a refusal returns `Ok(false)`, and `authenticate_password` / `authenticate_publickey` are only reached after `client::connect` returns. In `russh 0.65` `connect_stream` waits for the kex-done signal, so the callback runs before any authentication. `NotTrusted` says "closed before any password was sent".
- *Timeout = refusal:* yes for the bridge itself: `rx.recv_timeout(180 s)` → `unwrap_or(false)`; a dropped question (dialog closed, window gone) makes the channel disconnect and the worker refuses at once. Tests: `no_answer_is_a_refusal`, `the_engine_policy_pins_only_after_a_yes`. **But the 180 s is unreachable** (M1 below).
- *Changed key:* a pinned unit that presents another key is `HostKeyError::Changed` (a typed error that survives `anyhow` context, tested); `note_changed_key` sends `HostKeyChanged` from every worker that connects; `open_hostkey_changed` is a danger dialog with both fingerprints, a copyable `host-key forget` command and a single Cancel. After an install the pin of the new Debian key is replaced only if the policy is trust-on-first-use (`repin_after_install`, `session.rs:692`).
- *Agent-key path:* `recovery_session` probes the address; when it answers as the boot agent it connects as `root` with the unit's agent key under `HostKeyPolicy::KeyAuthOnly` (no password is sent to it; its host key is random per boot), otherwise with the page's sign-in under the trust-on-first-use policy. This is a sound split. See m3 for the one place it is missing (Open agent).

**Recovery success only when the engine read the unit back.** Yes for Boot DSM and Remove (`Unverified` otherwise). For Restore, `restore_flash_on` returns `Ok` with `readback_verified: true` as a constant once `synofurb-dom restore` exits 0 (the tool does its own read-back per its README) and does not check that `state_after.read == Read`. The UI then says "read back and compared". Acceptable, but the constant should be derived (m5).

**Return / Boot DSM honesty.** Fixed as described under B5. Return to DSM on the Boot manager considers any `Done` recovery result as "returned" (`bootmgr.rs`: `rec_result.level == Done`), including an unrelated one such as the earlier "The agent answers" result of Open agent (m6).

**`failure_handling`-driven text against `script.rs`.** Consistent: `failure_handling(plan)` is `dom_boot ? {60 s, 900 s} : halts`, and the script's `fail()` does the same (reset default, sleep 60 or 900, reboot) only when `DOM_BOOT = 1 && command -v synofurb-dom`. `synofurb-dom` rides in the installer initrd (`debian.rs:334`, `usr/local/sbin/synofurb-dom`). One thing I cannot confirm: whether `/usr/local/sbin` is in the installer shell's `PATH`; if it is not, `command -v` fails and the installer halts, so the page's "reboots in 60 s / 15 min" would be wrong on boot-flash units. A test with the real installer is needed (M5).

**Remaining risks of shipping without an installer rescue shell (DEV-UI-18).** The text is now honest, but the user is left with less than the approved design offers:
- After step 3 there is no retry and no way in: the options are Copy log, Export diagnostics, Recovery options and waiting 15 minutes (boot-flash) or the console (others). An SSH-only user cannot look at a half-built system. "Continue without the failing disk" and "retry a step" are gone.
- On non-boot-flash methods (kexec, kit) the installer halts for ever and "Read the log on the console" assumes a console they may not have; the log is also streamed to the desktop while the connection lasts.
- The 15-minute reboot into the loader is all the product does for a wiped unit; a user who is away reaches a Synology "Not installed" page without having seen any text.
Recommendation: acceptable for a first release only if release notes and the Help topic say plainly "after step 3 there is no repair; the way back is reinstalling DSM"; a rescue SSH in the installer is the proper follow-up.

**Open agent on a unit that already runs the agent.** Not handled. `facts::recovery_rows` disables Open agent only when `!flash`; `UnitBootState.running == Agent` is never read by the UI (`grep RunningSystem::Agent ui/` finds nothing). Pressing it opens "Reboot into the agent?" and then `enter_agent` connects with the page's DSM password (`SshSession::connect`), which a key-only agent refuses: the result is "The agent was not started: SSH authentication failed" for an agent that is up (M4).

---

## 3. Blocking findings (release blockers)

### B1. Manage works on the Connect form's host, not on the unit it shows

`screens/manage.rs` (`page_manage`, `read_manage_health`, `start_mgmt_update`, `confirm_mgmt_rollback`), `app.rs::target`, `screens/inventory.rs:309`.

The Manage page takes its title and web address from `manage_unit` (the unit the user clicked under "Running Debian"), but every SSH action (`read_manage_health`, `spawn_mgmt_update`, rollback) uses `self.target(cx)`, which reads the **Connect form** (`inp.host`, `inp.user`, `inp.password`). Clicking Manage on a converted unit sets only `manage_unit` and calls `go(Manage)`; nothing copies its address into the form and nothing reads its health on arrival. So:
- On a fresh start the form is empty: `target()` returns "Enter the address of the NAS", the report stays `None` and the page shows "Unreachable. Check power, cable and network." for a healthy unit. There is no sign-in on this page for a Debian unit (the Connect form holds a DSM user).
- If the user connected to another unit earlier, the form still holds that unit: **Check again reads that unit's health, Update pushes the management bundle to it, Roll back rolls it back**, while the page shows the other unit's name and web link. That is an action on the wrong NAS.

**Must fix:** Manage needs its own target (the unit's address, a sign-in prompt for the Debian user or key), read on arrival, and every action must show and use the same host. Until then the Update and Roll back buttons must not be offered.

### B2. Restore flash can write another unit's image

`screens/recover.rs::run_recovery_row` (RestoreFlash), `facts::latest_backup`.

The row picks `find_local_backups(&backups, &unit)` (images named for this unit's serial) but falls back to `facts::latest_backup(&backups)`, which returns the newest `synofurb-dom-backup-*.img` in the folder **of any unit**. The typed phrase is the generic `RESTORE FLASH`; the dialog body ends with the file path. With two units of the same model (the app's own twin case) a unit with no backup of its own gets the other unit's flash image written to its boot flash. The engine checks only that the file is a 128 MiB Synology DOM image with a valid checksum (`check_backup_file`), not whose it is.

**Must fix:** no fallback; if there is no image for this unit say "No saved image for this unit" (as it already does when the folder is empty); show the unit the image belongs to in the dialog; consider a phrase that includes the unit (like the erase phrase).

### B3. The first-contact host-key dialog cannot be answered at the pace the dialog asks for

`crates/synofurb-core/src/remote.rs:146`, `ui/trust.rs::ANSWER_TIMEOUT`, `overlays.rs::open_hostkey_dialog`.

`connect_with` wraps `client::connect` in `tokio::time::timeout(Duration::from_secs(20), ...)`. In `russh 0.65` `connect_stream` returns only after key exchange has finished (it awaits `kex_done_signal_rx`), and the host-key callback runs inside it. The bridge blocks that callback for up to 180 s waiting for the user, but the outer timeout fires after 20 s and abandons the connection ("connection to X timed out"). The dialog says "Compare this fingerprint with DSM (Control Panel, Terminal and SNMP)": reading 43 characters in another window takes longer than 20 seconds. The user sees "timed out" while the dialog is still open; the dialog stays on screen, and clicking Trust afterwards still writes the pin (the callback returns true inside the orphaned session) so the next attempt silently succeeds. DEV-UI-19 ("waits up to 3 minutes") is not what happens.

I rank this a blocker because it defeats the one purpose of the dialog: it trains users to click Trust within 20 seconds without comparing, or to retry blindly. It is also a cheap fix.

**Must fix:** do not count the host-key question against the connect timeout (apply the 20 s to the TCP and banner phase, the user's answer time separately), close the dialog when the connection ends, and add a test with a deliberately slow callback.

---

## 4. Major findings

**M1. The 180 s answer time is unreachable** (the same cause as B3, for the CLI and for any caller). Listed separately because the CLI's `--host-key` flow is unaffected; the GUI path needs the B3 fix.

**M2. Open agent on a Debian unit leaves it booting DSM.** `DLG_AGENT`: "The agent resets the default to DSM when it starts." On a converted Debian unit (`facts.debian`) Open agent is enabled; after the agent's watchdog (60 minutes) the unit reboots with DSM as the default, i.e. into the Synology loader, not into Debian. Nothing says "Debian will not start again until you choose it (`sf-boot reboot debian`)". Either disable the row for a running Debian unit, or say what happens and offer Return to Debian.

**M3. Manual recovery route 2 needs a tool the other computer does not have.** The dialog's second command is `sudo SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes <saved boot-flash image>`; the script lives only on the unit and in the repository. `boot/dom/README.md` also gives a plain `dd` path with `sha256sum -c`. The dialog should include that line (or the app should save `synofurb-dom` beside the backup), and the file name from the "Saved here" list instead of the placeholder.

**M4. Open agent does not know the unit is already in the agent** (see section 2). Disable it with the reason "The agent is already running", show its ssh command from the engine's state, and use the agent-key path in `enter_agent`.

**M5. The "reboots in 60 s / 15 min" claim depends on `synofurb-dom` being on the installer's `PATH`.** `script.rs:121` checks `command -v synofurb-dom`; the file is in `/usr/local/sbin` inside the initrd. If that directory is not in the installer's `PATH`, the check fails and the installer halts for ever while the Failed page says it reboots. A real-installer test (or `[ -x /usr/local/sbin/synofurb-dom ]`) is needed before the numbers can be trusted.

**M6. The Quick setup footer may keep showing "Use these" as unavailable after a layout is chosen.** `overlays.rs`: `unchosen` is computed when the dialog's builder closure runs; selecting a radio notifies only the inner `QuickView`. Whether `window.open_dialog` re-runs the builder on that notification I could not confirm without running it; if it does not, the button stays greyed with a "choose a layout" reason although a layout is selected (its handler would still work). Check on the real window.

---

## 5. Minor findings

1. **m1** The host-key dialog stays open after the connection has already failed or timed out (clicking Trust then does nothing visible).
2. **m2** The Recovery "Disks" chip comes from `self.install.past_no_return` (this session) even when the unit was read; a Debian unit installed in an earlier session shows "Disks: untouched".
3. **m3** `enter_agent` connects with the page's sign-in (not `recovery_session`), so it cannot recognise an agent that is already up (M4).
4. **m4** `Open agent` waits up to 10 minutes with a fresh, never-set cancel flag: the user cannot cancel.
5. **m5** `RestoreOutcome.readback_verified` is a literal `true`; derive it from the tool's output and the state read afterwards.
6. **m6** Return to DSM on the Boot manager treats any earlier `Done` recovery result as "returned".

---

## 6. What must still be done before a release

**Release blockers:** B1 (Manage targets its own unit), B2 (no foreign flash image), B3 (host-key dialog timing).

**Before any beta:** M2, M3, M4, M5 (test with the real installer), M6.

**Engine or product follow-ups (not blockers):** a rescue SSH in the installer (DEV-UI-18); a live countdown (DEV-UI-11); backup detection if the "Test your backup" line is ever to say more.

## 7. What I could not verify

- Everything dynamic: I read code and screenshots, and ran no tests or app (build volume full). Findings B1, B2 and M4 are certain from the code paths; B3 and M6 depend on `russh` and `gpui-component` behaviour that I checked in the vendored source (`connect_stream` awaits kex) but did not observe at run time.
- Whether `/usr/local/sbin` is on the installer `PATH` (M5).
- Whether the real boot agent accepts `root` with the agent key as `recovery_session` assumes.
