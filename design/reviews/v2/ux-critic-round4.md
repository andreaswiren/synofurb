# UX critic, round 4: desktop mockup v2

**Verdict: REJECT. I withdraw my round-3 approval.** There is one blocking finding (NB4-1). I should have caught it in round 3. I found it this round because I checked the timing claims against `boot/dom/README.md` and `boot/agent/docs/safety-model.md`, not just `session.rs`.

Everything the author changed in round 4 checks out except one number (N4-1). The blocker is older than round 4. The staging-failure screen (`failed-agent`) promises that DSM is the default and that a power cycle brings DSM back. For one of the two ways the agent can fail to answer, the project's own boot-flash README says that is false, and that case is the likelier one on an Untested unit.

Reviewed: `design/mockups-v2/desktop/` (`index.html`, `screens.json`, 104 states, `src/`), `design/v2/UX-SPEC.md`, `docs/RECOVERY.md`, `docs/BOOT.md`, `boot/dom/README.md`, `boot/agent/docs/safety-model.md`, and `crates/synofurb-core/src/session.rs`. I walked the prototype with Playwright (live clocks, keyboard, clipboard, dialogs). I did not read other reviewers' files. I did not edit the mockup and wrote nothing to `design/approvals/`.

Checks: `check_flows.js` 147 passed, 0 failed. `check_words.js` all 102 states within budget (max 59 words). My own sweeps of the new states found no button without an effect.

Reproduce with `design/mockups-v2/desktop/index.html?t=<unique>&nofirst=1&bare=1&<params>#<screen>`.

---

## Blocking finding

### NB4-1. `failed-agent` says DSM is the default and a power cycle returns DSM. The project's own docs say that is not known (High)

State: `unit=ds918&fail=agent#failed` (also `failed-agent-waiting`, `-back`, `-notback`).

What the screen asserts:
- DSM tag: **"Default"** (green check). Disks "Untouched". Subtitle "DSM is untouched."
- Way back popover: "DSM boots again by itself."
- "What happens next": "The agent's watchdog ... reboots ... You can power-cycle the NAS now: it boots DSM."
- Clock: "DSM returns in up to 20:00" (see N4-1).
- Status bar: "Failed safely".

What the engine and docs say:
- `boot/dom/README.md`, "Fail-safe design": `install-agent --arm` points the default at the agent for the next boot only, and the agent must reset it to DSM as its very first action. **"If the agent never runs, the unit would keep booting the agent: keep the manual recovery below at hand."** The manual recovery is: pick `SYNOLOGY_2` in the boot loader menu, or attach the DOM to another Linux machine and edit `SynoBootLoader.conf`, or restore the backup.
- `boot/agent/docs/safety-model.md` layer 1 ("DOM boot default is DSM before anything else is attempted") starts from `boot.sh`, which only exists once the agent's kernel and initramfs have booted. The failure table covers "nobody connects (wrong network, no DHCP, no key)": the watchdog reboots, `dom_default` was set to DSM at start. It does not cover a kernel or initrd that never boots.
- `session.rs:362`: after the 10 minute wait the code returns the error with `?`. Unlike the earlier steps (which call `default_to_dsm`), nothing resets the default, and the app cannot: DSM is not running.
- `docs/RECOVERY.md` and `docs/BOOT.md` say only "if nobody claims it in N minutes it reboots, which boots DSM". Neither mentions the never-ran case. `docs/BOOT.md` also says nothing here has run on a real Synology.

So "the agent did not answer" has two causes the app cannot tell apart:

| Cause | Default boot entry | A power cycle | Watchdog |
|---|---|---|---|
| (a) Agent booted but cannot be reached (DHCP on another network, wrong key) | DSM (first action ran) | boots DSM | reboots to DSM, about 20 to 30 min after it started |
| (b) Agent never ran (kernel or initrd does not boot on this model) | **still the agent** | **boots the agent again, in a loop** | none |

For the first install on an Untested unit, (b) is not exotic; it is the first time that kernel runs on that model. In (b) the user sees a screen telling them DSM is the default, to wait or power-cycle, and "Failed safely". The recovery they actually need (boot menu on a serial or VGA console, or the DOM in another computer) is not on the screen, in the dialogs, in Help (`recovery`, `troubleshooting`, `boot-manager`), in UX-SPEC, or in RECOVERY.md. I searched the source and spec for "never ran", "boot menu" and "serial": nothing relevant.

Why it hurts: this is a failure screen that asserts a recovery guarantee the product cannot give in the case where it is most likely to be needed. "Failed safely" and the green "Default" tag are the exact copy I am hired to catch. A person without a console who power-cycles in a loop has no next step.

What would convince me:
- The DSM tag on `failed-agent` is not green "Default". It is unknown, for example "Default: not confirmed".
- The screen and dialog give both outcomes: "If the agent started, DSM returns by itself (about 20 to 30 minutes) and a power cycle also works. If it did not start, the unit may keep booting the agent: choose SYNOLOGY_2 in the boot menu (needs a serial or VGA console), or take out the boot flash and use `synofurb-dom`." with a link to Help.
- `docs/RECOVERY.md` gets the same row, with the number (30 minutes for the install agent) and the never-ran case. Help > Recovery and Help > Troubleshooting get it too.
- The Way back popover on this screen no longer says "boots again by itself" without the condition.
- A screenshot of the changed state (`failed-agent`, and `failed-agent-notback`: "DSM did not return. Power-cycle the NAS." needs the same caveat, because it is the state a (b) user will reach).

---

## Status of round-3 findings

| ID | Status | Evidence |
|---|---|---|
| N3-1 staging clock vs engine | **Partly fixed, with an error** | Install agent watchdog 30 min (`session.rs:330`) and the app's wait of 10 min (`session.rs:362`) are now in the copy: "the boot agent did not appear within 10 minutes of the reboot; its watchdog reboots to DSM 30 minutes after it started". The clock is "DSM returns in up to 20:00". The inequality is the wrong way round, see N4-1. |
| N3-2 Boot manager 30 vs 60 | **Fixed** | `session.rs:422` is 60. Boot manager now says "Login timeout 60 min", "60 min watchdog", the Open the agent dialog says 60, Help > Boot manager says 60. UX-SPEC lines 271, 644, 648, 658 and 1516 say 60. `boot manager enter` re-arms the installed agent (`boot.rs:152-160`), so 60 is right for Recovery's "Open agent" as well. |
| N3-3 "Phase 5 of 7" | **Fixed** | `failed-agent` says "Phase 3 of 7"; `failed-backup` says Phase 3. |
| N3-4 hint under matched phrase | **Not re-tested** (in `confirm-keep` it still read "Add serial last 4." under a match in round 3). Low. |
| N3-5 24 px copy buttons on failure screens | **Unchanged** (Copy password, Show password, Copy command and Rescue notes are 24 px). Low. |
| N3-6 twin fixtures | **Fixed** | Hostnames differ: `ds918-3456` and `ds918-7710` (in System, After line and the Command: `--hostname ds918-3456 --confirm "ERASE DS918+ 3456"`). Disk serials differ per unit (`3VSN 4A3O 336L 3HHM` against `E4PH EJ0I EXBJ FBMK`). Units without a twin keep plain names (`ds923`, `ds1621`). |
| N3-7 suffix only for twins | **Unchanged, declared.** |
| N3-8 Quick setup defaults to Hybrid | **Fixed** | See below. |
| N3-9 Largest at 960x640 | **Accepted** (see "Confirm order"). |

---

## Round-4 changes verified

**Boot manager and the engine constants.** `session.rs:330` (install agent, 30), `:362` (app waits 10 min), `:422` (boot manager agent, 60). All three are now used correctly in the copy except for the arithmetic in N4-1.

**Quick setup** (`unit=ds1621#goal`, Quick setup): two radios, "Hybrid 1-disk · 19.85 TiB usable · survives 1 disk" and "RAID 6 · 7.15 TiB usable · survives 2 disks", neither preselected, "Choose a layout." and "Hybrid is built by synofurb; DSM may not adopt it." Continue to Confirm is disabled with the reason "Choose a layout first." (clicking it opens a popover with that reason). Picking RAID 6 enables Continue; it lands on Confirm with "Debian 13 · RAID 6 · 7.15 TiB usable · survives 2 disks · system 32 GiB × 3 · ds1621". DS918+ has one layout and no radios. Keyboard: the dialog traps focus and Space selects a radio. Notes (N4-3): focus order and arrow keys.

**Confirm After line** now says "survives N" (RAID 5: "survives 1 disk"; RAID 6: "survives 2 disks") and carries the unit's hostname.

**Boot manager after Return to DSM** (`boot-returned`): dialog "Return to DSM? The NAS reboots into DSM. Our entries stay on the boot flash until you remove them under Recovery."; then "Agent stopped", "Back in DSM. Our entries remain on the flash.", status "Flash changed", footer Recovery options and Done. Good.

**Recovery primary logic** (`facts=` combinations):

| Facts | Primary |
|---|---|
| default DSM, disks modified (also flash present) | Open guide (Boot DSM disabled: "DSM is already the default.") |
| flash clean, disks modified | Open guide; Start, Restore, Remove disabled |
| default Debian, disks modified | **Boot DSM** (see N4-4) |
| disks untouched | none |

**Confirm summary before gate** at Largest 960x640: order is backup line, Twin alert, Destroyed / Changed / Preserved cards, then the gate card, then the After line and the per-disk list. The phrase field sits below the footer until focused (it scrolls into view on focus). The footer's locked Erase button is always visible with its reason. At 1280x800 the cards, After line and gate are side by side. I accept this as designed (N4-5 notes the After line).

**UX-SPEC edits.** The 60 minute edits are right and consistent with the mockup. Line 514 says "the app waits up to 10 minutes for it and the install agent's watchdog is 30 minutes from its start, so the clock reads 'DSM returns in up to 20:00'". The facts are right; the "so" is wrong (N4-1). The spec has no row for the never-ran case (NB4-1).

**Other states re-walked and fine:** `failed-backup` ("Nothing was changed. DSM is untouched. DSM is still running." Boot flash "Untouched", DSM "Running", Back to Confirm resets the gate; this one is correct because the agent was never involved); pre-PONR and post-PONR failures; Keep gate wording; Connect failures; Boot manager progress and result.

---

## Non-blocking findings

**N4-1. "DSM returns in up to 20:00" has the inequality backwards (Medium).** `unit=ds918&fail=agent#failed`; the "About the clock" popover says "at most 20 minutes remain"; "What happens next" says the same. The watchdog counts from when the agent booted (`safety-model.md`: "AGENT_TIMEOUT_MIN after boot"; `svc-watchdog` compares uptime with a deadline). The app's 10 minute wait starts at the reboot command, so the agent booted b minutes in, where b is positive (kernel, initrd and network take about 1 to 3 minutes per `domboot.rs:318`). At the moment the app gives up, 10 − b minutes have elapsed, so 30 − (10 − b) = **20 + b minutes remain: at least 20, typically 21 to 23, never "at most 20"**. The clock will reach 0:00 and the screen will say "DSM did not return" while the watchdog is still counting. Say "in about 20 minutes or more", or start the clock higher. This is the error I asked the author to avoid; it is smaller than NB4-1, and the same fix pass should cover it.

**N4-2. `docs/RECOVERY.md` still says "N minutes" (Medium).** The numbers the UI now shows (30 install, 60 boot manager, 10 wait, 15 and 60 s) appear in UX-SPEC and the engine but not in the product documents the UI points to (Help > Recovery links to it). State them there.

**N4-3. Quick-setup dialog keyboard (Low).** The dialog opens focused on Cancel; Tab order is Cancel, Customize, the two radios, then back (Continue joins after a layout is chosen). The radios are the first content but the last stops. ArrowDown does not move between radios (only Space selects). Put the radio group first and give it arrow-key behaviour.

**N4-4. Recovery highlights Boot DSM on a Debian unit (Low-Medium).** `unit=ds918&facts=debian#recovery` (default Debian, disks modified): the primary button is "Boot DSM", and its dialog says DSM may be gone and Synology's install page can appear. For a unit that is running Debian the primary should be absent (nothing is broken) or "Open guide" if Debian is not reachable. The dialog does warn, which keeps this below blocking.

**N4-5. In the stacked layout the After line comes after the gate (Low).** At 960x640 Largest (and below ~1100 px) the user types the phrase before reading what will be built. Move the After line above the gate card, like the cards.

**N4-6. Recovery row buttons have no accessible names (Low).** The row buttons read "Boot", "Start", "Restore", "Remove", "Open guide" with no `aria-label` or `aria-labelledby` naming the row ("Open agent", "Remove entries"). Fine visually; a keyboard or screen-reader user hears only the generic verb.

**N4-7. Carried unchanged.** "Recommended" on Boot manager (needs a test). 24 px info buttons (`DEV-UI-3`). Phrase suffix only for detected twins (declared). The web UI's replace-disk plan (md0 and backup line) is for the web review. N3-4 hint under a matched phrase and N3-5 24 px copy buttons.

---

## What is good now

The mockup is otherwise very close. The gate (Wipe and Keep), the twin protection in both directions with distinct hostnames and disk serials, the Command dialog and its test fixture, the Quick setup dialog, the Boot manager flow with its 60 minute values, Recovery with typed confirms and disabled-with-reason rows, the status bar tied to facts, the install progress with the point of no return, connect failures and the pre- and post-step-3 failures with clocks and end states, all hold up under walking.

## What I could not verify

- Real GPUI behaviour, screen readers, contrast and type-scale numbers (not recomputed), the web UI, dark mode beyond spot checks.
- Whether a real Synology DOM kernel ever fails to boot the agent. I cannot test that; I rely on `boot/dom/README.md`, which says the loop is possible, and on `docs/BOOT.md`, which says nothing has run on real hardware.

## What would turn this into an APPROVE

1. NB4-1 closed as described: unknown default on `failed-agent` and `failed-agent-notback`, both outcomes in the copy, the manual recovery named, and `docs/RECOVERY.md` plus Help updated.
2. N4-1 corrected in the mockup and in UX-SPEC line 514.
3. I will re-run my checks: clock arithmetic against `session.rs`, the Way back popover, and the dialog text on the staging-failure states.
