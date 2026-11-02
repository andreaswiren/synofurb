# synofurb UX spec v2

Binding input: `design/BRIEF-v2.md` (the first mockups were rejected: too much text, hand-made widgets). This is a **spec, not mockups**. It defines, for the desktop app (GPUI, `gpui-component` 0.8.1 via `gpui-kit`) and the NAS web UI, every screen's exact visible elements, their word budget, where each removed explanation now lives, and which widget realises it. Facts follow `docs/RECOVERY.md`, `docs/BOOT.md`, `docs/COMPAT.md`, `docs/RAID.md` and `README.md`. The real application is later reviewed against this file.

Sample values (DS918+, 192.168.1.40, SN 1930PDN123456, `sdc` with 8 reallocated sectors) are only sample data. **No model is Verified today**; the primary path is **Untested**.

## 0. Rules that apply to every screen

1. **Budget.** At most 60 words of static prose per screen, shell included (titles, subtitles, labels, options, column headers, buttons, prose; data in braces is not counted). Title at most 4 words; subtitle at most 12; label at most 4; button a verb plus at most 2 words.
1c. **What is data (not counted) and what is prose (counted).** Data: entity names, sizes, addresses, hashes, versions, times, percentages and "n of m" progress, log lines, engine step names inside a list. Prose: any sentence or clause, every subtitle, every error cause (the engine sentence goes into a Popover and the log; the screen shows a 2 to 4 word label). The prototype checker must count any data span of more than 3 words that contains a verb.
1b. **What is counted.** Only what is visible in the default state of the screen. Closed menus, closed selects, dropdown items and popovers are not counted (their items are at most 4 words each). Open Sheets and Dialogs are counted as separate screens or bounded separately (section 4).
2. **Where explanations go.** Tooltip (one sentence, hover and keyboard focus) for a label or a disabled control; Popover (short paragraph + action) from an info icon; Dialog for decisions and anything long; Help topic for reference. Progress and logs are collapsed Accordions except the current step. Every info icon links to a Help topic id.
3. **The safety gate stays visible but terse**: identity, destroyed/changed/preserved as names and sizes, three confirmations, typed phrase.
4. **Copy module.** All strings live in one module per surface: `Copy { id, short, tip, help: Option<TopicId> }`. `short` is what is drawn, `tip` is the Tooltip (one sentence, at most 24 words), `help` is a topic id (bodies at most 150 words, section 7) or a Dialog id (section 4). Tests: per-screen word budget (sum of `short` of the screen plus shell), title/subtitle/label/button limits, `tip` single sentence, every `help` id exists, every topic at most 150 words, no string outside the module.
5. **Fonts**: Inter for the interface (Montserrat for the wordmark only), JetBrains Mono for code, logs, hashes, commands; bundled with their OFL licences. **Icons**: Lucide only. **Scrollbars**: themed (thin; theme tokens `scrollbar`, `scrollbar_thumb`, `scrollbar_thumb_hover`; light and dark).
5b. **GPUI input rules (verified in gpui-component 0.8.1).** Tooltips are hover driven (no focus path in `tooltip.rs`): they only supplement, and the prototype's `check_tips` enforces that every tooltip sits on a focusable control. Anything needed to act or to stay safe is on a focusable trigger (an info `Button`, or a `Tag` that is a Popover trigger). **Disabled controls are real disabled `Button`s and are not Tab stops; the reason is a sibling focusable info `Button` ("Why locked", "Why unavailable") that opens a Popover.** GPUI Buttons activate on `Enter` and `Space`; the prototype suppresses `Enter` on Erase with a key handler, which the real app must replicate (the safeguard is the full gate and the position of the control, not the key). Fonts are bundled as TTF/OTF, not WOFF. Scrollbars come from `.overflow_y_scrollbar()` with the three theme tokens.
6. **Accessibility baseline (stated in the UI)**: keyboard, contrast and scalable text are guaranteed; **screen readers are not guaranteed in the native app**. The documented CLI (`synofurb install ...`) is the equivalent path and is named in the first-launch Dialog, Help, and About. The web UI is semantic HTML and is screen-reader friendly. Status is never colour alone (Tag has icon + text).
7. **Layout**: 960x640 minimum; no horizontal scrolling; the footer with the primary action is pinned outside the scroll area; visible focus on every control.

## 1. App shells

### 1.1 Desktop shell

Three variants: **wizard** (D2 to D11, D13, D16), **page** (Manage D15/D18 and Recovery D14: no Stepper), **app** (Settings D12 and About D17: wordmark only). Find (D1) is the wizard shell without the unit header.

| # | Element | Visible text | Widget | Words |
|---|---|---|---|---|
| 1 | TitleBar: logo mark + wordmark | synofurb | `TitleBar` + SVG logo (custom asset); on macOS the traffic lights are native and left | 1 |
| 2 | TitleBar actions | (icons only: circle-question-mark = Help, settings = Settings, menu = About / Check for updates) | `Button` icon + `Tooltip`, `DropdownMenu` | 0 |
| 3 | Stepper (wizard only) | Find, Connect, Goal, Setup, Confirm, Install (six steps; Inspect is the second half of Connect, Done is the end of Install); all labels stay visible down to 960 px | `Stepper` + `StepperItem` (horizontal). **Not in the Tab order**: it is a landmark; `Alt+Left/Right` goes back through completed steps; completed items are mouse-clickable | 6 |
| 4 | Unit header (all unit screens, persistent) | {tile · DS918+ · 192.168.1.40 · SN 1930PDN123456 · DSM 7.2.2 or Debian 13 · 4 disks · 14.55 TiB · CPU · RAM} + compat Tag (Untested / Community-reported / ...) + conditional Tag Twin | `Label` rows (mono for data) + `Tag`. It replaces every identity line on D2 to D15. After conversion it shows the unit's current OS (Debian 13 · 6.12.57), never the old DSM version | 1 |
| 5 | StatusBar left | Risk Tag, at most 2 words (states below) | `StatusBar` + `Tag` (icon + text) | 2 |
| 6 | StatusBar right | Way back | `Button` (link) opening a `Popover` with the one-line fallback for the current state | 2 |

**Shell words: wizard 12** (wordmark 1 + Stepper 6 + unit-header Tag 1 + risk Tag 2 + Way back 2), **page 6**, **app 1**, Find 14.

**Risk Tag is a pure function of the unit's facts**, never of the screen name: *Read-only* (nothing written, no flash entries); *Plan only* (plan prepared, not applied); *Flash changed* (our entries or the agent are on the boot flash, no disk written; also after a cancel, a stopped install and a boot-manager install); *Disks changed* (the first disk was written; also on Recovery after a failed install); *Failed safely* (failed, disks untouched, flash may hold our entries, and the engine confirmed DSM as the default); *Boot unconfirmed* (the agent did not answer within the 10-minute wait: disks untouched, but the engine reset nothing and cannot tell whether DSM is the default, see D9); *Debian running* (verified, default boot is Debian); *Needs attention* / *Critical* / *Unreachable* (Manage: the worst health component or no answer: these replace *Debian running*). The Tag and Way back are **hidden on Settings, About and Help** (they are not about a unit). Way back popover, one sentence per state: Read-only/Plan only "Nothing to undo."; Flash changed "Power-cycle or Cancel: DSM boots. Our entries stay until removed."; Disks changed "Rescue shell 15 min, then the DSM loader; no data promise."; Failed safely "DSM boots again by itself."; Boot unconfirmed "DSM may not return by itself: see Help, Recovery."; Debian running "Recovery stays under Manage."; Unreachable "Check power and network, then Retry."

Manage (D15/D18), Recovery (D14) and Boot manager (D13) hide the Stepper; Help (D16) is a Sheet over the current screen; Settings (D12) and About (D17) replace the content area.

**Focus and navigation rules.** On every navigation focus moves to the page title (or the error summary / the first control of a dialog), not to `body`. Tab order starts in the content; the TitleBar buttons are reachable with `F6`. `F1` opens the Help topic of the focused control (its `help` id), falling back to the topic list. Wizard order (decision taken): six steps, Find, Connect, Goal, Setup, Confirm, Install. D3 Inspect opens automatically after Connect; D5 Plan and D6 System are the two tabs *Storage* and *System* of the Setup step; D10 Done is the end of Install. *Quick setup* on Goal skips Setup and lands on Confirm, which always shows the After line.

### 1.2 Web shell (all pages except Login)

| Element | Visible text | Widget (web) | Words |
|---|---|---|---|
| Brand | synofurb (+ {host}) | `header` + inline SVG logo | 1 |
| Navigation | Dashboard, Storage, RAID, Mounts, Network, Users, Boot, System, Audit, Help, About | `nav` (sidebar; `aria-current=page`), Lucide icons | 11 |
| Status strip | Tag "Needs attention" (worst health) + {RAID sync 24 % · up 2 h 14 min} | `role=status` strip, Tag | 2 |
| Rollback banner | only while a network change is pending (see W6) | Alert, `role=alert` | conditional |

**Shell words: 14.** Phones: nav collapses to a bottom tab bar (Home, Storage, Network, Users, More) with 44 px targets; the strip and rollback banner stay. Web "widgets" are named after the gpui-component equivalents so both surfaces share vocabulary: Tooltip = `role=tooltip` on hover and focus; Popover = `popover` attribute; Dialog = `<dialog>`; Sheet = `<dialog>` docked right; Accordion = `<details>`; Stepper = `<ol aria-current=step>`; Tag = `span.tag` with icon; Alert = `role=alert|status`; Toast = `role=status` live region.

## 2. Widget catalogue (gpui-component 0.8.1, read from `src/`)

**Exists and is used** (module: use): `sidebar` (Sidebar, SidebarMenu: not used on desktop, wizard navigation is the Stepper; Settings uses it), `stepper` (Stepper, StepperItem: wizard + install phases popover), `tab` (TabBar, Tab: Manage, Help), `table` (DataTable: inventory, disks), `tag` (Tag with Primary/Secondary/Danger/Success/Warning/Info variants: all statuses), `badge` (Badge: unread/update dot on the Settings icon), `alert` (Alert Default/Info/Success/Warning/Error, `.banner()` for the update banner: all inline problems), `tooltip` (Tooltip: every `tip`), `popover` (Popover: info icons, Way back), `hover_card` (HoverCard: names and sizes behind summary tags), `dialog` (Dialog, AlertDialog: confirmations, long text, host key), `sheet` (Sheet: Help centre), `accordion`/`collapsible` (Accordion, AccordionItem, Collapsible: steps, log, credits), `breadcrumb` (Breadcrumb: Manage sub-pages), `menu` (DropdownMenu, PopupMenu, ContextMenu: TitleBar menu, row actions), `kbd` (Kbd: shortcut list, tooltips), `button` (Button, ButtonGroup, DropdownButton, Toggle, ToggleGroup: segmented choices), `input` (Input incl. `mask_toggle` and search, NumberInput, OtpInput unused), `checkbox`, `radio`, `switch`, `slider`, `select`/`combobox`, `progress` (Progress, ProgressCircle), `spinner`, `skeleton`, `empty` (Empty: all empty states), `description_list` (DescriptionList: facts, logins), `group_box` (GroupBox: Goal cards, Destroyed/Changed/Preserved), `form` (Form, Field: all forms), `label`, `link`, `clipboard` (Clipboard: copy buttons), `notification` (toasts), `status_bar` (StatusBar), `title_bar` (TitleBar), `setting` (Settings, SettingPage, SettingGroup, SettingItem: Settings screen), `list` + `virtual_list` (List, VirtualList: topics, recovery rows, log lines), `text` (TextView::markdown: Help bodies, licence texts), `command` (Command palette: not used, search is an Input), `scroll` (Scrollbar via `.overflow_y_scrollbar()`; themed through the three scrollbar tokens), `separator`, `resizable` (not used), `tree`, `pagination`, `avatar`, `plot`/`chart` (not used).

**Icons**: `gpui-kit-assets` ships the full Lucide catalogue (1,830 SVGs) but embeds only 101 by default; the app registers its own `AssetSource` for the extra icons it uses (terminal, circle-question-mark, info, copy, triangle-alert, circle-check, circle-x, recycle, settings, search, book-open, download, wifi-off, hard-drive, life-buoy, external-link, refresh-cw, folder-open, key-round, clock, lock, server, shield, power) and does not embed `AllAssets`. **Fonts** are bundled as TTF/OTF (Inter, JetBrains Mono, Montserrat for the wordmark), not WOFF.

**Missing widgets and exceptions (each must stay listed in code with a comment):**

| Need | Why no widget | Exception |
|---|---|---|
| Disk / RAID / LVM layout diagram | No chart widget draws partition stacks | One custom element built from `div` segments inside a `GroupBox`; its legend is a `Tooltip` |
| Capacity / slices preview bar (RAID, Hybrid) | No chart widget for stacked slices | Same custom element as the layout diagram, reused; the numbers are `Tag`s and a `DataTable`, so the bar is decorative and the data is also in text |
| Countdown (60 s, 15 min, 90 s) | No timer widget | `Label` (JetBrains Mono) driven by an app-side tick; announced as a timer |
| Logo / wordmark | Brand asset | SVG via `img`/`Icon` custom path; Montserrat wordmark baked into the SVG outlines |
| Log viewer | No log widget | `VirtualList` of `Label` rows (mono) + `Input` search + `ToggleGroup` levels |
| Timeline (Manage, History) | No timeline widget | `List` rows with a Lucide icon and a time `Label` |
| Card | No Card widget | `GroupBox` (nearest) |
| Banner (top of window) | exists | `Alert::banner()` (no exception) |
| Typed-phrase match indicator | None needed | `Input` + inline `Tag` |
| Dialog/Sheet/Notification host | Needs `Root` | The window root must be `gpui_component::Root` |

## 3. Flows (minimal UI, all new features included)

**F1 First install (Untested path, boot-flash unit).** D1 first launch Dialog (once) -> D1 select unit -> D2 Connect (host-key Dialog on first contact) -> D3 Inspect (Tag *Untested*) -> D4 Goal (Boot manager marked *Recommended*; Install Debian one click away) -> D5 Plan (Wipe; Keep only behind *Advanced*) -> D6 System (**Kernel** Stable | Newest; **Source** Online | Offline; Boot via *Boot flash*) -> D7 Confirm -> D8 Install (steps 1 to 12; **No return** at step 3) -> D10 Done (copy the password to enable Finish) -> D15 Manage.

**F2 Online / Offline.** D6 Source toggle. *Online*: the NAS downloads packages (needs Internet; said in the *About 25 min* popover). *Offline*: Select a bundle (architecture-matched); if none, an inline error "No bundle" with Button *Download* that opens Settings > Data; the app needs Internet only to download the bundle. Offline also disables update check, list refresh and report-back (Tooltips explain). StatusBar shows *Offline* when the app itself is offline.

**F3 Kernel channel.** D6 Kernel toggle: *Newest* (7.x from trixie-backports, default: the product requirement is kernel 7+ preferred; falls back to Stable for online installs only, an offline bundle holds one kernel) | *Stable* (Debian 6.12 LTS). One Popover; the choice is part of the Copy command output.

**F4 CLI equivalent.** *Copy command* (terminal icon, tooltip only) on D5, D6 and D7 copies `synofurb install --host <ip> --user <admin> ...` for the current choices. On D7 the three confirmation flags `--have-backup --accept-data-loss --confirm` are included **only after** the three confirmations and the phrase are satisfied; before that the copied command is the `plan` form (read-only). Shortcut `Ctrl+Shift+C`.

**F5 Failure.** Before step 3: D9 with 60 s countdown ("DSM returns in"), *Disks untouched*, primary *Wait for DSM*, no rescue shell. After step 3: D9 with 15 min countdown ("Rescue shell closes in"), *Disks changed*, primary *Open rescue shell*, secondary *Export diagnostics* (first in the Dialog text, because the NAS is unreachable after the clock), Wipe and Keep variants differ only in the *What happens next* Dialog. Agent did not appear: D9 shows *Boot unconfirmed* and both outcomes (section 5, D9). Recovery options -> D14.

**F6 Boot manager.** D4 -> D13 -> D8-style progress (Boot flash accordion only) -> D15 Manage; Boot page (web W8) for next boot. Power-cycling is always the way back from another system.

**F7 Update.** D11 banner and Dialog; never during an install.

**F8 Web day-2.** W1 Login -> W2 Dashboard (Needs attention first) -> W3/W4 storage -> W13 RAID -> W6 Network (safe apply) -> W8 Boot.

**F9 RAID choice at install.** D5 *Data RAID* Select (Auto, None, 0, 1, 5, 6, 10, Hybrid 1-disk, Hybrid 2-disk) shows *Usable* and *Survives* live; Hybrid adds a *Slices* link (Dialog with the slices table) and, when relevant, a *Weaker slice* warning; D6 repeats the choice as a read-only *Data layout* tag; D7 lists it under Destroyed/Changed as sizes only.

**F10 RAID day-2.** Desktop: D15 Manage > Storage (D18) lists arrays and opens the same Operation plan Dialog; the web RAID page (W13 to W17) is the full tool. Replace a failed disk: W2 Needs attention -> W13 (degraded Alert) -> W16 -> rebuild progress. Create: W13 *Create array* menu -> W14 or W15. Foreign arrays: Assemble and Scrub only until the *Not mine* checkbox is ticked.

## 4. Dialog texts (opened on purpose; each at most 90 words, 2 to 4 short paragraphs)

| Dialog | Opened from | Buttons | Words | Text |
|---|---|---|---|---|
| How recovery works | D7 Confirm | Close | 71 | DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds. From step 3, a failure leaves a rescue shell open for 15 minutes, then the NAS reboots into the DSM loader, which shows Synology's install page. DSM's own system may be gone by then; reinstall it from Synology. We promise nothing about your data. |
| Keep existing volumes | D5 Plan (Advanced) | Use Keep / Cancel | 51 | Keep mode reuses only DSM's system and swap area, about 10 GiB per disk; the system is capped near 9.5 GiB. DSM, its settings, users and packages are erased. Your data volume is left alone on a best-effort basis, with no guarantee. It is not mounted automatically; import it read-only later. |
| What happens next (before step 3) | D9 Install failed | Close | 45 | No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before. If DSM is not back after 10 minutes, power-cycle the NAS. Our entries stay on the boot flash until you remove them under Recovery. |
| What happens next (agent did not answer) | D9 Install failed | Close | 66 | The app waited 10 minutes and cannot tell what happened. If the agent started, its watchdog reboots the NAS into DSM, 20 minutes or more from now. If it never started, the NAS may keep booting the agent after every power cycle. Then choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer. DSM is not confirmed. See Help, Recovery. |
| What happens next (after step 3, Wipe) | D9 Install failed | Close | 51 | The first disk was changed and DSM's own system may be gone. Export diagnostics first, then open the rescue shell. At zero DSM becomes the default and the NAS reboots into the DSM loader, which shows Synology's install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep. |
| What happens next (after step 3, Keep) | D9 Install failed | Close | 63 | The first disk was changed. Export diagnostics, then open the rescue shell. At zero the NAS reboots into the DSM loader and Synology's install page. Your data volume was not part of any write command and should be intact unless the failure was a hardware problem. Synology documents a reinstall that can keep data volumes; check its current documentation. We cannot guarantee it. |
| Trust this NAS? | D2 Connect | Trust / Cancel | 36 | First connection to {192.168.1.40}. Compare this fingerprint with DSM (Control Panel, Terminal and SNMP). If you did not expect a new key, cancel. A changed key shows the same dialog in danger style and offers only Cancel. |
| Reinstall DSM | D14 Recovery | Open find.synology.com / Close | 46 | The NAS boots Synology's loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model. Synology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it. |
| Restore boot flash? | D14 Recovery | Restore / Cancel | 29 | This writes the saved image {dom-backups/....img} back to the boot flash. It needs a working Linux on the unit, such as the agent or Debian. Type RESTORE FLASH to continue. |
| Report a result | D10 Done | Open issue / Cancel | 32 | Opens a pre-filled GitHub issue with: model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click. |
| Good to know (Welcome guide, step 3) | D1b Welcome guide | Learn more | 26 | Screen readers are not supported in this window. synofurb only reads until you confirm. The same steps work from the command line (see Help, Command line). |
| Apply with rollback? (web) | W6 Network | Apply / Cancel | 36 | After applying, open {https://192.168.1.45:8443}. The browser warns again; the certificate is the same: {SHA-256 5E:91:C4:0B:7A:D2:38:F6 ... 2D:83:3C:A1}. Passkeys do not work on an IP address. You have 90 seconds; you can also confirm over SSH {ssh andreas@192.168.1.45 sudo synofurb-net confirm}. Do nothing and it rolls back. |
| Enable write? (web) | W5 Mounts | Enable write / Cancel | 24 | DSM btrfs volumes can use features Debian handles poorly. Read-only is the safe default. Write only if you have a copy of the files. |
| Hybrid slices (desktop Plan) | D5 Plan (Slices link) | Close | 89 | Hybrid (SHR-style) cuts your disks' data space into slices at the sizes of the different disks. Each slice present on two or more disks becomes one array: RAID 1 for two members, RAID 5 for three or more (Hybrid 2-disk: RAID 6 for four or more). A slice on one disk has no partner and stays unused. The arrays are joined by LVM into one volume. This is built by synofurb with mdadm and LVM; DSM may not adopt it, and nothing here has run on a real Synology. |
| Update management app | D15 Manage | Update / Cancel | 47 | Copies version {0.9.2} to the unit (now {0.9.1}). The web UI restarts for about 10 seconds; signed-in sessions sign in again. If the health check fails, the previous version is restored automatically. You can also roll back afterwards. A warning line appears if an array is degraded or rebuilding. |
| Install boot agent? | D13 Boot manager | Install / Cancel | 53 | The whole boot flash is saved to this computer first and verified. Then entries and files are added to the flash; Synology's entry stays and stays the default. Your disks are not touched. The agent resets the default to DSM when it starts and reboots to DSM after 60 minutes without a login. |
| Operation plan | D18 / W13 to W17 | Run / Cancel | 58 | Shows the ordered steps for this operation, re-derived on the server when you run it, and the exact commands (collapsed). Destructive steps ask you to type a phrase such as REPLACE sdc. It refuses to touch a device that is mounted, holds the system or swap, belongs to another array, or while the array is resyncing or reshaping. |

## 5. Desktop screens

### D1 Inventory

**Purpose:** Find units on the network and pick one.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Find NAS | Label (page title) | 2 |
| 2 | Select the unit to work on. | Label (subtitle) | 6 |
| 3 | Filter units | Input (search placeholder) | 2 |
| 4 | Rescan | Button (ghost) + Spinner while scanning | 1 |
| 5 | Add by IP | Button (ghost) | 3 |
| 6 | Model, Address, DSM | DataTable column headers | 3 |
| 7 | Found by | DataTable column header | 2 |
| 8 | Status | DataTable column header + info icon (Popover) | 1 |
| 9 | {DS918+ · SN 1930PDN123456 · 192.168.1.40 · DSM 7.2.2 · Assistant mDNS · Untested} | DataTable rows; status = Tag (variant per status, with icon) | data |
| 10 | Running Debian | Label (section) above a second DataTable | 2 |
| 11 | Manage | Button (ghost, per row) | 1 |
| 12 | Connect | Button (primary); disabled until a row is selected | 1 |
| 13 | Twin | Tag (warning) on rows of the same model | cond. |
| 14 | Offline | StatusBar item (Tag) when offline | cond. |

**Word budget:** screen 24 + shell 11 = **35** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Status header (i) | Popover | Status | How much this model is proven. | help: compat-status |
| Found by values | Tooltip | Found by | Discovery methods that saw this unit. | help: first-install |
| Rescan (split) | Popover | Scan details | Five discovery methods run at once. | Lists each method with Tag ok/warn and the subnet; help: troubleshooting |
| Twin tag | Tooltip | Twin | Another unit of this model is on the network; check serials. | help: first-install |
| Connect disabled | Tooltip | Connect | Select a unit first. | - |
| Add by IP | Dialog | Add NAS | Enter an address to probe. | Input IP + port; buttons Add / Cancel |
| Offline tag | Tooltip | Offline | Compatibility list is {n} days old; LAN scan still works. | help: offline-mode |
| Welcome guide | Dialog (first launch until dismissed; Help and menu any time) | Replace DSM with Debian | Nothing changes until you confirm. | Five steps, see D1b. The old "Before you start" dialog is merged into it. |

**Primary:** Connect (selected row). **Secondary:** Rescan, Add by IP, Manage.

**Keyboard:** `Ctrl+F` filter; `Up`/`Down` row; `Enter` Connect; `R` rescan; `A` add; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Empty: "No NAS found" + buttons Scan again, Add by IP, link Help; *error*: A failing discovery method is a warning Tag in the Scan details popover; no page error; *loading*: Skeleton rows + Spinner on Rescan; rows stream in; *offline*: StatusBar Tag "Offline"; compatibility column uses the cached list.

### D1b Welcome guide

**Purpose:** Tell a new user in under a minute what synofurb does, what the process is and how to get back. Opens on top of D1 on the first launch until it is dismissed; also from Help (first row of the topic list) and the title-bar menu. It replaces the D1 "Before you start" dialog and keeps its three statements (read-only until you confirm; make a tested backup; screen readers are not supported, use the command line).

**Pattern (every step):** one Lucide icon in a neutral tile, one headline (type.title), at most two short lines (type.body), an info button for details, the dots row, then Skip on the left and Back plus the one primary on the right. Dialog width 640. The text block keeps one height on all steps, so the buttons do not move.

| Step | Icon | Headline | Lines | Info button (opened on purpose) |
|---|---|---|---|---|
| 1 | Server | Replace DSM with Debian | synofurb installs Debian 13 on your Synology over SSH. / Nothing changes until you confirm. | - |
| 2 | Layers | Six steps to Debian | Review each step; go back until Install starts. Below: the six steps as a row (the window's Stepper, not clickable). | - |
| 3 | Warning | Before you start | The disks you pick will be erased. / Make a tested backup; keep the NAS on and online. | Good to know: screen readers; only reads until you confirm; link Command line |
| 4 | Undo | The way back | DSM stays the default boot entry; Recovery restores it. / Boot manager tries other systems. F1 opens Help. | More ways back: Boot manager mode, Recovery; link Boot manager |
| 5 | Rocket | Ready when you are | Pick your NAS in Find to begin. / Reopen this guide from Help any time. | - (checkbox "Don't show again" with a tooltip) |

**Word budget:** 15 to 23 words of its own per step (steps 3, 4 and 5 are the largest with 23: headline, lines, Skip or the checkbox, Back and the primary) + shell 11 = at most **34** of 60. Counted: headline, lines, visible buttons and the checkbox label. Not counted: the info bodies (a click away), icons, dots. Enforced by `welcome.rs` and `copy.rs` tests.

**Primary:** Next (steps 1 to 4), Start (step 5): one primary per step. **Secondary:** Back (disabled on step 1 with the reason "Already at the start." behind an info button), Skip (steps 1 to 4; on step 5 Start is the way out).

**Keyboard:** `Right` next step (never presses Start), `Left` previous step, `Tab` / `Shift+Tab` through info button, checkbox, Back and primary, `Enter`/`Space` activate the focused control, `Esc` closes. Focus starts inside the dialog and returns to the page when it closes, like every other dialog.

**What "seen" means (`UiSettings.welcome_done`):** Skip, or Esc / the close button on steps 1 to 4, dismisses the guide for good. On step 5, Start and Esc follow "Don't show again", which is ticked by default; unticked, the guide opens again at the next launch. Replaying from Help or the menu never clears the flag. Files from before the guide (`first_launch_done`) count as seen.

**States:** no loading or error state (static content). Demo names `welcome`, `welcome-2` to `welcome-5`. Largest text: the dialog grows, nothing clips (gui-walk).

### D2 Connect

**Purpose:** Sign in to the NAS over SSH, read-only.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Connect | Label (title) | 1 |
| 2 | Sign in over SSH. Nothing changes. | Label (subtitle) | 6 |
| 3 | {DS918+ · 192.168.1.40 · SN 1930PDN123456} | Persistent unit header (shell, section 1.1); not repeated in the page | data |
| 4 | Port | Field + NumberInput | 1 |
| 5 | User | Field + Input | 1 |
| 6 | Sign in with | Field + ToggleGroup | 3 |
| 7 | Password | ToggleGroup option | 1 |
| 8 | Key | ToggleGroup option | 1 |
| 9 | Password | Field + Input (mask_toggle) | 1 |
| 10 | Same for sudo | Checkbox | 3 |
| 11 | Keep in keychain | Checkbox | 3 |
| 12 | Connect | Button (primary, loading state) | 1 |
| 13 | Back | Button (ghost) | 1 |
| 14 | SSH is off | Alert (error) when the port is closed | cond. |
| 15 | Login failed | Alert (error) after a failed login | cond. |

**Word budget:** screen 23 + shell 12 = **35** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Title (i) | Popover | What we read | Hardware, disks and RAID state; nothing is written. | List of the read-only commands; link "Show script" (Dialog, mono TextView); help: first-install |
| Subtitle | Tooltip | Nothing changes | This step only reads. | - |
| Same for sudo | Tooltip | Same for sudo | DSM asks for the password to read disk details. | - |
| Keep in keychain | Tooltip | Keep in keychain | Otherwise the password is forgotten on exit. | - |
| SSH is off | Link -> Help | How to enable | Control Panel, Terminal and SNMP, Enable SSH. | help: troubleshooting |
| Login failed | Tooltip on Alert | Login failed | DSM may auto-block this computer after a few failures. | help: troubleshooting |
| Unknown host key | Dialog (blocking) | Trust this NAS? | Compare the fingerprint with DSM. | Mono fingerprint + buttons Trust / Cancel; a changed key shows the same dialog in danger style and Trust is replaced by Cancel only |

**Primary:** Connect. **Secondary:** Back.

**Keyboard:** `Enter` Connect; `Alt+Left` Back; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Fields prefilled (port 22, user admin); *error*: Alert error above the form; failing field gets danger border; focus moves to the Alert; *loading*: Connect button loading; fields disabled; Esc cancels the attempt.

### D3 Inspect

**Purpose:** Show what was read from the unit.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Inspection | Label (title) | 1 |
| 2 | Read-only results. | Label (subtitle) | 2 |
| 3 | Model | DescriptionList label | 1 |
| 4 | CPU | DescriptionList label | 1 |
| 5 | RAM | DescriptionList label | 1 |
| 6 | Firmware | DescriptionList label | 1 |
| 7 | Boot flash | DescriptionList label + info icon (Popover) | 2 |
| 8 | Console | DescriptionList label | 1 |
| 9 | Backups | DescriptionList label | 1 |
| 10 | {DS918+ · Celeron J3455 · 8 GiB · BIOS · editable · ttyS0 115200 · none found · Untested} | DescriptionList values; compat = Tag + info icon | data |
| 11 | Disk warning | Alert (warning) + data line | 2 |
| 12 | {sdc · 8 reallocated sectors} | Alert body (data) | data |
| 13 | Disk, Model, Size, SMART | DataTable column headers | 4 |
| 14 | DSM volumes | AccordionItem (collapsed) | 2 |
| 15 | Current layout | AccordionItem (collapsed), custom diagram inside | 2 |
| 16 | Choose goal | Button (primary) | 2 |
| 17 | Back | Button (ghost) | 1 |

**Word budget:** screen 24 + shell 12 = **36** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Compat Tag (i) | Popover | Untested | No one has reported an install on this model. | What we checked / not known, button "Open Help"; help: compat-status |
| Boot flash (i) | Popover | Boot flash | Synology's internal boot module; we add entries to it. | help: boot-manager |
| Backups: none found | Tooltip | None found | No Hyper Backup, USB Copy or rsync tasks on this NAS. | help: backups |
| Disk warning | Tooltip | Disk warning | It is in the data RAID; consider replacing it first. | help: troubleshooting |
| Current layout | Accordion body | Current layout | DSM's disk, RAID and volume stack. | custom diagram (exception, section 2) |

**Primary:** Choose goal. **Secondary:** Back.

**Keyboard:** `Enter` Choose goal; `Space` toggles the focused Accordion item; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Zero disks detected: Alert error "No disks visible", Choose goal disabled; *error*: A failed probe row shows "not available" Tag; listed in the compat popover under "not known"; *loading*: Skeleton rows per section; progress is under 10 s.

### D4 Goal

**Purpose:** Pick what to do with this unit.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Choose goal | Label (title) | 2 |
| 2 | What should happen to this NAS? | Label (subtitle) | 6 |
| 3 | Boot manager | GroupBox title + Icon (terminal) | 2 |
| 4 | Recommended | Tag (info) on the card for Untested units | 1 |
| 5 | Try Linux. Disks untouched. | Label (card line) | 4 |
| 6 | Set up | Button (primary on Untested units) | 2 |
| 7 | Install Debian | GroupBox title + Icon (hard-drive) | 2 |
| 8 | Replace DSM with Debian. | Label (card line) | 4 |
| 9 | Plan install | Button | 2 |
| 10 | Restore / Recover | GroupBox title + Icon (life-buoy) | 2 |
| 11 | Undo or repair. | Label (card line) | 3 |
| 12 | Open recovery | Button | 2 |

**Word budget:** screen 32 + shell 12 = **44** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Boot manager (i) | Popover | Boot manager | Adds a small SSH agent to the boot flash. | Flash changes, disks untouched, one-shot, 60 min watchdog; help: boot-manager |
| Install Debian (i) | Popover | Install Debian | Erases disks; needs your backup. | Wipe or advanced Keep, about 25 minutes; data layout from None to RAID 10 or Hybrid (SHR-style); help: first-install, raid-levels |
| Restore (i) | Popover | Restore / Recover | Reads the unit's state before offering anything. | help: recovery |
| Recommended tag | Tooltip | Recommended | No one has reported this model; try the agent first. | help: compat-status |

**Primary:** Boot manager: Set up (Untested); otherwise Plan install. **Secondary:** the other two cards.

**Keyboard:** `Tab` between card buttons; `Enter` activates; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Plan install disabled for Unsupported units; Tooltip "Debian 13 cannot run on this hardware"; *loading*: n/a.

### D5 Plan

**Purpose:** Shape the storage layout; see Now and After and the consequence.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Storage plan | Label (title) | 2 |
| 2 | Nothing is written yet. | Label (subtitle) | 4 |
| 3 | Mode | Field + ToggleGroup | 1 |
| 4 | Wipe all | ToggleGroup option (only one visible) | 2 |
| 5 | Advanced | Link (opens the Keep Dialog) | 1 |
| 6 | System size | Field + Slider + value label | 2 |
| 7 | Mirror copies | Field + ToggleGroup 2 / 3 / 4 | 2 |
| 8 | Data RAID | Field + Select (closed list: Auto, None, 0, 1, 5, 6, 10, Hybrid 1-disk, Hybrid 2-disk; items not counted) | 2 |
| 9 | Usable | Tag (secondary) + data {10.82 TiB}; recomputed live | 1 |
| 10 | Survives | Tag (secondary) + data {1 disk} | 1 |
| 11 | Slices | Link, Hybrid only; opens Dialog with a DataTable (Slice, Disks, Level, Usable) | cond. |
| 12 | Hybrid: {+12.7 TiB} | Tag (info) shown beside Data RAID when disk sizes differ by more than 10 % and Auto is selected; click opens the Slices Popover | cond. |
| 13 | Weaker slice | Alert (warning), only for a Hybrid 2-disk slice with fewer than 4 members | cond. |
| 14 | Free space | Field + Slider | 2 |
| 15 | Now | GroupBox title (read-only layout: disks, md0/md1/md2, volume1) + Tag {DSM} | 1 |
| 16 | After | GroupBox title (planned layout) + info icon (Popover = text key) + Tag {Wipe} | 1 |
| 17 | {disk bars + array chips} | Custom diagram (exception, section 2); every array chip carries its name so colour is never the only key | data |
| 18 | Destroyed | Tag (danger) + data size | 1 |
| 19 | Changed | Tag (warning) + data | 1 |
| 20 | Preserved | Tag (secondary) + data | 1 |
| 21 | Disk warning | Alert (warning) + data line; focusable | 2 |
| 22 | Next: System | Button (primary) | 2 |
| 23 | Back | Button (ghost) | 1 |
| 24 | Command | Button (ghost, terminal icon + the one word Command; copies the equivalent synofurb command) | 1 |

**Word budget:** screen 31 + shell 12 = **43** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Mode (i) | Popover | Mode | Wipe erases all four disks. | help: backups |
| Advanced | Dialog | Keep existing volumes | Best effort; no guarantee. | Keep dialog, section 4; blocking error "Size does not fit" with fix button "Set to 9 GiB" |
| System size (i) | Popover | System size | Mirrored on every copy; 32 GiB is recommended. | help: first-install |
| Mirror copies (i) | Popover | Mirror copies | More copies survive more disk failures. | help: recovery |
| Data RAID (i) | Popover | Data RAID | Auto picks the level from the disk count. | Small table: level, minimum disks, survives; Hybrid row; "Open Help"; help: raid-levels |
| Hybrid suggestion | Popover | Hybrid: {+12.7 TiB} | Mixed disk sizes waste space at equal-size levels. | Shows Slices and the capacity difference; help: hybrid-raid |
| Usable / Survives | HoverCard | Capacity | Usable space and fault tolerance for this level and disk set. | Raw, usable, wasted, tolerated failures; help: raid-levels |
| Slices | Dialog | Hybrid slices | Each slice becomes one array. | Hybrid dialog, section 4; help: hybrid-raid |
| After (i) | Popover | Text key | Every segment is named; colours repeat the names. | Legend: boot, system, data, free, per-array names |
| Disk warning | Popover (focusable trigger) | Disk warning | A failing disk during the first sync is risky. | help: troubleshooting |
| Copy command | Tooltip | Copy command | Copies the equivalent synofurb command. | help: cli |

**Primary:** Next: System. **Secondary:** Back, Copy command.

**Keyboard:** `Left`/`Right` adjust a focused Slider; `Tab` through controls; `Ctrl+Shift+C` Copy command; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Blocking (too few disks for the level, Keep size does not fit): Alert error + inline fix, Next is a real disabled Button with a sibling info Button "Why unavailable" ("Fix the error first"); non-blocking: Alert warning; *loading*: Layout recomputes instantly.

### D6 System

**Purpose:** Set name, login, kernel, source and boot method.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | System | Label (title) | 1 |
| 2 | Name, login and boot. | Label (subtitle) | 4 |
| 3 | Hostname | Field + Input | 1 |
| 4 | User | Field + Input | 1 |
| 5 | Password | Field + Input (mask_toggle) + Clipboard + icon Button (regenerate) | 1 |
| 6 | SSH key | Field + ToggleGroup | 2 |
| 7 | Generate | ToggleGroup option | 1 |
| 8 | Paste | ToggleGroup option | 1 |
| 9 | None | ToggleGroup option | 1 |
| 10 | Time zone | Field + Select (searchable) | 2 |
| 11 | Console | Field + Input (read-only) | 1 |
| 12 | Web UI | Field + Switch | 2 |
| 13 | Kernel | Field + ToggleGroup | 1 |
| 14 | Stable | ToggleGroup option | 1 |
| 15 | Newest | ToggleGroup option | 1 |
| 16 | Source | Field + ToggleGroup | 1 |
| 17 | Online | ToggleGroup option | 1 |
| 18 | Offline | ToggleGroup option | 1 |
| 19 | Data layout | Field + Tag (read-only) {RAID 5 · 10.82 TiB} with link back to Plan | 2 |
| 20 | Boot via | Field + Tag (read-only) "Boot flash" or "Installer kit" | 2 |
| 21 | Next: Confirm | Button (primary) | 2 |
| 22 | Back | Button (ghost) | 1 |
| 23 | Bundle | Field + Select + Button "Download" when Offline | cond. |
| 24 | Command | Button (ghost, terminal icon + the one word Command; copies the equivalent synofurb command) | 1 |

**Word budget:** screen 32 + shell 12 = **44** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Password (i) | Tooltip | Password | Generated; copy it on the last screen. | help: first-install |
| SSH key (i) | Tooltip | SSH key | The key pair is created on this computer. | help: first-install |
| Console (i) | Tooltip | Console | Read from DSM; a USB serial adapter also works. | help: serial-console |
| Web UI (i) | Tooltip | Web UI | HTTPS on port 8443, self-signed certificate. | help: first-install |
| Kernel (i) | Popover | Kernel | Stable is 6.12 LTS; Newest comes from backports. | Newest falls back to Stable if it cannot be installed; neither has run on this model; help: first-install |
| Source (i) | Popover | Source | Offline uses a downloaded bundle. | Online needs Internet on the NAS; Offline needs a bundle for this architecture (Settings, Data); help: offline-mode |
| Bundle missing | Alert (error) inline | No bundle | Download one in Settings or switch to Online. | Button "Download" opens Settings > Data |
| Boot via (i) | Popover | Boot via | Debian starts through the boot flash on this unit. | Backed up first, DSM stays default; Installer kit shown for units without a flash; help: boot-manager |
| Data layout | Tooltip | Data layout | Chosen in Plan; click to change it. | help: raid-levels |
| Copy command | Tooltip | Copy command | Copies the equivalent synofurb command. | help: cli |

**Primary:** Next: Confirm. **Secondary:** Back, Copy command.

**Keyboard:** `Tab` order = reading order; `Space` toggles Switch/Checkbox; `Enter` Next; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Defaults filled; Password generated on entry; *error*: Field-level: danger border + Tooltip on focus; console auto-detect failed -> Input becomes editable; *loading*: n/a.

### D7 Confirm

**Purpose:** Final gate: consequences, three confirmations, typed phrase. Identity is in the unit header and repeated on the checkbox.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Confirm erase | Label (title) | 2 |
| 2 | Last step before changes. | Label (subtitle) | 4 |
| 3 | Disk warning | Tag (warning), focusable Popover trigger | 2 |
| 4 | Untested | Tag (info), focusable Popover trigger (also in the unit header) | cond. |
| 5 | About 25 min | Tag (secondary), Popover "Before you start" | 3 |
| 6 | Destroyed | GroupBox (danger rule): ONE row per group, e.g. {sda sdb sdc sdd · 4 x WDC WD40EFRX · 14.55 TiB}, {volume1 · btrfs · 6.2 TiB used}, {DSM · settings · users} | 1 |
| 7 | Changed | GroupBox (warning rule): {Boot flash · backed up first · 128 MiB} | 1 |
| 8 | Preserved | GroupBox (success rule): {Nothing on the disks} | 1 |
| 9 | ERASE {DS918+} | Gate card header (danger) | data |
| 10 | Backup is tested | Checkbox | 3 |
| 11 | I accept data loss | Checkbox | 4 |
| 12 | This is | Checkbox label followed by data {192.168.1.40 · …123456} | 2 |
| 13 | Type phrase | Field + Input (paste allowed; leading/trailing whitespace trimmed; case-sensitive) + inline Tag Matches / No match; twin units: phrase = ERASE <MODEL> <last 4 of serial> | 2 |
| 14 | Free to cancel until step 3; then no undo. | Label (the one-line recovery summary) + info icon; "no undo" is the rule, the Dialog explains that DSM may be gone and what survives | 9 |
| 15 | How recovery works | Link (opens Dialog) | 3 |
| 16 | Erase and install | Button (danger); a REAL disabled Button until the gate is complete (not a Tab stop); the sibling info Button "Why locked" (focusable) opens the Popover "Missing: ..." | 3 |
| 17 | Back | Button (ghost) | 1 |
| 18 | Twin | Alert (warning) + Tag "Twin" in the unit header, only when a same-model unit exists: {192.168.1.58 · SN …7710}; the demo inventory ALWAYS contains a twin pair (DS918+ at .40 and .58) | cond. |
| 19 | Command | Button (ghost, terminal icon + the one word Command; copies the equivalent synofurb command) | 1 |

**Word budget:** screen 42 + shell 12 = **54** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Disk warning tag | Popover (Tag is the focusable trigger) | Disk warning | {sdc: 8 reallocated sectors}; consider replacing it first. | help: troubleshooting |
| Untested tag | Popover | Untested | No one has reported this model; your install is the test. | help: compat-status |
| About 25 min tag | Popover | Before you start | Keep this computer awake and the NAS online. | Checklist: stay awake and online; NAS needs Internet unless Source is Offline; unplug USB and eSATA disks; first RAID sync about 9 hours; help: first-install |
| Changed row | HoverCard | Boot flash | Entries and files are added; the whole flash is backed up first. | Backup file name (mono); help: backups |
| Preserved row | Tooltip | Preserved | Wipe mode preserves nothing on the disks. | help: backups |
| Twin | Alert + Tag | Twin | Check the serial next to the checkbox. | The phrase becomes ERASE <MODEL> <last 4 of serial>; hint "Add serial last 4." until typed; a wrong suffix shows No match |
| Erase (incomplete) | Popover from the sibling info Button "Why locked" | Missing | Lists which confirmations or the phrase are missing. | - |
| Phrase match | Tag inline | Matches / No match | - | No echo of the expected text |
| How recovery works | Dialog | How recovery works | Free to cancel before step 3; failures return to DSM. | Dialog text, section 4 |
| Copy command | Tooltip | Copy command | Includes the confirmation flags only after you confirm here. | help: cli |

**Primary:** Erase and install (danger). **Secondary:** Back, Copy command.

**Keyboard:** Tab order after navigation (focus starts on the title; Stepper is not a tab stop): Disk warning, Untested, About 25 min, Checkboxes (`Space`), phrase Input, How recovery works, Back, Copy command, Erase. `Enter` in the phrase field never submits; GPUI Buttons activate on `Enter` and `Space`, so the safeguard is the position (last in the order) and the full gate, not the key. `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: All controls cleared, Erase styled disabled; *error*: Phrase mismatch: inline Tag "No match"; *loading*: n/a.

### D8 Install

**Purpose:** Show progress; make the point of no return and the cancel rules obvious.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Installing | Label (title) | 1 |
| 2 | Keep this window open. | Label (subtitle) | 4 |
| 3 | {71 %} | Progress + big value | data |
| 4 | {Install the Debian base system · Step 9 of 12 · about 7 min left} | Label: current step ONLY (engine text and numbers) | data |
| 5 | Staging | Tag (info) with the current phase; opens Popover with the 7 phases | 1 |
| 6 | Boot flash | AccordionItem (open until finished): backup, agent, reboot, installer send; Tag {Backed up} | 2 |
| 7 | Steps | AccordionItem (collapsed after step 3, current step visible in the header): the 12 engine steps as a List with status icons | 1 |
| 8 | No return | Tag (danger) on step 3 inside the Steps list | cond. |
| 9 | Log | AccordionItem (collapsed): Input search, ToggleGroup level, VirtualList | 1 |
| 10 | Cancel | Button (secondary); a real disabled Button after step 3, with the sibling info Button "Why unavailable" (Popover "Not possible after step 3"); an open Cancel dialog re-evaluates when step 3 starts ("Too late to cancel", Stop install disabled) | 1 |
| 11 | Connection lost | Alert (warning, conditional) + Button "Retry" | cond. |

**Word budget:** screen 11 + shell 12 = **23** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Phase Tag | Popover | Phases | Seven phases from connecting to verifying. | List with current phase marked; help: first-install |
| No return Tag | Popover | No return | From step 3 the first disk changes; cancel is free before. | help: recovery |
| Cancel (after step 3) | Popover | Cancel | Not possible after step 3. | help: recovery |
| Cancel (before step 3) | AlertDialog | Cancel install? | The NAS returns to DSM within 60 seconds. | Buttons Stop install / Keep going. After Stop the app shows D9 in its "stopped" variant: Disks untouched, Flash changed (our entries), a 60 s "DSM returns in" countdown, primary Wait for DSM, secondary Remove our entries. It never returns to Inspect with a Read-only Tag. |
| Boot flash | Popover | Boot flash | Backed up and verified before the first write. | help: backups |
| Open log file (icon) | Tooltip | Open log file | Opens the log in your editor. | - |

**Primary:** none while running (Cancel is secondary). **Secondary:** Cancel, Open log file (icon).

**Keyboard:** `Ctrl+F` search log when open; `Space` toggles Accordion items; `Esc` never cancels; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Failure replaces this screen with D9; connection loss shows the Alert and keeps polling; *loading*: Progress indeterminate during staging until the first byte count.

### D9 Install stopped

**Purpose:** Say what happened, whether disks and flash changed, the clock, and the one next action.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Install stopped | Label (title) | 2 |
| 2 | {Step 11 of 12 · Configure packages, users, boot} | Label (subtitle, engine step name) | data |
| 3 | Disks changed | Tag (danger) or "Disks untouched" Tag (success) | 2 |
| 4 | Flash restored | Tag (warning) or "Flash changed" / "Flash untouched" | 2 |
| 5 | Boot entry check failed | Alert (error): SHORT cause label only; the engine sentence is the Alert tooltip/Popover and the Log excerpt | 4 |
| 6 | Rescue shell closes in | Label + mono countdown (custom timer, exception); no decorative icon | 4 |
| 7 | Rescue | GroupBox: mono {ssh root@192.168.1.40} + Clipboard, credential Field (mask_toggle + Clipboard); persistent, never a toast | 1 |
| 8 | Export first. | Label (the one instruction; the NAS is unreachable after the clock) | 2 |
| 9 | Open rescue shell | Button (primary, pinned in the footer): copies the command and focuses the Rescue GroupBox | 3 |
| 10 | Export diagnostics | Button (secondary) | 2 |
| 11 | Recovery options | Button (secondary) | 2 |
| 12 | What happens next | Link (opens Dialog) | 3 |
| 13 | Log excerpt | AccordionItem (collapsed, mono) | 2 |
| 14 | Default: not confirmed | Tag (warning), only in the agent-did-not-answer variant; replaces any green "Default" Tag; never "Failed safely" | cond. |
| 15 | Boot unconfirmed | Status-bar risk Tag (warning) in the same variant | cond. |

**Word budget:** screen 29 + shell 12 = **41** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Alert | Popover | Cause | The installer's message, in full. | Engine text: {boot entry failed read-back; the original flash was restored and verified} |
| Countdown | Popover | Rescue shell | Then DSM becomes the default and the NAS reboots. | help: recovery |
| Disks changed tag | Popover | Disks changed | Repartitioned at {time}; DSM's own system may be gone. | help: recovery |
| Flash tag | Popover | Boot flash | Entries are on it; the full backup is on this computer. | help: backups |
| What happens next | Dialog | What happens next | Order of actions and what survives. | Three variants in section 4 (before step 3, after step 3 Wipe, after step 3 Keep) |
| Export diagnostics | Tooltip | Export diagnostics | Do this before the clock ends; secrets are removed. | help: troubleshooting |
| Default: not confirmed (agent variant) | Popover | Default not confirmed | We cannot tell if the agent ran; if it did not, a power cycle boots it again. | The dialog above; manual recovery: choose SYNOLOGY_2 in the boot menu (serial or VGA console), or restore the boot flash from another computer; help: recovery |

**Primary:** Open rescue shell (after step 3) / Wait for DSM (before step 3). **Secondary:** Export diagnostics, Recovery options.

**Keyboard:** Footer button pinned and focusable first; `Tab` order: primary, secondary, Rescue copy buttons, links; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Variants: flash backup did not verify (nothing changed, DSM running, primary Back to Confirm); agent did not appear (Phase 4 of 7, Booting; the engine resets nothing after the 10-minute wait, so the screen shows *Default: not confirmed* and *Boot unconfirmed*, never "Failed safely" or "DSM is the default". It states both outcomes: (a) the agent booted but is unreachable: the DSM default was set at its start and its watchdog reboots to DSM 30 minutes after the agent started; (b) the agent never ran: the default is still the agent, a power cycle boots it again and no watchdog exists. The clock reads "DSM may return in about 20 minutes or more": the watchdog counts from when the agent booted, 1 to 3 minutes after the reboot command, so 20 plus that many minutes remain when the app gives up, typically 21 to 23; it is an estimate, not a promise, and at 0:00 the screen says "DSM not back" and keeps the manual recovery in view. Manual recovery, named on the screen: choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or take the boot flash to another computer and restore the backup. Primary Wait for DSM; the Way back text never says "by itself" without "if the agent started"); stopped by you before step 3 (60 s clock, Flash changed); failed before step 3; failed after step 3 Wipe or Keep running (15:00 clock, Rescue panel); waiting (clock 0: "The shell closed. The NAS is rebooting.", no Rescue panel); loader (only Reinstall DSM); lost (Reconnect; the clock keeps running on the NAS); *loading*: Waiting for DSM shows Spinner and elapsed time.

### D10 Done

**Purpose:** Hand over the logins; require the password to be copied.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Done | Label (title) | 1 |
| 2 | Debian is running. | Label (subtitle) | 3 |
| 3 | Address | DescriptionList label | 1 |
| 4 | User | DescriptionList label | 1 |
| 5 | Password | DescriptionList label (Input mask_toggle + Clipboard) | 1 |
| 6 | SSH | DescriptionList label (mono + Clipboard) | 1 |
| 7 | Host key | DescriptionList label (short mono + Clipboard) | 2 |
| 8 | Web UI | DescriptionList label + Link + Button "Open" | 2 |
| 9 | Certificate | DescriptionList label (short mono) + Link "Show full" | 1 |
| 10 | {192.168.1.40 · andreas · ssh -i key · SHA256 ... · https://...:8443 · SHA-256 5E:91 ...} | values | data |
| 11 | Open | Button (primary-ghost) | 1 |
| 12 | Show full | Link (opens Popover) | 2 |
| 13 | Password copied | Checkbox | 2 |
| 14 | Finish | Button (primary); disabled until the Checkbox is ticked | 1 |
| 15 | Report result | Button (secondary, opens Dialog) | 2 |
| 16 | Next steps | AccordionItem (collapsed) | 2 |
| 17 | Action needed | Alert (warning) ARM only + Button "Show commands" | cond. |

**Word budget:** screen 23 + shell 12 = **35** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Password row | Tooltip | Password | Copy it now; it is not stored or shown again. | help: first-install |
| Host key / Certificate | Tooltip | Fingerprint | Compare the full value when the browser warns. | help: troubleshooting |
| Show full | Popover | Certificate | All 32 bytes, with Copy. | help: troubleshooting |
| Finish disabled | Tooltip | Finish | Copy the password first. | - |
| Next steps | Accordion body | Next steps | Open the web UI; replace warned disks; check BIOS boot order. | help: first-install |
| Report result | Dialog | Report a result | Opt-in; nothing is sent without your click. | Shows exactly what is included; pre-filled GitHub issue; help: report-result |
| Show commands (ARM) | Dialog | U-Boot commands | Read from this unit; not verified on real boards. | Mono commands read from the NAS, Copy, original bootcmd; help: serial-console |

**Primary:** Finish. **Secondary:** Open, Report result.

**Keyboard:** `Tab` order follows rows; `Ctrl+C` copies the focused value; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Verification failed: title becomes "Unreachable", Alert error with network hints; *loading*: "Verifying" Spinner until mDNS, SSH and :8443 answer.

### D11 Update

**Purpose:** Offer an app update without interrupting work.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Update available | Alert (info, top banner) | 2 |
| 2 | {0.9.3} | Tag | data |
| 3 | Details | Button (ghost) | 1 |
| 4 | Later | Button (ghost) | 1 |
| 5 | Download | Button (primary, in Dialog) | 1 |
| 6 | Restart and update | Button (primary, enabled after verification) | 3 |
| 7 | Skip version | Button (ghost, in Dialog) | 2 |
| 8 | {release notes: 3 bullets · size · date · progress · signature state} | Dialog body: List + Progress + Tag | data |

**Word budget:** screen 10 + shell 12 = **22** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Banner | Tooltip | Update | Never offered while an install is running. | help: troubleshooting |
| Dialog signature step | Tooltip | Signature | Checked with the project key and SHA-256 before use. | help: troubleshooting |
| Restart disabled | Tooltip | Restart and update | Waiting for verification or for the install to finish. | - |

**Primary:** Download -> Restart and update. **Secondary:** Later, Skip version, Details.

**Keyboard:** `Enter` Download; `Esc` closes the dialog; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Verification failed: Alert error "Update discarded"; nothing changes; *loading*: Progress in the Dialog; banner hidden during installs.

### D12 Settings

**Purpose:** Change app behaviour and manage offline bundles.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Settings | Label (title); gpui-component Settings with SettingPage tabs | 1 |
| 2 | General, Updates, Data, Logs, Privacy | Sidebar/SettingPage entries (5) | 5 |
| 3 | Theme | SettingItem + ToggleGroup System / Light / Dark | 1 |
| 4 | System, Light, Dark | ToggleGroup options | 3 |
| 5 | Text size | SettingItem + ToggleGroup | 2 |
| 6 | Default, Large, Largest | ToggleGroup options | 3 |
| 7 | Channel | SettingItem + ToggleGroup Stable / Beta | 1 |
| 8 | Auto-update | SettingItem + Select | 1 |
| 9 | Compatibility list | SettingItem label + data version | 2 |
| 10 | Refresh | Button | 1 |
| 11 | Offline bundle | SettingItem + Select (architecture) | 2 |
| 12 | Download | Button + Progress | 1 |
| 13 | Log folder | SettingItem + Input (read-only) | 2 |
| 14 | Open | Button | 1 |
| 15 | Send results | SettingItem + Switch (default off) | 2 |

**Word budget:** screen 28 + shell 1 = **29** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Auto-update | Tooltip | Auto-update | Never installs while a conversion is running. | help: troubleshooting |
| List (i) | Popover | Compatibility list | Fetched from the project repository; works offline from the cache. | help: compat-status |
| Offline bundle (i) | Popover | Offline bundle | Installer files for one CPU architecture. | Size, age, Delete; help: offline-mode |
| Send results | Tooltip | Send results | Asks before every report; serial numbers are left out. | help: report-result |
| Text size | Tooltip | Text size | Scales the whole window. | - |

**Primary:** none (changes apply immediately). **Secondary:** Refresh, Download.

**Keyboard:** `Ctrl+,` open; `Tab`; `Esc` close; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: No bundle: row shows "None" Tag; *error*: Refresh/download failure: Notification (error) with Retry; cache kept; *loading*: Progress inside the row.

### D13 Boot manager

**Purpose:** Install the one-shot SSH agent on the boot flash, with a confirmation, progress and a result.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Boot manager | Label (title) | 2 |
| 2 | SSH agent on the boot flash. | Label (subtitle) | 6 |
| 3 | Will change flash | Tag (warning); wording is what WILL happen, the status bar shows the current state | 3 |
| 4 | Disks untouched | Tag (success) | 2 |
| 5 | SSH key | Field + ToggleGroup | 2 |
| 6 | Generate | ToggleGroup option | 1 |
| 7 | Paste | ToggleGroup option | 1 |
| 8 | Login timeout | Field + Input (read-only, 60 min) | 2 |
| 9 | After another OS starts, only a power cycle returns. | Alert (warning) | 9 |
| 10 | Install agent | Button (primary): opens the confirm AlertDialog "Install boot agent?" | 2 |
| 11 | Back | Button (ghost) | 1 |
| 12 | Agent ready | Result panel: Tag (success), mono {ssh boot@192.168.1.40} + Clipboard, Tag {60 min watchdog}, Buttons "Return to DSM" and "Done"; after Return to DSM: "Back in DSM. Our entries remain on the flash." with Buttons "Recovery options" and "Done". NO Stepper on this screen | cond. |

**Word budget:** screen 31 + shell 12 = **43** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Flash changes | Popover | Flash changes | Entries and files are added; DSM's entry stays. | Whole flash saved to this computer first and verified; help: boot-manager |
| Disks untouched | Tooltip | Disks untouched | No partition table is written. | help: boot-manager |
| Login timeout | Tooltip | Login timeout | The agent reboots to DSM after 60 minutes without a login. | help: boot-manager |
| Title (i) | Popover | One-shot agent | It resets the default to DSM when it starts. | kexec starts Linux kernels only; help: boot-manager |
| Alert | Popover | Other OS | After kexec nothing of ours runs; power-cycling returns to the default. | help: boot-manager |
| Install agent | AlertDialog | Install boot agent? | The boot flash is saved here first, then modified. | Text: Install boot agent dialog, section 4; buttons Install / Cancel |
| Progress | D8 pattern | Boot flash | Backup, add agent, reboot, agent answers. | Steps: Boot flash backup, Agent added, Reboot into the agent, Agent answers (no installer step); Cancel is free during the read-only backup and disabled afterwards with a sibling Why unavailable button; the risk Tag reads Flash changed |

**Primary:** Install agent. **Secondary:** Back.

**Keyboard:** `Enter` Install agent; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Flash not editable: Alert error, Install agent is a real disabled Button with a sibling "Why unavailable" info Button; *loading*: D8 Boot-flash progress; Cancel is free until the first flash write; Find then shows the unit with the Tag "Boot agent installed".

### D14 Recovery

**Purpose:** Read the unit's real state and offer only what is possible and safe.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Recovery | Label (title) | 1 |
| 2 | State read from the unit. | Label (subtitle) | 5 |
| 3 | {Flash: ours present · Default: DSM · Disks: modified 14:25} | Tags (state facts); the status-bar risk Tag shows the same state, never Read-only | data |
| 4 | Boot DSM next | List row label | 3 |
| 5 | Open agent | List row label | 2 |
| 6 | Restore flash | List row label | 2 |
| 7 | Remove our entries | List row label | 3 |
| 8 | Reinstall DSM | List row label | 2 |
| 9 | Run | Button per row (ghost; only the safe row is primary); "Reinstall DSM" uses "Open guide" | 1 |
| 10 | Saved here | AccordionItem (collapsed): file paths + Button "Open folder" | 2 |
| 11 | Open folder | Button | 2 |

**Word budget:** screen 23 + shell 6 = **29** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Each row (i) | Popover | Row name | One sentence per action, from RECOVERY.md. | help: recovery |
| Disabled row | Popover from a sibling info Button "Why unavailable" | Unavailable | The reason, for example that our entries are the active boot path. | - |
| Remove our entries | AlertDialog | Remove our entries? | Debian may no longer start. | Disabled when Debian is the default boot entry; otherwise confirm with the phrase REMOVE ENTRIES |
| Restore flash | AlertDialog (typed phrase) | Restore boot flash? | Needs a working Linux on the unit. | Type RESTORE FLASH; backup file name shown |
| Reinstall DSM | Dialog | Reinstall DSM | Use Synology's page; no data promise. | Opens find.synology.com; dialog text, section 4 |
| Saved here | Tooltip | Disk metadata backup | First and last 1 MiB of changed disks; not your files. | help: backups |

**Primary:** Run (the safe row). **Secondary:** other rows.

**Keyboard:** `Up`/`Down` row; `Enter` Run; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Unit unreachable: Alert "Cannot reach unit" with the manual steps link; *error*: Action failed: Notification error; state re-read; *loading*: Skeleton tags while the state is read.

### D15 Manage: Overview and Health

**Purpose:** One page per converted unit: health first, then the repair, then the management-app update.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Overview, Storage, Logins, Recovery, History | TabBar + Tab (5); Recovery opens D14 inside Manage (the TabBar stays); no page title: the unit header is the title | 5 |
| 2 | Needs attention | Health Tag (success Healthy / warning Needs attention / danger Critical / secondary Unreachable) = the WORST component; reason chips {sdc failed · md1 degraded} (Tags, data); Button icon Refresh | 2 |
| 3 | Management app | Card title + info icon (Popover) + Tag {Update available \| Updating \| Updated \| Rolled back} | 2 |
| 4 | On unit | Label + mono {0.9.1} | 2 |
| 5 | In this app | Label + mono {0.9.2} | 3 |
| 6 | Update | Button: PRIMARY only when health is Healthy; secondary otherwise; while Updating it shows four phases (Upload, Install, Restart, Health check) as a Stepper-style Progress | 1 |
| 7 | Open web UI | Button (secondary) | 3 |
| 8 | Disks | Card title + info icon: rows {sda 37 °C OK ... sdc Failed [Replace]} | 1 |
| 9 | Arrays | Card title + info icon: rows {md0 RAID 1 x3 Degraded [Replace disk] ...} | 1 |
| 10 | Volumes | Card title: rows with Progress {/ 41 %, /srv/data 16 %} | 1 |
| 11 | Services | Card title: Tags {ssh synofurb-mgmt smartd mdadm unattended-upgrades}; a down service is a danger Tag with a Restart button | 1 |
| 12 | Updates | Card title: {12 pending · 3 security}, Tag {Reboot needed} | 1 |
| 13 | Boot default | Card title: {Debian 13 · 6.12.57} and a Tag when the DSM loader entry remains | 2 |
| 14 | Replace | Button (primary when Critical because of a disk or array): opens the Operation plan Dialog with a New disk Select | 1 |
| 15 | Diagnostics | Footer Button (secondary) | 1 |

**Word budget:** screen 27 + shell 6 = **33** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Health Tag | Popover (focusable) | Health | The worst component decides. | Lists each component with its state; help: troubleshooting |
| Management app (i) | Popover | Management app | The web UI service on the NAS. | Update copies the version in this app to the unit; rolls back by itself if the health check fails; help: troubleshooting |
| Update | Dialog | Update management app | The web UI restarts for about 10 seconds. | Dialog text, section 4; buttons Cancel / Update. A warning line appears when an array is degraded or rebuilding. |
| Update (not primary) | Tooltip | Update | Repair first; updating is not urgent. | - |
| Replace | Dialog | Operation plan | Review the steps and exact commands first. | Steps, New disk Select (free disks, size check), Commands Accordion, typed phrase `REPLACE sdc`; buttons Run / Cancel |
| Restart (service) | AlertDialog | Restart service? | Restarting synofurb-mgmt ends web sessions. | Buttons Restart / Cancel |
| Password not kept (Logins tab) | Tooltip | Password | synofurb does not store it; the NAS keeps a hash. | help: first-install |
| Default boot | Tooltip | Default boot | Debian; the DSM loader entry remains. | help: recovery |

**Primary:** Replace (Critical) / Update (Healthy, update available). **Secondary:** Open web UI, Diagnostics.

**Keyboard:** `Left`/`Right` tabs; `Tab` through cards in reading order; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: No history: Empty; *error*: Unreachable: Health Tag "Unreachable", skeleton cards, Alert "Cannot reach unit" + Retry; Update and Open web UI are real disabled Buttons, each with a sibling "Why unavailable" info Button opening a Popover reason; the status bar says Unreachable, never Debian running; *loading*: Skeleton cards while reading status; *updating*: Tabs stay usable; Update shows phases; web UI restarts about 10 s; *update failed*: Tag "Rolled back to {0.9.1}" + Alert with Retry.

### D16 Help centre

**Purpose:** Searchable short topics, shortcuts and docs links.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Help | Sheet title (opens from the right, 480 px; full-window below 960 px) | 1 |
| 2 | Search help | Input (search) | 2 |
| 3 | Topics, Shortcuts | TabBar | 2 |
| 4 | First install, Status meanings, Backups, Recovery, Boot manager, Serial console, Offline mode, Command line, Troubleshooting, Report a result, RAID levels, Hybrid RAID, Replace disk, Scrubbing | List (14 items, one line each) | 25 |
| 5 | Open docs | Link (external-link icon) | 2 |
| 6 | Back | Button (ghost) in topic view | 1 |
| 7 | {topic body ≤150 words} | TextView.markdown + "More" Link | data |

**Word budget:** screen 33 + shell 12 = **45** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Contextual (i) icons | Open this Sheet | Help | Opens the matching topic. | Every Popover/Tooltip links to a topic id (section 6) |
| Search | List filter | Search help | - | Filters titles and bodies; empty result shows Empty "No topics" |

**Primary:** Open topic. **Secondary:** Open docs, Back.

**Keyboard:** `F1` open (topic of the focused control); `Ctrl+F` search; `Esc` close; `Up`/`Down` topics; `Enter` open

**States:** *empty*: Empty "No topics match"; *error*: Docs link offline: Tooltip "Needs Internet"; *loading*: n/a (bundled).

### D17 About

**Purpose:** Identity, version, credits and diagnostics.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | About | Label (title) | 1 |
| 2 | {logo mark + wordmark} | Image (SVG) in Montserrat wordmark | data |
| 3 | Reinstall Synology NAS units with Debian. | Label (tagline) | 6 |
| 4 | {0.9.2 · build 4c1e9d2} | Tag | data |
| 5 | Copy diagnostics | Button (primary) + Clipboard + Notification | 2 |
| 6 | Documentation | Link | 1 |
| 7 | Source code | Link | 2 |
| 8 | Report issue | Link | 2 |
| 9 | Credits | AccordionItem (collapsed) | 1 |
| 10 | {Debian · Lucide · Inter · JetBrains Mono · Montserrat · GPUI Kit · gpui-component · synodeb note} | List inside Credits (name + licence) | data |

**Word budget:** screen 15 + shell 1 = **16** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Copy diagnostics | Tooltip | Copy diagnostics | Version, OS, settings; no secrets. | help: troubleshooting |
| synodeb note | Popover | synodeb | Boot-flash technique documented by synodeb (GPL-2.0). | No synodeb code is used; synofurb is MIT; link to project |
| Licence rows | Link | Licence | - | Opens bundled licence text (OFL, ISC, Apache-2.0, MIT) in a Dialog TextView |

**Primary:** Copy diagnostics. **Secondary:** links.

**Keyboard:** `Tab`; `Enter`; `Esc` back; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Clipboard failure: Notification error; *loading*: n/a.

### D18 Manage: Storage / RAID

**Purpose:** List the unit's arrays and run RAID operations over SSH or the management plane.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Storage | Tab content under D15 (TabBar tab) | 1 |
| 2 | Array, Level, State, Size, Progress | DataTable column headers | 5 |
| 3 | {md1 · RAID 5 · Rebuilding · 10.82 TiB · 24 %} | DataTable rows; State = Tag (success Clean / warning Degraded / info Rebuilding / warning Foreign) + Progress in the row | data |
| 4 | Actions | DropdownMenu per row: Add spare, Replace disk, Grow, Change level, Scrub, Stop, Tune (closed list, items not counted). v1 minimum on desktop: Replace disk and Scrub; the rest may stay web-only IF the menu says "In web UI" on those items | 1 |
| 5 | Open web UI | Button (primary when the desktop cannot do the chosen operation) | 3 |
| 6 | Array degraded | Alert (warning) + Button "Replace disk" | cond. |
| 7 | No arrays | Empty + Button "Open web UI" | cond. |

**Word budget:** screen 10 + shell 6 = **16** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| State Tag | Popover | State | Clean, degraded, rebuilding or foreign. | help: replace-disk |
| Level column (i) | Popover | Level | RAID level and minimum disks. | Same table as D5; help: raid-levels |
| Actions item disabled | Popover from a sibling info Button | Unavailable | Wait for the rebuild to finish. | Other reasons: in use by LVM or mounted; foreign array needs "not mine" |
| Every operation | Dialog (Operation plan) | Operation plan | Review the steps and exact commands first. | Ordered steps, New disk Select where a disk is added, Accordion "Commands" (mono, collapsed), typed phrase per section 6.1; buttons Run / Cancel |
| Foreign Tag | Popover | Foreign | Created by DSM or another system; you can assemble and scrub it. | help: hybrid-raid |
| A disabled primary-looking "Create array" must not exist | Rule | - | - | If an operation is not available on the desktop, the menu item says "In web UI" and opens it; no dead buttons |

**Primary:** Open web UI / Replace disk. **Secondary:** Actions.

**Keyboard:** `Up`/`Down` row; `Enter` opens the row's Actions menu; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Empty "No arrays" + Open web UI; *error*: Unit unreachable: Alert "Cannot reach unit"; an operation failing shows the real error and what was rolled back; *loading*: Skeleton rows; Progress per syncing array.

## 6. Web UI screens

Web screens use the same table format. Interaction rules: every Tooltip is reachable by keyboard focus; Popovers open on click/Enter and close on Esc; Dialogs trap focus and return it; skeletons use `aria-busy`; the rollback banner is `role=alert` with a `role=timer` countdown; icon-only buttons have `aria-label` equal to their tooltip.

### W1 Login

**Purpose:** Sign in; explain pauses and the certificate without prose.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Sign in | h1 | 2 |
| 2 | {ds918} | Brand line (host name) | data |
| 3 | User | label + input | 1 |
| 4 | Password | label + input[type=password] with show toggle button | 1 |
| 5 | Sign in | button.primary | 2 |
| 6 | Passkey | link-button (ghost) | 1 |
| 7 | Certificate | link-button opening a Popover | 1 |
| 8 | Help | link | 1 |
| 9 | Sign-in paused | Alert (error) when locked, with {mm:ss} timer | cond. |
| 10 | Change pending | Alert (warning) at a new address, with {mm:ss} | cond. |

**Word budget:** screen 9 + shell 1 = **10** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Passkey | Tooltip | Passkey | Passkeys work with the name, not an IP address. | help: sign-in |
| Certificate | Popover | Certificate | Compare this fingerprint with the install summary. | SHA-256 first 8 + last 4 bytes, "Show full" (all 32), Copy; help: certificate |
| Sign-in paused | Tooltip | Sign-in paused | Wait, or restart the service over SSH. | Command in mono with Copy: sudo systemctl restart synofurb-mgmt; help: sign-in |
| Change pending | Tooltip | Network change | Sign in, then press Keep settings, or confirm over SSH. | help: safe-apply |

**Primary:** Sign in. **Secondary:** Passkey, Certificate, Help.

**Keyboard:** `Enter` Sign in; `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Fields empty, user focused; *error*: Alert error "Wrong user or password" (role=alert), password cleared; *loading*: Button disabled with Spinner icon.

### W2 Dashboard

**Purpose:** Health from the worst component, what needs attention, key numbers.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Dashboard | h1 | 1 |
| 2 | {ds918 · Debian 13 · up 2 h 14 min} | subtitle (data) | data |
| 3 | Needs attention | Alert (warning) heading; list of items below | 2 |
| 4 | {1 disk warning · RAID sync 24 %} | Alert items (data) | data |
| 5 | View disk | button.ghost | 2 |
| 6 | Updates available | list item | 2 |
| 7 | Review | button.ghost | 1 |
| 8 | CPU, Memory, Temperature, Uptime | Stat cards (4) with progress bars | 4 |
| 9 | Disks | h2 + table | 1 |
| 10 | Disk, Model, Temp, SMART | table headers | 4 |
| 11 | Arrays | Card with {md0, md1} progress rows | 1 |
| 12 | Network | Card with {eth0 address} | 1 |

**Word budget:** screen 19 + shell 14 = **33** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Needs attention | Tooltip | Needs attention | Health shows the worst component. | help: storage |
| SMART value | Tooltip | SMART | Reallocated sectors can mean a failing disk. | help: storage |
| Array row | Popover | Array | State, members and sync speed. | help: storage |
| Temperature | Tooltip | Temperature | CPU; disks listed below. | - |

**Primary:** View disk (first attention item). **Secondary:** Review.

**Keyboard:** `Tab` through attention items then cards; `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Right after boot: "No data yet" Skeleton cards for 60 s; *error*: A failed source shows "unavailable", never 0; *loading*: Skeleton cards.

### W3 Storage

**Purpose:** See volumes, arrays, LVM and disks; extend or delete volumes.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Storage | h1 | 1 |
| 2 | Volumes, LVM, Disks | Tabs (role=tablist); arrays moved to the RAID page | 3 |
| 3 | RAID | link to W13 | 1 |
| 4 | Volume, Mount, Size, Used | table headers | 4 |
| 5 | {data · /srv/data · 8.66 TiB · 16 %} | table rows | data |
| 6 | Create volume | button.primary | 2 |
| 7 | Extend | button.ghost (row) opens Popover | 1 |
| 8 | Delete | button.ghost danger (row) opens Dialog | 1 |
| 9 | Add | Popover: number input (GiB) + button "Extend" | cond. |

**Word budget:** screen 13 + shell 14 = **27** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Extend | Popover | Extend | Grows online; it cannot be shrunk again. | Result preview: new size and free space; help: storage |
| Delete | Dialog | Delete {name}? | Destroys {n} TiB; cannot be undone. | In use by {2 shares, 1 export}; Checkbox "I have a copy" + Input phrase `delete {name}`; button Delete (danger) |
| RAID link | Tooltip | RAID | Arrays, spares and sync have their own page. | help: raid-levels |
| Create volume | Tooltip | Create volume | Uses free space in the volume group. | help: storage |

**Primary:** Create volume. **Secondary:** Extend, Delete.

**Keyboard:** `Left`/`Right` tabs; `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Empty "No free space" with link Help; *error*: Command failure: Alert error with the real message and what was rolled back; *loading*: Sync shows progressbar.

### W4 Create volume

**Purpose:** Four steps; one click at the end; nothing existing is touched.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Create volume | h1 | 2 |
| 2 | Space, Name, Preview, Create | Stepper (ol, aria-current=step) | 4 |
| 3 | {vg_data · 2.16 TiB free} | Step 1: radio list of volume groups | data |
| 4 | Name | Step 2: label + input | 1 |
| 5 | Size | Step 2: label + number input + slider | 1 |
| 6 | Filesystem | Step 2: label + select (xfs, ext4, btrfs) | 1 |
| 7 | Mount | Step 2: label + input (under /srv or /mnt) | 1 |
| 8 | {layout bar: existing + new + free} | Step 3: custom bar (exception) | data |
| 9 | Next | button.primary | 1 |
| 10 | Back | button.ghost | 1 |
| 11 | Create volume | button.primary (step 4, one click) | 2 |

**Word budget:** screen 14 + shell 14 = **28** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Space | Tooltip | Space | Only free space in a volume group is used. | help: storage |
| Filesystem | Tooltip | Filesystem | xfs is the default and can grow online. | help: storage |
| Preview | Popover | Preview | Existing volumes are not touched. | Lists the exact changes: LV, filesystem, fstab entry |

**Primary:** Next / Create volume. **Secondary:** Back.

**Keyboard:** `Enter` Next; `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: No volume group with free space: Empty, link Help; *error*: Field-level errors; failure shows Alert with what was rolled back; *loading*: Create button shows Spinner.

### W5 Mounts and imports

**Purpose:** Mounted filesystems and old Synology volumes imported read-only.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Mounts | h1 | 1 |
| 2 | Mounted, Imported | Tabs | 2 |
| 3 | Mount, Device, Type, Mode, Used | table headers | 5 |
| 4 | {/srv/data · vg_data/data · xfs · rw · 1.4 TiB} | table rows | data |
| 5 | Read-only | Tag (secondary) on imported rows | 1 |
| 6 | Mount | button (opens Dialog Add mount) | 1 |
| 7 | Unmount | button.ghost (row) | 1 |
| 8 | Enable write | button.ghost (imported row; opens Dialog) | 2 |

**Word budget:** screen 13 + shell 14 = **27** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Read-only | Tooltip | Read-only | DSM btrfs volumes mount read-only by default. | help: imports |
| Enable write | Dialog | Enable write? | DSM btrfs may use features Debian handles poorly. | Two short paragraphs; checkbox "I have a copy"; button Enable write (danger) |
| Add mount | Dialog | Add mount | Supported: xfs, ext4, btrfs, exFAT, FAT32, NTFS. | Device select, mount point under /mnt, options; button Mount |

**Primary:** Mount. **Secondary:** Unmount, Enable write.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Imported tab: Empty "No old Synology disks found"; *error*: Mount failure: Alert with the kernel message; *loading*: Spinner on row during mount.

### W6 Network

**Purpose:** Edit interfaces with safe apply and automatic rollback.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Network | h1 | 1 |
| 2 | {eth0 · 192.168.1.40/24 · DHCP · up} | Interface cards (data) | data |
| 3 | Edit | button.ghost per card; opens a docked panel (Sheet, right) | 1 |
| 4 | IPv4 | Panel: label + toggle group DHCP / Static | 1 |
| 5 | Address | Panel: label + input | 1 |
| 6 | Gateway | Panel: label + input | 1 |
| 7 | DNS | Panel: label + input | 1 |
| 8 | IPv6 | Panel: label + toggle group | 1 |
| 9 | Apply safely | Panel: button.primary (opens Dialog) | 2 |
| 10 | Discard | Panel: button.ghost | 1 |
| 11 | Confirm within | Rollback Alert (warning, persistent on every page) + {1:23} | cond. |
| 12 | Keep settings | Rollback Alert: button.primary | cond. |
| 13 | Roll back | Rollback Alert: button | cond. |
| 14 | Cannot reach? | Rollback Alert: link opening Popover | cond. |

**Word budget:** screen 10 + shell 14 = **24** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Apply safely | Dialog | Apply with rollback? | You have 90 seconds to confirm at the new address. | Shows new URL, certificate fingerprint (short + Show full), "the browser will warn again", "passkeys do not work on an IP address", the SSH confirm command with Copy; buttons Apply / Cancel |
| Cannot reach? | Popover | Do nothing | At the deadline the old settings return. | The address may differ: look for {host}.local or scan with synofurb |
| Apply disabled while pending | Tooltip | Pending change | Keep or roll back the pending change first. | - |
| DNS | Tooltip | DNS | Comma-separated servers. | - |
| Rollback Alert | Tooltip | Pending change | Confirm from the new address or over SSH. | help: safe-apply |

**Primary:** Apply safely. **Secondary:** Discard, Keep settings, Roll back.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Interface with no cable: Tag "No cable"; *error*: Apply failed: immediate rollback, Alert explains; *loading*: Applying: Spinner; countdown starts when applied.

### W7 Users

**Purpose:** Manage local accounts with a docked edit panel.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Users | h1 | 1 |
| 2 | Name, Groups, Keys, Sign-in, Last login | table headers | 6 |
| 3 | {andreas · sudo · 1 · active · today} | table rows | data |
| 4 | New user | button.primary | 2 |
| 5 | Full name | Panel: label + input | 2 |
| 6 | Groups | Panel: label + multiselect | 1 |
| 7 | SSH keys | Panel: label + textarea (mono) | 2 |
| 8 | Password login | Panel: checkbox | 2 |
| 9 | Save | Panel: button.primary | 1 |
| 10 | Cancel | Panel: button.ghost | 1 |
| 11 | Edit, Lock, Password, Delete | Row icon buttons with tooltips only | cond. |

**Word budget:** screen 18 + shell 14 = **32** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Row icons | Tooltip | Edit / Lock / Password / Delete | Each icon button has a text tooltip and aria-label. | - |
| SSH keys | Tooltip | SSH keys | One public key per line. | help: sign-in |
| Delete | Dialog | Delete {name}? | Removes the account; the home directory is kept. | Buttons Delete (danger) / Cancel |
| Last admin | Tooltip on Delete | Delete | At least one admin must remain. | - |

**Primary:** New user. **Secondary:** Edit, Lock.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Only the admin: hint Tag "Create a second admin" with Tooltip; *error*: Field errors inline; weak password message in Tooltip; *loading*: Save shows Spinner.

### W8 Boot

**Purpose:** Choose the next boot entry; be honest about what returns you.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Boot | h1 | 1 |
| 2 | Power cycle returns to default. | Alert (info), one line | 5 |
| 3 | Debian, Debian previous kernel, Boot agent, DSM loader | Radio group (4 entries) with Tags | 8 |
| 4 | Default | Tag on the default entry | 1 |
| 5 | One-shot | Tag on Boot agent | 1 |
| 6 | Reboot into entry | button.primary (opens Dialog) | 3 |
| 7 | Next boot only | button | 3 |
| 8 | Make default | button (typed Dialog for non-Debian entries) | 2 |

**Word budget:** screen 24 + shell 14 = **38** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Boot agent | Popover | Boot agent | It resets the default to DSM and reboots after 60 idle minutes. | kexec starts Linux kernels only; after kexec nothing of ours runs; help: boot |
| DSM loader | Popover | DSM loader | Starts Synology's loader; DSM's own system may be gone. | Shows Synology's install page; help: recovery |
| Reboot | Dialog | Reboot now? | Running tasks and SSH sessions are listed. | RAID sync pauses and resumes; buttons Reboot / Cancel |
| Make default | Dialog | Make default? | A headless NAS starts this entry after a power loss. | Type phrase `make default {entry}`; button danger |

**Primary:** Reboot into entry. **Secondary:** Next boot only, Make default.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: n/a; *error*: Entry unavailable: row disabled with Tooltip reason; *loading*: Rebooting: page shows reconnect Spinner.

### W9 System

**Purpose:** Identity, services, updates, power.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | System | h1 | 1 |
| 2 | Identity, Services, Updates, Power | Accordion (4 sections; first open) | 4 |
| 3 | Hostname | label + input | 1 |
| 4 | Time zone | label + select | 2 |
| 5 | Service, State | table headers | 2 |
| 6 | Restart | button.ghost per row | 1 |
| 7 | Install all | button.primary | 2 |
| 8 | Details | button.ghost (opens Sheet) | 1 |
| 9 | Reboot | button (opens Dialog) | 1 |
| 10 | Shut down | button.danger (opens Dialog) | 2 |

**Word budget:** screen 17 + shell 14 = **31** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Services | Tooltip | Services | Stopping the web service ends this session. | Start again over SSH: sudo systemctl start synofurb-mgmt; help: sign-in |
| Updates | Tooltip | Updates | Security fixes install by themselves and never reboot. | A kernel update needs a reboot you choose; help: boot |
| Reboot | Dialog | Reboot now? | Running tasks and SSH sessions are listed. | Buttons Reboot / Cancel |
| Shut down | Dialog | Shut down? | You will need to power it on by hand. | Buttons Shut down (danger) / Cancel |

**Primary:** Install all. **Secondary:** Restart, Details.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: n/a; *error*: Command failure: Alert with message; *loading*: Updates: progressbar.

### W10 Audit log

**Purpose:** Append-only record of sign-ins and changes.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Audit log | h1 | 2 |
| 2 | Search | input (search) | 1 |
| 3 | All, Sign-ins, Storage, Network, Users | Toggle filter chips | 5 |
| 4 | Time, Who, From, Action, Result | table headers | 5 |
| 5 | Export CSV | button.ghost | 2 |

**Word budget:** screen 15 + shell 14 = **29** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Result tag | Tooltip | Result | Ok, failed or in progress. | - |
| Lockout row | Tooltip | Lockout | 5 failures paused sign-in; anyone on the network can cause this. | help: sign-in |

**Primary:** Export CSV. **Secondary:** filters.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Empty "No entries match"; *error*: n/a; *loading*: Skeleton rows.

### W11 Help centre

**Purpose:** Searchable short topics and shortcuts.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Help | h1 in a Sheet (right, 420 px; full width on phones) | 1 |
| 2 | Search help | input (search) | 2 |
| 3 | Safe apply, Sign-in, Certificate, Storage, Imports, Boot, Recovery, Report, RAID levels, Hybrid RAID, Replace disk, Scrubbing | List of topics (12) | 16 |
| 4 | Shortcuts | Tab | 1 |
| 5 | Open docs | link | 2 |
| 6 | Back | button.ghost in topic view | 1 |

**Word budget:** screen 23 + shell 14 = **37** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Contextual (i) | Opens this Sheet | Help | Opens the matching topic. | Every Tooltip/Popover names a topic id (section 6) |

**Primary:** Open topic. **Secondary:** Open docs.

**Keyboard:** `?` or `F1` open; `Esc` close; `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: Empty "No topics match"; *error*: n/a; *loading*: n/a (bundled).

### W12 About

**Purpose:** Version, licences, diagnostics.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | About | h1 | 1 |
| 2 | {logo mark + wordmark} | Inline SVG (light/dark) | data |
| 3 | {0.9.2} | Tag | data |
| 4 | Copy diagnostics | button.primary + Toast | 2 |
| 5 | Documentation | link | 1 |
| 6 | Source code | link | 2 |
| 7 | Report issue | link | 2 |
| 8 | Credits | details/summary | 1 |
| 9 | {Debian · Lucide · Inter · JetBrains Mono · Montserrat · synodeb note} | List inside Credits | data |

**Word budget:** screen 9 + shell 14 = **23** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Copy diagnostics | Tooltip | Copy diagnostics | Version, host, service state; no secrets. | help: sign-in |
| synodeb note | Popover | synodeb | Boot-flash technique documented by synodeb (GPL-2.0). | No code used; project link |

**Primary:** Copy diagnostics. **Secondary:** links.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: n/a; *error*: Clipboard failure: Toast error; *loading*: n/a.

### W13 RAID

**Purpose:** List arrays and open one for detail and operations.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | RAID | h1 | 1 |
| 2 | Arrays, spares and sync. | subtitle | 4 |
| 3 | Array, Level, State, Size, Members | table headers | 5 |
| 4 | {md1 · RAID 5 · Rebuilding · 10.82 TiB · sda3 sdb3 sdc3 sdd3} | table rows; State = Tag + progressbar with speed and time left | data |
| 5 | Create array | DropdownButton (primary): RAID array, Hybrid array | 2 |
| 6 | Actions | Row menu button: Add spare, Replace disk, Grow, Change level, Scrub, Stop, Tune (closed menu, items not counted) | 1 |
| 7 | Level, State, Size | Detail Sheet (right): dl labels | 3 |
| 8 | Spares, Bitmap | Detail Sheet: dl labels | 2 |
| 9 | Last check | Detail Sheet: dl label | 2 |
| 10 | Mismatches | Detail Sheet: dl label | 1 |
| 11 | Disk, State, Size | Detail Sheet: members table headers | 3 |
| 12 | Array degraded | Alert (warning) + button "Replace disk" | cond. |
| 13 | Foreign | Tag (warning) on arrays created elsewhere | cond. |
| 14 | No arrays | Empty + button "Create array" | cond. |

**Word budget:** screen 24 + shell 14 = **38** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Level column (i) | Popover | Level | RAID level and minimum disks. | Table: level, minimum disks, survives; help: raid-levels |
| State Tag | Tooltip | State | Clean, degraded, rebuilding or foreign. | help: replace-disk |
| Rebuilding progress | Tooltip | Rebuilding | Speed and time left; the array stays usable. | help: replace-disk |
| Foreign Tag | Tooltip | Foreign | Created by DSM or another system; you can assemble and scrub it. | Create, grow and destroy need an "I understand this is not mine" checkbox first; help: hybrid-raid |
| Actions item disabled | Tooltip | Unavailable | Wait for the rebuild to finish. | Other reasons: mounted or used by LVM (Stop), array not degraded (Replace failed) |
| Mismatches | Tooltip | Mismatches | Blocks that disagree between copies or parity. | help: scrubbing |
| Tune | Popover | Tune | Sync speed limits, stripe cache, bitmap. | Number inputs with units; no typed confirm; help: scrubbing |

**Primary:** Create array. **Secondary:** Actions.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets; `Enter` opens the focused row; arrow keys move in the level picker

**States:** *empty*: Empty "No arrays" + Create array; *degraded*: Alert warning + row Tag danger "Degraded" + button Replace disk; *rebuilding*: Row progressbar; Actions disabled except Scrub-stop and Tune; Tooltip "Wait for the rebuild"; *foreign*: Tag "Foreign"; only Assemble and Scrub enabled until the "not mine" checkbox is ticked; *error*: Command failure: Alert error with the real message and what was rolled back; *loading*: Skeleton rows.

### W14 Create array

**Purpose:** Pick a level, see capacity live, choose disks, confirm.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Create array | h1 | 2 |
| 2 | Level, Disks, Preview, Confirm | Stepper (ol, aria-current=step) | 4 |
| 3 | Level | Step 1: label + radio cards: None, 0, 1, 5, 6, 10 (closed set; each shows min disks) | 1 |
| 4 | Name | Step 1: label + input (default {data2}) | 1 |
| 5 | Spares | Step 1: label + number input | 1 |
| 6 | Usable | Live preview Tag + data {7.22 TiB} | 1 |
| 7 | Survives | Live preview Tag + data {1 disk} | 1 |
| 8 | Disk, Size, Free | Step 2: table headers with checkbox per row | 3 |
| 9 | Commands | Step 3: details/summary (collapsed, mono) | 1 |
| 10 | Type phrase | Step 4: label + input; phrase code chip {CREATE data2} | 2 |
| 11 | Next | button.primary | 1 |
| 12 | Back | button.ghost | 1 |
| 13 | Create array | button.danger (step 4) | 2 |
| 14 | Not mine | Step 4 checkbox, only for foreign arrays | cond. |

**Word budget:** screen 21 + shell 14 = **35** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Level cards | Tooltip | Level | Minimum disks and failures survived. | help: raid-levels |
| Usable / Survives | Popover | Capacity | Usable space and fault tolerance, live. | Raw, usable, wasted (smallest member size is used on every member); help: raid-levels |
| Disks step | Tooltip | Disks | Only unused disks and partitions are listed. | Mounted, system and other-array devices are never offered |
| Commands | Accordion | Commands | Exact commands, run in order. | e.g. mdadm --create /dev/md/data2 --level=5 --raid-devices=3 --metadata=1.2 /dev/sdb3 /dev/sdc3 /dev/sdd3 |
| Create array | Tooltip | Create array | Erases the selected partitions. | Typed phrase CREATE {name}; see section 5b |

**Primary:** Next / Create array. **Secondary:** Back.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets; `Enter` opens the focused row; arrow keys move in the level picker

**States:** *empty*: No unused disks: Empty with link Help; *error*: Too few disks for the level: inline Alert, Next disabled with Tooltip; *loading*: Create shows progress of each ordered step.

### W15 Create hybrid

**Purpose:** Build an SHR-style layout from disks of different sizes.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Create hybrid | h1 | 2 |
| 2 | Disks, Level, Slices, Confirm | Stepper | 4 |
| 3 | Disk, Size, Free | Step 1: table headers + checkbox per row | 3 |
| 4 | 1 disk, 2 disks | Step 2: radio group (Hybrid 1-disk / 2-disk) | 4 |
| 5 | Usable | Live preview Tag + data {8 TB} | 1 |
| 6 | Survives | Live preview Tag + data {1 disk} | 1 |
| 7 | Unused | Live preview Tag (warning) + data {4 TB} | 1 |
| 8 | Slice, Disks, Level, Usable | Step 3: slices table headers | 4 |
| 9 | {1 · sda sdb sdc · RAID 5 · 8 TB / 2 · sdc · none · unused} | slices table rows | data |
| 10 | Commands | details/summary (collapsed) | 1 |
| 11 | Type phrase | Step 4: label + input; chip {CREATE hybrid} | 2 |
| 12 | Next | button.primary | 1 |
| 13 | Back | button.ghost | 1 |
| 14 | Create hybrid | button.danger | 2 |
| 15 | Weaker slice | Alert (warning) when a Hybrid 2-disk slice has fewer than 4 members | cond. |

**Word budget:** screen 27 + shell 14 = **41** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Hybrid choice | Popover | Hybrid | Mixes disk sizes; every shared slice becomes an array. | Example 4, 4, 8 TB = 8 TB usable; SHR-style, not adopted by DSM; help: hybrid-raid |
| Slices table | Tooltip | Slice | A slice is space present on the same disks. | help: hybrid-raid |
| Unused | Tooltip | Unused | Space on a single disk has no partner and stays unused. | help: hybrid-raid |
| Weaker slice | Tooltip | Weaker slice | This slice tolerates fewer failures than you chose. | help: hybrid-raid |
| Commands | Accordion | Commands | Partitions, arrays data1 to dataN, then vg_data. | Per slice: partition each disk, mdadm --create, then vgcreate vg_data and lvcreate; listed in order |
| Create hybrid | Tooltip | Create hybrid | Erases the selected disks. | Typed phrase CREATE hybrid; section 5b |

**Primary:** Next / Create hybrid. **Secondary:** Back.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets; `Enter` opens the focused row; arrow keys move in the level picker

**States:** *empty*: Fewer than 2 unused disks: Empty; *error*: Fewer than 4 disks for Hybrid 2-disk: option disabled with Tooltip; *loading*: Ordered steps with progress.

### W16 Replace disk

**Purpose:** Swap a failed or healthy member; watch the rebuild.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Replace disk | h1 | 2 |
| 2 | Member, New disk, Preview, Confirm | Stepper | 5 |
| 3 | Disk, State | Step 1: member table headers (failed member preselected) | 2 |
| 4 | Disk, Size, Free | Step 2: new-disk table headers (must be at least as large) | 3 |
| 5 | Steps | Step 3: ordered list; for Hybrid arrays a table headed Step, Disk, Action | 1 |
| 6 | Commands | Step 3: details/summary (collapsed) | 1 |
| 7 | Type phrase | Step 4: label + input; chip {REPLACE sdc} | 2 |
| 8 | Next | button.primary | 1 |
| 9 | Back | button.ghost | 1 |
| 10 | Replace disk | button.danger | 2 |
| 11 | Rebuilding | Progress view: Tag + progressbar + labels Speed, Left | cond. |

**Word budget:** screen 20 + shell 14 = **34** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Failed vs healthy | Tooltip | Replace | A healthy member is replaced without losing redundancy. | Failed: --fail, --remove, --add. Healthy: --add then --replace --with; help: replace-disk |
| New disk | Tooltip | New disk | It is erased; it must be at least as large as the member. | help: replace-disk |
| Steps | Accordion body | Steps | Reviewed list; re-derived on the server when you run it. | help: replace-disk |
| Commands | Accordion | Commands | Exact commands, run in order. | mdadm /dev/md1 --fail /dev/sdc3; mdadm /dev/md1 --remove /dev/sdc3; mdadm /dev/md1 --add /dev/sde3 |
| Replace disk | Tooltip | Replace disk | Starts a rebuild that takes hours. | Typed phrase REPLACE {disk}; section 5b |

**Primary:** Next / Replace disk. **Secondary:** Back.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets; `Enter` opens the focused row; arrow keys move in the level picker

**States:** *empty*: No free disk: Empty "Insert a disk" with link Help; *error*: Step failure: Alert with the real message; the array state is re-read; *rebuilding*: Progress view replaces the wizard; leaving the page is safe.

### W17 RAID operation

**Purpose:** Shared dialog for Add spare, Grow, Change level, Scrub, Stop, Assemble.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | {Grow data1} | Dialog title (verb + array) | data |
| 2 | {ordered steps: 1 add disk, 2 reshape, 3 pvresize, 4 lvextend, 5 xfs_growfs} | ol (server-derived plan) | data |
| 3 | Commands | details/summary (collapsed, mono) | 1 |
| 4 | Not mine | checkbox, foreign arrays only | cond. |
| 5 | Type phrase | label + input; chip per section 5b | 2 |
| 6 | Run | button.danger (button.primary for non-destructive Scrub check, Assemble) | 1 |
| 7 | Cancel | button.ghost | 1 |

**Word budget:** screen 5 + shell 14 = **19** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Plan | Tooltip | Plan | Re-derived on the server when you run it. | help: replace-disk |
| Refusal | Alert (error) in the dialog | Not possible | - | Reasons: mounted or in use by LVM, already resyncing or reshaping, system device |
| Scrub check / repair | Toggle in dialog | Check / Repair | Check counts mismatches; repair rewrites them. | Repair needs the typed phrase; help: scrubbing |

**Primary:** Run. **Secondary:** Cancel.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: n/a; *error*: Step failure: Alert with message and what was rolled back; *loading*: Ordered steps with status icons.

### 6.1 RAID operations (shared by D18, W13 to W17; source: docs/RAID.md)

Every operation: server-side plan, typed confirmation where listed, ordered steps, audit entry; the plan is re-derived on the server at execution; sizes and names pass the existing validators; **the exact commands are always behind a collapsed "Commands" Accordion** (mono). A foreign array (created by DSM or another system) can be listed, assembled and scrubbed; create, grow and destroy on it first require the checkbox "Not mine" ("I understand this is not mine").

| Operation | Typed phrase | Commands shown behind the disclosure | Refused when |
|---|---|---|---|
| List arrays | none | `mdadm --detail /dev/mdX`, `cat /proc/mdstat`, `/sys/block/mdX/md/*` | n/a |
| Create array | `CREATE <name>` | `mdadm --create /dev/md/<name> --level=<L> --raid-devices=<N> --metadata=1.2 <partitions>` | device mounted, holds `/`, `/boot` or swap, or belongs to another array; too few disks |
| Create hybrid (SHR-style) | `CREATE hybrid` | per slice: partition each disk, `mdadm --create` (arrays `data1`, `data2`, ...), then `vgcreate vg_data` and `lvcreate` (linear over the arrays) | fewer than 2 disks (4 for Hybrid 2-disk); a single-member slice is reported as unused, never used |
| Add spare | `ADD <disk>` (the disk is erased) | `mdadm --manage /dev/mdX --add /dev/<disk>` | array not healthy enough, device in use |
| Replace a failed member | `REPLACE <disk>` | `mdadm /dev/mdX --fail <old>`; `--remove <old>`; `--add <new>` | array already rebuilding |
| Replace a healthy member | `REPLACE <disk>` | `mdadm /dev/mdX --add <new>`; `mdadm /dev/mdX --replace <old> --with <new>` | array resyncing or reshaping |
| Grow capacity | `GROW <name>` | `mdadm --grow /dev/mdX --raid-devices=N` (RAID 5/6) or `--size=max` after larger disks; then `pvresize`, `lvextend`, `xfs_growfs` | resyncing; may need a backup file for some levels |
| Change level | `LEVEL <name>` | `mdadm --grow /dev/mdX --level=<new>` (+ `--raid-devices`); only transitions mdadm supports are listed (for example RAID 1 to RAID 5 with 3 members, RAID 5 to RAID 6 with one more disk) | unsupported transition; resyncing |
| Scrub: check | none (simple confirm) | `echo check > /sys/block/mdX/md/sync_action`; stop with `idle` | resyncing or reshaping |
| Scrub: repair | `REPAIR <name>` | `echo repair > /sys/block/mdX/md/sync_action` | resyncing or reshaping |
| Stop | simple confirm with consequences | `mdadm --stop /dev/mdX` | mounted or used by LVM |
| Assemble | simple confirm | `mdadm --assemble --scan` | n/a |
| Tune | none (Tooltip explains units) | `speed_limit_min` / `speed_limit_max`, `stripe_cache_size`, write-mostly, bitmap on/off | n/a |
| SHR add disk / replace disk | `REPLACE <disk>` or `ADD <disk>` | a reviewed list of steps: partition the new disk like the others, then add it to each slice's array or create new slices | array resyncing |

Capacity preview rule (Create array and Create hybrid): usable bytes follow `docs/RAID.md` (RAID 1 = length, RAID 5 = (m-1) x length, RAID 6 = (m-2) x length per slice; equal-size levels use the smallest member on every disk); the preview is a `Tag` pair (*Usable*, *Survives*) plus, for Hybrid, *Unused* and the slices table. It recomputes on every change and never promises more than the planner returns. Worked examples to show as the sample data: 4, 4, 8 TB Hybrid 1-disk = 8 TB; 2, 4, 4, 8 TB = 10 TB; 1, 2, 2, 3 TB = 5 TB.

Health summary: mail on degrade comes from `mdadm --monitor` (`mdmonitor.service`); the Dashboard shows the worst component (a degraded array is *Needs attention*).

## 7. Help centre content

**Desktop Help centre (D16)**: a `Sheet` from the right (480 px). Top: `Input` search. Tabs: *Topics*, *Shortcuts*. Topic view (14 topics: first install, status meanings, backups, recovery, boot manager, serial console, offline mode, command line, troubleshooting, report a result, RAID levels, Hybrid RAID, replacing a failed disk, scrubbing): `TextView::markdown` body (at most 150 words) + a *More* `Link` to the docs page + *Back*. Opened by `F1` (topic of the focused control), every info icon, and the TitleBar Help button. Search matches titles and bodies. Offline: the topics are bundled; only *More* needs Internet.

| Id | Title | Words | Docs target |
|---|---|---|---|
| first-install | First install | 112 | README.md, docs/INSTALL.md |
| compat-status | Status meanings | 96 | docs/COMPAT.md |
| backups | Backups | 93 | docs/RECOVERY.md |
| recovery | Recovery | 150 | docs/RECOVERY.md |
| boot-manager | Boot manager | 137 | docs/BOOT.md |
| serial-console | Serial console | 83 | docs/BOOT.md, docs/ARM.md |
| offline-mode | Offline mode | 66 | docs/INSTALL.md |
| cli | Command line | 80 | README.md |
| troubleshooting | Troubleshooting | 139 | docs/RECOVERY.md |
| report-result | Report a result | 76 | docs/COMPAT.md |
| raid-levels | RAID levels | 108 | docs/RAID.md |
| hybrid-raid | Hybrid RAID | 131 | docs/RAID.md |
| replace-disk | Replacing a failed disk | 128 | docs/RAID.md, docs/RECOVERY.md |
| scrubbing | Scrubbing | 87 | docs/RAID.md |

#### first-install: First install

1. Scan, pick your unit and connect. Enable SSH in DSM first (Control Panel, Terminal and SNMP).
2. Inspect is read-only.
3. Choose Boot manager to try the agent, or Install Debian.
4. Plan storage, set the system options, then confirm.
The install takes about 25 minutes. Kernel: Stable is Debian's 6.12 LTS; Newest is 7.x from trixie-backports and falls back to Stable if it cannot be installed. Source: Online needs Internet on the NAS; Offline uses a bundle you downloaded in Settings. Afterwards the data array syncs for about 9 hours; the NAS works meanwhile with reduced protection. Copy the generated password on the last screen: synofurb does not keep it.

#### compat-status: Status meanings

Each model has one status.
**Verified**: maintainers or two independent people installed it, with logs. No model has this yet.
**Community-reported**: at least one report exists. DS923+ has one, from another project.
**Untested**: no report. The hardware class should work in theory; that is not a promise, and your install is the test.
**Known issues**: reports show something fails or only partly works.
**Unsupported**: Debian 13 cannot run on this hardware; synofurb refuses to install.
Only Unsupported blocks. The list ships with the app and refreshes from the project repository; offline it uses the cached copy.

#### backups: Backups

Your own backup is essential. In Wipe mode nothing on the disks is preserved; in Keep mode the data volume is left alone on a best-effort basis with no guarantee.
synofurb also saves on this computer:
- **Boot-flash image**: the whole flash, hash-verified, before the first flash write. Keep it off the NAS.
- **Disk metadata backup**: the first and last 1 MiB of every disk we change (partition tables, GPT headers). That is metadata only, not your files, and it cannot bring back a formatted filesystem.
- The plan, the install script and the log.

#### recovery: Recovery

DSM stays the default boot entry; every failure points it back at DSM, except the "Agent silent" row.
| When | What happens |
|---|---|
| Before the boot flash is changed | Nothing to undo |
| Agent started | It sets DSM as default first; reboots if nobody logs in |
| Before step 3 | A failure reboots to DSM after 60 s; a power cycle also works |
| Agent silent after 10 minutes | Not confirmed. If it started, DSM returns 20 minutes or more later. If not, a power cycle loops: use SYNOLOGY_2 in the boot menu (serial or VGA console), or restore the flash from another computer. |
| After step 3 | Rescue shell 15 minutes, then the DSM loader; DSM may be gone, reinstall it from Synology; no data promise |
| Debian installed | Pick the previous kernel or DSM in the boot menu |
Commands: `synofurb recover`, `synofurb boot-manager enter`.

#### boot-manager: Boot manager

x86 units boot through a small internal boot flash. Boot manager adds an SSH agent entry to it. The whole flash is saved to this computer first; disks are untouched; DSM's entry stays the default.
The agent is one-shot: it resets the default to DSM when it starts and reboots to DSM after 60 minutes without a login. From it, `sf-boot kexec <kernel-url> <initrd-url>` starts another Linux kernel (not Windows or UEFI-only ISOs). Once another system runs, nothing of ours is running: power-cycling returns to the default entry, and that system can change your disks. If the agent never starts, nothing resets the default and nothing reboots the unit: choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer.

#### serial-console: Serial console

A serial console shows boot messages and lets you log in without a network. Intel units usually use `ttyS0` at 115200 8N1; the app reads the setting from DSM's kernel command line. AMD units have a memory-mapped UART; the exact console argument is copied from DSM. ARM boards need a 3.3 V TTL serial adapter (not RS-232 levels); without it you cannot recover an ARM unit over SSH alone. Debian also opens a login on any USB serial adapter you plug in later.

#### offline-mode: Offline mode

Offline installs use a bundle: the installer files and packages for one CPU architecture. Download it in Settings, Data (this computer needs Internet once). In System, choose Source: Offline and select the bundle. The NAS then needs no Internet during the install. The compatibility list also works offline from its cached copy. Update checks, list refresh and result reports need Internet and are disabled while offline.

#### cli: Command line

Everything in the app is also a command, and the command line is the screen-reader accessible path (the app window does not support screen readers).
`synofurb scan` discovers units.
`synofurb plan --host <ip> --user <admin>` shows the plan, read-only.
`synofurb install ... --have-backup --accept-data-loss --confirm` converts a unit.
`synofurb recover` and `synofurb boot-manager enter` handle recovery.
The Copy command button on Plan, System and Confirm copies the equivalent command. On Confirm the three confirmation flags are included only after you confirm.

#### troubleshooting: Troubleshooting

**No units found**: wrong network or VLAN, NAS still booting, firewall (UDP 9999, mDNS 5353, TCP 22). Use Add by IP.
**SSH is off**: enable it in DSM, Control Panel, Terminal and SNMP.
**Login failed**: DSM may auto-block this computer after a few failures; check the account is an administrator.
**Installer did not come up**: not confirmed. If the agent started, DSM returns by itself in 20 minutes or more; a power cycle also works. If it never started, the unit may keep booting the agent: choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer (Help, Recovery).
**Boot flash write failed**: the tool restores the flash and verifies it; use the rescue shell within 15 minutes.
Logs: `~/.local/share/synofurb/logs` (secrets removed). Settings, Logs opens the folder; About copies diagnostics.

#### report-result: Report a result

Reports move a model from Untested towards Verified. After an install, press Report result on the Done screen. It opens a pre-filled GitHub issue (compat report template) with the model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click, and Settings, Privacy has an off switch. A maintainer checks the log and changes the status through a reviewed pull request.

#### raid-levels: RAID levels

RAID combines disks into one array. **None**: one disk, no protection. **RAID 0**: striped and fast; any failure loses everything (2+ disks). **RAID 1**: mirror, survives all but one disk (2+). **RAID 5**: survives one failed disk; usable space is (n-1) disks (3+). **RAID 6**: survives two; usable (n-2) disks (4+). **RAID 10**: mirrored stripes, survives one disk per mirror, half the capacity (4+). Equal-size levels use the smallest disk's size on every member, so larger disks waste space; Hybrid avoids that. RAID is not a backup: it does not protect against deletion, corruption or fire. A new array syncs for hours and works meanwhile with reduced protection.

#### hybrid-raid: Hybrid RAID

Hybrid (SHR-style) mixes disk sizes. synofurb cuts the disks' data space into slices at the sizes of the different disks; every slice on two or more disks becomes an array (RAID 1 for two members, RAID 5 for three or more; Hybrid 2-disk uses RAID 6 for four or more). LVM joins the arrays into one volume. Example: 4, 4 and 8 TB with Hybrid 1-disk gives 8 TB usable; the extra 4 TB on the large disk has no partner and stays unused. Hybrid 1-disk survives any one disk; Hybrid 2-disk any two where a slice has four or more members. It is built with mdadm and LVM, so Debian manages it. DSM may not adopt it, and DSM-made volumes import read-only. Nothing here has run on a real Synology.

#### replace-disk: Replacing a failed disk

A failed member leaves the array degraded: it works, but one more failure may lose data (RAID 5). 1. Check the disk on the Dashboard or RAID page. 2. Choose the new disk; it is erased and must be at least as large. 3. The tool marks the old member failed, removes it and adds the new one. 4. The rebuild runs for hours; progress and speed are shown, and other operations wait. Do not power off. To upgrade a healthy member, the tool adds a spare and uses a replace operation, so redundancy is kept during the swap. After replacing every member with a larger disk you can grow the array. Each step shows its exact commands and asks you to type REPLACE and the disk name.

#### scrubbing: Scrubbing

A scrub reads every block and compares copies or parity to find silent errors. Check only counts mismatches; Repair also rewrites them and needs a typed confirmation. Debian's mdcheck timer runs a scrub monthly; you can start one or stop it (idle) from the RAID page. The array stays usable but slower while it runs, and speed limits can be tuned. A non-zero mismatch count on a healthy array deserves attention: check SMART on every member. A scrub cannot start while the array is resyncing or reshaping.

**Shortcuts tab (desktop)**

| Keys | Action |
|---|---|
| `F1` | Help for the focused control |
| `Ctrl+,` | Settings |
| `Ctrl+F` | Search (inventory filter, log, help) |
| `Alt+Left / Alt+Right` | Previous / next completed step |
| `F6` | Jump between regions |
| `Ctrl+Shift+C` | Copy command |
| `Esc` | Close popover or dialog; never cancels an install |
| `Space` | Press the focused erase button (Enter does not) |

**Web Help centre (W11)**: a right `Sheet`; topics below; same rules (at most 150 words, *More* link).

| Id | Title | Words |
|---|---|---|
| safe-apply | Safe apply | 70 |
| sign-in | Sign-in | 61 |
| certificate | Certificate | 50 |
| storage | Storage | 74 |
| imports | Imported volumes | 36 |
| boot | Boot page | 99 |
| recovery | Recovery | 118 |
| report | Report a result | 32 |
| raid-levels | RAID levels | 108 |
| hybrid-raid | Hybrid RAID | 131 |
| replace-disk | Replacing a failed disk | 128 |
| scrubbing | Scrubbing | 87 |

#### web:safe-apply: Safe apply

Network changes are applied at once, then you have 90 seconds to confirm. Open the new address (shown before you apply), accept the certificate warning, sign in and press Keep settings, or run the SSH confirm command. If nothing is confirmed, the old settings return. The address may differ after a rollback; look for the host name (`<host>.local`) or scan with synofurb. Passkeys do not work on an IP address.

#### web:sign-in: Sign-in

Five failed attempts in ten minutes pause sign-in for that account for five minutes. Anyone on your network can trigger a pause, so it does not always mean you mistyped. Wait, or restart the service over SSH: `sudo systemctl restart synofurb-mgmt`. Passkeys work when you open the NAS by name, not by IP address. SSH keys: one public key per line.

#### web:certificate: Certificate

The web UI uses a self-signed certificate, so the browser warns once per address. Compare the SHA-256 fingerprint with the one shown at the end of the install (first 8 bytes and last 4 are shown; Show full gives all 32). If they differ, stop and do not sign in.

#### web:storage: Storage

Volumes live in LVM volume groups on RAID arrays. Create volume uses only free space in a volume group and never touches existing volumes. Extend grows xfs online and cannot be shrunk again. Delete destroys the files; the dialog lists what uses the volume. Health shows the worst component: a SMART warning or a degraded array means Needs attention. A new RAID 5 array syncs for hours; the NAS works meanwhile with reduced protection.

#### web:imports: Imported volumes

Disks from an old Synology are assembled and mounted read-only at `/mnt/dsm/<name>`. Copy your files off with rsync. Enable write only if you have a copy: DSM btrfs can use features Debian handles poorly.

#### web:boot: Boot page

These are the entries in the boot flash menu: Debian (default), the previous kernel, the one-shot boot agent and the DSM loader. Power-cycling always returns to the default entry. The agent returns to DSM after 60 idle minutes, but only if it started; if it never starts, the default can stay on the agent: choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer. Once another system is started with kexec, nothing of ours is running. The DSM loader shows Synology's install page; DSM's own system may be gone.

#### web:recovery: Recovery

If something fails: DSM stays the default boot entry, except while the agent is armed for one boot. Before the installer's step 3 a failure reboots to DSM in 60 seconds; if the agent does not answer within 10 minutes, DSM is not confirmed: if the agent started its watchdog reboots to DSM, if it never started a power cycle boots the agent again, so choose SYNOLOGY_2 in the boot menu on a serial or VGA console or restore the boot flash from another computer; after it a rescue shell stays open for 15 minutes, then the DSM loader runs and DSM must be reinstalled from Synology. We promise nothing about data. Commands: `synofurb recover` on your computer.

#### web:report: Report a result

Use Report issue on the About page to open the compatibility report template. Include the model, DSM version, synofurb version, what worked and the log. Do not include serial numbers or passwords.

#### web:raid-levels: RAID levels

RAID combines disks into one array. **None**: one disk, no protection. **RAID 0**: striped and fast; any failure loses everything (2+ disks). **RAID 1**: mirror, survives all but one disk (2+). **RAID 5**: survives one failed disk; usable space is (n-1) disks (3+). **RAID 6**: survives two; usable (n-2) disks (4+). **RAID 10**: mirrored stripes, survives one disk per mirror, half the capacity (4+). Equal-size levels use the smallest disk's size on every member, so larger disks waste space; Hybrid avoids that. RAID is not a backup: it does not protect against deletion, corruption or fire. A new array syncs for hours and works meanwhile with reduced protection.

#### web:hybrid-raid: Hybrid RAID

Hybrid (SHR-style) mixes disk sizes. synofurb cuts the disks' data space into slices at the sizes of the different disks; every slice on two or more disks becomes an array (RAID 1 for two members, RAID 5 for three or more; Hybrid 2-disk uses RAID 6 for four or more). LVM joins the arrays into one volume. Example: 4, 4 and 8 TB with Hybrid 1-disk gives 8 TB usable; the extra 4 TB on the large disk has no partner and stays unused. Hybrid 1-disk survives any one disk; Hybrid 2-disk any two where a slice has four or more members. It is built with mdadm and LVM, so Debian manages it. DSM may not adopt it, and DSM-made volumes import read-only. Nothing here has run on a real Synology.

#### web:replace-disk: Replacing a failed disk

A failed member leaves the array degraded: it works, but one more failure may lose data (RAID 5). 1. Check the disk on the Dashboard or RAID page. 2. Choose the new disk; it is erased and must be at least as large. 3. The tool marks the old member failed, removes it and adds the new one. 4. The rebuild runs for hours; progress and speed are shown, and other operations wait. Do not power off. To upgrade a healthy member, the tool adds a spare and uses a replace operation, so redundancy is kept during the swap. After replacing every member with a larger disk you can grow the array. Each step shows its exact commands and asks you to type REPLACE and the disk name.

#### web:scrubbing: Scrubbing

A scrub reads every block and compares copies or parity to find silent errors. Check only counts mismatches; Repair also rewrites them and needs a typed confirmation. Debian's mdcheck timer runs a scrub monthly; you can start one or stop it (idle) from the RAID page. The array stays usable but slower while it runs, and speed limits can be tuned. A non-zero mismatch count on a healthy array deserves attention: check SMART on every member. A scrub cannot start while the array is resyncing or reshaping.

**Shortcuts tab (web)**: `?` Help; `/` Search; `Esc` Close; `Tab / Shift+Tab` Move focus.

## 8. About page (desktop D17, web W12)

| Element | Content | Widget |
|---|---|---|
| Logo | The "synofurb" logotype: a mark in the style of the Lucide `recycle` icon (three chasing arrows forming a loop) + wordmark (Montserrat). Light and dark, mark-only and wordmark variants; app icon sizes; favicon on the web | SVG |
| Version | Tag with {version · build} | `Tag` |
| Tagline | "Reinstall Synology NAS units with Debian." | `Label` |
| Copy diagnostics | Copies: version, OS, window/theme settings, compatibility list version, last log file path, (web: host name, service state). Never secrets. Toast "Copied" | `Button` primary + `Clipboard` + `Notification` |
| Links | Documentation, Source code, Report issue | `Link` |
| Credits | Accordion, collapsed. Each row: name, role, licence, link: **Debian** (the installed system, DFSG licences, trixie); **Lucide** (icons, ISC); **Inter** (interface font, OFL); **JetBrains Mono** (code font, OFL); **Montserrat** (wordmark only, OFL); **GPUI Kit** and **gpui-component** (UI framework, Apache-2.0); **synodeb** note | `Accordion`, `List`, `Link` |
| synodeb note | Popover on the row: "The boot-flash technique is documented by the synodeb project (GPL-2.0). synofurb contains none of its code and is MIT licensed." | `Popover` |
| Licence texts | Each row opens the bundled licence text (OFL, ISC, Apache-2.0, MIT) | `Dialog` + `TextView` |

## 9. Word counts per screen (static prose, own + shell)

| Screen | Own | Shell | Total | Budget |
|---|---|---|---|---|
| D1 Inventory | 24 | 11 | 35 | ok |
| D1b Welcome guide (worst steps: 3, 4, 5) | 23 | 11 | 34 | ok |
| D2 Connect | 23 | 12 | 35 | ok |
| D3 Inspect | 24 | 12 | 36 | ok |
| D4 Goal | 32 | 12 | 44 | ok |
| D5 Plan | 31 | 12 | 43 | ok |
| D6 System | 32 | 12 | 44 | ok |
| D7 Confirm | 42 | 12 | 54 | ok |
| D8 Install | 11 | 12 | 23 | ok |
| D9 Install stopped | 29 | 12 | 41 | ok |
| D10 Done | 23 | 12 | 35 | ok |
| D11 Update | 10 | 12 | 22 | ok |
| D12 Settings | 28 | 1 | 29 | ok |
| D13 Boot manager | 31 | 12 | 43 | ok |
| D14 Recovery | 23 | 6 | 29 | ok |
| D15 Manage: Overview and Health | 27 | 6 | 33 | ok |
| D16 Help centre | 33 | 12 | 45 | ok |
| D17 About | 15 | 1 | 16 | ok |
| D18 Manage: Storage / RAID | 10 | 6 | 16 | ok |
| W1 Login | 9 | 1 | 10 | ok |
| W2 Dashboard | 19 | 14 | 33 | ok |
| W3 Storage | 13 | 14 | 27 | ok |
| W4 Create volume | 14 | 14 | 28 | ok |
| W5 Mounts and imports | 13 | 14 | 27 | ok |
| W6 Network | 10 | 14 | 24 | ok |
| W7 Users | 18 | 14 | 32 | ok |
| W8 Boot | 24 | 14 | 38 | ok |
| W9 System | 17 | 14 | 31 | ok |
| W10 Audit log | 15 | 14 | 29 | ok |
| W11 Help centre | 23 | 14 | 37 | ok |
| W12 About | 9 | 14 | 23 | ok |
| W13 RAID | 24 | 14 | 38 | ok |
| W14 Create array | 21 | 14 | 35 | ok |
| W15 Create hybrid | 27 | 14 | 41 | ok |
| W16 Replace disk | 20 | 14 | 34 | ok |
| W17 RAID operation | 5 | 14 | 19 | ok |

Highest desktop screen: 54 words. Highest web screen: 41 words. Dialog and Help bodies are opened on purpose and are bounded separately (dialogs at most 90 words, topics at most 150).

## 10. Review checklist for the real-app screenshots

Proposed `synofurb gui --demo --screen <name>` names: `inventory`, `inventory-offline`, `connect`, `connect-hostkey`, `inspect`, `goal`, `plan`, `plan-keep`, `system`, `system-offline`, `confirm`, `confirm-keep`, `install`, `install-after-ponr`, `failed-pre`, `failed-post`, `failed-keep`, `done`, `done-arm`, `update`, `settings`, `boot-manager`, `recovery`, `manage` (= Overview with health), `manage-health-ok`, `manage-health-faildisk`, `manage-health-rebuild`, `manage-health-services`, `manage-health-unreachable`, `manage-update`, `manage-updating`, `manage-update-failed`, `manage-storage`, `boot-manager-confirm`, `boot-manager-done`, `install-stopped`, `help`, `help-topic`, `about`, web `raid`, `raid-create`, `raid-shr`, `raid-replace`, `raid-operation`, `raid-degraded`, `raid-foreign`, `raid-empty`, and the dialogs `dlg-recovery`, `dlg-keep`, `dlg-next-pre`, `dlg-next-wipe`, `dlg-next-keep`. For each: light and dark at 960x640; focus ring visible; no horizontal scroll; word count at or under the table above; every element is the widget named here (hand-made imitations are defects); tooltips and popovers captured open for at least one screen per pattern; fonts Inter / JetBrains Mono visible; scrollbars themed.

Placeholders to confirm with engineering (shown as sample values in this spec): the web lockout numbers (5 failures in 10 minutes, 5 minute pause), the SSH confirm command name `synofurb-net confirm`, `synofurb-dom`/`sf-boot` command names as in `docs/BOOT.md` and `docs/RECOVERY.md`, and the `--screen` names above.

## 11. Revision after the prototype review (design/reviews/v2/ux-expert-review.md)

Folded into the spec from the UI expert's prototype (these are now requirements): **persistent unit header** (section 1.1; identity lines removed from D2 to D15); **Plan Now | After** two-panel layout with a text key (D5); **Manage Overview / Health** with the worst-component Health Tag, reason chips, six cards, and the **management-app update** (version on unit vs in this app, four phases, auto-rollback, Rolled back / Updated states, D15 and the Update management app dialog); **Operation plan dialog** for Replace with a typed phrase; **Boot flash / Steps / Log** accordions on Install; **GroupBox rules** (Destroyed, Changed, Preserved) with one row per group; **Setup merge** and **Install with defaults** as open decision O1.

Corrected after the review: fact-derived risk Tag everywhere (B2); Boot manager confirm, progress and result (B1); persistent Rescue panel and the "Export first." instruction (B3); New disk Select in every disk-adding operation (B4); counting rule for prose inside data (B5); Confirm one-line recovery text and identity on the checkbox (M1, M2); Hybrid suggestion on Auto with mixed disks and a text key for diagram colours (M4); stepper out of the Tab order, focus to title, `F1` contextual (M5); GPUI tooltip and disabled-button rules (M6); text size must scale the whole window (M7); Health to action (Restart a down service, Replace on degraded arrays, Update demoted unless Healthy) (M8); Recovery rows with confirmation and a safe primary (M9); stopped-install outcome (M10); Boot manager alert copy (M11); one-row groups so all three lists fit at 960x640 (M12); `Alert::banner()` exists; TTF/OTF fonts.

Open decisions: **O1** (closed in round 3: merged into *Setup*); **O2** which RAID operations are native on desktop in v1 (minimum: Replace disk, Scrub; the rest say "In web UI"); **O3** block or warn on management-app update while an array is degraded (spec: warn); **O4** whether the NAS reports the rescue-shell deadline so the app can re-attach after a restart.

## 12. Round 2 and 3 folds (design/reviews/v2/ux-expert-review-round2.md and round3.md)

Adopted from the final prototype and now requirements: (1) **twin phrase suffix**: with a same-model unit on the network (the demo inventory always contains one, DS918+ at .40 and .58) the unit header carries a `Twin` Tag, the gate shows a `Twin {IP · SN last 4}` Alert, the checkbox reads "This is {IP · …last 4}", and the typed phrase is `ERASE <MODEL> <last 4 of serial>` (here `ERASE DS918+ 3456`); the hint "Add serial last 4." shows until typed and a wrong suffix shows No match; without a twin the phrase is `ERASE <MODEL>`; (2) **Recovery rows**: label, one line, one action, info icon; Boot DSM and Open agent confirm in an AlertDialog, Restore flash needs `RESTORE FLASH`, Remove entries needs `REMOVE ENTRIES` and is disabled with a reason when Debian is the default entry, Reinstall DSM is *Open guide*; Recovery is also a tab of Manage; (3) **D9 variants**: flash backup did not verify (nothing changed, *Back to Confirm*), agent did not answer (30:00 watchdog, *Wait for DSM*), stopped by you (60 s), failed before step 3, failed after step 3 running (15:00, Rescue panel), waiting (clock 0, "The shell closed. The NAS is rebooting."), loader (only *Reinstall DSM*), lost (*Reconnect*, the clock keeps running on the NAS), Keep variant; a control whose resource is gone disappears or is disabled with a reason, and an open Cancel dialog re-evaluates when step 3 starts ("Too late to cancel", *Stop install* disabled); (4) **disabled controls are real disabled Buttons and not Tab stops; the reason is a sibling info Button** ("Why locked", "Why unavailable") that opens a Popover (this replaces the earlier styled-disabled-but-focusable rule); (5) **six-step wizard**: Find, Connect (Inspect opens automatically after the host-key Trust), Goal, Setup (TabBar Storage | System), Confirm, Install; Boot manager and Recovery show **no Stepper**; Goal's Install card has *Quick setup* (summary Dialog: Cancel, Customize, Continue to Confirm; for mixed disk sizes it defaults to Hybrid 1-disk and says what RAID 6 would give) and *Customize*; Confirm always shows an After line (RAID level, usable, system size, hostname); the hostname follows the unit; (6) **Manage**: Health card, management-app card (Update primary only when Healthy; updating while an array is degraded or rebuilding is allowed with "The rebuild keeps running."), read-only Storage with intent-named *Open web UI* hand-offs; (7) **Boot manager**: confirm AlertDialog, progress (Boot flash backup, Agent added, Reboot into the agent, Agent answers), result with *Return to DSM*, "Back in DSM. Our entries remain on the flash.", and the Tag "Boot agent installed" on the unit in Find; the page Tag reads "Will change flash"; (8) **Colour key** button and a Hybrid suggestion beside Data RAID that states capacity and fault tolerance of both options; (9) the footer button is labelled **Command** (terminal icon) on Setup and Confirm; (10) the one-line recovery text on Confirm is "Free to cancel until step 3; then no undo." (the dialog says DSM may be gone); (11) text size is `rem` based, px-sized widgets must be verified in the real app.

**Round 4 fold (NB4-1, N4-1).** After the 10-minute agent wait the engine resets nothing, so D9's agent-did-not-answer variant no longer claims DSM is the default or that the unit "boots again by itself". It shows *Default: not confirmed* and the status-bar Tag *Boot unconfirmed*, states both outcomes (agent booted but unreachable: watchdog reboots to DSM 30 minutes after the agent started; agent never ran: default still the agent, power cycles loop, no watchdog) and names the manual recovery (choose SYNOLOGY_2 in the boot menu on a serial or VGA console, or restore the boot flash from another computer). The same wording is in the Way back popover, a new "What happens next (agent did not answer)" dialog, and the Help topics recovery, boot-manager, troubleshooting, web boot and web recovery. The clock is "about 20 minutes or more" (20 plus the 1 to 3 minutes the agent needed to boot, typically 21 to 23), not "up to 20:00". Phase 4 of 7 (Booting) for this variant; flash-backup failure stays Phase 3.
