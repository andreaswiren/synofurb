# UI critic, round 4: desktop mockup v2

Artifact: `design/mockups-v2/desktop/` (104 states, `index.html` built 19:06 from `src/` files; I confirmed each of `app.css` and `js1-7` is embedded verbatim). All 426 screenshots now have timestamps newer than `index.html` (0 older). Criteria: BRIEF-v2, `DESIGN-SYSTEM-V2.md` (section 13, DEV-UI-1..8), `tokens-v2.json`, gpui-component 0.8.1 source. I did not read other reviewers' files and did not edit the mockup. Earlier reports of mine: round 1 to 3 in this folder.

State URL prefix: `file:///home/user/synofurb/design/mockups-v2/desktop/index.html?bare=1&theme=<light|dark>&` + query from `screens.json` + `#<screen>`.

## Verdict: APPROVE (design level), zero blocking findings

NB-R3-1 is fixed, the four wording and scope items I raised in round 3 are fixed, and my re-run of every probe found no regression. I approve the mockup as a statement of the visual design. This is not an approval of the build: nothing here has been rendered by gpui-component (see "What I could not verify"), and a real-app review against the same brief is still required.

## Evidence base

* Looked at 2 regenerated shipped screenshots and 8 live renders (1280x800, 960x640, 960x640 at Largest; both themes where noted) of the Confirm gate (ready, keep, twin; I captured scroll 0 and 170 px in both themes), Quick setup (mixed disks, also after selecting a layout and at Largest), Recovery (default DSM and Debian default) and the failed-post screen; the other states were checked by the DOM probes below, not by eye.
* Live probes over all 104 states: type scale and weights, scrollers, horizontal scroll and clipped text at all three sizes; tooltip inventory; targets and button sizes; primary / danger counts per screen; contrast of 1,791 rendered text samples in both themes; Tab and radio behaviour in the Quick setup dialog; positions of the gate controls in the Confirm scroller.
* `design/v2/tools/contrast_v2.py`: 642 of 642, lowest text 4.74, non-text 3.25. DS timing paragraph checked against `crates/synofurb-core/src/session.rs` (timeout_min 30 at line 330, 10 min wait at 362, timeout_min 60 at 422): matches.

## Round-3 findings: status

| Item | Status | Evidence |
|---|---|---|
| **NB-R3-1** Confirm: summary after the gate at 52 rem or less | **Fixed** | `app.css` line 324: `grid-template-areas: "a" "t" "g" "l"` (alerts, summary, gate, list). At 960x640 Largest, measured in the scroller (main region 403 px): summary triad 233 to 315 px, gate header at 326 px, first checkbox 406, phrase 589 to 627. At scroll 0 the alerts, the three Destroyed / Changed / Preserved cards and the ERASE header are on screen (`g4-ready-light-0`, `g4-keep-dark-0`, `g4-twin-dark-0`, shipped `confirm-ready-960-130-*`). Same order in the keep and twin variants. |
| DEV-UI-3 wording | **Fixed** | Now "the DS default control height is 32 px; BRIEF-v2 states no pixel minimum"; 24 px is the WCAG 2.2 AA minimum (2.5.8). See N1 for a remaining mismatch with row buttons. |
| DEV-UI-5 wording | **Fixed** | Names the 52 rem order (alerts, summary, gate, details), the hidden unit header on Confirm and the compacted alerts, page gap and padding, and the 40 rem diagram stacking; `app.css` lines 324-327 match the text. |
| Diagram horizontal scroll (DS section 12) | **Fixed** | DS now says the diagram never scrolls horizontally (one row per disk, proportional bars, chips wrap); my probe finds no horizontal scroll or clipped text in any state at 1280, 960 and 960 Largest. |
| 32 px rescue buttons | **Fixed for the failure screens** | Copy, show and rescue-notes buttons on `failed-*` are 32 px (my size scan of every failed, install, recovery, done, boot and manage state: no `btn` on a failure screen below 32 px). Recovery row actions are a different case (N1). |
| Recovery primary logic | **Fixed** | `js5.js` lines 29-30 state the rule; behaviour matches: default DSM with written disks gives the primary to "Open guide" and disables "Boot" with a reason button; Debian default gives the primary to "Boot" and disables "Remove" with an "Unavailable" Popover (`recovery-debian-light`). Row buttons no longer repeat the row label ("Boot", "Start", "Restore", "Remove"). |
| Quick setup radio dialog | **Fixed** | Mixed-size disks show two radio cards (Hybrid 1-disk, RAID 6) with usable size and survivable failures; "Continue to Confirm" stays disabled until one is chosen (visible hint "Choose a layout."), enabled after the choice; 71 words, within the dialog budget; at 960x640 Largest the dialog scrolls with a pinned footer. |
| Screenshots regenerated | **Fixed** | 426 of 426 newer than `index.html`; I did not re-diff them this round (they were pixel-identical to live renders in round 3); the two I viewed match my live renders. |
| Duplicate "Check again" (my R3 N1) | **Fixed** | `failed-agent-notback` has one "Check again". |

## Your question: is it acceptable that the phrase field needs a scroll at Largest on 960x640?

Yes. The reading order is now: what is lost, what is changed, what is preserved, then consent, then the phrase. At Largest on the minimum window, the summary and the gate heading are on the first screen; the three checkboxes (406 px) and the phrase (589 px) need a scroll of roughly 225 px; the Erase button is pinned and stays disabled until the checkboxes and phrase are satisfied, with a reason button next to it. That is the right trade: scrolling to type is a deliberate act, whereas the previous order let the user finish consent without the summary having been on screen. At 100% on 960x640 and at 1280x800 the summary and the gate remain on one screen (two columns), so only the maximum-text corner scrolls.

## Re-verified, no regression

* Tooltips: 0 of 86 distinct tooltips on non-focusable elements; 0 tooltips on keyboard focus; hover tooltips still work (DEV-UI-1).
* Type scale over all states: sizes 12, 13 (mono), 14, 16, 24 (and 31.2 / 18.2 / 15.6 only in the Largest state), weights 400, 600, 700, no other.
* Scrollers: only `.main`, plus modal overlays at Largest (Help sheet, dev menu, the Quick setup dialog with a pinned footer). No horizontal scroll or clipping at 1280, 960, 960 Largest.
* One primary per screen: only dialog states (the page plus the dialog) and the Components specimen page have more than one.
* Contrast: no enabled control text below 4.5:1; lowest 4.99 (dark Erase label), next 5.13. Disabled buttons use the widget style (DEV-UI-2).
* Targets: no interactive element below 24 px apart from mock chrome, sliders and the Web UI switch (widget sizes, DEV-UI-3).
* Stepper, Tag (25 px), selected-row radio, no native select, dark danger set: unchanged and still correct.

## New blocking findings

None.

## Non-blocking items

* **N1 DEV-UI-3 over-claims.** It says "every primary, secondary and danger action is 32 px", but list and row actions are `btn sm` at 24 px: Recovery "Boot / Start / Restore / Remove / Open guide" (including the primary "Open guide", 96x24), Done "Copy password", "Open", "Show full", install-log "Copy" and "Open file", Manage "Restart". The DS allows Small in dense tables, so either reword the register ("except Small row actions in lists") or move Recovery, the screen people use when the NAS is broken, to 32 px.
* **N2 Radio cards in Quick setup** are `role=radio` buttons that are two Tab stops; arrow keys do not move the selection (checked after Down: unchanged). A `RadioGroup` should rove (one Tab stop, arrows select). Check the widget behaviour in the real build.
* **N3 Inventory tag crowding** with a twin or boot agent (`find-boot-agent`): two tags inside the Model cell. Cosmetic.
* **N4 Two sources for GPUI-incompatible behaviour remain by design:** container queries (DEV-UI-5), the Enter guard on Erase (DEV-UI-8), the dev-only mock chrome. All documented; the real app needs explicit implementations.

## What I could not verify

GPUI rendering of any screen; whether `RadioGroup`, `Popover` focus return and the Select popup behave as drawn; `tnum` exposure; real HiDPI and platform title bars; the web surface; the hosted Artifact copy.

## Recommendation to the coordinator

UI critic approves the desktop mockup v2 for the owner, with N1 and N2 as follow-ups that need not block. The next gate is the real-app review: `synofurb gui --demo --screen <name>` screenshots reviewed against this brief, in particular the Stepper (completed connectors, current label), the Table selected row (1 px border plus radio), Tag sizes, the disabled Erase and its reason button, and the scrollbar.
