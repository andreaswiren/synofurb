# UX critic: the new desktop flows for 0.2.3

Review only. No gate verdict, nothing written to `design/approvals/`, no code edited, no cargo build run (disk is tight; nothing was created in the scratchpad either).

Branch HEAD `03d9d3b`. Read: `crates/synofurb/src/ui/{screens/inventory.rs, scanview.rs, netdlg.rs, netsel.rs, addhost.rs, addip.rs, screens/inspect.rs, facts.rs, addons.rs, screens/addons.rs, screens/confirm.rs, screens/done.rs, overlays.rs, app.rs, copy.rs}`, `crates/synofurb-core/src/{addons.rs, model.rs, plan.rs, inspect.rs, script.rs, discovery/networks.rs}`, `docs/{ADDONS,DISCOVERY}.md`, UX-SPEC D1 and D6b, `apps/mgmt-plane/addons/{portainer,plex}`, and these screenshots: scanning, scan-failed, empty, added (light), dlg-networks, dlg-add-probing, inspect-volumes, addons-chosen, confirm-addons, dlg-quick-addons. Everything below is judged from code and screenshots: I did not run the app.

## Verdict

The four flows are much better than what they replace. Four things are done well and should not be touched: every disk gets a visible "Use" tag, the removable-but-SATA bays of the real DS918+ are targets (the transport, not the `removable` flag, decides), the Add by IP dialog never ignores a click or an Enter and shows each check as it finishes, and the privilege tags on the Add-ons tab are visible text with a plain explanation ("which is as good as administrator rights on the NAS").

But two honesty gaps sit exactly where the stakes are highest, and one of them is a release blocker: **the Quick setup presets reach Confirm without the user ever seeing what Portainer is allowed to do**, although the engine already computes that text and the GUI drops it. A second group of findings is about claims the screens make that the code does not check (a scan of a /16 reported as a scan of a /16, "DSM data on them stays as it is", "the redundant data volume").

| | Count |
|---|---|
| Blocking for 0.2.3 | 1 |
| Major | 8 |
| Minor | 10 |

---

## Blocking

### B1. The Quick setup add-on presets hide what Portainer can do, and the first boot leaves it open on the LAN

Four facts, all from the code:

1. **Every preset includes Portainer** (`PRESETS`: "Plex" is `docker + portainer + plex`; UX-SPEC D6b says "each includes Docker and Portainer as the base"). The Quick setup dialog lists "None / Docker + Portainer / Frigate NVR / Plex / Frigate + Plex" with no summary, no tag and no (i). Someone choosing "Plex" is not told they also get Portainer.
2. **Quick setup goes straight to Confirm** (`apply_quick` → `go(Screen::Confirm)`), skipping the Add-ons tab where the privilege tags ("Controls Docker", "Uses NAS network") live.
3. **Confirm shows only names and one Internet line** ("Will be installed after first boot: Docker, Portainer, Frigate NVR, Plex Media Server" / "The first boot needs Internet to pull the container images."). `addons::warnings()` in the core already produces the missing text (privileges with `privilege_note`: "Whoever can log in to it controls Docker, which is as good as being administrator of the NAS", disk use, data-volume need, "without Internet the add-on shows needs attention"), but the GUI never calls it; `confirm_notes` builds only the Internet and offline lines, and passes `root_mib: None`.
4. **Portainer starts unattended at the first boot** with the Docker socket mounted, on `0.0.0.0:9443`. Its manifest says: create the administrator within 5 minutes or it locks, and "the same as root on this unit". The Done screen says nothing about add-ons (grep of `done.rs`, `install.rs`: no mention), and the desktop app never reads the first-boot result (`RESULT_FILE` is referenced nowhere in the GUI).

So a first-time user can pick "Plex" in a dialog called "Quick setup: recommended defaults", confirm an erase, and end up with a root-equivalent web console on their LAN whose administrator account is created by whoever opens the page first, within a window that starts at an unpredictable moment (after Docker is installed and the image is pulled), with no warning at the moment of consent and no pointer afterwards. The privilege confirmation the management plane insists on ("shown before install, must be confirmed") is not reproduced in the installer.

**Fix, any one of which would do for 0.2.3:** (a) show the privilege lines of `addons::warnings()` on Confirm for every ticked add-on, whichever way it was ticked; (b) in Quick setup, name what a preset installs ("Plex: Docker, Portainer, Plex") with the privilege tags; (c) drop Portainer from the Plex and Frigate presets (it is not needed for them, and "Docker + Portainer" stays the one preset that has it). Also: the Done screen needs a line per installed add-on ("Portainer: open https://<ip>:9443 within 5 minutes of the first boot").

---

## Major

### M1. A scan of a /16 is reported as a scan of a /16

`ResolvedNetwork` carries both `cidr` and `sweep` (a network of this computer larger than a /20 is swept only in the /24 around our address, `docs/DISCOVERY.md`). The UI never reads `sweep` or `sweep_hosts` (grep): the headline says "Scanning 10.0.0.0/16 · 87 of 253 hosts" at best, and the summary and the empty state say "Scanned 10.0.0.0/16 with 4 methods in 6 s." after probing 253 addresses. A user on a /16 home or lab network with the NAS in another /24 gets "No NAS found", believes the network was covered, and has no hint. Same for a Tailscale address (a /32 sweeps 0 hosts, per the core's own test): "Scanned 1 network" while nothing was probed.
**Fix:** show the swept range ("Scanned 10.0.1.0/24, the part of 10.0.0.0/16 around this computer") and say "nothing to sweep" for a /32; the data is already in `ResolvedNetwork`.

### M2. "DSM data on them stays as it is" is not checked, and a partly wiped array is possible

`plan.rs` warns "Not used and not touched: sde (usb disk), ... DSM data on them stays as it is", and Confirm repeats "Not touched: ...". Nothing checks whether a DSM data array has members on **both** a target disk and a not-touched disk. In `WipeAll` every candidate disk is reclaimed entirely (`analyse`), so an array with a member on an excluded disk is silently broken, while the text promises the opposite for that disk. The real DS918+ is fine (all four bays are targets), but the exclusion rule has a documented fallback ("removable, transport unknown") and old saved facts have no transport at all; on such a unit two of four bays could be "not touched" while their array is half destroyed.
**Fix:** a plan error ("md2 spans sda, sdb (not touched) and sdc, sdd (erased)") instead of the reassurance; or, at least, change the sentence to "synofurb does not write to them; a DSM volume that also uses an erased disk is lost".

### M3. Quick setup skips every plan warning, including the resyncing array

`Confirm` shows `DiskScope` ("Not touched: ...") but never `plan.warnings` (only `screens/plan.rs` renders them). The owner's md2 at 90.8% resync is an Alert on Inspect only; Quick setup then lands on Confirm with no trace of it, nor of low RAM, hybrid layout caveats or the not-touched warning. The destructive list on Confirm is complete and good, so this is not a data-loss gap; it is the last screen before the erase silently forgetting what the earlier screen said. The Inspect wording is also off for non-redundancy cases: `health_note()` covers "checking", where redundancy is intact, but the text says "until then the pool is not fully redundant".
**Fix:** one line on Confirm per remaining warning ("md2 is resyncing 90.8%: your data is not fully protected right now"), and only claim reduced redundancy for resync, rebuild or degraded.

### M4. Frigate and Plex are offered where `/srv/data` will not exist

The manifest requires bulk storage below `/srv/data`, "a mount of its own". The installer creates that mount only when the plan has a `vg_data` LV (`script.rs` `has_data_lv`; absent in Keep mode, where "your data volume is ... not mounted automatically", and when "free percent" is 100). The Add-ons tab does not look at the plan: tick Frigate in Keep mode and the first boot ends in "needs attention", found out later in a web page. The tab's own summary also says "the redundant data volume", which is false for "RAID 0" and "No RAID", both selectable.
**Fix:** use `plan.vgs` to disable the data-hungry rows with a visible reason ("Needs the data volume: not created in Keep mode"), and say "the data volume" without "redundant".

### M5. Nothing tells the user what happens after Done

The pages say "installed after the first boot" three times. What follows is not anywhere in the app: no per-add-on status, no address, no wait time (the catalog knows `approx_size_mib`, 3000 for Frigate, and the UI never shows it), no "if it fails" path, even though the engine is designed to exit 0 and leave an add-on "needs attention" ("the installation itself does not fail"). A user who ticked Docker with the Offline source (where the Alert says "its packages download at first boot", not "the NAS needs Internet at the first boot or nothing is installed") finds no Docker and no explanation.
**Fix:** on Done, one row per chosen add-on with its address or "installing at first boot, about N minutes", the Portainer note from B1, and where to see the result (the Manage page). Say plainly in the offline Alert that without Internet at the first boot Docker and everything on it is not installed.

### M6. Add-ons are presented as finished, while the docs say they have never run

`docs/ADDONS.md`: "Nothing in this part has run against a real Docker daemon ... First run on a unit (Docker install from the Debian mirror, a pull, a Portainer start) is the open check." The Add-ons tab, the presets and Confirm carry no such caveat. Together with B1 and M5 this is the weakest part of 0.2.3: a feature that has been exercised against test doubles only, reachable in two clicks from the Goal screen, in the same dialog as the erase.
**Fix:** either an "Experimental" tag and one sentence on the tab and in the release notes, or hold the presets back until one real first boot has passed. This is an accepted limit only if it is visible.

### M7. Changing networks does not change the results, and nothing says so

"Done" in Networks to scan only closes the dialog (`netdlg.rs`); no rescan, no "scan again to use this". The chip updates to "Networks: 2 of 5" while the list still shows the old scan. Tick Tailscale, press Done, see the same rows.
**Fix:** a rescan on Done when the selection changed (the dialog title could say so), or a visible "Scan again" state on the chip.

### M8. A custom SSH port is applied once, then forgotten

`on_add_done` → `select_row(key, port)` writes the typed port into the Connect form, but selecting the row later (`select_host(&ip)`, used by Connect and the row click) only sets the host. The port is stored in `added-hosts.json` (`AddedHost.target.port`) and never read back. Add `nas:2222`, select another unit, come back: the Connect form has whatever port was last typed; restart the app and it is 22. A unit on 2222 then fails; a different unit on 22 gets 2222. Safe (host-key pinning still applies), but the "Added by you" row is supposed to be the unit.
**Fix:** restore the stored port in `select_host` for an added row (and show it in the Address column when it is not 22).

---

## Minor

1. **Scan panel word count.** The "inventory" copy audit omits the scanning state. By my count it is about 29 words of copy plus the network label repeated on every line (four methods times one network, eight times with two), against the page's 35: roughly 64 to 72 of 60. The cheap fix is to show the network once per network, not once per method line.
2. **Confirm with add-ons is heavier than its budget.** Spec: 54 of 60. The add-ons card adds 23 words, and the privilege text from B1 will add more; make the card one line plus a disclosure, not two sentences.
3. **Plex claim token.** The field invites pasting a token whose real lifetime (4 minutes, stated only behind the (i)) is far shorter than the time to the first boot, so a pasted token will normally have expired. The catalog's own help says "normally leave this empty". If the honest advice is "leave it empty", do not offer the field in the installer; or put that sentence in the visible label, not behind a click.
4. **"Install target" is a status, not a consequence.** The Inspect "Use" column is clear. It is worth one line near it, "Install target disks are erased in Wipe mode", so the word "target" is not the only warning on that page; the erase is spelled out only from Confirm.
5. **Confirm lists disks, not what the user knows.** The "Erased disks" list is complete and good. The unit's DSM volumes (volume1, volume2, with used space) from the new Inspect table do not reappear on Confirm; "volume2 btrfs, 2.33 TiB used" is what people actually lose.
6. **`.to_lowercase()` mangles reasons.** "Under 8 GiB" becomes "under 8 gib", "USB disk" becomes "usb disk" in the Use tag, the plan warning and Confirm.
7. **Volumes show levels, not the name people know.** The real DS918+ is an SHR-like pool: "RAID 1 + RAID 5" is accurate but "SHR" is the word on the Synology screen. Add "(SHR-like)" when two levels sit on one VG.
8. **Empty state is factual only.** "Scanned 192.168.1.0/24 with 4 methods in 6 s." plus three buttons and Help. The two common causes (the NAS is on another network or VLAN; the NAS is asleep or firewalled) are in the Help topic only. One sentence would do.
9. **Add by IP: false "No answer".** The checks time out after 1.5 s each; on a slow Wi-Fi or VPN link a live NAS can read "No answer", and Connect is then disabled for that row with only "Check again" or "Remove" as exits. An override ("Connect anyway") or a longer timeout on retry would remove the dead end.
10. **Demo data disagrees between screenshots.** The same "DS918+ 192.168.1.20" is SN 18A0PDN014408 with 4 x 7.28 TiB on Inspect (29.11 TiB), SN 1920PDN123456 with 4 x 3.64 TiB on Confirm (14.55 TiB), and RAID 5 10.82 TiB on Quick. The real-unit fixture's volumes are 15.82 and 5.68 TiB, not the owner's 15.99 and 5.83. Harmless for the product, confusing for anyone comparing the series; the unit tests only assert structure (two volumes, `vg1`, `cachedev`).

---

## What I checked and found right

- **Every disk is accounted for.** Inspect lists every disk with "Install target" or "Not used: reason" (the boot flash included), the header counts "4 of 5 disks" when an 8 GiB-or-larger disk is left alone, Confirm lists each erased disk by name, model, serial tail and size, and adds "Not touched: ..." for the rest. `excluded_disks()` is documented as "never silently dropped" and the UI honours it.
- **sda/sdb report `removable=1` and are targets.** `Disk::exclusion()` keeps a removable disk on SATA, SAS, NVMe or virtual transport; only "removable and unknown transport" is excluded, and then it is shown. Tests: `the_real_ds918_keeps_all_four_internal_disks_even_though_two_report_removable`, `usb_and_unknown_removable_disks_stay_excluded_and_old_lines_still_parse`.
- **Volumes.** `volumes()` joins `/volumeN` mounts behind `cachedev_N` with the LV and the arrays, skips `@docker` and FUSE sub-mounts, and the Inspect table shows filesystem, size, used percent and the disks behind each; the first accordion is open.
- **Busy arrays.** A visible Alert on Inspect with the percentage (see M3 for the limits).
- **Find.** One honest progress panel (per method and network, Stop, "Stopping"), a one-line summary with the failed-method tag, "Stopped after 3 s" when stopped, a scrollable line list so five networks do not push the table off the page, and a sane network default (VPN and virtual off, "Recommended" chip, a /20 limit with its own message).
- **Add by IP.** Seven validation messages in plain words, Enter does what Add does, Cancel while probing stops and keeps the address, a stale probe is ignored by a ticket number, the three outcomes have three distinct tags with the detail one click away, "Check again" replaces the row instead of adding a second, Remove is one click and loses nothing the user cannot retype.
- **Add-ons.** Docker is on, disabled and labelled "Required by ..." in visible text; privileges are warning tags, not buried; a secret has `mask_toggle`; the copied command never contains the token (`@env:`); Next is disabled with a reason when the web interface is off or a value is invalid.

---

## Release status for 0.2.3

**Blocker:** B1.

**Fix before the 0.2.3 tag if it is a day's work, otherwise state in the release notes:** M1 (swept range), M2 (the not-touched claim), M4 (data volume gating), M5 (Done screen), M6 (visibly experimental). M3, M7 and M8 are small and close real dead ends.

**Accepted limits (not blockers, but say them):**
- No real-hardware run of the new flows, including the first real boot of Docker, Portainer, Frigate and Plex (M6 asks for the visible caveat).
- No installer rescue shell (DEV-UI-18) and no keychain (DEV-UI-20), as before.
- Image signatures and digests are not verified (images are pulled by tag, as `ADDONS.md` says), and containers have no egress policy.

## What I could not verify

- Keyboard order and focus on the Find table, the Networks dialog and the Add-ons tab beyond what the code declares (disabled Docker checkbox, field focus on dialog open, Enter does not close Add or Networks).
- The contents of the Copy command dialog beyond the flags (whether it tells the user to export `PLEX_CLAIM`).
- Whether `owner's real unit` classifies exactly as the fixture does: the fixture's sizes differ from the real ones.
