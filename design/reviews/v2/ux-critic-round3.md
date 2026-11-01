# UX critic, round 3: desktop mockup v2

**Verdict: APPROVE** (UX, for the desktop mockup as a design). I found no blocking findings in the 104 states. Both round-2 blockers are fixed and verified by walking the states.

This is my verdict only. I wrote nothing to `design/approvals/`; that is the owner's decision. Approving the mockup does not approve the real application. `BRIEF-v2.md` makes the real app's screenshots the review artifact, and I will apply the same checks there. Section 3 lists the reconciliations the author must make before the real app can pass.

Reviewed: `design/mockups-v2/desktop/` (`index.html`, `screens.json` with 104 states, `src/`, `README.md`), the screenshots, `design/v2/UX-SPEC.md`, `docs/RECOVERY.md`, `docs/BOOT.md`, `boot/agent/docs/*`, and the engine (`crates/synofurb-core/src/session.rs`, `domboot.rs`, `script.rs`). I used Playwright against the prototype: live runs, typed input, Tab and key presses, a clipboard read. I did not read other reviewers' files. I did not edit the mockup.

Reproduce with `design/mockups-v2/desktop/index.html?t=<unique>&nofirst=1&bare=1&<params>#<screen>` (params in `screens.json`).

Checks run: `check_flows.js` 134 passed, 0 failed; `check_words.js` all 102 states within budget, max 59 words, prose in "data" spans counted. My own dead-end sweep (every enabled button on the new failure, Keep Confirm and Boot manager states) found no button without a visible effect. I did not recompute contrast (`check_contrast.js`) or the type scale (`check_scale.js`).

---

## 1. Status of round-2 findings

### Blocking, both fixed

| ID | Status | Evidence |
|---|---|---|
| **NB1** Keep gate said volume1 "is erased" | **Fixed** | `confirm-keep` (`unit=ds918&keep=1&sys=8&gate=ready`): "No backup found. volume1 · 6.2 TiB may still be lost." with the same backup popover; Preserved: "volume1 · best effort"; Destroyed: "40 GiB DSM system"; an After row "Debian 13 · volume1 kept · system 8 GiB × 3 · ds918". `confirm-twin-keep` (`unit=ds918b&keep=1&sys=8`) says "volume1 · 1 TiB may still be lost" for the other unit. The Wipe line still says "is erased" (`confirm`). |
| **NB2** No staging-phase failure | **Fixed** | `failed-agent` and `failed-backup` exist and I walked both (below). |

**`failed-agent`** (`unit=ds918&fail=agent`): "Install stopped · Phase 5 of 7", "The agent did not answer", "Agent reboots to DSM in 30:00", "DSM is untouched.", Disks Untouched, Boot flash "Entries added", DSM "Default". Footer: Recovery options, Wait for DSM. Status bar "Failed safely". "What happens next" (55 words): "No disk was changed. Our entries are on the boot flash, with a full backup on this computer. The agent's watchdog sets DSM as the default and reboots in 30 minutes. You can power-cycle the NAS now: it boots DSM. Then remove our entries under Recovery if you want them gone." Live (`run=1&clock=2`): Waiting for DSM, then "DSM is back. The NAS answered on its DSM address." with Done. `phase=notback` gives "DSM did not return. Power-cycle the NAS." with Check again. The Engine message popover and Log excerpt are present.

**`failed-backup`** (`unit=ds918&fail=backup`): "Phase 3 of 7", "Flash backup did not verify", "Nothing was changed. DSM is untouched. DSM is still running.", Disks Untouched, Boot flash **Untouched**, DSM **Running**. Footer: Recovery options, Back to Confirm (resets the gate; I verified `S.gate` is cleared, so the user must re-confirm). This matches `docs/RECOVERY.md` ("Boot flash backup: nothing changed; abort, nothing changed").

### Non-blocking items from round 2

| ID | Status | Evidence |
|---|---|---|
| N-R2-1 Confirm never says what will be built | **Fixed** | After row on every Confirm: "Debian 13 · RAID 5 · 10.82 TiB usable · system 32 GiB × 3 · ds918". Storage's primary is now "Next: System" and goes to the System tab; System's is "Next: Confirm". |
| N-R2-2 Wrong hostname default per unit | **Fixed**, one note | `unit=ds1621#system` gives `ds1621`, DS923+ gives `ds923`, the Command dialog follows. Note: the twin at .58 also defaults to `ds918`, so the two units would collide on `ds918.local`. See N3-6. |
| N-R2-3 "Matches plan" had no test | **Fixed** | `check_flows.js:142-144` is a fixture that sets every control, checks that the command carries every plan input, and that each control changes it; `:63` checks that a command differing from the plan is detected. I re-ran it myself on twin B, Hybrid 1-disk and Keep. |
| N-R2-4 Twin suffix only when a twin was seen | **Fixed in the demo, rule unchanged** | The default demo (no params) lists both DS918+ units with "Twin" tags, so the phrase carries the suffix: `ERASE DS918+ 3456` (A), `ERASE DS918+ 7710` (B). Units without a twin still get the plain phrase (`ERASE DS923+`, `ERASE DS1621+`). That is the declared deviation; see N3-7. |
| N-R2-5 Quick setup silently preferred a worse layout | **Fixed** | `unit=ds1621#goal`, Quick setup: "Wipe all 6 disks · Hybrid 1-disk · 19.85 TiB usable · survives 1 disk ... Your disks differ in size. RAID 6 would give 7.15 TiB and survive 2 disks." plus a Customize button. DS918+: "RAID 5 · 10.82 TiB usable · survives 1 disk". |
| N-R2-6 "Recommended" on Boot manager | **Unchanged** | Still on Boot manager. Needs a usability test, not a design change (README test plan). Not blocking. |
| N-R2-7 Small targets | **Unchanged, declared** | 24 px info buttons remain (`DEV-UI-3` in `DESIGN-SYSTEM-V2.md`). See N3-5 for the part that is not an info button. |
| N-R2-8 Odd states and copy | **Fixed** | `recovery-fresh` is now consistent ("Flash: clean · Default: DSM · Disks: untouched · Nothing to recover"). `recovery-debian` shows "Flash: ours present · Default: Debian · Disks: modified". Boot-manager progress no longer lists "Send installer" (steps: Boot flash backup, Agent added, Reboot into the agent, Agent answers). Done > Next steps replaced the BIOS line with "Reboot once to check it starts". Twin disks now differ from the original fixture, but still match each other (N3-6). |

---

## 2. Author's claim about 180 s and 30 minutes

The author says the 180 s "agent did not answer" and the 30 minute watchdog are consistent with `docs/BOOT.md` and `docs/RECOVERY.md`. They are not stated in those documents, and one number contradicts the engine.

| Number in the mockup | `docs/BOOT.md`, `docs/RECOVERY.md` | Engine and agent docs | Verdict |
|---|---|---|---|
| **30 min** watchdog, install flow (`failed-agent`) | Neither document gives a number. RECOVERY.md: "if nobody claims it in N minutes it reboots, which boots DSM." BOOT.md: "within the timeout." | `session.rs:330`: install flow arms the agent with `timeout_min: 30`. The agent's own default is 20 (`overlay-contract.md`, `lib.sh`). | **Consistent with the engine.** The documents should state the number. |
| **180 s** the app waits for the agent | Not stated. | `session.rs:362`: `wait_for_agent(..., Duration::from_secs(10 * 60) ...)`: the app waits **10 minutes**. `domboot.rs:318`: "waiting for the NAS to reboot into the boot agent (usually 1-3 minutes)". The failure text there is "the boot agent did not appear within N minutes". | **Contradicts the engine.** The engine message in the mockup ("did not answer within 180 s") will not appear. |
| **30 min** "Login timeout" and "30 min watchdog" (Boot manager, Recovery "Open agent") | Not stated. | `session.rs:422`: the boot-manager install arms the agent with `timeout_min: 60`. | **Contradicts the engine** (60). |
| **60 s** installer failure before step 3 | RECOVERY.md: "reboots after 60 s". | `script.rs` failure handler. | Consistent. |
| **15 min** rescue shell after step 3 | RECOVERY.md: "reboots after 15 minutes". | | Consistent. |

Consequences for the screen: the countdown on `failed-agent` starts at 30:00 when the app gives up. The watchdog started when the agent booted, which is at least the app's wait earlier (3 min in the mockup, 10 min in the engine). So the true remaining time is about 27 minutes in the mockup's world and about 20 in the engine's. The label should say "within 30 min" or the clock should start lower. Harm is low: the page says the user can power-cycle now, and DSM boots. I rate it non-blocking, but the numbers must be reconciled before the real app is reviewed (N3-1, N3-2).

---

## 3. Non-blocking findings (new or carried)

**N3-1. The staging-failure clock and 180 s do not match the engine (Medium).** Section 2. Pick one source of truth: change the engine wait (10 min) or the sample text, and start the clock from the real watchdog start (or label it "up to"). Add the agent timeout to `docs/RECOVERY.md`.

**N3-2. Boot manager and Recovery say 30 min where the engine arms 60 (Medium).** `unit=ds918#boot` ("Login timeout 30 min"), `boot-result` ("30 min watchdog"), the "Open the agent?" dialog ("returns to DSM by itself after 30 minutes"). `session.rs:422` uses 60. Either the design shows a different value than the product ships, or the engine changes. Decide and align.

**N3-3. `failed-agent` says "Phase 5 of 7" (Medium-Low).** The install screen puts "Reboot into the agent" under "Phase 3 of 7" (Staging), and `PHASES` has Phase 5 = "Installing". A failure to reach the agent is phase 3 (or 4, "Booting installer"). "Installing" next to "Disks untouched" confuses the one fact the screen exists to settle. `failed-backup` says Phase 3, which is right.

**N3-4. Leftover hint under a matched phrase (Low).** `confirm-keep`: the field shows "Matches" and the hint "Add serial last 4." is still under it. The hint should disappear once the phrase matches.

**N3-5. Copy and show buttons on the failure screens are 24x24 (Low).** `unit=ds918&fail=post#failed`: Copy command, Show password and Copy password, Rescue notes (134x24) are 24 px high. `DEV-UI-3` covers info buttons and inline links, not the copy buttons the user needs inside a 15 minute window. The brief's minimum is 32 px.

**N3-6. Fixture issues (Low).** Both DS918+ units show identical disk serial suffixes (K7P2, M3Q9, R5T1, H8D4); real units would differ. The twin defaults to hostname `ds918`, which would collide on the LAN (`ds918.local`); suggest the suffix (`ds918-7710`) when a twin exists.

**N3-7. Phrase suffix only for detected twins (Low; declared).** The plain phrase for non-twin units is fine, but a twin that is off, on another VLAN or not yet scanned makes the gate weaker without any notice. Always appending the last four digits would give the same muscle memory everywhere. I accept the declared deviation from UX-SPEC D7 as an improvement.

**N3-8. Quick setup defaults to Hybrid on mixed disks (Low).** The dialog states the trade-off, but Hybrid is the layer `synofurb` builds itself ("DSM may not adopt it, nothing here has run on a real Synology" in the Slices dialog). The Quick setup dialog should carry a one-line "built by synofurb" caution or link to the Hybrid help topic.

**N3-9. Known limit, accepted: Largest at 960x640 (Low).** `size=largest` at 960x640: the phrase field is at y 565, below the pinned footer (top 526). Focusing the field scrolls it into view (y 304). There is no horizontal scroll and no clipped footer button on any state. The Erase button stays visible and locked, with its reason one click away. Accepted as the known limit.

**N3-10. Carried unchanged.**
- "Recommended" on Boot manager (usability test needed).
- 24 px info buttons (`DEV-UI-3`).
- Replace-disk now happens in the web UI (owner decision). The web plan must cover md0 as well as md1 and put the backup line before the rebuild. That belongs to the web review.
- Actions that leave the app are toasts (declared). "Open rescue shell" copies the command and the persistent on-page command and password back it, so I accept this.

---

## 4. Verified and good

- **Gate** (Wipe and Keep): the no-backup line is specific and true for each mode; the unit's IP and serial are in the card; the "This is 192.168.1.40 · …3456" checkbox and the suffix phrase tell the twins apart in both directions; Enter on Erase does nothing; Space and click start the install; the locked state explains itself.
- **Failure design** covers every phase now: staging (backup mismatch, agent silent), preflight, after step 3 (Wipe and Keep), lost contact, with a clock and an end state for each, a persistent rescue command and password, and a dialog per case.
- **Status bar** follows facts across the states I sampled (Failed safely, Flash changed, Disks changed, Debian running, Needs attention, Critical, Unreachable).
- **Recovery** rows have outcomes, typed confirms where they edit the flash, and disabled-with-reason rows.
- **Install**: the point of no return is a flag in the progress card; counters stay in range.
- **Connect** failures, **Copy command**, **F1 contextual help**, **Boot manager** confirm, progress and result, **Manage** read-only Storage with the Hybrid layer and "Open web UI".
- **Word budget**: all 102 states within the limit with prose in data spans counted (max 59).

## 5. What I could not verify

- Real GPUI behaviour, widget fidelity, screen readers.
- Contrast and type-scale numbers (the author's checks exist; I did not recompute).
- Dark theme beyond spot checks.
- The web UI, including the replace-disk plan.
- Whether the engine will show the 180 s message, which it will not today (section 2).

## 6. What I will check in the real application

- The agent wait and watchdog numbers shown equal the engine's (N3-1, N3-2), and `docs/RECOVERY.md` states them.
- The staging failures (backup mismatch, agent silent) are reachable and render as in `failed-backup` and `failed-agent`, with the right phase.
- Enter on Erase, both twin directions, the copied command against the plan, the clock end states, status bar truth, PONR visible in the live run, Keep gate wording.
