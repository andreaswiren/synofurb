# UX critic: implementation fidelity of the desktop app, round 1

**This is an implementation-fidelity review, not a release-gate verdict.** I wrote nothing to `design/approvals/` and I give no approve or reject for the gate. The question is whether the implemented app does what the approved mockup (`design/mockups-v2/APPROVED.md`) and my round-5 findings say it must, and which gaps make it unsafe or misleading if it shipped as it is.

**Summary.** The pure decision layer is good: the erase gate, the risk Tag, the Recovery rows, the command-vs-plan match, the twin phrase. The screens built on it are not finished in the places where the owner cared most. I found **6 blocking findings, 7 major and 8 minor**. The worst pattern is that parts of the approved safety text were not carried over. The round-4 blocker (the agent that never answered, NB4-1) is fixed in the Tag, the status bar and the Recovery chips, but the Failed page still shows the old false promise in the red banner, in the "What happens next" dialog and in Help. Several buttons show a green success message after doing nothing.

Evidence: `crates/synofurb/src/ui/**` (read in full: `facts.rs`, `cli_cmd.rs`, `mgmt.rs`, `overlays.rs`, `screens/{confirm,failed,recover,manage,bootmgr}.rs`, the relevant parts of `app.rs`, `state.rs`, `render.rs`, `worker.rs`, `copy.rs`), the engine (`crates/synofurb-core/src/{session,domboot,script,remote,model}.rs`), `boot/dom/README.md`, `docs/GUI.md`, DEV-UI-9 to DEV-UI-14, the 27 `design/screenshots/v2-app-*.png` renders (I opened four: confirm, failed, recovery-agent, system; the rest I judged from the code). I did not run the app, and I did not get the unit tests to run: `cargo test --offline -p synofurb --lib ui::` was still compiling when I stopped it. Everything below is from reading the code and the pictures. File:line references are to the tree as I read it.

---

## 1. What is solid (verified in the code)

- **Erase gate.** `state::ConfirmGate` requires the plan to be valid, the three confirmations (tested backup, accept data loss, "This is <ip · …tail>") and the typed phrase. The phrase is case-sensitive; surrounding whitespace is trimmed (a pasted newline is fine). `App::start_install` (`app.rs`, "Start the installation") re-checks the gate itself, so a stray click cannot bypass it. Unit tests cover the open and closed cases, `Missing: ...` reasons and phrase progress (`state.rs` tests; I read them, I did not run them).
- **Enter on Erase.** `confirm.rs` wraps the button in a `div().capture_key_down` that calls `stop_propagation()` on Enter. Space and click start the install. DEV-UI-9 explains why there is no global Enter binding. I could not run it under Xvfb, so this is "correct by reading", plus the author's own note that it was verified with injected keys.
- **Twin suffix.** `confirm_phrase_for(base, serial, twin)` appends the last four characters of the serial. `twin_of_selected` (`app.rs:443`) finds another unit of the same model with a different serial and a different address, so it is symmetric (it works for either unit of the pair). The checkbox carries the identity (`This is 192.168.1.20 · …3456`).
- **Risk Tag and Way back are pure.** `facts::risk` takes `UnitFacts` and a `Where`. The order of rules is right: a failure after step 3 is never "safe"; an unconfirmed agent shows `BootUnconfirmed`. `Risk::way_back` returns the approved sentences, including "DSM may not return by itself: see Help, Recovery." for the unconfirmed case. A test keeps the copy module and the function in step.
- **Recovery rows and typed confirms.** `recovery_rows` says why a row is off (reasons match the mockup). `recovery_primary` highlights nothing on a running Debian unit, and with an unconfirmed agent makes Manual recovery the primary. Restore and Remove need `RESTORE FLASH` / `REMOVE ENTRIES` in a real dialog with a disabled button and a reason (`typed_confirm`).
- **"Matches plan"** is computed from the text (`CliCommand::matches` compares the parsed flags with the command regenerated from the plan). The test drops every flag in turn and checks the match breaks. No secret is on the line (`--prompt-password`). The confirmation flags appear only when a phrase is passed.
- **Update gate.** `mgmt::update_gate` returns `AllowedWithWarning` for a degraded array, blocked while updating, when up to date, or when the unit is unreachable. The gate is correct; what it gates is not (B3).
- **Backup line, honest.** The implementation did not copy the mockup's "No backup found. 6.2 TiB on volume1 is erased." It says "Test your backup." and its popover says "synofurb does not look at DSM's backup tasks; only you can confirm a backup." That is better than the mockup, because the engine has no backup detection (I searched `synofurb-core` for Hyper Backup, rsync tasks: nothing). The mockup claim must not be restored unless the engine learns to detect backups.
- **Twin and Keep pure functions, PONR handling, `Msg::Finished` mapping** all read correctly.

---

## 2. Blocking findings

Blocking means: shipping this build would show a user something false about their data or about how to get their NAS back, or would claim an action happened that did not.

### B1. The agent-never-answered fix (my NB4-1) is only half carried over

`unit=` any, `--screen failed-agent` (`state.rs:161`, fixture `Agent`), `screens/failed.rs`, `overlays.rs:78`, `copy.rs`.

What is right: `app.rs:681` sets `agent_unknown` from the engine error (`is_agent_timeout` matches `"boot agent did not appear"`, which is what `domboot.rs:322-325` produces), so the status bar shows "Boot unconfirmed", the Way back reads "DSM may not return by itself", and Recovery shows "Default: not confirmed" with Manual recovery as primary.

What is not carried over, all on the same Failed screen:
- The red banner is `Alert::error("fail-banner", err)` with the engine's own text, which is: "the boot agent did not appear within 10 minutes. **The NAS defaults to DSM: power-cycle it if it is stuck and it will boot DSM normally.**" (`domboot.rs:323-326`, and the demo uses the same string, `app.rs:538`). That is exactly the promise `boot/dom/README.md` says is false when the agent never runs ("the unit would keep booting the agent"). The mockup replaced it with "the boot agent did not appear within 10 minutes of the reboot; nothing was reset after the wait".
- "What happens next" always opens `DLG_NEXT_PRE` when the point of no return was not passed (`overlays.rs:79-85`; there is no agent branch). It says: "No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before. If DSM is not back after 10 minutes, power-cycle the NAS." In the unconfirmed case that is the round-4 false promise again, verbatim.
- Help: `T_TROUBLESHOOTING` (`copy.rs:608`) still says "**Installer did not come up**: DSM is already the default; wait, then power-cycle." `T_RECOVERY` (`copy.rs:598`) still says "every failure points it back at DSM" with no unconfirmed row. `T_BOOT_MANAGER` (`copy.rs:600`) has no never-ran paragraph and says 30 minutes (B7 below).
- No "Default: not confirmed" DSM tag, no "DSM may return in about 20 min or more" line, no "About the clock" popover, no "Wait for DSM" with a clock on the Failed page. The page shows the disks tag, the flash tag, the banner and the Manual recovery button; the subtitle for this state is "Waiting for the boot agent" (`failed.rs:35`), which reads as if something is still in progress.

**Must change before release:** (1) core: change the `bail!` text in `domboot.rs:323-326` to say nothing about DSM being the default (and ideally return a typed `AgentTimeout` error instead of matching a substring); (2) add an agent variant of "What happens next" (both outcomes, SYNOLOGY_2, the manual recovery) and use it when `agent_unknown`; (3) update the three Help topics as in the approved UX-SPEC (rows and numbers); (4) show the unconfirmed tag and the "20 minutes or more" line on the Failed page.

### B2. The Manual recovery dialog says something else than the README and the mockup

`copy.rs:496` (`DLG_MANUAL`), shown from Failed and Recovery.

Implemented text: "Power the NAS off, wait ten seconds, power it on. If DSM starts, you are done. If it does not, use the unit's console (VGA or serial) to restore the saved boot-flash image, or reinstall DSM from find.synology.com with the .pat file for your model."

What `boot/dom/README.md` ("Manual recovery") says, and the approved mockup follows: (1) use the boot loader menu on a console to choose `SYNOLOGY_2` for this boot, then run `synofurb-dom default dsm --yes` on DSM; (2) or take the boot flash to another Linux computer and restore the saved image with `synofurb-dom restore --yes FILE`. In the dialog:
- The first step is a power cycle with "If DSM starts, you are done", which in the never-ran case boots the agent again and sends the user round in a loop.
- The instruction that does solve the problem (choose SYNOLOGY_2 in the boot menu) is missing.
- "Use the unit's console to restore the saved boot-flash image" is not a README procedure: restoring the flash needs a working Linux with the flash attached, not the NAS console.
- There are no commands, no `sudo` (`synofurb-dom` exits with "must be run as root", script line 176), no note that both routes need physical access to the NAS, and nothing about the file's real name.

**Must change:** replace with the approved wording (and my round-5 N5-1 corrections: `sudo`, the real backup file name from "Saved here", `SYNOFURB_DOM=` when the tool cannot find the flash, "needs access to the NAS itself").

### B3. The Manage "Update" is a fake in real runs

`screens/manage.rs:39-40, 80-98, 519-535`, DEV-UI-13.

`start_mgmt_update` is not demo-gated. Its doc comment says "Demo: a scripted update ... Real runs: see the report ('Engine API')", but the code has no `self.demo` check. In a real run the Manage page shows constants (`UNIT_VERSION = "0.9.1"`, `APP_VERSION = "0.9.2"`) as "On unit" and "In this app", with an "Update available" tag, whatever the unit really runs. Pressing Update runs a scripted progress bar (Upload, Install, Restart, Health check) for about 3.5 seconds and then shows "Up to date" and "Updated". Nothing is sent to the unit. "Roll back" only sets a local variable back to `Idle`. DEV-UI-13 says "simulated in demo and not wired", which understates it: it is simulated everywhere.

The engine has what is needed: `synofurb_core::manage::push_update` (`manage.rs:1564`) and `push_rollback` (`manage.rs:1759`), used by `cli/manage_cmd.rs:361, 390`.

**Must be wired to the engine before release:** read the unit's real management version from the health report, take the app's version from the bundled payload, call `push_update` / `push_rollback` on a worker thread with their events, show real success or the real rollback, and remove the constants. Until then the Update button and the version row must not render in a real run (or must be labelled as not available), because they claim an update that never happened.

### B4. Recovery says "State read from the unit" but reads nothing from the unit

`screens/recover.rs`, `render.rs:148-160`, `app.rs:681`, `state.rs`, `copy.rs:477,495`.

`unit_facts()` is built from this session's memory: `bootmgr` job state, `install_started && dom`, `install.past_no_return`, `agent_unknown`, `result`. `synofurb_core::model::DomFacts` (the only boot-flash fact the engine returns on inspect) has no field for "our entries are on the flash" or "the default entry". So:
- A user who closes the app after a stuck or half install and reopens it gets, on the Recovery page, "Flash: clean · Default: DSM · Disks: untouched · **Nothing to recover. The unit is as it was found.**" for a unit that may be booting the agent or sitting in the DSM loader. The subtitle says "State read from the unit."
- `agent_unknown` is set once (`app.rs:681`) and **never reset**: not in `start_over` (`app.rs:1103`), not in `on_inspected`. After one failed install the tag "Boot unconfirmed" and the chip "Default: not confirmed" would follow the user to the next unit they select. `bootmgr` and `install_started` are reset in `start_over` but not when another unit is inspected, so facts about one unit leak to the next.

**Must be wired to the engine:** read the real state when the Recovery page opens (parse `synofurb-dom status` over SSH: our entries, the default entry, whether DSM's own files are present, the backup file); keep facts per unit (key by serial) and clear them on `start_over` and on selecting another unit. Until then the subtitle and the "Nothing to recover" line must go, because they assert a reading that was not made.

### B5. Buttons that report success after doing nothing

`screens/recover.rs:182-228` (DEV-UI-12), `screens/bootmgr.rs:185-195`.

- **Open agent** sets `RecResult { ok: true, title: "Run on this computer", detail: "The agent boots once and returns to DSM by itself." }` and renders a **green success Alert** plus the line `synofurb-cli boot-manager enter --host <ip> --user <admin>`. The NAS did not boot anything. It also repeats the "returns to DSM by itself" promise (true only if the agent runs).
- **Restore flash**, after the typed `RESTORE FLASH`, shows a green success Alert "Run on the unit: The engine restores the flash on the unit itself." and the command `synofurb-dom restore <cfg>/dom-backups/<backup>.img`. The sentence says the engine does it (it does not); "on the unit" is wrong, because the image path is on this computer and the restore is meant for a flash on another Linux computer; the command has a placeholder file name, no `--yes`, no `sudo`.
- **Remove entries**, after the typed `REMOVE ENTRIES`, shows a green success Alert and `synofurb-dom remove` (no `--yes`, no `sudo`).
- **Return to DSM** on the Boot manager page (`bootmgr.rs:190-193`) sets `bm_returned = true` on click, so the page immediately shows the green "Back in DSM. Our entries remain on the flash." The recovery job then runs `recover_to_dsm` with the DSM sign-in (`self.target(cx)`), which the agent does not accept (it takes the generated key). If it fails, nothing on this page says so, and even if it succeeds `recover_to_dsm` only resets the default; it does not reboot (`session.rs:440-446`: "reboot the unit to return to it").
- **Boot DSM** (`recover.rs`, `BootDsm`) starts the same job with no confirmation dialog (the mockup had "Boot DSM?") and reports the engine's output as success; the user is not told the NAS still has to be rebooted.

Typing a phrase in a dialog to get a command is also friction with no effect.

**Must change:** either wire these to the engine (the CLI already has `enter_agent`, restore and remove in `cli/boot.rs`; lift them into `synofurb-core`) or show them as what they are: a plain command (neutral tone, no "success", no "the engine does it") with the correct runner ("Run on the unit" vs "Run on another Linux computer"), real file names, `sudo` and `--yes` shown. "Back in DSM" must only appear after the unit answered as DSM; Boot DSM and Return to DSM must either reboot (an explicit step with its own confirmation) or say "Reboot the NAS to start DSM".

### B6. Keep-mode gate says everything is erased (my NB1 again)

`screens/confirm.rs`, backup `Alert` (`copy::CONF_ERASED`).

The Alert is built unconditionally: "Test your backup. **Everything on the disks is erased.**" The page does branch for Keep elsewhere (Destroyed: "40 GiB DSM system", After: "volume kept", Preserved lists the volume, `PLAN_PRESERVED_KEEP` popover). In Keep mode the most prominent line contradicts the Preserved card next to it. I blocked this in round 2 as NB1; the implementation reintroduces it (the new, honest "Test your backup." wording makes it worse only in that the sentence after it is the false one).

**Must change:** in Keep mode say what is at risk, for example "Your data volume is kept on a best-effort basis; it can still be lost." (the `CONF_ERASED` item needs a Keep variant).

---

## 3. Major findings

### M1. Quick setup is not the approved explicit choice

`overlays.rs:322-460`, `facts.rs`.
- `QuickView { sel: 0 }` renders `RadioGroup::selected_index(Some(self.sel))`, so the first option is **preselected** and "Use these" is enabled at once. The owner's decision (APPROVED.md): "explicit Hybrid vs RAID 6 choice", my round-4 check: nothing preselected, Continue disabled until chosen.
- The two options read "Hybrid 1-disk · 19.85 TiB" and "**Auto** · 7.15 TiB". "Auto" is not "RAID 6 · survives 2 disks"; the trade-off the choice is about (survives 1 against 2) is not shown. The line "Hybrid is built by synofurb; DSM may not adopt it." is missing (`QUICK_*` has three items).
- Which option is first depends on `quick_defaults`: Hybrid when it gives more than 20 % extra space, so the preselected default is the layout the project says nothing has verified on a real unit.
- Quick setup does not show or set the kernel (see M2) or the boot method (`self.sys.boot = Auto`).

### M2. The kernel defaults to Newest

`app.rs:879` (`self.form.kernel = KernelChannel::Newest;` in `on_inspected`), `screens/system.rs:127-130`. The spec (F3) and the approved Quick setup both say "Stable" by default; the System tab's segmented control starts on "Newest" and the command says `--kernel newest`. Newest installs Linux 7.x from trixie-backports on a model nobody has run (docs/BOOT.md: "None of it has run on a real Synology yet"). Either change the default to Stable or defend it in the design.

### M3. After step 3 the app promises a rescue shell the installer does not have

`screens/failed.rs` (`RESCUE_TIP`, `FAIL_RESCUE_IN`), `copy.rs` (`DLG_NEXT_WIPE/KEEP`), `crates/synofurb-core/src/script.rs:88-120`.
- The failure handler in the generated installer (`fail()`) sets the default to DSM, then `sleep 900` and `reboot` (DOM units), or "Installation HALTED" forever (other methods). There is no SSH or rescue shell in the script. The approved mockup's Rescue panel (ssh command, password, notes) has no engine behind it (DEV-UI-14 admits "a real rescue shell" is not built).
- The page therefore says "Rescue shell closes in: 15 min", the dialog says "Export diagnostics first, then **open the rescue shell**", and the primary button "Open rescue shell" is a disabled button whose reason says "Not available in this version; use the NAS console (VGA or serial)". The product's target user "has only SSH". There is no answer to "what do I do now?" between failing and the 15 minute reboot except export diagnostics.
- The log excerpt on the same page shows the engine line (`script.rs:103`) "Disks were already modified. The NAS reboots in 15 minutes into the DSM loader (recovery / reinstall DSM **keeps data volumes**)". That is a data promise, in the app's own log, on the screen that must make none.
- The static labels "DSM returns in 60 s" and "15 min" (`failed.rs`, no check of the boot method) are only true for the boot-flash method; for kexec and kit installs the script halts forever, so nothing reboots by itself.
- **Must:** decide: either add a real rescue SSH to the installer and wire its address and one-time password into the event stream, or remove "rescue shell" from the copy and the dialog and say what is true (log, export, wait 15 minutes, DSM loader). Fix `script.rs:103`. Show the 60 s and 15 min lines only when the unit used the boot-flash method.

### M4. "Wait for DSM" does not wait

`screens/failed.rs`, `start_recover`.
The pre-step-3 primary button runs `recover_to_dsm`, an SSH sign-in to the unit's DSM address. While the unit reboots into DSM that fails and a red engine error appears. None of the approved after-the-clock states exist: waiting, "DSM is back", "DSM did not return. Power-cycle", "The NAS is in the DSM loader". Only the static label "60 s" is shown. The 10 minute rule in the dialog cannot be acted on. **Wire:** a DSM probe (TCP 22 and 5000 or mDNS) with a timeout, driving those states.

### M5. No SSH host-key verification

`crates/synofurb-core/src/remote.rs:44-57`, DEV-UI-14, `screens/connect.rs`.
The handler accepts every server key and only records the fingerprint ("Trust-on-first-use: the fingerprint is shown to the user and written to the log"). It is not shown before the sign-in, not stored, and not compared next time. The DSM administrator password (and the sudo password) go to whatever answers at that address. A changed key can never be reported, so the approved "Host key changed" block cannot exist. **Wire:** store the fingerprint per unit, show it with the "Trust this NAS?" dialog before the password is sent, block on a mismatch.

### M6. The Failed page cannot tell "nothing changed" from "flash changed"

`render.rs:154`: `flash: agent || (install_started && dom) || agent_unknown`. A failure during the boot-flash backup (backup does not verify, nothing written) shows "Flash changed" and "Failed safely", and Recovery offers Restore and Remove for entries that do not exist. The approved `failed-backup` says "Boot flash: Untouched, DSM: Running". Conservative, but not true; the engine event stream needs a "first flash write" event.

### M7. Timing numbers disagree inside the app

`copy.rs:233` (`GOAL_BOOTMGR`: "30 minute watchdog") and `copy.rs:601` (`T_BOOT_MANAGER`: "after 30 minutes") against `copy.rs:458,466` (`BM_LOGIN_TIMEOUT`, `BM_WATCHDOG`: 60). Engine: boot manager arms the agent with `timeout_min: 60` (`session.rs:422`), the install flow with 30 (`session.rs:330`), the app waits 10 minutes for the agent (`session.rs:362`). The Boot manager page is right; the Goal card and the Help topic are wrong. Also missing: any "30 minutes from when it started, 20 minutes or more remain" arithmetic on the unconfirmed screen (B1).

---

## 4. Minor findings

1. The After line prints the *request* ("Auto") rather than the resolved level ("RAID 5"): `confirm.rs` uses `raid::raid_label(plan.request.data_raid)`. Show the plan's actual level.
2. The default demo unit is shown with the compat Tag "Verified" in every screenshot (`v2-app-*.png`); `docs/BOOT.md` says nothing is verified. A "Demo" tag is in the status bar, but screenshots that go to the design gate should use "Untested".
3. The Done "Next steps" copy still says "check BIOS boot order" (`copy.rs:401`); the approved copy is "Reboot once to check it starts" (BIOS order does not apply to boot-flash units).
4. Time zone defaults to `UTC` (the mockup detected it from the computer).
5. The destroyed count in Keep mode is `candidate_disks().len() * 10 GiB` (`confirm.rs`), a constant per disk, not read from the unit.
6. The subtitle "Waiting for the boot agent" on the stopped screen (B1) and the missing phase ("Phase 4 of 7") from the approved text.
7. The 24 px info and copy buttons are DEV-UI-3 (known). The agent-unknown Failed page has no Copy for the manual commands.
8. Test gaps: Enter-on-Erase and the Failed/Recovery copy have no automated test; `facts.rs` tests check the labels, not the dialogs, which is how B1 got through. Add a test that no copy reachable when `agent_unknown` contains "boots DSM" or "DSM starts as before".

---

## 5. Your specific questions

- **Erase gate:** correct (section 1). Remaining issue is the Keep wording (B6).
- **Risk Tag and Way back as pure functions, incl. the agent-not-confirmed case:** correct and tested in `facts.rs`; the inputs are the weak part (B4: not read from the unit, never reset).
- **Recovery rows and typed confirms:** logic correct; the effect is not (B5).
- **Manual recovery wording versus boot/dom/README.md:** does not follow it (B2).
- **Timing constants versus `session.rs`:** Boot manager screens use 60 (correct). The Goal card and the Boot manager Help topic say 30 (wrong). The 10 minute wait is only in the engine text and the demo string; 20-minute remainder is not shown (M7, B1).
- **Quick setup defaults and the explicit choice:** not as approved (M1, M2).
- **Command-vs-plan "Matches plan":** good; computed, tested, fixture-style. One residual: the command is generated from `CliCommand`, which is filled from form fields; nothing proves that `build_job` and `CliCommand` read the same fields (the default kernel makes this visible: both say Newest).
- **Manage update warn-but-allow:** the gate is right; the update itself is fake (B3).
- **DEV-UI-12 (commands only):** misleading, because of green success messages and wrong runner text (B5). Shown honestly it would be acceptable for a first release.
- **DEV-UI-13 (update simulated):** unsafe, see B3.
- **DEV-UI-14 (no host-key dialog, no keychain, no rescue shell):** no keychain: fine (nothing in the UI pretends to save passwords). No host-key trust: security gap (M5). No rescue shell: acceptable only if the copy stops promising one (M3).

---

## 6. What must be wired to `synofurb-core` before a release

1. **`domboot.rs`:** a typed agent-timeout error and a message that says nothing about DSM being the default (B1).
2. **Unit state read** on Recovery open (and per unit): our entries, default entry, DSM files present, backup file path (`synofurb-dom status`); store facts per serial (B4, M6, with an event for "first flash write").
3. **Recovery actions as engine functions:** enter agent, restore flash, remove entries, boot DSM with an explicit reboot (lift `enter_agent`, restore and remove out of `cli/boot.rs`) (B5).
4. **Management update:** `push_update` / `push_rollback` on a worker, the unit's real version from the health report (B3).
5. **Wait-for-DSM probe** with a timeout, driving the after-the-clock states (M4).
6. **Rescue:** either an SSH rescue in the installer script with its address and one-time password in the events, or delete the claims (M3); fix `script.rs:103`; decide the non-DOM halt behaviour.
7. **SSH host keys:** per-unit pinning and a trust step before the password is sent (M5).

## 7. What I could not verify

- Whether the Enter wrapper behaves under real key events, and the test results (compilation did not finish).
- The screenshots only cover a subset of states: no render of the Failed screen in the agent-unknown state, the Manual recovery dialog, Quick setup with mixed disks, Keep-mode Confirm, or the Manage update row mid-run. I judged those from the code.
- Whether the engine's installer image contains `synofurb-dom` (the failure handler checks `command -v synofurb-dom`); if it does not, the "60 s" and "15 min" reboots never happen on the boot-flash path either.
