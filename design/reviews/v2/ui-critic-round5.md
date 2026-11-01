# UI critic, round 5: regression check of the revised desktop mockup v2

Artifact: `design/mockups-v2/desktop/` (107 states; `index.html` built 19:43; I confirmed `app.css` and `js1-7` are embedded verbatim). 438 screenshots, **0 older than `index.html`**. I did not read other reviewers' files and did not edit the mockup. Previous round: `ui-critic-round4.md` (APPROVE at design level, zero blockers).

State URL prefix: `file:///home/user/synofurb/design/mockups-v2/desktop/index.html?bare=1&theme=<light|dark>&` + query from `screens.json` + `#<screen>`.

## Verdict: APPROVE (design level), zero blocking findings. Round-4 approval holds.

No regression in any of the checks you listed. One documentation sentence became inaccurate (N1), and one new state is worth a look from the UX side (N2). Neither blocks.

## What I checked, and the result

| Check | Result | Evidence |
|---|---|---|
| Type scale | **Holds** | Over all 107 states (5,471 text nodes at 1280x800, and again at 960x640 and Largest): sizes 12, 13 (mono), 14, 16, 24 only; 31.2, 18.2 and 15.6 px appear only in the Largest-text state; weights 400, 600, 700 only. |
| Contrast, new warning tags, both themes | **Pass** | Rendered, composited over their real backgrounds: warning Tags (Twin, Flash changed, Disks changed, Failed safely, Default: not confirmed, Boot unconfirmed chip) text 7.16 to 7.41:1 in light, borders the same; neutral status chips (Read-only, Plan only) 7.04 text, 3.56 border; token check `warn_fg`: light 6.77 to 8.22 on every surface including `accent_bg` (7.16) and the alert tints (7.63, 7.74); dark 8.52 to 12.07. Whole-app scan of 1,825 text samples in both themes: no enabled text below 4.99:1 (dark Erase label); disabled buttons follow the widget style. `contrast_v2.py` (earlier run, tokens unchanged): 642 of 642. |
| Tooltips on focusable controls only | **Holds** | 87 distinct tooltips, 0 on non-focusable elements; tabbing 14 stops raises 0 tooltips; hover works; info button opens its Popover on Enter and Escape returns focus. The new Manual recovery dialog traps focus (Copy commands, Open guide, Close) and closes on Escape. |
| One primary per screen | **Holds** | Only dialog states (page plus its dialog) and the Components specimen page have more than one: new `failed-agent-manual` and `failed-agent-next` follow the same pattern ("Wait for DSM" beneath, "Open guide" or "Manual recovery" in the dialog). `quick-mixed` shows "Continue to Confirm" as a disabled primary. |
| No horizontal scroll or clipping | **Holds** | Zero hits in all 107 states at 1280x800, 960x640 and 960x640 Largest. The only second scrollers are modal overlays at Largest (Help sheet, dev menu, Quick setup dialog with a pinned footer). |
| Screenshots newer than `index.html` | **Holds** | 438 of 438. |

## Round-5 changes: verified

* **Row actions raised to 32 px (DEV-UI-3 rewritten).** My button scan over every failed, install, recovery, done, boot and manage state: the only `btn` below 32 px is the service "Restart" chip (24 px), which the rewritten DEV-UI-3 lists. Recovery rows, Done "Copy password", log Copy / Open file are 32 px (`recovery-agent-1280-dark`). Remaining small targets: info buttons and inline links (24, declared), Find row action "Manage" (24, declared), sliders and switch (widget sizes), mock close (20).
* **Quick-setup radios with roving tabindex.** Only the first radio is a Tab stop (tabindex 0 and -1), ArrowDown / ArrowUp move focus and selection and wrap, Space keeps the selection, "Continue to Confirm" enables on the first selection, Tab leaves the group to Cancel. This closes my round-4 N2.
* **New failed-agent states and Manual recovery dialog.** `failed-agent`, `-waiting`, `-back`, `-notback`, `-manual`, `-next`: the three-card state (Disks untouched, Boot flash entries added, DSM "Default: not confirmed" with a warning Tag and icon), the clock "DSM may return in about 20 min or more" with a visible info button, the "Manual recovery" button as a secondary action beside "Recovery options" and the primary "Wait for DSM". The dialog is an ordered list with the two commands in a code block and a Copy commands button; word count within the 80-word dialog budget (my dialog and popover scan found none over 80 or 40).
* **Warning tags.** Icon, word and border style on every one (circle-question-mark for "not confirmed"), never colour alone.
* **Inventory tags moved into the Status cell** (`find-boot-agent`): Untested, Twin, Boot agent sit together in the Status column; the Model cell is clean again (closes my round-4 N3).
* **"After" line moved into the summary block** (`confirm-ready`): Debian 13, RAID level, usable size, survival and system layout in one row under the three summary cards. See N1.
* **Recovery accessible names.** Each row action has an `aria-label` that starts with the visible label ("Boot: Boot DSM", "Start: Open agent", "Restore: Restore flash", "Remove: Remove entries", "Open guide: Reinstall DSM"), so label-in-name (WCAG 2.5.3) holds. Recovery on an unconfirmed agent shows a warning alert with a sibling "Manual recovery" button (alert plus action, as the DS prescribes) and gives the single primary to "Boot".

## New blocking findings

None.

## Non-blocking items

* **N1 DEV-UI-5 sentence is no longer true.** It says that at Largest on 960x640 "the summary and the gate heading share the first screen". With the After line (about 70 px) in the summary block, the ERASE card now starts 395 px down a 403 px viewport (`gateL4`: gate top 395, first checkbox 475, phrase 659), so only its top border shows. The ordering that matters (alerts, Destroyed / Changed / Preserved, After line, then consent) is preserved and the phrase still needs a scroll of about 255 px with Erase pinned and disabled until satisfied; I accept that. Reword the sentence ("the summary and the After line fill the first screen; the gate follows after a short scroll").
* **N2 Unconfirmed agent: two routes to the same dialog.** `failed-agent` offers "Manual recovery" in the footer and "What happens next" opens a dialog whose primary is also "Manual recovery". That is fine, but check with the UX expert whether three recovery entry points (Recovery options, Manual recovery, What happens next) on one screen are one too many for a user under stress.
* **N3 Focus after URL-opened dialogs.** Escape from a dialog opened by a URL parameter (no trigger) drops focus to the document; a dialog opened by a click returns it to the trigger. Not reachable by a real user, but note it for the GPUI implementation.
* **N4 Still open from earlier rounds, unchanged:** nothing here has been rendered by gpui-component; GPUI-specific behaviour (`RadioGroup` roving, Popover focus return, Select popup, `tnum`) is unverified.

## What I could not verify

GPUI rendering of any screen; real-app keyboard behaviour; the hosted Artifact copy; platform title bars and HiDPI.

## Recommendation

UI critic approval of the desktop mockup v2 stands. Fix N1 in the DS text when convenient. The next gate is the real-application review from `synofurb gui --demo --screen <name>` screenshots against this brief.
