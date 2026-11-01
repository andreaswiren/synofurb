# Concept A: linear guided wizard with a persistent safety rail (revised after review)

Open `index.html` (main flow and required states) and `states.html` (edge states). The bar at the top jumps between screens; `&#8599;` items open the other file. Window-size buttons include **960x640**; the **Text 130 %** button checks large text. **All data is sample data** (tag in every title bar).

## The idea

One road, nine steps: `Find, Connect, Inspect, Goal, Storage, System, Confirm, Install, Done`. A **stepper** across the top; a **safety rail** on the right that never goes away and answers *what has been changed, can I cancel, what is my way back*. The rail is **derived from facts**: Read-only, Plan not applied, **Boot flash changed** (confirmed, no disk yet), **Disks modified**, Failed with disks untouched, Debian running. Restore reads the unit and shows its real state.

Borrowed ideas: from C, a compact "If you confirm this plan: Destroyed / Changed / Preserved" summary beside the layout; from B, a **Manage** unit page for converted units.

## What the review changed

* Recovery and install now follow `docs/RECOVERY.md` and `docs/BOOT.md`: the boot-flash flow (backup, one-shot agent, installer), one point of no return (step 3), DSM stays the default, 60 s reboot to DSM before step 3, 15 minute rescue shell after it, then the DSM loader and a reinstall from Synology with no data promise.
* The boot flash is listed as **changed (backed up first)**, never preserved. The only extra backup is the **disk metadata backup** (first and last 1 MiB of each disk: metadata, not files; cannot recover a formatted filesystem).
* The primary path is **Untested** (no Verified model exists); Boot manager is recommended first; Verified only appears in the legend.
* The gate shows IP, serial, disk serials and a same-model warning, carries every unresolved warning, and is interactive (real checkboxes and input; the pinned erase button enables only when satisfied).
* Keep-volumes mode is behind "Advanced" and has its own Confirm and failure screens.
* Accessibility baseline stated in the UI: keyboard, contrast and text size are supported; screen readers are not; the CLI path is the equivalent.

## Pros / cons / when it fails

Pros: easiest to learn and test; the risk state is visible on every screen; lowest chance of skipping a step; simple for GPUI (stepper + scroll column + rail + pinned footer).
Cons: walking back to change one value is slow; the rail takes 200 to 226 px (it narrows and drops its meter at 960 px); after the install the wizard is not the right home for ongoing management (hence Manage); at Largest text the stepper labels crowd and should collapse to "Step N of 9"; Restore does not fit a strict step order.

## Key screens

`confirm` (interactive gate), `confirm-keep`, `plan-error`, `progress` and `progress-ponr`, `failed-pre` (60 s clock), `failed-post` and `failed-post-keep` (15 min clock), `restore`, `manage`, `done-arm`.
