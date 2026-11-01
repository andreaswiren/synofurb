# synofurb UX: information architecture, flows and safety patterns

Revised after the independent UX review (`design/reviews/ux-critic-mockups.md`). Only the recommended options were revised: desktop **concept A** (with C's live "destroyed / kept" summary and B's unit page for converted units) and management plane **mgmt-a** (with mgmt-B's status strip, "Needs attention" block and docked edit panel). Concepts B, C and mgmt-b are kept as alternatives and were **not revised after review**. A Revision log at the end maps every blocking finding (D1 to D9, M1 to M4) to what changed.

Where this file and the engineering docs (`README.md`, `docs/RECOVERY.md`, `docs/BOOT.md`, `docs/COMPAT.md`) disagree, the engineering docs are right and this file is wrong.

Scope: the desktop app (Rust/GPUI, 960x640 minimum) and the NAS management plane (`https://<ip>:8443`). Layout, flow and copy only; visual language belongs to the separate UI track.

**All mockup data is sample data** (a "MOCKUP . sample data" tag sits in the title bar of every desktop screen). It is not a claim about any real hardware.

Sample units used in the mockups:

| Unit | Role | Badge (matches `docs/COMPAT.md` today) |
|---|---|---|
| DS918+ (Celeron J3455, BIOS, editable boot flash, 4 x WDC WD40EFRX, `sdc` with 8 reallocated sectors, DSM volume `volume1` btrfs 10.9 TiB, 6.2 TiB used) | the main story | **Untested** (no report) |
| A second DS918+ at 192.168.1.58 | same-model collision for the gate | Untested |
| DS1621+ (AMD Ryzen V1500B) | | Untested |
| DS923+ (AMD Ryzen R1600) | | Community-reported (one report, from another project) |
| RS816 (Marvell Armada 385, ARMv7, U-Boot) | ARM "Done" variant | Untested; serial adapter needed |
| DS211j (Kirkwood, ARMv5) | | Unsupported (Debian 13 cannot run on ARMv5) |
| `attic` (DS218+ already converted) | "Already running Debian" and Manage | n/a |

**There is no `Verified` model yet.** The badge exists and is explained in a legend (Find and Settings/About), clearly marked "no model has this yet". The primary path through every screen is the **Untested** one.

Numbers in the plan follow the README: system mirror default 3 copies, 20 % of the data VG left free, `/srv/data` xfs, Keep-volumes mode reuses only DSM's system + swap area (about 10 GiB per disk, system capped near 9.5 GiB, data best effort), the generated installer has 12 steps and a point of no return at step 3.

---

## 1. Who and what we design for

* **Primary user**: a home-lab owner with a Synology that is out of support or too limiting. Comfortable with SSH, not necessarily with mdadm/LVM. Does this **once per unit**, nervously, with real data nearby, on a model nobody has reported on yet.
* **Secondary**: a tinkerer with several units who wants to try other Linux systems through the boot agent.
* **Day-2 admin** (management plane): the same person, months later, on a phone, because a disk warning showed up or the IP changed.

Consequences: the install is rare and high-stakes (spend screens and words on it); the app makes the **state of risk** visible at every moment; every dead end offers a next step that is **true** (it matches what the engine does); the management plane is small, boring and conservative about anything that can lock you out.

---

## 2. Information architecture

### 2.1 Desktop app

```
synofurb
|- Find NAS (inventory)       discovered units (nothing preselected), "Already running Debian" -> Manage, manual add, scan methods, badge legend
|- Unit
|   |- Connect                SSH credentials, host key, what we read (read-only)
|   |- Inspect                hardware, boot flash check, disks (SMART), DSM volumes, compatibility ("what we checked / not known")
|   |- Goal                   Boot manager (recommended first for Untested units) | Install Debian | Restore / Recover
|   |   |- Install Debian     Storage -> System -> Confirm gate -> Install -> Done
|   |   |                     (advanced, behind a link: keep existing volumes, best effort)
|   |   |- Boot manager       back up flash, add the one-shot SSH agent (disks untouched)
|   |   `- Restore / Recover  reads the unit's real state, then lists only what is possible
|   `- Manage (converted)     unit page: status, logins (no stored password), default boot entry, recovery, history
|- Settings and About         update channel / auto-update, compatibility list + legend, logs, theme, text size, accessibility statement + CLI equivalent
`- Global                     offline banner, update banner, connection-lost banner, diagnostics export
```

Concept A files: `concept-a/index.html` holds the main flow and the required states; `concept-a/states.html` holds the edge states (offline, host key, login error, SSH off, keep-volumes confirm and failure, connection lost, ARM Done). They share one shell; the split only keeps each file under the size budget.

### 2.2 Management plane

```
https://<ip>:8443
|- Login                      user + password; lockout message; certificate fingerprint check (full value one click away)
|- Dashboard                  health from the WORST component, Needs attention, tasks, CPU/RAM/temp/uptime, disks, RAID + LVM, network, services, activity
|- Storage                    Volumes | RAID arrays | LVM | Disks; create volume (4 steps, one click at the end); extend; delete (checkbox + typed phrase)
|- Mounts                     mounted FS, fstab, add mount, imported old-DSM volumes (read-only)
|- Network                    per-interface DHCP/static v4/v6, DNS; safe apply with 90 s rollback, new URL + fingerprint shown BEFORE applying
|- Users                      list + docked edit panel (create, lock, password, SSH keys, delete)
|- Boot                       boot-menu entries, next boot, reboot; honest about what can return you
|- System                     hostname, services (with SSH restart commands), updates, power
`- Audit log                  append-only, filterable
```

Persistent chrome on every page: left navigation, breadcrumb + help, **status strip** (worst health, running task, host, uptime), and, while a change is pending, the **rollback banner**. Phone: header with status chip, bottom tab bar.

---

## 3. Primary task flows

Screen ids in brackets are the switcher ids in the mockups.

### F1. First-time conversion (the Untested path, DS918+ with a boot flash)

1. Open the app. It scans at once; results stream in. **No unit is preselected.** Two DS918+ units show a warning so the user knows serials matter later. [find, find-scan]
2. A badge legend is available; "Verified" is marked "no model has this yet". The a11y statement and the CLI equivalent are on this first screen (see section 6).
3. **Connect** to the chosen unit. The screen says what it needs ("Enable SSH in DSM: Control Panel > Terminal & SNMP") and exactly what it will read, including a read-only check that the boot flash matches Synology's loader. [connect, connect-key, connect-err, connect-closed]
4. **Inspect** (read-only): hardware, boot flash (editable, matches the loader), disks with SMART, current DSM layout, DSM volumes, "Backups on this NAS: none found". The compatibility panel says plainly: **no one has reported an install on this model; your install is the test**; lists what we checked and what is not known. The `sdc` warning is raised here. [inspect]
5. **Goal.** Three cards. For an Untested unit the **Boot manager card is highlighted "Recommended first"**: add the one-shot agent, look around, disks untouched. Install Debian remains one click away. [goal]
6. **Storage**. Mode is "Wipe everything"; "Advanced: keep existing volumes (best effort)" is a text link, not a peer option. Live layout beside the controls, plus C's compact "If you confirm this plan: Destroyed / Changed / Preserved" summary. [plan]
7. **System** settings. Boot method defaults to "Through the boot flash" (this unit has an editable flash). [settings]
8. **Confirm** (section 4): destroyed, **changed (boot flash, backed up first)**, preserved, "For the next 25 minutes", the unit's identity (IP, serial, disk serials, same-model warning), the two unresolved warnings (SMART and Untested), the recovery plan, the plan being confirmed, then the gate (backup box, acceptance box, typed `ERASE DS918+`, paste allowed). [confirm]
9. **Install.** Seven phases; Staging and Booting installer are the boot-flash steps: (1) the whole flash is streamed to this computer and verified, (2) the agent is added and armed (DSM stays the default), (3) reboot into the agent, (4) the agent starts the installer. Then the engine's 12 steps; **step 3 is the point of no return** and cancel is free until it. [progress, progress-lost, progress-ponr]
10. **Done**: address, user, password (copy now, not stored, not shown again), SSH command, host key, key location, management URL and certificate fingerprint (short form + Show full), serial console, next steps, opt-in report. `Finish` is disabled until "I have copied the password" is ticked. [done, done-arm]
11. The unit appears under "Already running Debian" with **Manage**: a unit page, not the wizard. [manage]

### F2. Unit fails mid-install

All paths set DSM back as the default boot entry (RECOVERY.md). What differs is the clock and what survives.

**F2a. Fails before the point of no return** (sample: step 2 preflight, `sdc`'s partition table changed since confirmation) [failed-pre]
1. "The installer stopped before changing any disk." Green: **Disks touched: No.** Amber: **Boot flash: our entries are on it** (with the backup file name).
2. A visible **60 s countdown**: the NAS sets DSM as default and reboots by itself.
3. Next steps in order: wait for DSM (the app polls); if DSM is not back after 10 minutes power-cycle (DSM is already the default); inspect again and re-plan; later remove our boot-flash entries.
4. Primary button (pinned): `Wait for DSM`. **No rescue shell and no "restore DSM" options here**: nothing is listening and nothing needs restoring.
5. Not drawn but specified: *agent never answers after the reboot* (before the installer starts). The default was already reset to DSM by the agent, the agent reboots to DSM on its own, and a power cycle also returns to DSM. Same screen, no rescue shell, "NAS may be hung: power-cycle".

**F2b. Fails after the point of no return**
* **Wipe mode** (sample: step 11, the boot-flash entry failed read-back verification; the tool restored the flash automatically; Debian is on the disks but cannot start yet) [failed-post]
* **Keep mode** (sample: step 9, the NAS cannot reach the Debian mirror) [failed-post-keep]

1. "Install stopped at step N of 12. Your disks have been changed." Red **Disks touched: Yes** with the time of the first write; green/amber line for the boot flash.
2. A **15-minute countdown** with the app's own timer: "Rescue shell open for 11:48. Then DSM becomes the default and the NAS reboots into the DSM loader." This is the most prominent object after the banner.
3. Next steps, in time-critical order: **export diagnostics now** (the NAS is unreachable after the clock), **open the rescue shell**, fix and try again from inside it (Wipe: write the boot entry with `synofurb-dom`; Keep: fix the network, run the install again from step 1); or let the clock run out.
4. Pinned primary: `Open rescue shell (11:48 left)`.
5. The DSM explainer states only what RECOVERY.md states: the loader shows Synology's install page; reinstall DSM from Synology (find.synology.com); Synology documents a keep-data reinstall, **check its current documentation, this project cannot guarantee it**. Wipe: no data left to keep. Keep: `volume1` was not part of any write command and is expected to be intact unless the failure was a hardware problem. **No RESET-button sequence is asserted** (not validated for any model).
6. Saved on this computer: the boot-flash image, plan, install script, log, and the **disk metadata backup** (first and last 1 MiB of every disk we change, partition tables and GPT headers). The UI says exactly this: metadata only, not your files, cannot bring back a formatted filesystem, **no help recovering data in Wipe mode**. It has no restore button.

### F3. Boot-manager trial

1. Goal > **Boot manager** (recommended first for Untested units). The screen states what changes (**the boot flash is modified**; whole flash backed up and verified first; disks untouched; DSM stays the default), how the agent behaves (**one-shot**: resets the default to DSM first thing; reboots to DSM by itself after **30 minutes** without a login; `kexec` starts **Linux kernels only**) and what it does not do. [bootmgr]
2. A warning: **once you `kexec` another system nothing of ours is running; power-cycling returns to the default entry; that system can see and change your disks.** We make no promise of an automatic return from a system we did not install.
3. `Back up flash and install boot manager`, one reboot. From the NAS web UI Boot page or SSH, run `sf-boot kexec <kernel-url> <initrd-url>`.
4. Back: power-cycle, wait out the 30 minutes, or Restore / Recover > Remove our boot-flash entries.

### F4. Update flow

*App update* [update]: daily check; non-blocking banner; release notes dialog; download, signature (minisign) and SHA-256 verification; `Restart and update` enabled only after verification; **never offered while a conversion runs** (banner hidden on run screens, button disabled with the reason).

*NAS update*: System page lists updates (3 security, kernel needs reboot). Security fixes install automatically and never reboot; the kernel update shows "reboot needed"; Power confirm lists running tasks and SSH sessions.

### F5. Day-2 management (management plane)

* **Glance (phone)**: sign in, see **"Needs attention: 1 disk warning, 1 task running"**, not "Healthy". [dash, dash-mobile]
* **Create a volume**: Space > Name and type > Preview > Create. The last step is a **single click** (it touches nothing existing); typed names are reserved for deleting. [volume-wizard, volume-confirm]
* **Delete a volume**: shows what uses it, asks for a checkbox ("I have a copy somewhere else") **and** a typed phrase (`delete data`, not the mount word). [storage-delete]
* **Change IP safely** (the new address is a new origin): before applying, the form shows the new URL, that the browser will warn again, the certificate fingerprint to compare, that passkeys do not work on IP addresses, the 90 s limit and the SSH way to confirm. After applying, the banner repeats it with a timer; the **login page at the new address shows the pending change and the countdown** [network-arrived]; Apply is disabled while pending; if nothing is confirmed the old settings come back and the page found again says so [network-rolled]. The rolled-back copy never promises the old address (DHCP may differ): "look for ds918.local or scan". [network, network-pending, users-rb]
* **Boot page**: honest entries (Debian, Debian previous kernel, Boot agent, DSM loader). "Make default" on a non-Debian entry needs a typed phrase. [boot, boot-default]
* **Locked out**: say how long, say that anyone on the network can trigger it, and give the SSH remedy (`sudo systemctl restart synofurb-mgmt`). No serial console. [login-locked]
* **Reboot / services**: confirm lists RAID sync and SSH sessions; stopping the web service says how to start it again over SSH. [system-power]

### F6. Return to DSM (Restore / Recover) [restore]

The screen **reads the unit first** ("Boot flash: our entries present, default = DSM. Disks: modified at 14:25 (Wipe). Debian: installed, no boot entry") and lists only what is possible: make DSM the default (`synofurb recover`), open the agent/rescue shell, restore the boot flash from the backup, remove our entries, reinstall DSM from Synology. The disk metadata backup is listed as information with no action. The rail on this screen reflects the unit's actual state (here: disks modified), not "Reading only".

---

## 4. Safety patterns for destructive actions (and why)

| # | Pattern | Why |
|---|---|---|
| 1 | **Read-only until the gate**, said on every earlier screen. | People hesitate at Connect and Inspect because they fear side effects. |
| 2 | **A state-derived risk rail** with six states computed from facts: Read-only; Plan, not applied; **Boot flash changed** (confirmed, no disk yet); **Disks modified**; Failed, disks untouched; Debian running. Restore reads the unit and shows the true state. | "Did it touch my disks?" must never need the log. The first draft's rail said "Ready to erase" after confirmation and "nothing changed" on Restore. |
| 3 | **Three explicit lists on the gate**: WILL BE DESTROYED, WILL BE CHANGED (boot flash: entries and files added, backed up first, DSM entry stays), WILL BE PRESERVED. The boot flash is never "preserved". | Honesty about what we write. |
| 4 | **"No guarantee" and the mode name**: Wipe preserves nothing on the disks; Keep is "left alone, best effort"; Keep also lists what it does not carry over (DSM, settings, users, packages, shared-folder definitions) and that preserved volumes are not mounted automatically and mount read-only. | Keep mode is the most over-promised feature. |
| 5 | **Three independent gates**: backup checkbox, acceptance checkbox, typed `ERASE <MODEL>` (case-sensitive, paste allowed). **The identity of the unit is on the gate card**: model, IP, serial, disk serials, and a warning if another unit of the same model is on the network. | The phrase cannot tell two same-model units apart; the serial and IP shown next to it can. Cross-track decision: model phrase + visible identity + paste allowed (the UI track's hostname-only, paste-blocked variant must be aligned to this). |
| 6 | **The red button states the consequence**, is pinned in the footer, and is disabled with the reason beside it. Enter never triggers it. | |
| 7 | **The gate is a page**, not a modal. Real form controls (checkboxes, text input, labels) so keyboard use is demonstrable. | Modals are dismissed by reflex. |
| 8 | **The recovery plan before the point of no return**, in terms of the engine: before step 3 DSM stays the default and a failure reboots to DSM in 60 s; from step 3 on, a failure leaves a rescue shell for 15 minutes, then the DSM loader runs, DSM's own system may be gone and **we promise nothing about data**. | The exit must be true. |
| 9 | **The point of no return is an object in the step list** (step 3), with "Cancel unavailable" and a reason after it. One number, from the engine. | |
| 10 | **Pre-flight re-check at step 2**: serial, size and partition table of every disk must still match what you confirmed; a mismatch aborts before any write (this is also failed-pre). | |
| 11 | **Two backups, described exactly**: (a) the boot-flash image, saved before the first flash write, hash-verified, name shown; (b) the disk metadata backup, first and last 1 MiB of each disk we change, partition tables and GPT headers only. We say (b) is not file data and cannot recover a formatted filesystem. We never call it a way back for the user's data. | The first draft's "disk-header backup" read as "my data can be restored". |
| 12 | **Failure halts, keeps a rescue shell, and shows the clock.** The app never retries on its own after the point of no return. | |
| 13 | **Warnings travel verbatim** (Inspect, Plan, Gate "Warnings you have not resolved", Done): same sentence for the SMART warning, plus the "Untested" fact at the gate. | One-time warnings are forgotten. |
| 14 | **Risk labels come from the operation**: Boot manager is "Boot flash changed, disks untouched" (not "nothing changed"). | |
| 15 | **Management confirmations scale** (from ForgeVault): none for creating a volume, checkbox + typed phrase for delete and for making a non-Debian boot entry the default, simple confirm with consequences for reboot. | Friction proportional to loss. |
| 16 | **Safe apply with auto-rollback, designed for the new origin**: preview shows new URL + fingerprint + 90 s + the SSH confirm; banner persists on every page; login at the new address shows the countdown; Apply locked while pending. | |
| 17 | **No promise we cannot keep about foreign systems**: the Boot page says power-cycling is the way back from anything we did not install. | |
| 18 | **Status is never colour alone**: icon + word + shape. | |
| 19 | **Health is the worst component**: the dashboard never says "Healthy" with a SMART warning or a degraded array. | |
| 20 | **Accessibility statement is part of the safety design**: screen readers are not supported in the native window, and the equivalent `synofurb install ... --have-backup --accept-data-loss --confirm` path is named at the first screen and in Settings. | Better to say it than to let a blind user fail at the gate. |

Not done on purpose: countdown timers on the erase button, "type DELETE" without identity, confirmation by double-click, hiding the gate behind an advanced toggle, auto-dismissing toasts for data-loss events.

---

## 5. State inventory

| Screen | Empty | Loading | Error | Other |
|---|---|---|---|---|
| Find | "No Synology found" with causes and actions [find-empty] | Streaming rows, per-method status [find-scan] | A method failing is a warning chip on that method | Offline banner with cached list age [find-offline]; same-model warning; nothing preselected |
| Connect | n/a | "Connecting..." | Wrong password with attempt count and DSM auto-block warning [connect-err]; **SSH off** with the exact DSM menu path [connect-closed]; host key changed (blocking, loud; specified, not drawn) | Host key trust [connect-key] |
| Inspect | Zero disks: blocking | per-section progress | A failed probe is listed under "not known", never omitted | Untested compatibility text; Unsupported disables Install |
| Goal | n/a | n/a | Install disabled for Unsupported with reason | Boot manager highlighted for Untested |
| Storage | n/a | live recompute | **Blocking** in Keep mode (system size does not fit) with a non-primary "Set to 9 GiB" and the cap explained [plan-error]; **warning** (SMART) never blocks | Mode link is "Advanced" |
| Confirm | n/a | n/a | Mismatch shown as "Not a match yet" (no expected-value echo) | Wipe [confirm] and Keep [confirm-keep] |
| Install | n/a | Phases, boot-flash sub-steps, steps, log | **Connection lost** banner, reconnecting, cancel still free before step 3 [progress-lost] | Before / after the point of no return |
| Failure | n/a | n/a | Before PONR with 60 s clock [failed-pre]; after PONR with 15 min clock, Wipe [failed-post] and Keep [failed-post-keep] | Diagnostics export |
| Done | n/a | "Verifying..." | If verification fails: "Installed, but we could not reach it" (specified) | ARM variant from the unit's own values [done-arm] |
| Settings/About | n/a | refreshing | update/list refresh failure keeps the cache | Update available [update] |
| Mgmt Login | n/a | n/a | Wrong credentials; pause with SSH remedy [login-locked] | Arrived at new address with countdown [network-arrived] |
| Mgmt Dashboard | first 60 s after boot | skeleton | source failed = "unavailable", never 0 | Needs attention, degraded RAID, rebuild |
| Mgmt Network | "no cable" | applying | apply failed = immediate rollback | pending, arrived, rolled back |

Offline: the app works on the LAN. Disabled with explanation: list refresh, update check, report-back. The NAS needs Internet during the install (said at the gate).

---

## 6. Accessibility and keyboard plan

**Baseline decision (designed for): keyboard operation, contrast and scalable text are guaranteed. Screen-reader support in the native app is not guaranteed.** The UI says so (Find screen note, Settings/About) and names the documented CLI path `synofurb install ... --have-backup --accept-data-loss --confirm` as the equivalent. The management plane is semantic HTML and is screen-reader friendly.

**Targets**: WCAG 2.2 AA contrast for text and components; 32 px minimum hit target; 12 px minimum text; text size Default / Large / Largest; the demo bar has a 130 % toggle to check **960x640 at large text**: the footer action and the rail stay reachable, content scrolls, the stepper labels crowd (rule: at Largest the stepper collapses to "Step 7 of 9: Confirm").

**960x640 rules** (verified in the mockup with the 960x640 button): the footer with Back and the primary action is **pinned outside the scroll area**; the safety rail narrows to 200 px and drops the meter and the plan summary but keeps **state, can I cancel, target, way back**; the destroyed/changed/preserved content scrolls; the gate controls are in the scroll area and the pinned red button explains what is missing.

**Not colour alone**: status = icon + word + shape.

**Focus order**: title bar (Settings) -> stepper (landmarks) -> main content in reading order -> safety rail (jump with `F6`) -> footer. On entering a screen focus goes to its `h1`; on errors to the error summary. Disabled primary buttons stay focusable and carry the reason. The typed-phrase field announces match state in a live region (mockup: real `input`, `label for`, `aria-live`). The install screen has a polite live region for phase changes and failures; failure countdowns are `role=timer`.

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | move focus |
| `Enter` | primary action of the screen, **except** the erase button, which needs `Space` on the focused button |
| `Alt+Left` / `Alt+Right` | previous / next completed step |
| `Ctrl+F` | search the log |
| `F1` | help | 
| `F6` | jump between regions |
| `Esc` | close dialog; never cancels an install |
| `Ctrl+,` | Settings and About |

**Management plane**: `role=status` for the strip, `role=alert` for the rollback banner, visible focus, labels, `<th>`, 200 % zoom, no horizontal scroll at 390 px (checked), 44 px tab bar targets.

---

## 7. Content and copy rules

**Tone**: calm, specific; first person plural only for the tool; no exclamation marks, emoji, jokes at failure.

**Data-loss wording** (in this order): what is lost, with numbers; whether it can come back ("There is no undo"); what we cannot promise ("No guarantee any data survives"); what to do ("Back up first. A backup you have not tested is a hope, not a backup").

**Rules**
* Destructive buttons are verb + object ("Erase DS918+ and install Debian"). Name the unit with IP and serial on destructive screens.
* Every error says what happened, **whether disks were touched**, **whether the boot flash was touched**, and what to do, in that order.
* Say "point of no return" and always mean installer step 3.
* **Never describe the boot flash as unchanged or preserved.** Say "entries and files added, backed up first, DSM entry stays".
* **Never call the metadata backup a way to recover data.** Exact wording: "the first and last 1 MiB of every disk we change (partition tables, GPT headers). That is metadata only, not your files, and it cannot bring back a formatted filesystem."
* Do not state Synology procedures we have not validated (no RESET sequence). Point to find.synology.com and say to check Synology's current documentation.
* "Untested" is the normal state: "No one has reported an install on this model. Your install is the test."
* Units binary (GiB, TiB). Compatibility terms fixed: Verified, Community-reported, Untested, Known issues, Unsupported (reason).
* Secrets: "synofurb does not keep the password"; no "shown again later".
* ARM: only show values read from the unit; never show examples next to a Copy button; say the recipes are not verified on real boards.

| Instead of | Write |
|---|---|
| "DSM starts again exactly as it is now" | "DSM stays the default boot entry. Our entries stay on the boot flash until you remove them." |
| "disk headers saved, restore if needed" | the exact metadata sentence above |
| "Healthy" with a SMART warning | "Needs attention: 1 disk warning, 1 task running" |
| "Locked out, use the serial console" | "Wait 4:32, or restart the service over SSH: `sudo systemctl restart synofurb-mgmt`" |

---

## 8. Open questions for the product owner and engineering

1. **Disk metadata backup**: confirm the engine writes it (first and last 1 MiB of every touched disk), where it lives, how long it is kept. The UI promises only metadata.
2. **Resume after failure**: is there a supported way to re-run or resume from inside the rescue shell (`synofurb-dom` for the boot entry; "run the install again from step 1" for Keep)? The screens say what RECOVERY.md says and nothing more.
3. **Preflight for the boot flash**: can the preflight (before step 3) check that Debian's kernel + initrd fit on the flash and that a write-verify test passes, so a flash failure after step 3 becomes rarer? The Wipe failure sample shows the cost of not checking.
4. **Network confirm window**: is 90 s enough to open a new origin, accept a certificate warning and sign in? Proposal: the window pauses or extends once the login page at the new address has been loaded. Also: the `synofurb-net confirm` SSH command is a placeholder name.
5. **Certificate and name**: can the management certificate list the new IP and `<host>.local` so the warning is a single click and passkeys can work on the name?
6. **Unlock**: does restarting the web service really clear the lockout (the copy now says so), or is an explicit unlock command needed?
7. **Boot Page "Make default"**: confirm the agent always resets the default to DSM (so a "default = agent" never persists) and what the Boot page should show after that.
8. **App closed or PC asleep after step 3**: does the installer continue alone, and can the app reattach and replay the log? The gate currently says "keep this computer awake".
9. **Keep mode in v1**: hidden behind "Advanced" in these mockups; confirm.
10. **Reporting**: pre-filled GitHub issue (assumed) or an endpoint; serial numbers excluded.
11. **Offline installs**: the NAS needs Internet for packages; local mirror?
12. **ARM**: show the serial-adapter requirement on the Goal card (currently only the Find row and Done).
13. **Accessibility**: confirm the baseline above and the CLI equivalence; consider a keyboard-only "text summary" gate for the native app.
14. **Cross-track**: align the UI track's Confirm (hostname phrase, paste blocked, a different PONR step number) to: model phrase, visible identity, paste allowed, PONR = step 3.
15. **Localisation**: English only for v1?

---

## Revision log (after the UX review)

Each row: finding id, what changed (in `concept-a`, `concept-a/states.html`, `mgmt-a` and these documents), or why not.

| Id | Resolution |
|---|---|
| **D1** disk-header backup promise | Removed "disk-header backup" as a recovery claim everywhere. Replaced by (a) the boot-flash image with file name, verified, and (b) the **disk metadata backup** described exactly (first and last 1 MiB, metadata only, cannot recover a formatted filesystem, no help in Wipe mode). Restore has no restore action for (b). Restore card for the flash is the real `synofurb-dom restore`. FLOWS pattern 11, F2b item 6 rewritten. |
| **D2** install model contradicts engine | Phases kept but defined for the boot-flash flow (Staging = flash backup + agent add + send installer; Booting installer = reboot into agent, agent starts installer). The 12 steps are the engine's `INSTALL_STEPS`. **One point of no return: step 3.** Settings default boot method is "Through the boot flash"; kit is the alternative. The boot flash moved from "preserved" to "WILL BE CHANGED (backed up first)". "DSM starts again exactly as it is now" removed. Done no longer says the BIOS order is correct. |
| **D3** failure screens | Post-PONR shows the **15 minute rescue-shell countdown** and what happens at zero (DSM loader, Synology install page). Keep-mode failure drawn [failed-post-keep]; Keep text follows RECOVERY.md (volumes untouched unless hardware problem; Synology keep-data path documented, check current docs, no guarantee). RESET-button sequence removed. "Password is in the log" removed (summary on this computer). `failed-pre` redrawn (60 s clock, disks untouched, flash entries present), without rescue shell or restore options. Recommended action pinned in the footer (verified at 960x640). |
| **D4** gate identity | Gate card shows model, IP, serial, disk serials and a same-model collision warning; the sample inventory has two DS918+ units; phrase `ERASE DS918+`, paste allowed; mismatch hint no longer echoes the expected value. Cross-track alignment is open question 14. |
| **D5** Verified story | All sample data is Untested except DS923+ (Community-reported, from another project); DS211j is the Unsupported example; Verified appears only in the legend ("no model has this yet"); mockup tag in every title bar; Goal promotes Boot manager for Untested; Confirm shows the Untested fact. Inspect copy rewritten: "your install is the test". |
| **D6** rail wrong | Rail states are now fact-derived: Read-only; Plan, not applied; Boot flash changed (confirmed and running before step 3); Disks modified; Failed, disks untouched; Debian running. `#confirm` shows "Plan, not applied" (not "You confirmed"); `#progress` shows "Boot flash changed"; Restore shows the unit's real state ("Disks modified") with failure-aware wording; failure screens have their own cancel/way-back text. |
| **D7** Done screens | ARM Done is generated for RS816 (target in rail, button, report text, no `sdc`, no BIOS text); U-Boot lines are "read back from this unit" with the original `bootcmd`, labelled unverified on real boards, no example values next to Copy. Password: "copy now, not stored, not shown again"; `Finish` needs "I have copied the password". |
| **D8** gate content | Gate now carries the SMART warning verbatim in "Warnings you have not resolved", the Untested fact, "For the next 25 minutes" (stay awake, Internet needed, unplug USB/eSATA, 9 h sync), and the full plan summary. Button label matches. |
| **D9** Keep mode | Behind "Advanced: keep existing volumes (best effort)". "Never written" removed; the plan screen lists what Keep leaves and does not carry over; the blocking error is a typed-in value (16 GiB) that cannot fit, with a non-primary "Set to 9 GiB"; Keep Confirm drawn [confirm-keep] with its own destroyed/left-alone lists; Keep failure drawn. |
| **A11y** | Baseline decided and stated in the UI (Find note, Settings/About) with the CLI equivalent; the Confirm gate uses real checkboxes, labels, an input and a live status message, and enables the pinned erase button only when satisfied (tested); 960x640 and 130 % text checked by screenshot. |
| **M1** safe apply | New URL, certificate fingerprint (short + Show full), passkey caveat, 90 s and the SSH confirm are shown **before** applying and in the banner; login page at the new address shows the countdown [network-arrived]; Apply locked while pending; rolled-back copy does not assume the old address. Open question 4 (window length) and 5 (certificate names) remain. |
| **M2** health | Health is the worst component; dashboard leads with "Needs attention"; header chip on mobile; uptime/sync numbers made consistent (2 h 14 min, 24 %). |
| **M3** Boot page | No 15-minute promise for a foreign OS; the page says power-cycling returns to the default and that kexec leaves nothing of ours running; "DSM loader" entry shown and explained instead of "unavailable"; "Make default" on a non-Debian entry needs a typed phrase. |
| **M4** login | Lockout remedy is wait or SSH restart; says anyone on the LAN can trigger a pause; fingerprint shown as first 8 bytes + last 4 with Show full (full 32 bytes). |
| N1 | Preserved list trimmed to what matters; "no backup found on this NAS" added to Inspect. |
| N2 | Cancel install is a neutral button. |
| N3 | Login error: 2 attempts, "Try once more" is secondary; SSH-off state drawn; host-key-changed specified. |
| N4 | Nothing preselected on Find. |
| N5 | Copy slips fixed; the plan footer says the warning is shown again at the gate. |
| N6 | Manage unit page drawn [manage]. |
| N7 | Connection-lost state drawn [progress-lost]. |
| N8 | Cross-track conflict recorded as open question 14. |
| N9 to N14 | Create volume is one click; delete has checkbox + `delete data`; mobile network has DNS and IPv6; contradictory text fixed; services text gives the SSH start command; lockout scope stated. |
| Not fixed | Concepts B, C and mgmt-b are unchanged (marked "alternative, not revised after review"). Real GPUI behaviour, real screen readers and real users remain untested. |
