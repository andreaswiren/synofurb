# Recommendation (revised after the UX review)

Only the recommended options were revised after review: **desktop Concept A** (plus the two borrowed ideas) and **mgmt-a** (plus three borrowed pieces). Concepts B, C and mgmt-b are unchanged and marked "alternative, not revised after review". The review's findings and what changed are in the Revision log at the end of `FLOWS.md`.

## Desktop app: Concept A as the spine, with two borrowed ideas

**Ship Concept A (guided wizard + safety rail).** The central risk is a nervous person erasing the wrong disks, or panicking when something stops half-way, on a model nobody has reported on yet. One road with a persistent, **fact-derived** safety rail answers "what has been changed, can I cancel, what is my way back" on every screen. It is also the cheapest layout to build, test and make keyboard-friendly in GPUI.

Borrowed, now drawn in the revised mockup:

1. **From C: the live "If you confirm this plan: Destroyed / Changed / Preserved" summary** beside the layout on the Storage step. The gate itself stays its own full screen (A), not a pane (C).
2. **From B: the unit page for converted units** (`Manage`): status, logins (no stored password), default boot entry, recovery, history. First-run stays in the wizard.

What the review changed about the design (see FLOWS Revision log): the recovery story now matches `docs/RECOVERY.md` and `docs/BOOT.md` (boot flash flow, one point of no return at step 3, 60 s and 15 min clocks, DSM loader and Synology reinstall, no data promises); the primary path is **Untested** (no Verified model exists) and recommends the boot manager first; the gate shows unit identity (IP, serial, same-model warning) and carries every unresolved warning; Keep mode is behind "Advanced"; the accessibility baseline is stated in the UI with the CLI equivalent.

Why not B or C as the base: B is the right model for the long life of a unit, but a first-time visitor meets a sidebar and tabs before guidance; C is the best tool for exploring a plan and the worst for a hard stop (the gate shares a screen with the thing planned, and three panes do not fit 960x640 without collapse rules).

### Risks (desktop)

| Risk | Mitigation / what to watch |
|---|---|
| **The recovery copy drifts from the engine again.** | Treat `docs/RECOVERY.md` and `docs/BOOT.md` as the source; every sentence on the gate and failure screens should be traceable to a row there. Re-review when either changes. |
| **Gate fatigue**: users tick, paste and proceed. | Test comprehension, not completion. Keep lists specific; add no other confirmations. |
| **Flash write failure after the point of no return** (the Wipe failure sample) is expensive. | Ask engineering for a preflight check and write test of the boot flash before step 3 (open question 3). |
| **Screen readers are not supported** in the native window. | Stated in the UI; CLI path is the equivalent; decide before building whether a text-only gate is worth it (open question 13). |
| **960x640 at large text**: stepper labels crowd, content scrolls. | Rail narrows, primary action is pinned; collapse the stepper to "Step 7 of 9" at Largest. Test in GPUI. |
| **Keep mode** over-promises. | Hidden behind Advanced; consider removing it from v1. |
| **Post-PONR 15-minute clock**: user needs the log and the shell in time. | The app shows the clock and puts "Export diagnostics now" first. |
| **ARM needs a serial adapter**. | Detect at Inspect; put it on the Goal card (open question 12). |

## Management plane: mgmt-a as the base, with three pieces of mgmt-B

**Use mgmt-a (ForgeVault-style sidebar shell)** for continuity, and take from mgmt-B (now in mgmt-a): the always-visible **status strip**, the **"Needs attention"** block at the top of the dashboard (health is the worst component), and the **docked edit panel** (users, interface edit). Keep tabs inside Storage on desktop and the bottom tab bar on phones.

### Risks (management plane)

| Risk | Mitigation |
|---|---|
| **Locking yourself out through a network change.** Safe apply must work across a new origin (new certificate warning, new session). | The new URL and fingerprint are shown before and after applying; the login page at the new address shows the countdown; SSH confirm exists. Test the whole loop on a real unit, including DHCP giving a new address after rollback. Open questions 4 and 5. |
| **Passkeys and IP origins.** | Not promised: the UI says passkeys need the name. Verify browser behaviour. |
| **Lockout abuse** (anyone on the LAN pauses the owner). | Says so, gives the SSH restart; consider per-source scoping. |
| **Foreign OS cannot be pulled back by us.** | The Boot page says power-cycling is the way back. |
| **Irreversible storage operations**. | Delete: checkbox + typed phrase + list of what uses it; extend says it cannot shrink. |

## What to usability-test first

Five to eight home-lab users and three less technical ones, on a laptop with a real or demo NAS, using the clickable mockups first. The mockup Confirm is interactive.

1. **The gate (highest value).** Show Confirm cold. Ask: "What will happen to your photos?", "Could any data survive?", "What changes on the boot flash?", "What if it fails half-way?", "How long do you have to read the log?". Success: all of it erased, nothing guaranteed, boot flash changed and backed up, **15 minutes then the DSM loader, DSM reinstalled from Synology**. Record whether they read the lists before ticking, and whether anyone believes the metadata backup restores files.
2. **Same-model wrong-unit.** Two DS918+ units; ask them to proceed on 192.168.1.40, then switch the target halfway. Does the serial on the gate catch it? (The first test plan used different models and would not have.)
3. **The Untested first run.** Do users pick Boot manager first when recommended? Do they understand "flash changed, disks untouched"? Do they understand that Untested means "your install is the test"?
4. **Compatibility legend.** Ask them to rank the five badges by "would I proceed?". Check Untested is not read as "will not work" and Verified (legend only) is not read as "safe".
5. **Keep mode.** Ask them to "keep my existing data" and see whether they find Advanced, read what it leaves, and notice the size cap.
6. **Failure after the point of no return.** Show `failed-post` with the timer running; ask for next steps in order. Success: they export diagnostics and open the rescue shell before the clock ends, and do not reach for "restore DSM" first.
7. **Failure before the point of no return.** Show `failed-pre`; do they wait or panic-power-cycle? Do they understand DSM comes back by itself?
8. **Network rollback.** Ask them to change eth0 to a static IP and then follow the banner: open the new URL, accept the certificate warning, sign in, confirm, all inside 90 s. Then a run where they do nothing.
9. **Phone glance.** At 390 px: "Is anything wrong with the NAS?" (answer must be yes: disk warning) and "How long until the sync finishes?" in under 10 seconds.
10. **960x640 and large text.** Can the user find the primary action and the safety rail on every screen?

Measures: task success, time, errors, **verbatim comprehension quotes**, confidence (1 to 5) before and after the gate, the number of times the log is opened to find "did it touch my disks?" (target zero).
