# UX critic: implementation fidelity of the desktop app, round 3

Implementation review only. No gate verdict, nothing written to `design/approvals/`, no code edited.

Scope: the round-2 findings in `ux-critic-impl-round2.md` (3 blocking, 6 major, 6 minor) re-checked against the latest push (engine `31827b2`, GUI up to `00c1fc3`). I read the code named in the brief and ran a subset of tests with `CARGO_TARGET_DIR=/tmp/gpui-probe-target`:

| Run | Result |
|---|---|
| `synofurb-core` lib: `remote::`, `hostkey::`, `script::` | 30 passed |
| `synofurb-core` lib: `recovery::` | 16 passed |
| `synofurb` bins/lib: `mgmt::`, `trust::`, `facts::` | 31 passed |

I did not re-run the full workspace, clippy, `smoke.sh` or the gate/mgmt suites: I rely on the coordinator's numbers for those. I did not run the GUI, so everything about rendering and dialog refresh is read from code, not observed. No hardware.

## Verdict in one paragraph

All three round-2 blockers are fixed in code, and the fixes are real, not cosmetic. Reading the fixes more closely found **no new hard blocker** but **two major findings** in the same families as the old blockers: results of Manage actions are not tied to the unit that produced them, and Restore flash checks the image against what the UI remembers about the unit, not against the unit it is connected to. The second one is cheap to fix and I recommend fixing it before a release, because it is the last gap between "the dialog names unit A" and "the flash of unit B is written". Everything else is minor or an accepted limit.

---

## 1. Round-2 findings: status

| Round 2 | Status | Evidence |
|---|---|---|
| **B1** Manage bound to the Connect form's host | **Fixed** | `mgmt.rs::manage_target_for(addr, user, secret)` takes the managed unit's address and a sign-in typed on the Manage page (`mg_user`, `mg_password` / `mg_key_path`); the Connect form is not an input. `App::target()` returns `manage_target()` when `manage_bound()` (Manage, or Recovery opened from it), else `form_target()`. `open_manage` binds the unit with `ManageBinding::bind`; binding another unit clears report, error, update state, unit-session state and the password field. The page shows a sign-in form ("Read health") instead of "Unreachable" when nothing was typed. Tests: `manage_actions_target_the_unit_on_screen_never_the_connect_form`, `binding_another_unit_forgets_everything_of_the_first`. Residual: M1 below. |
| **B2** Restore flash used the newest image of any unit | **Fixed at the dialog and in the engine** | `recover.rs` uses `backups_for_unit(&backups, &id)` (same serial, nothing else; `Unknown` ownership is never offered) and no longer falls back to `latest_backup`. Without a serial the row says so (`REC_NO_SERIAL`, "use Manual recovery"). The typed phrase is `RESTORE FLASH <model> <last 4 of serial>`; the dialog shows model, serial, file name, date and size of the unit and the image. The engine re-checks in `restore_flash_for_unit_on` → `check_backup_belongs(.., allow_other_unit=false)` (the GUI never sets it) before anything is uploaded. Tests: `two_units_of_the_same_model_never_see_each_others_backups`, `restore_for_a_unit_refuses_another_units_image_without_running_a_command`, `the_restore_phrase_names_the_unit`. Residual: M2 below (the "unit" is the UI's idea of the unit). |
| **B3** Host-key dialog vs 20 s connect timeout | **Fixed** | `remote.rs`: the 20 s is no longer a wall-clock `timeout` around `client::connect`. `ConnectClock` counts only TCP and handshake time; `check_server_key` calls `begin_wait()` / `end_wait()` around `policy.decide`, so the time a person spends comparing a fingerprint is not connect time. A first-time key is pinned only after the handshake finished on a live connection (`pending_pin`); a refusal, an answer after `answer_timeout` (`HostKeyError::AnswerTimeout`, the late answer is thrown away) or a connection that died during the question pins nothing. Tests I ran: `the_connect_clock_stands_still_while_a_person_answers_and_the_key_is_pinned` (1.2 s answer, 0.4 s budget, pinned), `no_answer_refuses_and_a_late_trust_pins_nothing`, `the_connect_budget_still_applies_to_a_server_that_never_answers`, `a_refused_host_key_closes_the_connection_before_the_password_is_sent`. The 180 s of DEV-UI-19 is now reachable. |
| **M1** 180 s unreachable | **Fixed** (same cause as B3) | See B3. |
| **M2** Open agent on Debian leaves DSM as default | **Fixed** | `DLG_AGENT_DEBIAN` says the agent makes DSM the default, that after the watchdog or any reboot the unit starts DSM's loader instead of Debian until Debian is chosen again from the agent, and that Debian stays on the disks. `run_recovery_row` picks it when `unit_facts().debian`. |
| **M3** Manual recovery route 2 needed a tool the other computer lacks | **Fixed** | `manual_recovery_commands` gives three lines: `synofurb-dom default dsm --yes` (route 1), `SYNOFURB_DOM=/dev/sdX synofurb-dom restore --yes <image>` (route 2 with the script) and `sha256sum -c` + `lsblk` + `dd ... && sync` (route 2 without it). The text says "with synofurb-dom there ... without it the third", "check twice", "Both ways need physical access". Matches `docs/RECOVERY.md`, "Manual recovery, step by step". Two small gaps: m1, m2 below. |
| **M4** Open agent on a unit that already runs the agent | **Fixed** | `UnitFacts.agent_running` comes from the unit's own `UnitBootState.running == Agent`; the row is off with "The agent is already running. Open a shell on it instead." (test in `facts.rs`). |
| **M5** failure-reboot numbers depend on `synofurb-dom` being on the installer PATH | **Fixed** | The script looks for the tool in absolute directories first (`/usr/local/sbin`, `/usr/local/bin`, ...) and only then `command -v`. On a boot-flash plan it has a preflight before anything is touched: no tool means `fail "preflight: ... nothing was changed"`. `failure_handling_with_tool(plan, installer_has_dom_tool)` returns "reboots by itself: 60 s / 15 min" only for a boot-flash plan whose installer image contains the tool, else "halts"; `installer_has_dom_tool()` is tied to `INSTALLER_DOM_TOOL` and the embedded script. Tests: `the_failure_handler_finds_synofurb_dom_by_absolute_path_not_only_on_path`, `a_boot_flash_install_refuses_to_start_when_the_tool_is_missing_and_the_numbers_follow`, the fake-tool run asserting `default dsm --yes`, `sleep 60`/`sleep 900`, `reboot -f`. Not exercised on a real installer (accepted limit). |
| **M6** Quick setup footer may stay disabled | **Probably fine, not observed** | The footer's `unchosen` is still computed in the dialog builder from `view.read(cx).sel`. In GPUI an uncached view is re-rendered on every draw, and the dialog layer is part of the root render, so the builder reruns and the button state follows the radio. The same assumption carries the new host-key dialog (which also has a once-a-second ticker). I could not confirm it without opening the window. Keep one manual check on the first real run: pick a layout, the button must enable. The click handler itself is safe either way (`let Some(sel) = ... else { return }`). |
| m1 host-key dialog stays open after the attempt failed | **Partly fixed** | `TrustWatch` ends the question; the dialog shows `HK_ENDED` and disables Trust (`over`), and `on_msg` ends all open questions on `Inspected`, `ManageHealth`, `UnitState`, `Finished` errors. Not on `RecoveryDone`, `MgmtDone`, `BootManagerDone` or `DsmWaitDone` failures: n3 below. |
| m2 Disks chip from session memory | **Open** | `render.rs::unit_facts` still sets `disks: self.install.past_no_return` in both branches. n4 below. |
| m3 `enter_agent` uses the page sign-in | **Moot** | The case it mattered for (agent already up) is gated off by M4's fix. `enter_agent` also reads the connected unit's serial itself for the agent-key name, which is the right pattern (see M2 below). |
| m4 Open agent has no cancel | **Open** | `spawn_recovery` still passes a fresh `AtomicBool::new(false)` to `enter_agent`. n5 below. |
| m5 `readback_verified` is a literal | **Open** | `restore_flash_on` still sets `readback_verified: true` once the tool exits 0 (the tool does its own read-back). n6 below. |
| m6 Return to DSM treats any `Done` as returned | **Open** | `bootmgr.rs:177` `returned = rec_result.level == Done`. n7 below. |

---

## 2. Your specific checks

- **Connect clock vs host-key answer time.** Correct as described under B3. One point worth knowing: `decide` blocks inside russh's session task for up to the answer time. The outer `block_on(select!)` runs on the calling thread, so the watchdog still ticks (every 20 ms) and, by design, does not count the waiting time. If the server closes the connection while the person is thinking (sshd's `LoginGraceTime` is typically 120 s, shorter than the 180 s answer time), the person can still press Trust, the handshake then fails and nothing is pinned. That is safe; the user sees a connection error and the dialog is closed by their own click. Acceptable.
- **`ui/trust.rs`.** Closing the dialog, Cancel, Escape, a dropped `TrustAsk` and a timeout all refuse; `TrustAsk::answer` returns false when the attempt is over; the countdown is the engine's number (`timing::HOST_KEY_ANSWER_TIMEOUT_SECS`), not a second constant. Good.
- **Manage binding.** Fixed for the actions. Not fixed for results: M1.
- **Restore phrase, identity, `backups_for_unit`.** The phrase and the backup selection are right. The identity they use is not read from the connected unit: M2.
- **Open agent gating.** Right for running agent (M4), Debian (M2), no flash. The row is also off with "No agent on the flash. Set it up under Boot manager." when nothing of ours is on the flash.
- **Manual recovery vs `docs/RECOVERY.md`.** Same two routes, same three commands, same warnings. Differences: the dialog does not say where `synofurb-dom` lives on DSM or how to get it (`/usr/local/bin/synofurb-dom` if the app installed it, otherwise from the repository), and it says "Your disks are untouched." as a statement of fact (n1, n2).
- **Quick setup.** Nothing preselected for mixed disks; "survives N" and the Hybrid caution; "Newest kernel" stated in the summary; "Use these" gated with a reason. See M6 above.
- **`failure_handling_with_tool` and the script's lookup.** Consistent with each other and with the Failed page (`fail_clock()` reads `failure_handling(plan)`). On a boot-flash plan the missing tool is caught in a preflight before step 1, not after the disks are touched; the installer then halts, which is safe because the agent already set the default to DSM when it started.

---

## 3. Major findings

### M1. Manage results are not tied to the unit that produced them

`worker.rs` (`ManageHealth`, `MgmtDone` carry no unit), `app.rs::on_msg`, `app.rs::target`.

The actions are bound to the right unit, but the answers come back anonymous:
1. Start "Check again" or "Update" on unit A, go Back, open unit B: `open_manage` resets the page, then A's `ManageHealth` / `MgmtDone` arrives and is written into B's page. B's page can show A's health, or "Updated" / "Update failed" with A's headline.
2. `Msg::MgmtDone` ends with `read_manage_health(cx)`, which calls `target()`. Nothing locks navigation during an update. If the user has left Manage by then, `manage_bound()` is false, `target()` falls back to the Connect form, and the health read goes to the **Connect form's host with the Connect form's credentials**, and the answer is stored as the managed unit's report. Read-only, and to a host the user typed, but it is the old B1 failure mode in a narrow form.

Fix: tag these messages with the unit key (`ManageBinding`) and drop a result whose key is not the bound one; read health after an update with the target captured when the update started, not with `target()`; or keep the user on Manage while it runs. Test: bind A, send a result tagged B, assert it is ignored.

### M2. Restore flash trusts the UI's idea of whose unit it is, not the unit it is connected to

`screens/recover.rs::run_recovery_row` (RestoreFlash) → `app.rs::unit_id` → `worker.rs::spawn_recovery` → `restore_flash_for_unit_on(.., id, ..)`.

`unit_id` is `(model, serial)` of `self.facts` (the last inspected unit; or the managed unit on a Manage-opened Recovery) plus the Connect form's host. The SSH target is `self.target()`, which reads the Connect form's host again at click time. `facts` is not cleared when the host field changes. So, if the address is edited after an inspect and the user reaches Recovery without inspecting again (Goal, Failed and Boot manager all link to it), the dialog names unit A's model and serial, the typed phrase names A, `backups_for_unit` offers A's image, the check passes against A's identity, and the image is written to whatever answers at the new address. The CLI's `restore_flash` does it right (it runs `inspect_remote` on the live session and builds the identity from that), and so does `enter_agent`. The GUI path bypasses that read.

This needs a stale-state sequence, so I do not call it a blocker, but it is a flash write on a twin-unit product and the fix is small: in the `Restore` branch of `spawn_recovery`, read the serial and model from the connected session (as `restore_flash` does), refuse if the serial differs from the one the dialog used (or use the live identity for `check_backup_belongs`), and say so in the result. Also clear or re-validate `facts` when the Connect host or port changes.

---

## 4. Minor findings

- **n1** Manual recovery's first command assumes `synofurb-dom` is on DSM. Say where it is and that route 2 needs no tool.
- **n2** "Your disks are untouched." in `DLG_MANUAL` is true of the recovery steps, not of the unit after a failure past the point of no return. Reword to "These steps do not touch your disks."
- **n3** The host-key question is not ended by failures delivered as `RecoveryDone`, `MgmtDone`, `BootManagerDone` or `DsmWaitDone`; Trust stays offered until the answer time runs out. Harmless (nothing is pinned for a dead connection) but confusing.
- **n4** The Recovery "Disks" chip is session memory (`install.past_no_return`) even when the unit's boot state was read.
- **n5** Open agent waits up to the watchdog time with a cancel flag nobody can set.
- **n6** `RestoreOutcome.readback_verified` is a constant `true`; derive it from the tool's result and the state read afterwards.
- **n7** Return to DSM on the Boot manager accepts any earlier `Done` recovery result as "returned".
- **n8** `find_local_backups(&backups, &unit)` (name only) is still used for the "Show command" text and the Manual recovery dialog. With a serial it is accurate. With an address name it can pick an image another unit took at that address earlier, and with an empty unit key it returns every backup, newest first. These are texts the user copies to another computer where no engine check runs; use `backups_classified` and show the image's owner, or list nothing when the unit is unknown.

---

## 5. Release status

**Release blockers (my view): none open.** B1, B2 and B3 are fixed and tested; the other round-2 items are fixed or minor.

**Fix before release (small, same families as the old blockers):**
1. M2 above: read the identity from the connected unit before restoring.
2. M1 above: ignore Manage results of another unit and do not read health through `target()` after an update.

**Before a wider beta:** n1, n2, n8 (texts and the name-only matcher); n3 to n7 as time permits.

**Accepted limits, not blockers if stated in the release notes and Help:**
- **No installer rescue shell (DEV-UI-18).** After step 3 of an install there is no retry and no way in for an SSH-only user. The pages now say what happens (60 s / 15 min reboot on boot-flash units, a halt elsewhere, "no rescue shell"), so this is a documented limit. Release notes should say plainly: after the point of no return the way back is reinstalling DSM.
- **No keychain (DEV-UI-20).** Passwords live in memory and are retyped; Manage asks for its own sign-in each session. A usability cost, not a safety one.
- **No real-hardware run.** Things I could not and the tests do not prove: that the real boot agent accepts `root` with the agent key as `recovery_session` assumes; that a real installer image contains `synofurb-dom` where the script looks (the tests use a fake tool); how a real DOM behaves under `restore`; GUI refresh behaviour (M6). Treat the first hardware run as the test of these, and do it on a unit whose boot flash image has been saved.

## 6. What I could not verify

- Everything dynamic in the GUI (dialog refresh, countdown repaint, Manage page behaviour): code reading only.
- The full test suite and `smoke.sh`: taken from the coordinator's report.
- M1 and M2 above are certain from the code paths; whether a user reaches them depends on navigation I did not click through.
