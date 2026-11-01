# Concept C: single-canvas plan board

> **Alternative, not revised after review.** The independent UX review covered only the recommended options (desktop Concept A, mgmt-a). This concept still shows the first-draft content (for example a Verified sample unit, a "disk-header backup", a boot flash listed as preserved). Do not treat its copy as a spec. Layout ideas only.

Open `index.html`. Use the bar at the top to jump to any screen or state. Window-size buttons show the 960x640 minimum.

## The idea

One screen, three panes, left to right: **1 Inspect** (what is there), **2 Plan** (what you want, with the disk diagram central and editable), **3 Confirm** (what that will destroy and keep, the recovery plan, the gates). Change a slider in the middle and the destroyed / preserved lists on the right change at once. Goal (Install Debian | Boot manager | Restore / Recover) is a segmented control in the top bar, with five stage chips (Inspect, Plan, Confirm, Install, Done). Locked panes are hatched and say why.

Install turns the **same board into a run view**: left = phases and steps, centre = the live disk diagram (what has been written so far, layer by layer) and log, right = Safety (point of no return, cancel, what is saved on this computer). Failure keeps the board and turns the right pane into Recovery. Settings and About slide in as a drawer; Update is a toast + dialog.

## Why it is shaped like this

* Storage planning is exploratory: you want to see the consequence of every choice, immediately and in one place. Wizards hide consequences behind "Next".
* Putting the destroyed list next to the plan makes data loss a live property of the plan, not a surprise at the end.
* The run view reusing the diagram shows progress as "what is written", which is more convincing than a percentage.

## Pros

* Fastest for iteration and for experienced users; the diagram is the interface.
* Strongest sense of consequence: plan edits update the Confirm pane.
* Failure state can point at the exact layer that was written (diagram) and the exact step (left pane).

## Cons / when it fails

* Dense. At 960x640 the three panes squeeze to about 224 / 440 / 290 px; the diagram labels truncate and the Plan pane scrolls. Needs a collapse rule (Confirm slides over Plan below ~1000 px).
* The gate shares the screen with the plan, so it is easier to glance past than a dedicated step. We mitigate by expanding the Confirm pane to 440 px and locking the plan (confirm screen), and by hatching it while the plan is invalid.
* Harder to teach a first-time user: nothing says "do this next" except the chips.
* The most complex layout for GPUI (three scrolling panes, per-pane lock states) and the hardest for screen readers (landmark-heavy single page).
* Find and Connect are cramped into two panes; they are better served by a plain list screen.

## Key states to look at

`plan` (valid with warning), `plan-error` (Confirm hatched and locked), `confirm` (wide Confirm), `progress-ponr` (live diagram + Safety), `failed-post` (Recovery pane), `about` (drawer).
