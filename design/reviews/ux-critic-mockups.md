# UX critic review: synofurb desktop concept A and management plane mgmt-a

Reviewer stance: unsentimental, evidence-driven. Every finding below was observed in a file I opened. Where a finding rests on a rendered screen, the URL hash is given so it can be reproduced: open `design/mockups/ux/<concept>/index.html#<screen-id>` in a browser, viewport 1100x760 (window button 960x640 for the minimum size) or 390x800. Where it rests on product facts, the file and row/section are cited.

Files read: `design/mockups/ux/{FLOWS.md,RECOMMENDATION.md}`, all five `ux/*/README.md`, `ux/concept-a`, `mgmt-a` (every screen id, text and screenshots), `concept-b` and `concept-c` Confirm screens (text diff against A only), `ui/direction-1` (README, RECOMMENDATION.md, Confirm screenshot, typed-confirmation markup), `README.md`, `docs/RECOVERY.md`, `docs/BOOT.md`. To check whether copy promises something the product cannot do, I also grepped `crates/synofurb-core/src/script.rs`, `crates/synofurb/src/ui/state.rs` and `cli.rs`; see D1 and D2. I did not modify anything outside this file.

---

## Verdicts

| Surface | Verdict |
|---|---|
| **(a) Desktop: Concept A (wizard + safety rail), with borrowed ideas from B and C** | **APPROVE WITH CONDITIONS.** The structure is the right one: one road, a persistent risk rail, a full-page gate, a named point of no return. But the screens as drawn are not a usable build spec. Their recovery and install content describes a product that is not the one in `docs/RECOVERY.md`, `docs/BOOT.md` or the engine. Nine blocking findings (D1 to D9) must be closed before any screen is built. D1 to D3 would make the app tell a nervous person false things at the exact moment they are deciding to erase their NAS. |
| **(b) Management plane: mgmt-a (sidebar shell)** | **APPROVE WITH CONDITIONS.** The shell is conventional and fine. The safe-apply flow, the health summary and the boot page each contain a claim or a state the design has not shown to be survivable. Four blocking findings (M1 to M4). |

Blocking findings: **13 in total (9 desktop, 4 management plane).**

---

## Scores (1 to 5; 5 = would ship). Evidence is in the findings.

| Criterion | (a) Concept A | (b) mgmt-a |
|---|:-:|:-:|
| Task success, primary flow (first conversion / first day-2 task) | 3 | 3 |
| Safety of the destructive flow | 3 | 3 |
| Error and recovery design (is "what now?" answerable, is it TRUE?) | 2 | 2 |
| Information architecture | 4 | 4 |
| Wording and copy (specific, honest) | 3 | 3 |
| Accessibility (keyboard, focus, not-colour-only, size, SR reality) | 2 | 3 |
| State coverage (empty, loading, offline, partial, failure variants) | 3 | 3 |
| Consistency between the two surfaces | 3 | 3 |
| Fit for "user has only SSH" | 2 | 2 |
| Agreement with engineering facts (README, RECOVERY, BOOT, code) | **1** | 2 |

Why A's task success is 3 and not 4: the screens only walk the one story that no real first-time user will have (a `Verified` DS918+ that boots by kexec). The boot-flash flow that `docs/BOOT.md` and the engine use for x86 units is nowhere on screen. See D2 and D5.

---

## Blocking findings

### Desktop (Concept A)

**D1. Critical. The "disk-header backup" is a recovery promise the product does not make, and could not keep as described.**
- Where: `concept-a#confirm` ("If something goes wrong", item 2, and the "WILL BE PRESERVED" list: "On this computer: disk-header backup"); `#progress-ponr` (step 2 "Save disk headers and DSM facts", "4 x 1 MiB saved to this computer"); the rail "Your way back" on `#progress-ponr` and `#failed-post` ("Disk headers from before are saved on this computer"); `#goal` (Restore card: "Uses the disk-header backup if one exists"); `#restore` ("Restore disk headers ... Works only if the data areas were not formatted afterwards"); `#failed-post` button "Restore disk headers from backup...". FLOWS section 4 pattern 11 and F2b item 5 specify it.
- Evidence: `docs/RECOVERY.md` lists exactly one pre-write backup: the whole boot flash ("DOM") streamed to the computer and hash-checked. It never mentions a disk-header backup. `crates/synofurb-core/src/script.rs` has no step that saves anything from the disks (step 2 is "Preflight: locating disks and verifying they match"), and a search of `crates/` and `payload/` for header or "first MiB" backups finds nothing. `crates/synofurb-core/src/domboot.rs` implements `backup_dom` and nothing for disks. FLOWS open question 5 admits nobody has decided whether recovery may be promised.
- Why it hurts: the first MiB of a GPT disk holds the protective MBR and the primary GPT. DSM's md superblocks sit inside the partitions, which start at or after 1 MiB, so "partition tables, RAID headers" is false for the RAID headers. In Wipe mode the installer then runs `mdadm --create` and `mkfs` over the old data areas (`#progress-ponr` log lines 14:21:46 and step 8), so the user's own caveat "works only if the data areas were not formatted" is always violated. A nervous person reads "we keep a copy of each disk's first 1 MiB" as "my data can be restored". It is the strongest comfort sentence on the gate, and it is false in Wipe mode and unproven in Keep mode.
- Must change: remove every mention until the engine does it and it is proven. If it stays, say what it restores ("partition table only, not your files") and that it does not help after a wipe. Replace it with the backup that really exists, the boot-flash image, and name the file. The `Restore disk headers` card and button must be disabled with a reason when no backup exists.

**D2. Critical. The install phases, point of no return, boot method and "preserved" list contradict the engine and the docs.**
- Where: `concept-a#progress`, `#progress-ponr`, `#confirm`, `#settings`, `#inspect`.
- Evidence:
  - Point of no return. Mock: steps 1 to 3 are free, PONR is the divider after step 3, "Cancel is free until install step 4" (`#confirm` rail, `#progress` divider). Engine: `PONR_STEP = 3` (`crates/synofurb/src/ui/state.rs:440`), and `script.rs` labels step 3 "POINT OF NO RETURN: releasing existing RAID/LVM and wiping reclaimed areas". The Direction 1 Confirm screen says a third thing: "Step 5 'Write partition tables' is the point of no return" (`ui/direction-1`, screenshot of "6 Confirm data loss"). Three artefacts, three PONRs.
  - Step list. The mock's 12 steps (e.g. "2. Save disk headers", "11. Install kernel and bootloader (GRUB)") are not the engine's 12 (`INSTALL_STEPS` in `state.rs`: "Load installer components", "Preflight: match disks", "Release RAID/LVM, wipe reclaimed areas", ...).
  - Boot method. `#settings` offers only "Automatic (kexec)" and "Installer kit". The engine's default for a unit with a boot flash is "Through the internal boot flash" (`state.rs` `BootChoice::Dom`: back up the whole flash, add a boot agent, reboot into it, start the installer). `#inspect` shows "Synology boot flash present" for the main-story unit, so the mock's own data says it is a flash unit. The pre-install phases (flash backup, agent install, agent reboot) appear nowhere in the 7-phase strip.
  - `#confirm` lists "Synology boot flash" under WILL BE PRESERVED. `docs/BOOT.md` says the tool adds menu entries `SYNOFURB_BOOT`, `DEBIAN`, `DEBIAN_OLD` to that flash and edits it. It is not preserved; it is modified (with a backup, and with DSM left as the default).
  - `#confirm` and `#failed-post` say before the PONR "DSM starts again exactly as it is now". The flash has already been written once the agent is armed. The honest statement per RECOVERY.md is "DSM boots again; the boot flash contains our entries until you remove them".
  - `#done` says "BIOS: boot order is already correct". The app cannot know that, and `README.md` tells users to check the BIOS boot order on x86 units.
- Why it hurts: the screens that carry the recovery story are built on the wrong boot model. A reviewer or engineer implementing from the mock builds the wrong phases, and the gate teaches the wrong exit.
- Must change: re-draw Install, Failure and Settings from the engine's step list and the DOM flow; use a single PONR number from one source; list the boot flash as "modified, backed up to <file>, DSM stays the default".

**D3. Critical. The post-PONR failure screen omits the 15-minute clock, contradicts RECOVERY.md on data-keeping, and the pre-PONR failure screen shows restore options that cannot apply.**
- Where: `concept-a#failed-post`, `#failed-pre`.
- Evidence:
  - `docs/RECOVERY.md` row "After the point of no return": the default is set to DSM and "the unit reboots after 15 minutes (so you can read the log) into the DSM loader". `#failed-post` tells the user "the installer is still running", offers "Continue without sdc", "Retry step 11" and "Open the rescue shell", and mentions no deadline. A person who reads the log, then goes looking for a USB cable, loses the rescue shell mid-task. Nothing says what the NAS will show after the reboot (Synology's "Not installed" page).
  - Item 3 of "Getting original DSM back" reads: "If the original DSM data volumes were never touched (Keep mode, install not yet past step 4) Synology's installer offers to keep your data." In Keep mode the PONR is the step the mock numbers 4, so "not yet past step 4" is the pre-PONR case. A Keep-mode user who fails at step 9 is therefore told, by implication, that keeping data is off the table. `docs/RECOVERY.md` says the opposite: after a Keep-mode failure the preserved volumes "are untouched unless the failure was a hardware problem" and Synology documents the keep-data reinstall, with "this project cannot guarantee it". The Keep-mode variant of this screen is not drawn at all (only the Wipe/GRUB-on-sdc story exists).
  - The RESET-button procedure ("hold RESET until it beeps, release, press and hold again") is asserted as fact. `docs/RECOVERY.md` points only to find.synology.com and Synology Assistant and says to check current Synology documentation; FLOWS open question 4 says nobody has validated the per-model text. `crates/synofurb/src/cli.rs:446` prints only the find.synology.com instruction.
  - `#failed-post`: "ssh root@192.168.1.40, password is in the log". The README says logs have secrets redacted, and the screen elsewhere says the log is exported.
  - `#failed-pre`: the first line says the NAS "never reappeared on the network", yet "Other options" offers "Rescue shell: ssh root@192.168.1.40" (nothing is listening) and "Try to restore original DSM...". FLOWS F2a explicitly says "No 'restore DSM' material is shown here: nothing needs restoring." The mock shows it anyway. A person who did nothing wrong is offered a restore action that sounds like the thing they need.
  - `#failed-post` at 960x640 (the minimum window): the first button the eye lands on is "Try to restore original DSM..." in the right column. The recommended "Continue without sdc" button is below the fold, and the footer holds only "Open log file".
- Must change: put the 15-minute deadline, in plain words, beside the rescue-shell option, with the app's own countdown. Draw the Keep-mode failure. State the Synology keep-data conditions exactly as RECOVERY.md does, or link without asserting. Remove restore and rescue-shell options from `failed-pre` unless the agent is reachable. Pin the recommended action in the footer at 960x640.

**D4. High. The typed phrase does not protect against the wrong unit it is claimed to protect against, and the unit's identity is not on the gate.**
- Where: `concept-a#confirm`; FLOWS section 4 pattern 5 ("The model in the phrase protects against the wrong unit when several NAS are listed"); `ui/direction-1` Confirm uses a hostname instead.
- Evidence: the phrase is `ERASE DS918+`. Two units of the same model (the common home-lab case; two DS220+ or two DS918+) pass it. The mismatch hint prints the expected string in a code chip (`#confirm`: "Not yet a match: expected ERASE DS918+"), and the label already shows it, so the check is a transcription exercise that needs no reading. The unit's IP and serial exist only in the safety rail ("TARGET DS918+ . 192.168.1.40"), and FLOWS section 6 puts the rail outside the default focus order ("reachable by F6, skipped by default"). FLOWS usability test 2 uses DS918+ against a different model, so it would not catch the same-model case. Cross-track conflict: Direction 1 types the hostname and "Paste is disabled on purpose"; FLOWS says paste allowed; `ui/RECOMMENDATION.md` says allow paste. DSM's default hostname (`DiskStation`) is shared across units too, so the hostname is no better.
- Why it hurts: the gate's job is "is this the unit you mean". For identical units it cannot tell them apart, and the one fact that does (serial, IP, disk serials) is outside the form.
- Must change: the typed value or the gate header must contain something that differs between two units of the same model (last digits of the serial, or the IP), shown inside the gate card with the disk serials. Test the same-model case. Pick one phrase rule and one paste rule for both tracks.

**D5. High. The only fully drawn story is a "Verified" unit that, under the project's own facts, cannot exist; the real first-run path (Untested, boot flash, maybe ARM) is not designed.**
- Where: `concept-a#find`, `#inspect`, `#plan`, `#goal`, `#confirm`.
- Evidence: `#inspect` says "Verified ... List v2026.10.3 ... 3 maintainer reports. Installs have been completed on real DS918+ hardware with Debian 13 and the 6.12 LTS kernel." `#plan` says "6.12 LTS is what DS918+ installs were verified with". `README.md` says everything was "developed and tested without a Synology unit" and `docs/BOOT.md` says "None of it has run on a real Synology yet ... every other model is Untested" (the DS923+ being the only one with an outside report). The mock labels DS923+ "Untested, no reports" and DS1621+ "Community-reported, 2 reports". The Confirm screen does not repeat the compatibility state at all. The Goal screen has one variant; the "promote Boot manager for Untested units" behaviour in FLOWS F1 step 7 is not drawn.
- Why it hurts: the first real users will all see "Untested". Their gate must say "this model has never been run by the maintainers; first install on your hardware is the test; have a serial console ready" (README's own banner). The mock trains the team to design for the reassuring case, and the sample data would read as a claim if the mock is ever screenshotted for docs.
- Must change: mark all demo data visibly as demo; draw the Untested path end to end (Goal with Boot manager promoted, Confirm with the compatibility line). Fix the sample data to match `docs/BOOT.md`.

**D6. High. The safety rail, the product's central claim, is wrong in three states.**
- Where / evidence (rail text read from `data-s-lvl` on each section):
  - `#progress` (install running, pre-PONR; user has already confirmed): rail says "Ready to erase / Waiting for your go. You can still back out." and "Your way back: Cancel, or reboot the NAS: DSM starts as before." The go was given; the NAS has been written to (kernel uploaded; with the DOM flow, the flash too).
  - `#confirm`: the four-step meter highlights "3 You confirmed" before the user has confirmed anything.
  - `#restore` (rail level 0): "Reading only ... Nothing on the NAS has been changed" and "Nothing changed, nothing to undo. Power-cycle: DSM starts as before." This is the screen a user reaches after a failed or completed conversion (`#goal` card, "For failed or converted units"). The rail tells a person with a wiped NAS that a power cycle brings DSM back.
  - `#failed-pre`: "Failed safely / Your disks were not changed" is true for disks; the flash state is not mentioned (D2).
- Why it hurts: the rail is the one element the design says never forgets the risk state, and it is the trust anchor on the two screens where a person is most anxious. Wrong text there is worse than no rail.
- Must change: derive rail state from real facts (flash written, disks written, install running), add a state for "install running, confirmed", and make Restore/Recover read the unit's actual state before it says anything.

**D7. High. Done screens contain wrong or unsafe content; the ARM variant is a copy-paste of the DS918+ screen.**
- Where: `concept-a#done-arm`, `#done`.
- Evidence: `#done-arm` is the RS816 screen but repeats DS918+ content: "Replace disk sdc", "BIOS: boot order is already correct ... press Del on the serial console", the footer button "Manage DS918+", and the report text "DS918+ . Debian 13 . kernel 6.12 . success". It also shows U-Boot commands the user is told to paste into the bootloader of a headless board, labelled "Illustrative values: yours are saved in /boot/uboot-env.txt". `README.md` lists U-Boot recipes as not verified on real boards and `docs/RECOVERY.md` says a serial adapter is needed and SSH-only recovery is not possible there. "Copy commands" next to a box that says its values are illustrative is a hazard. `#done`: "Shown now and once more if you reopen this unit. It is not stored by synofurb." If it is shown again later, it is stored somewhere; the sentence contradicts itself.
- Must change: generate ARM content from the unit; show only values the app has actually read from this unit, with a diff against the saved environment; resolve what "stored" means and say it once.

**D8. High. The gate leaves out consequences the user needs, and FLOWS claims a warning travels that does not.**
- Where: `concept-a#confirm`, `#settings`, `#progress`, `#plan`, FLOWS pattern 13.
- Evidence: FLOWS pattern 13 says the `sdc` SMART warning appears "in Inspect, Plan and Confirm and Done, in the same words". Counting "reallocated" in the screen text: `#inspect` 1, `#plan` 1, `#done` 1, `#confirm` 0, `#settings` 0. The wording also differs ("8 reallocated sectors", "Disk sdc reports 8 reallocated sectors", "Replace disk sdc: it reported reallocated sectors"). The plan screen itself calls a failing disk during the first sync "the most common way people lose data"; it is the one warning that should reach the gate. The gate also omits: keep this computer awake and on the network for about 25 minutes (appears only on `#progress`: "do not power off or sleep this computer", and "keep this window open" on `#progress-ponr`, while FLOWS open question 8 says nobody has decided what happens if the app is closed after the PONR); the NAS needs Internet during the install (open question 7); the first RAID sync takes about 9 hours with reduced protection. `#settings` ends with the button "Review and confirm", but A's Confirm screen does not review the plan (hostname, user, boot method); B and C do. A's only plan line is in the rail.
- Must change: carry the disk warning to the gate verbatim; add a "what you must do for the next 25 minutes" line; show the plan on the gate or rename the button.

**D9. High. Keep-volumes mode cannot ship as drawn.**
- Where: `concept-a#plan-error`; `#plan`; RECOMMENDATION risk "Keep mode promises more than it can deliver".
- Evidence (`#plan-error`, 960x640 screenshot): the banner "System size 32 GiB does not fit" offers a primary-blue "Use 9 GiB (recommended maximum)" while the same page says "recommended 32", "max 9.5 GiB" and (on `#plan`) "8 GiB works but is tight with the 7.x kernel and Docker". One click accepts the size the page elsewhere calls tight, styled as the recommended fix. The diagram labels every data partition "KEPT, never written" in absolute words while the legend and body say "Preserved (best effort)" and "nothing is guaranteed". The error is only discovered after the user toggles the mode and sees the whole layout turn red ("carried over from Wipe mode"). Nowhere does the mock say what Keep mode actually leaves: DSM's OS is gone, shared folders, users and packages do not carry over, and the preserved volume is not mounted automatically and mounts read-only (`README.md`, "Backups, data loss and keep my volumes"). There is no Keep-mode Confirm screen. The mock Confirm only shows Wipe, so the phrase `ERASE DS918+`, the "WILL BE PRESERVED (best effort)" list and the "keep" consequences (the point of this mode) were never tested.
- Must change: draw the Keep-mode Confirm and failure; remove "never written" absolutes; state what Keep leaves in the plan screen itself. Either hide Keep behind Advanced (the recommendation's own suggestion) or fix the above first. I recommend hiding it for v1.

### Accessibility decision (counted under D-series as a condition, not a separate number)

The plan in FLOWS section 6 is good. The mock demonstrates none of it: in `#confirm` the checkboxes are `<span aria-hidden="true">`, the phrase field is a `<div class="inp">`, the `<label>` is not tied to a control, and nothing is tab-reachable except table rows. That is acceptable for a mock but means no keyboard path of the gate has been shown. Larger: `ui/RECOMMENDATION.md` says GPUI has no known screen-reader tree, and FLOWS open question 14 leaves the baseline undecided. A blind user cannot complete the gate. The README already offers a headless `synofurb install --have-backup --accept-data-loss --confirm`. The condition is that the app states plainly, on the first screen and in README, that it is not screen-reader accessible and points to the CLI path, and that the decision is taken before screens are built. Also: the Confirm lists (the destroyed/preserved text that matters most) are 13 px and the caption text is 12 px at default size; the "Largest" text setting has not been shown at 960x640.

### Management plane (mgmt-a)

**M1. High. Safe apply cannot be confirmed as drawn: the "new address" is a new origin, and the flow never shows arriving there.**
- Where: `mgmt-a#network-pending`, `#network-rolled`, `#network-mobile`, `#login`, `#users`.
- Evidence: the banner reads "If you can read this page at the new address, the settings work" and the info box "you have 90 seconds to confirm from this page at the new address". At `192.168.1.45` the browser has a new origin: the self-signed certificate warning appears again, the host-only session cookie is not sent, and a login is required. `#users` shows `andreas` with "2FA on", and `#login` offers "Use a passkey instead". Passkeys are bound to a relying-party ID derived from the host name, and browsers do not accept an IP address as one (I did not test this; see "could not verify"). The countdown banner never appears on the login page in the mock. The 90 s therefore has to cover: accept certificate, type password, type TOTP, and only then see "Keep these settings". The rolled-back banner (`#network-rolled`) is drawn on the old address page, a page the user who lost the connection never sees; the new-address tab just fails. The copy hard-codes the way back ("you reconnect at 192.168.1.40"), while RECOMMENDATION itself lists "DHCP gives a new address after rollback" as a case to test. In `#network-pending` the form still shows an active "Apply again" while a change is pending.
- Why it hurts: the failure is a loop in which a change that works cannot be confirmed in time and rolls back, teaching the owner the feature is broken. The sentence "do nothing" is only safe if the user who succeeds can also confirm.
- Must change: design the arrival state (login or certificate step with the countdown visible and extended, or a confirm path that does not need a fresh session, such as an SSH command shown beside the countdown); show the dead-tab state and what the user should do (find the unit again by name or scan, not a fixed address); disable "Apply again" while pending.

**M2. High. The dashboard says "Healthy" on a unit the same design calls at risk.**
- Where: `mgmt-a#dash`, `#dash-mobile` (390x800).
- Evidence: top chip "Healthy" (mobile header) and hero card "Healthy . 1 task running", while the disk table shows sdc with "8 reallocated sectors" in both md0 and md1, and md1 is in its first resync (38 %, [UUUU]). `RECOMMENDATION.md` says to take mgmt-B's "Needs attention" list to the top of the dashboard. mgmt-a, the surface being approved, does not have it. Usability test 8 ("Is anything wrong with the NAS?") has the answer "no" on this screen. The desktop plan calls the same condition the most common way people lose data. Minor mock inconsistency: uptime 3 d 4 h with a first sync still at 38 % and "about 9 h" total.
- Must change: derive the health summary from the worst component; the card says why. Do not ship mgmt-a's dashboard without the borrowed "Needs attention" block.

**M3. High. The Boot page promises a safety net the docs do not provide and hides the documented way back.**
- Where: `mgmt-a#boot`.
- Evidence: "If the system you boot does not call back within 15 minutes, the NAS reboots into the default entry." `docs/BOOT.md` describes a timeout only for the agent (it reboots into DSM if nobody logs in). Once an arbitrary OS (Netboot: netboot.xyz) is started by `kexec`, no part of the product runs; FLOWS open question 2 admits this is unresolved ("If not, the 15-minute auto-return cannot be promised"). Power-cycling is the only guaranteed return in `docs/RECOVERY.md`. "Make default" sits next to "Set for next boot only" and "Reboot into selected entry" with no warning for a netboot entry, and a headless box that loses power would come back into netboot. "DSM rescue: unavailable on this unit: DSM was replaced and its boot area reused" contradicts `docs/RECOVERY.md`/`BOOT.md`: the DSM loader entry remains on the boot flash and is the documented route to Synology's reinstall page.
- Must change: either prove a hardware watchdog and say so, or say "power-cycle returns to the default". Make "Make default" on a non-Debian entry a typed confirm. Relabel the DSM entry "DSM loader (starts Synology's reinstall page; DSM itself is gone)" instead of disabling it.

**M4. High. The login page tells an SSH-only user to find a serial console, and its certificate check is a 16-bit check.**
- Where: `mgmt-a#login-locked`, `#login`; RECOMMENDATION risk "keep the serial-console unlock".
- Evidence: "Locked by mistake? Use the serial console: synofurb-unlock andreas." The stated constraint is SSH only, and the installed system has SSH enabled with a sudo user (`README.md`); the lockout is a web login lock. The unlock should be an SSH command. `#login`: "Check that it ends in ...3C:A1". Two bytes can be matched by an attacker who generates certificates for a few tens of thousands of attempts, so the check gives a false sense of verification. The desktop Done screen shows the first 12 bytes plus the last 2 (better, still truncated).
- Must change: SSH unlock command first, serial last. Show the full fingerprint in a place the user can compare (or its first 8 bytes and last 4 with a "copy full" control).

---

## Non-blocking findings

**Desktop**
- N1. The WILL BE PRESERVED list is padded with trivia ("NAS hardware, BIOS settings", "MAC addresses and your router's DHCP reservation") under the heading "(best effort)". It makes the preserved column look as big as the destroyed one and dilutes the one line that matters ("No data on the four disks is preserved"). `ui/direction-1` does better (config backup, USB drive, and "A way to get DSM back: DSM_DS918+_72806.pat cached here, but your data and settings will not return"; also "No backup found on this NAS: No Hyper Backup, USB Copy or rsync tasks configured"). A's Confirm has neither; both are better than what A has.
- N2. `#progress`: "Cancel install" is a red outline button, and the divider is red. FLOWS says red is reserved for data loss and the PONR with one red element per screen. The safe action looks like the dangerous one.
- N3. `#connect-err`: "Login failed ... (2 of 3 tries)" and a primary "Try again" directly under a warning that DSM may auto-block this computer. The 3rd try is encouraged. Also FLOWS lists "port closed: SSH is off" and "host key changed" as designed states; only `connect-key` and the wrong-password state are drawn.
- N4. `#find` pre-selects the first row (DS918+) and enables "Connect to DS918+". With several units the first Verified one is a default target for a destructive tool. Start with nothing selected.
- N5. Copy slips: "312 lines lines" (`#progress-ponr`); "Plan is valid. 1 warning." while the warning is a failing disk with no way to acknowledge it.
- N6. Concept A offers "Manage DS918+" from Done into the inventory, while the recommendation wants B's unit hub. Not designed yet; no screen to check.
- N7. Nothing shows how the app behaves when the laptop sleeps or the Wi-Fi drops during the upload (the only trace is a WARN log line "upload slow ... wifi?" on `#progress`).
- N8. `ui/direction-1` and `ux/concept-a` disagree on gate details (hostname vs model, paste blocked vs allowed, PONR step). The recommended pair is therefore under-specified as a combined spec.

**Management plane**
- N9. `volume-confirm` makes the user type the volume name to create a volume ("Existing data is not affected"). `RECOMMENDATION.md` says never to add confirmations that train click-through; FLOWS says friction proportional to loss. Typed-name belongs on delete, not create.
- N10. `storage-delete`: "Is it backed up elsewhere?" is a question with no control; "Type data to confirm" asks for a four-letter word that is also the mount path on the same screen. It does list what uses the volume (2 SMB shares, 1 NFS export), which is good.
- N11. `network-mobile` has no DNS or IPv6 fields, so a static address on a phone cannot match the desktop form.
- N12. `network` pending info box contradicts itself: "confirm from this page at the new address ... You will be redirected".
- N13. `system-power`: confirm lists SSH sessions and RAID sync (good); "Stopping synofurb-mgmt ends this session: confirm required" does not say how to restart it (SSH command).
- N14. Lockout of 5 failed attempts in 10 minutes per account allows anyone on the LAN to lock out the owner; the page says other accounts are unaffected, but not that the owner can be locked by someone else. Add source scope or an SSH-side override (see M4).

---

## What is genuinely good (with evidence)

- **The Confirm page is a page, with specifics.** `concept-a#confirm`: "volume1 (btrfs, 10.9 TiB, 6.2 TiB in use): 3 shared folders, 1 Docker project", "All 14 btrfs snapshots (they live on volume1, so they are not a backup)", destroyed list first, and above the fold at 960x640. The red button names the consequence ("Erase DS918+ and install Debian") and the reason it is disabled sits next to it ("Tick both boxes and type the phrase to enable"). Keep this wording pattern.
- **Honest failure facts.** `#failed-pre` says "Disks touched: No" with a green mark; `#failed-post` says "Disks touched: Yes. All 4 disks were repartitioned at 14:21:07. DSM and volume1 no longer exist." with the time. The option order puts "Continue without sdc's boot copy" first and "Restore original DSM" last. That is the right priority; it only needs D3 fixed.
- **The PONR is an object in the list.** `#progress-ponr`: a red labelled divider with the pass time, and the footer button reads "Cancel unavailable" rather than disappearing.
- **Compatibility badges** differ by shape and glyph, not just colour (`#find`: filled "Verified", outlined "Community-reported", dashed "Untested", amber "Known issues" with a triangle, red "Unsupported" with a cross) and each non-Verified badge has its reason underneath. "Verified" is explained as "still does not guarantee your disks or your data survive" (`#inspect`).
- **The host key is shown before credentials** (`#connect-key`) and the Connect screen lists exactly what will be read and offers "Show the exact script".
- **Rollback banner** (`mgmt-a#network-pending`, `#users-rb`, `#network-mobile`): persists on every page, countdown plus a progress bar, "Keep these settings" and "Roll back now" side by side, "do nothing and the old settings come back", fits at 390 px without horizontal scroll (checked: scrollWidth 390 on dash, network, storage, boot). The rolled-back copy ("Rolled back automatically at 14:53:33 ... your edit is kept as a draft") is specific.
- **Power confirm** (`system-power`) lists running RAID sync and open SSH sessions.
- **Direction 1's** cached `.pat` idea and "no backup found on this NAS" check are better gate content than A's; borrow them.

---

## The three riskiest assumptions

1. **That the recovery story is true.** The gate teaches an exit (header backup, Synology keep-data reinstall, rescue shell, "DSM starts as before") that D1, D2, D3 and M3 show is partly not built, partly not what `docs/RECOVERY.md` says, and partly unverified (RESET sequence). If a user decides to proceed because of one of those sentences, the product has misled them at the only point that matters.
2. **That a user can stay connected and reach the NAS in the bad moments.** After the PONR the app asks the user to keep a window open for 25 minutes (no decision on sleep or Wi-Fi drops), the rescue shell expires after 15 minutes, and safe-apply requires a fresh login at a new certificate-warning origin inside 90 s. The whole design assumes the human is present and the network is good at exactly the moments when neither is guaranteed.
3. **That the typed phrase plus rail teaches consequences and catches the wrong unit.** Nothing in the design has been tested for comprehension; the phrase fails for same-model units (D4); the rail is wrong in three states (D6); the gate is built around a Verified unit the project does not yet have (D5).

---

## What I could not verify

- **Real GPUI behaviour.** All mocks are HTML. Rendering, focus rings, text scaling, high-contrast modes and whether a screen reader can read the app at all (the project's own UI review says there is likely no tree). I did not run `synofurb gui --demo`.
- **Keyboard and screen-reader paths.** The mocks use fake controls (spans and divs), so tab order, live regions and the "Enter never triggers the erase button" rule are claims in FLOWS, not observed.
- **Real users.** No comprehension or wrong-unit data exists. Scores are my judgement.
- **Synology facts.** The RESET button sequence, whether Synology's installer keeps data volumes after a Keep-mode failure on DSM 7.2, and the "Not installed" page behaviour on a unit after our loader runs. I only compared the mock against `docs/RECOVERY.md`, which itself says it cannot guarantee these.
- **WebAuthn on IP origins** (M1): I believe browsers reject an IP address as a relying-party ID, but I did not test it. If passkeys only work on a hostname (for example `ds918.local`), the login page needs a note and M1 gets worse.
- **Whether DS918+ really has an editable boot flash.** I relied on the mock's own "Synology boot flash present" line and `docs/BOOT.md`'s statement that x86 units have one; the code decides this per unit at run time.
- **Concepts B and C and mgmt-b** were only compared against A through text of their Confirm screens and the recommendation; I did not review them in full. B and C do show the plan on the gate, which A does not (D8).
- **Dark mode, light/dark contrast, the 130 % text scale, Windows/macOS rendering**, and the mgmt-a pages not named above (mounts, audit, users dialog, create-volume steps 1 to 3).
- **Whether the engine will gain a disk-header backup.** My search found none today (D1); if one is planned, the copy still needs the caveats listed there.
