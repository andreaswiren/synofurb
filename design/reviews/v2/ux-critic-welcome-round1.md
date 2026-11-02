# UX critic: Welcome guide (D1b), round 1

Review only. No gate verdict, nothing written to `design/approvals/`, no code edited.

Read: `design/v2/UX-SPEC.md` D1b, `crates/synofurb/src/ui/{welcome.rs, screens/welcome.rs, overlays.rs (open_welcome), settings.rs, copy.rs (WELCOME_*), screens/help.rs, render.rs (menu), app.rs (first-launch hook)}`, `docs/GUI.md`, DEV-UI-23, `docs/RECOVERY.md`, `docs/BOOT.md`, and the screenshots (steps 2, 3, 4, 5 in light, dark and Largest as listed below). I did not run the app or the tests; the GUI is judged from code and screenshots.

## Verdict

The mechanics are good: five steps, one primary per step, honest dots, a fixed text height, arrow keys that never press Start, a tested "seen" rule, and a sane migration of the old flag. The visible wording is short and plain on steps 1, 3 and 5.

But the guide's one job after "what is this" is to say what can go wrong and how to get back, and **step 4 says something untrue**. It must not ship as written. Four more findings are major: a stray click can dismiss the whole guide for good, the replay checkbox does nothing, the accessibility notice moved behind a hover, and the "process" step does not explain the process.

| | Count |
|---|---|
| Blocking | 1 |
| Major | 4 |
| Minor | 8 |

---

## Blocking

### B1. Step 4 "The way back" over-promises, and contradicts step 3 and the docs

Visible text, step 4: **"DSM stays the default boot entry; Recovery restores it."** and, behind the (i): "Recovery restores the flash that was saved before the install." Checked against `docs/RECOVERY.md` and `docs/BOOT.md`:

1. **After a finished install the default is Debian, not DSM.** RECOVERY.md: "The default boot entry is switched to Debian only by the very last successful step of an install." BOOT.md: DSM "is the default except while the agent is armed (one boot) or after an install has switched to Debian". The sentence is the headline of the design rule, without its qualification, and it is false at exactly the moment the user would rely on it.
2. **After the point of no return there is no way back to a working DSM.** Step 3 says the disks you pick are erased. RECOVERY.md: the loader runs "but DSM's operating system is gone"; the only route is reinstalling DSM from Synology, and "synofurb cannot promise that any data survives". Step 4 then titles itself "The way back" and says Recovery restores DSM. A reader who skimmed step 3 gets the picture "erase, but I can always go back". The app's own status-bar link, "Way back" (tip: "What returns the unit to DSM from here."), is state-aware for this reason; the guide is not.
3. **The agent exception is missing.** RECOVERY.md ("read this first") and the engine's `AgentTimeout`: if the boot agent never started, nothing reset the default, the unit may boot the agent after every power cycle, and "a power cycle is not a recovery". "DSM stays the default; Recovery restores it" is the sentence that caveat exists to prevent. Recovery also needs the unit to answer over SSH; the real worst case needs a console, a boot menu or the flash in another computer.
4. **"Recovery restores the flash that was saved before the install"** is true only if a saved image of *this* unit exists (the app now matches by serial) and the unit answers; it restores the boot flash, not DSM.

The word budget is what produced this: a conditional truth ("before the disks change, a failure puts DSM back; after that, DSM must be reinstalled") was compressed into an unconditional one. The budget counts the visible words only, and the info body is free; the caveat was put on the wrong side of that line.

**Fix (words stay at about the same count):**
- Line A: "Before the disks change, a failure leaves DSM as it was."
- Line B: "After that, DSM must be reinstalled. Keep your backup."
- (i) body: Recovery and Manual recovery for a unit that no longer answers; the agent caveat in one sentence ("If the boot agent never starts, the unit may keep booting it: use Manual recovery"); a pointer to "Way back" in the status bar. Drop "DSM stays the default boot entry" unless it says "until the install finishes".
- Keep the title "The way back" only if it is true for the case the user fears; "If something goes wrong" says less than it promises.

---

## Major

### M1. Skip, Esc, the close button and a click on the backdrop dismiss the guide for good, even on step 1, and nothing says so

`WelcomeView::close` / `overlays.rs::open_welcome` `on_close`: any close on steps 1 to 4 calls `store_seen`. `gpui-component` 0.8.1 dialogs are `overlay_closable: true` by default and `open_welcome` does not change it (I read the default, I did not click it; check that `on_close` also fires for a backdrop click). So on the first launch, a click outside the dialog, an Esc pressed out of habit, or "Skip" on step 1 means the user never sees "the disks you pick will be erased" unprompted. The Skip tooltip says only "Close the guide.", not "and do not show it again". The only visible control over persistence is the checkbox on step 5, which the people most likely to skip never reach.

The erase warning is not lost app-wide (Confirm still has the erase phrase and "Test your backup"), so I do not call this blocking; but the guide is the one place the user is told before investing time.

**Fix:** make the persistence visible and narrower. Options: set `overlay_closable(false)`; Skip says "Skip for now" and sets "seen" only when step 3 was shown (or re-shows once more at the next launch); Esc on steps 1 and 2 closes without storing; the Skip tooltip states what it stores.

### M2. On replay, the "Don't show again" checkbox is ticked and unticking it does nothing

`Guide::new()` always starts with `dont_show: true`; `remember_on_close` only ever *sets* `welcome_done`, and "replay never clears the flag". Replayed from Help or the menu after the flag is set, step 5 shows a ticked box. A user who unticks it (reasonably expecting the guide to come back at the next launch) gets nothing: the flag stays set. The box shows a state it does not own.

**Fix:** open the replay with `dont_show` taken from the stored flag; or hide the box when `welcome_done` is already true (the replay then ends in Start only); or make unticking clear the flag, and say so in the tooltip.

### M3. The screen-reader notice is now a hover tip on an icon

The old dialog said, in plain view, that screen readers are not supported and to use the command line. Now it is the one-sentence tooltip of the (i) on step 3, with "the same steps work from the command line" in a popover. Someone who cannot use this window with a screen reader is best served by being told on the first screen; here the fact is two interactions deep, in a window they cannot read. The spec accepted this as a trade for the word budget; I record it as a regression of safety-adjacent content, since this was a *visible* statement before.

**Fix:** one short visible line on step 1 or 5 ("Screen readers: use the command line, see Help") and keep the (i) for the detail. It costs about 9 words; step 1 has the room (15 of 25).

### M4. Step 2 does not explain the process

"Six steps to Debian. Review each step; go back until Install starts." plus a row of six labels: Find, Connect, Goal, Setup, Confirm, Install. For a first-time user "Goal" and "Setup" mean nothing, and the row does not say where anything changes. What a plain-language guide should tell them, and does not: that the first steps only read; that **Confirm** is where they type the unit's name and the changes begin; that the NAS restarts and the install takes a while; that a copy of the boot flash is saved on their computer first. The page the user needs most to understand the process is a decorative stepper.

**Fix:** two lines, still within budget: "Find, Connect, Goal, Setup only read." / "Confirm asks you to type the unit's name; Install then changes it." Optionally put "restarts, about N minutes" behind the (i) with the real number from the engine's timing constants.

---

## Minor

1. **Pre-ticked "Don't show again" on step 5.** Defensible: the user who reached Start has read it all, Help and the menu replay it, and an unticked default would nag every launch. It is not a dark pattern. My conditions: M2 fixed (the box must reflect the real state), and the persistence of Skip and Esc made visible (M1), because as it stands the box looks like the choice but the real decision is made earlier, silently. The extra (i) beside the checkbox just to explain a checkbox is noise: put "You can reopen this from Help" in the label's own line (it already says so in step 5 line B).
2. **"Make a tested backup" does not say of what or by whom.** It reads as if synofurb makes the backup. synofurb saves only the boot flash; the data backup is the user's (Confirm says so). Suggest "Back up your data yourself and test it."
3. **"Boot manager tries other systems"** is on the "way back" step, is undefined for a new user, and is not a way back. The (i) says it works "without touching DSM", but Boot manager does write to the boot flash (GOAL_BOOTMGR: "The flash changes, the disks stay untouched."). Remove it from step 4 or move it to step 2 as a goal; say "the disks stay untouched" instead of "without touching DSM".
4. **Step 1 promises "installs Debian 13 on your Synology"** while the list behind the dialog shows unsupported units (DS220j, RS816 in the demo). Add "supported models" or "x86 units".
5. **Step 5 spends a step on "Pick your NAS in Find to begin."** The user is already on that page. This is where M4's "Confirm is where it changes" or the data backup reminder belongs.
6. **Keyboard.** Left and Right work through the view's focus handle (read in code); Esc closes. After open, focus is on the container, not on Next, so Enter does nothing until the first Tab. The spec says "Enter/Space activate the focused control", true but the primary is not the default focus. Most other dialogs in the app have an Enter-to-confirm; this one should too, with Right arrow kept. Not verified by running.
7. **Migration.** `first_launch_done` is accepted by an alias, so existing users who dismissed the old dialog are not shown the guide. That is the conservative choice and consistent with "first launch", but it means the only people who will see the corrected step 4 are new users. If the owner wants everyone to see the safety wording once, a new flag name is needed (and then the old alias must not set it). Downgrade is harmless (an older build ignores `welcome_done` and shows its old dialog once).
8. **Language and layout.** "Find" in step 5 is the step name; the page says "Find NAS". Step 3's warning icon is a lone brown glyph beside text: fine (the word "erased" carries it). Layout in the screenshots (light, dark, Largest, step 2 with the stepper row) is clean; no clipping seen at Largest on step 2. Steps 3 and 4 at Largest I did not open.

---

## Your specific questions

- **Plain language, what the app does and the process:** what it does, yes (step 1, in one line); the process, no (M4).
- **Safety content preserved and visible:** "erased" and "tested backup" are visible text on step 3 (good); "nothing changes until you confirm" is visible on step 1 (good, but not repeated beside the erase warning); "only reads until you confirm" and the screen-reader notice are behind an (i) (M3); "way back" is visible but wrong (B1).
- **Word budget, "too much text":** met (tests enforce it); not too much text. The budget is the problem for B1, not the cure.
- **Keyboard and focus:** minor 6.
- **Pre-ticked box; what Skip and Esc persist:** minor 1 and M1.
- **Replay from Help and menu:** both exist (first row of the Help list, a title-bar menu item, "Replay the short intro."); the replay flow has M2.
- **Migration:** minor 7.
- **Misleading claims about Boot manager, Recovery, DSM as default:** B1 and minor 3. Against RECOVERY.md and BOOT.md and the engine's agent-did-not-answer caveat, the sentence "DSM stays the default boot entry; Recovery restores it." over-promises on three counts.

## What must change before release

Blocking: B1. Fix before release, small: M1, M2. Before a wider release: M3, M4, minor 2, 3, 4.
