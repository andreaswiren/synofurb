# Concept B: unit-centric dashboard

> **Alternative, not revised after review.** The independent UX review covered only the recommended options (desktop Concept A, mgmt-a). This concept still shows the first-draft content (for example a Verified sample unit, a "disk-header backup", a boot flash listed as preserved). Do not treat its copy as a spec. Layout ideas only.

Open `index.html`. Use the bar at the top to jump to any screen or state. Window-size buttons show the 960x640 minimum.

## The idea

The **unit is the object**, not the wizard. A sidebar lists every NAS the app knows (running Debian, discovered), each with a plain-language status ("Inspected", "Installing 5 / 7 · step 9", "Failed at step 11", "Debian, managed"). Selecting a unit opens its page: **Overview** (hardware, disks, compatibility, history), **Actions** (three cards: Install Debian, Boot manager, Restore / Recover), **Install** (live run), **Logins**, **Recovery**. A **history timeline** records what happened and when.

Actions that change things open as a **focused flow** that replaces the whole window (no sidebar) with an `Exit` button and a "Nothing written yet" indicator. The Confirm step puts the destroyed / preserved / recovery lists side by side and docks the three gates (backup, acceptance, typed phrase, red button) in a bar along the bottom edge so the gate is always on screen.

## Why it is shaped like this

* People with more than one NAS, or who return later to try the boot manager, think "my DS918+", not "step 4".
* The sidebar status doubles as a safety rail: you can leave a running install, look at another unit, and still see "Installing 5 / 7" and "Failed at step 11".
* Putting Install in a focused flow isolates the dangerous part from browsing; leaving the flow is always possible and always safe (until the installer starts).

## Pros

* Best fit for the long life of a unit: Manage, boot-manager trials, recovery, history.
* Reconnecting to a running or failed install is natural (click the unit).
* Confirm layout shows all three lists at once and keeps the gate visible.
* Scales to a home-lab with several units.

## Cons / when it fails

* More surface: a new user sees tabs and a sidebar before they have done anything; the first-run path is less obvious than a wizard (we mitigate with Actions cards and a "Choose an action" call to action on Overview).
* Two navigation systems (sidebar + tabs) plus a modal-like flow take more GPUI work than a linear column.
* At 960 px the 238 px sidebar plus tables is tight; it should collapse to icons under about 1000 px.
* The persistent sidebar competes for attention during the flow; hiding it in flows is right but costs the "where am I" cue.

## Key states to look at

`plan` (diagram on top, controls below), `confirm` (docked gate bar), `progress-ponr`, `failed-post` (recommended steps in a highlighted card), `done` (logins + history), `update` (sidebar card + dialog).
