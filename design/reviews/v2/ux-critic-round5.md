# UX critic, round 5: desktop mockup v2

**Verdict: APPROVE** (UX, for the desktop mockup as a design). I found no blocking findings in the 107 states. The round-4 blocker (NB4-1) is fixed and every N4 item is fixed. I did find real defects in the manual-recovery wording and in the documents. None of them is an over-promise in the mockup, so I list them as non-blocking. Several of them are quick fixes that should land before the real application is reviewed.

This is my verdict only. I wrote nothing to `design/approvals/`. Approval of the mockup does not approve the real application, which `BRIEF-v2.md` makes the review artifact.

Reviewed: `design/mockups-v2/desktop/` (`index.html`, `screens.json`, 107 states, `src/`), `design/v2/UX-SPEC.md`, `docs/RECOVERY.md`, `docs/BOOT.md` (both updated), `boot/dom/README.md` and the `boot/dom/synofurb-dom` script, `boot/agent/docs/safety-model.md`, and `crates/synofurb-core/src/session.rs`. I walked the prototype with Playwright (live clocks, dialogs, popovers, keyboard, DOM state). I did not read other reviewers' files. I did not edit the mockup.

Checks: `check_flows.js` 158 passed, 0 failed. `check_words.js` all 105 states within budget (max 59 words). I did not recompute contrast or the type scale.

Reproduce with `design/mockups-v2/desktop/index.html?t=<unique>&nofirst=1&bare=1&<params>#<screen>`.

---

## Round-4 blocker: fixed

### NB4-1. `failed-agent` over-promised DSM recovery: **Fixed**

I walked `failed-agent`, `failed-agent-waiting`, `failed-agent-back`, `failed-agent-notback`, `failed-agent-manual`, `failed-agent-next` and `recovery-agent` (`unit=ds918&facts=agent`).

| Element | Round 4 | Now |
|---|---|---|
| DSM tag | green "Default" | "Default: not confirmed" (question mark, tooltip: "We cannot tell if the agent ran; if it did not, a power cycle boots it again.") |
| Subtitle line | "DSM is untouched." | "Data untouched." |
| Status bar | "Failed safely" | "Boot unconfirmed" (also in `-waiting`, `-notback`, `recovery-agent`; `-back` shows "Failed safely" only after "DSM is back" is observed, DSM tag "Running") |
| Way back | "DSM boots again by itself." | "DSM may not return by itself: see Help, Recovery." |
| "What happens next" (80 words) | power-cycle: "it boots DSM" | "The app waited 10 minutes and cannot tell what happened. If the agent started, its watchdog reboots the NAS into DSM, 20 minutes or more from now. If it never started, the NAS may keep booting the agent after every power cycle. Then choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer. DSM is not confirmed. Your disks are untouched. See Help, Recovery." with Manual recovery button |
| Clock | "DSM returns in up to 20:00" | "DSM may return in about 20 min or more", 20:00; the popover says: "watchdog counts 30 minutes from when it booted, 1 to 3 minutes after the reboot command. The app already waited 10, so 20 minutes or more remain: an estimate, not a promise. If the agent never started there is no watchdog and DSM does not come back by itself." |
| `notback` | "DSM did not return. Power-cycle the NAS." | "DSM not back. The agent may never have run. Use the manual recovery." Footer: Recovery options, Manual recovery, Check again |
| Engine message | "within 180 s" | "the boot agent did not appear within 10 minutes of the reboot; nothing was reset after the wait (watchdog: 30 minutes after the agent booted, if it ran)" |
| Recovery screen | n/a | `facts=agent`: chips "Flash: ours present", "Default: not confirmed", "Disks: untouched" and a note "The agent may still be the default. If DSM does not return, use the manual recovery." with a Manual recovery button |

I checked the numbers against the engine and the safety model. The 10 minute wait is `session.rs:362`. The 30 minute install watchdog is `session.rs:330`. The 60 minute boot-manager watchdog is `session.rs:422`. The watchdog counts from agent boot (`safety-model.md`: "AGENT_TIMEOUT_MIN after boot"). With the agent booting 1 to 3 minutes after the reboot command, 30 − (10 − b) = 20 + b minutes remain, so "20 minutes or more" is correct. The never-ran case matches `boot/dom/README.md` ("If the agent never runs, the unit would keep booting the agent").

Help carries it too. Help > Recovery has the row "Agent silent after 10 minutes: Not confirmed. Agent started: it reboots to DSM, 20 minutes or more later. Never started: the default stays the agent and a power cycle loops. Use SYNOLOGY_2 in the boot menu (serial or VGA console), or restore the flash from another computer." Troubleshooting and Boot manager topics say the same. `docs/RECOVERY.md` now has the exception and the timings in the "Agent installed, armed" row (30 install, 60 boot manager, 10 app wait, counted from agent boot). `docs/BOOT.md` line 58 has the timings and the never-ran pointer. UX-SPEC has the variant (rows 14 and 15 of D9, the popover, the dialog, states).

### Manual recovery wording against `boot/dom/README.md`

I compared the `Manual recovery` dialog (footer and Recovery screen; 66 words) with the "Manual recovery" section of `boot/dom/README.md` and the script.

| Dialog text | README | Verdict |
|---|---|---|
| "On a serial or VGA console, choose SYNOLOGY_2 in the boot menu, then run the first command on DSM." | Item 1: "Use the boot loader menu to choose `SYNOLOGY_2` for this boot, then run `synofurb-dom default dsm --yes` or `synofurb-dom remove --yes` on DSM." | **Correct.** README does not name serial or VGA; that is the only way to see the GRUB-style menu, and the dialog's addition is reasonable. It leaves out the `remove --yes` alternative, which is fine. |
| "Or take the boot flash to another Linux computer and restore the saved backup with the second command." | Item 2: "Restore a backup ... `synofurb-dom restore --yes FILE` with `SYNOFURB_DOM=/dev/sdX`" and "Everything below can be done from any Linux box with the DOM attached (a DOM is a normal 128 MB USB flash on the mainboard header; a USB adapter works)" | **Correct in substance; incomplete in three details (N5-1).** |
| `synofurb-dom default dsm --yes` | `cmd_default` | Correct, but see "needs root" in N5-1. |
| `synofurb-dom restore --yes synofurb-dom-backup-*.img` | `cmd_restore` takes exactly one file | The wildcard is the right shape for the engine's file name (`synofurb-dom-backup-<unit>-<stamp>.img`, `domboot.rs:65`) and fails safely if it matches two files (`usage: restore FILE`). The Recovery screen's "Saved here" shows `dom-backups/DS918plus-20261101-1402.img`, which is not the engine's name (N5-1). |

So "choose SYNOLOGY_2 in the boot menu" and "restore the flash from another computer" are what the README says. The dialog does not say they need physical access, which is not obvious to an SSH-only user.

---

## Status of the round-4 non-blocking items

| ID | Status | Evidence |
|---|---|---|
| N4-1 clock inequality | **Fixed** | See above. The popover shows the arithmetic. The UX-SPEC D9 states line matches. |
| "Phase 4 of 7" (asked by author) | **Fixed** | `failed-agent` says Phase 4 of 7; the live install labels "Reboot into the agent" and "Send installer" Phase 4 of 7 ("Booting installer"), and "Boot flash backup" and "Boot agent added" Phase 3 of 7 ("Staging"). |
| N4-2 RECOVERY.md numbers | **Fixed** | See above. Two absolute sentences were left behind: N5-2. |
| N4-3 Quick-setup radios | **Fixed** | Focus opens on the first radio; the group is `role=radiogroup` with roving tabindex; ArrowDown moves and selects (`["false","true"]`); Space selects; Tab order Cancel, Customize, then the radios; focus trap holds; Continue is disabled until a layout is chosen, with the reason "Choose a layout first." Nothing is preselected. |
| N4-4 Recovery on a Debian unit | **Fixed** | `facts=debian` and `facts=flash,disks,debian`: no primary button; Remove entries disabled. |
| N4-5 After line below the gate | **Fixed** | At 960x640 Largest the order is cards (y 355), After line (458), gate (517), disk list (886). At 1280x800 cards, After and gate are side by side. |
| N4-6 Recovery accessible names | **Fixed** | Buttons now read "Boot: Boot DSM", "Start: Open agent", "Restore: Restore flash", "Remove: Remove entries", "Open guide: Reinstall DSM"; info buttons "About Boot DSM" and so on. |
| N4-7 carried | Unchanged, declared | "Recommended" on Boot manager (needs a test); 24 px info and copy buttons (`DEV-UI-3`); suffix only for detected twins (declared). |

---

## New non-blocking findings

**N5-1. Manual recovery commands as written can fail or mislead (Medium).** `unit=ds918&fail=agent&dlg=manualrec`, `unit=ds918&facts=agent#recovery` (button Manual recovery).
- Both `synofurb-dom` commands need root (`need_root: "must be run as root"`, script lines 175-177, 1280, 1610). The dialog shows them without `sudo`. A DSM admin over SSH will get "must be run as root" at the worst moment. Show `sudo synofurb-dom ...` for the DSM command.
- `restore` without `SYNOFURB_DOM` auto-detects the flash only if it still looks like a Synology DOM; if it does not, it dies with "restore needs the whole device: set SYNOFURB_DOM" (script line 1635; README says to set it and to check twice that the device is the 128 MB flash). Name that in the dialog.
- The dialog uses `synofurb-dom-backup-*.img`, while the Recovery screen's "Saved here" lists `dom-backups/DS918plus-20261101-1402.img`. The two do not agree with each other or with the engine's `synofurb-dom-backup-<unit>-<stamp>.img` (`domboot.rs:65`). Show the real file name from "Saved here" in the dialog. Copy commands should paste it.
- Both routes need hands on the NAS (a serial adapter or screen for the boot menu, or the boot flash connected to another computer). The dialog does not say so. The product's target user has "only SSH". Say "This needs access to the NAS itself" in the first line.
- "Take the boot flash to another Linux computer": `boot/dom/README.md` says the DOM is "a normal 128 MB USB flash on the mainboard header; a USB adapter works"; `docs/BOOT.md` line 14 says it is "soldered to the mainboard". The two documents disagree; the dialog copies the first. Reconcile the documents.

**N5-2. Two absolute sentences in `docs/RECOVERY.md` and one in `docs/BOOT.md` now contradict the exception (Medium; documents, not the mockup).**
- `RECOVERY.md` line 9: "every failure path points the default back at DSM".
- `RECOVERY.md` lines 31-32: "powering it off and on again is always a valid recovery from the pre-point-of-no-return phases". The agent-never-ran case is a pre-point-of-no-return phase and a power cycle loops there.
- `BOOT.md` line 44: "every failure path sets the default back to DSM".
- The UI's Help topic "Recovery" links to these documents, so a user can read the exception and the rule on the same page. Add "except when the agent never runs" to each.

**N5-3. Recovery's primary button in the unconfirmed state is "Boot DSM" (Medium-Low).** `unit=ds918&facts=agent#recovery`. The primary is "Boot DSM", and its dialog says "The NAS reboots and starts DSM. It takes about a minute." Clicking it shows "Rebooting. DSM returns in about 60 seconds." and the screen stays unchanged (the chip still says "Default: not confirmed"). This action only works if the unit answers (`recover_to_dsm` SSHes to it). In the unconfirmed state it often will not. The mock has no failure branch ("The NAS did not answer. Use the manual recovery."). Make Manual recovery the primary in this state, and add the failure branch.

**N5-4. Help topics exceed the 150 word limit and Boot manager has a duplicated paragraph (Low-Medium).** `topic=boot-manager` is 179 words (about 174 without "Back" and "Open in docs"). It contains two near-identical sentences ("If the agent never runs, nothing resets the default and there is no watchdog ..." and "If the agent never starts, nothing resets the default and nothing reboots the unit ..."). `topic=recovery` is 155 words. UX-SPEC and BRIEF set 150 as the maximum; `check_words.js` only fails a sheet above 400 words, so it does not enforce it.

**N5-5. "the last row" is the fourth row (Low).** Help > Recovery says "...except the unconfirmed case in the last row". The unconfirmed row ("Agent silent after 10 minutes") is the fourth of six. UX-SPEC line 1507 has the same text.

**N5-6. Log excerpt and header disagree (Low).** `failed-agent`, Log excerpt: "@@FAIL phase 5/7 staging stopped" while the page says "Phase 4 of 7".

**N5-7. Carried unchanged (Low).** The 24 px copy and show buttons on the failure screens, the hint "Add serial last 4." under a matched phrase, the phrase suffix for twins only (declared), "Recommended" on Boot manager.

---

## What is good now

- **The staging-failure screen tells the truth.** It states what the app knows, names both outcomes, gives a bounded estimate with its arithmetic, and has a Manual recovery path that matches the project's own README.
- **The status bar and tags are derived from facts** and say "Boot unconfirmed" where the app cannot know.
- **Help, UX-SPEC and (mostly) the product documents agree** on the numbers: 10 wait, 30 install watchdog, 60 boot manager, 60 s, 15 min.
- **Everything from rounds 3 and 4 holds:** the gate in Wipe and Keep, twin protection in both directions with distinct hostnames and disk serials, the Command dialog and its fixture, Quick setup, Boot manager, Recovery with accessible names, install progress with the point of no return, connect failures, post-step-3 failures.

## What I could not verify

- Real GPUI behaviour and screen readers, contrast and type-scale numbers, the web UI, dark mode in depth.
- Whether the Synology DOM is removable or soldered, and whether a unit can show its boot menu on a serial or VGA console (the two documents disagree; I cannot test hardware).
- Whether a real agent kernel ever fails to boot on a given model. The design now handles both outcomes, so this no longer decides the verdict.

## What I will check in the real application

- `failed-agent` and `recovery-agent` as above, with the engine's actual message for the 10 minute wait.
- That the Manual recovery dialog shows the real backup file, `sudo`, and the physical-access note (N5-1).
- That `docs/RECOVERY.md` and `docs/BOOT.md` no longer contain the three absolute sentences (N5-2).
- Help topic lengths (N5-4).
