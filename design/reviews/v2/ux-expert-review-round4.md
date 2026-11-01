# UX expert review, round 4

Scope: the UI expert's hand edits to UX-SPEC.md (lines ~115, 271, 514, 644, 648, 658, 1516), checked against `crates/synofurb-core/src/session.rs`; R3-1, R3-2 and Recovery primary logic checked in the real prototype (`design/mockups-v2/desktop/`, 104 states, Playwright at 1280x800). The mockup was not edited.

## 1. Engineering check of the UI expert's spec edits

| Claim | Source | Verdict |
|---|---|---|
| Install-flow agent watchdog 30 min | session.rs:330 | Correct. |
| The app waits 10 min for the agent | session.rs:362 | Correct. |
| Boot manager agent watchdog 60 min | session.rs:422 | Correct. Earlier spec text said 30; the UI expert was right. |
| New-system wait 20 min | session.rs:451 | Not used in the edits; fine. |
| Failed-agent "Phase 3 of 7" | `Phase::Booting` is called before the agent wait (4th of 7: Connecting, Inspecting, Staging, Booting, Installing, WaitingForReboot, Verifying) | **Wrong. It is Phase 4 of 7.** Flash backup failure stays Phase 3 (Staging). |
| Clock "up to 20:00" | 30 min watchdog counted from agent start, minus 10 min the app waited | Roughly right but it is an estimate (agent start precedes the app's reboot command by some seconds to minutes). It must not read as a countdown guarantee. |

## 2. Spec fixes made in this round (UX-SPEC.md only)

- D9 failed-agent variant: "Phase 3 of 7" became "Phase 4 of 7, Booting"; clock described as an estimate ("DSM returns in about 20 min") with the 10-min wait and 30-min install-agent watchdog stated.
- Web W8 Boot agent popover: "30 idle minutes" became "60 idle minutes".
- Web help topic `boot`: "30 idle minutes" became "60 idle minutes".
- Boot manager figures (D4 popover, D13 Login timeout field and tooltip, Agent ready tag, "Install boot agent?" dialog, desktop help topic) are 60 min everywhere; no stale 30 min remains for the boot manager.
- Generator sources in `scratchpad/v2gen` got the same 60-min changes, but the generator no longer reproduces the UI expert's line 514 variant text. **UX-SPEC.md is now the source of truth; do not regenerate it.**

## 3. Round 3 follow-ups, verified in the HTML

- **R3-1 Quick setup: fixed.** Two explicit radio-style options, nothing preselected, Continue disabled until one is chosen. Confirm After shows the survive count: ds1621 "RAID 6 . 7.15 TiB usable . survives 2 disks", ds918 "RAID 5 . 10.82 TiB usable . survives 1 disk".
- **R3-2 Boot manager after Return to DSM: fixed.** Shows "Agent stopped / Back in DSM. Our entries remain on the flash." with Recovery options and Done. No dead end.
- **Recovery primary logic: sound.**
  - Flash ours present, disks untouched: Boot DSM disabled with a reason (DSM is already default), Open agent, Restore flash and Remove entries secondary, Reinstall DSM is "Open guide" outline. No primary; correct, nothing is urgent.
  - Flash ours present, disks modified: Boot DSM disabled with the hint "Loader may show Synology's page.", **primary is Open guide on Reinstall DSM**. Correct, because the disks are changed and DSM may be gone.
  - Flash, disks, Debian: primary is Boot DSM; Remove entries disabled while Debian is default.
  - Flash only: no primary, Boot DSM disabled.
  Disabled buttons are paired with a focusable info button (the disabled-reason pattern holds).

## 4. Remaining issues

### Blocking
None.

### Major
1. **Mockup says "Phase 3 of 7" for the failed-agent state** (`src/js4.js` line ~116: `stage?'Phase 3 of 7'` covers both `agent` and `backup`). Agent failure must read "Phase 4 of 7"; only backup stays at 3. It contradicts the engine and the spec, and the Phases popover on the same page. Applies to the fail=agent states (agent, waiting, back, notback).
2. **Mockup clock "up to 20:00" must be phrased as an estimate** (`js4.js` ~121 `clockInfo`). Use "about 20 min" or label it "estimate", and do not run it as an exact mm:ss countdown, since the real time left is 20 to 23 min depending on when the agent started.

### Minor
1. The running Installing page shows "Phase 3 of 7" for all four pre-install flash steps (`js4.js` ~94, `s<4`). The agent boot and wait belong to Phase 4. Split the label at the step where the reboot into the agent begins.
2. Spec section 12 and the Help text now carry the 60-min figure only because someone hand-edited the .md; if the generator is ever reused, port line 514 and the other six edits first.
3. Carried from round 3, not re-verified this round: the Largest-text phrase sits below the fold at 960x640 (R3-3); "Twin" and "sdc" tag labels would benefit from a tooltip naming what they mean; duplicate "Open web UI" labels on the finished page.

## 5. Verdict

Round 4 changes are acceptable once the two mockup items (phase number, estimate wording) are fixed by the UI expert. The spec is consistent with session.rs. No blocking issues.
