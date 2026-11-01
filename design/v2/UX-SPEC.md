# synofurb UX spec v2

Binding input: `design/BRIEF-v2.md` (the first mockups were rejected: too much text, hand-made widgets). This is a **spec, not mockups**. It defines, for the desktop app (GPUI, `gpui-component` 0.8.1 via `gpui-kit`) and the NAS web UI, every screen's exact visible elements, their word budget, where each removed explanation now lives, and which widget realises it. Facts follow `docs/RECOVERY.md`, `docs/BOOT.md`, `docs/COMPAT.md` and `README.md`. The real application is later reviewed against this file.

Sample values (DS918+, 192.168.1.40, SN 1930PDN123456, `sdc` with 8 reallocated sectors) are only sample data. **No model is Verified today**; the primary path is **Untested**.

## 0. Rules that apply to every screen

1. **Budget.** At most 60 words of static prose per screen, shell included (titles, subtitles, labels, options, column headers, buttons, prose; data in braces is not counted). Title at most 4 words; subtitle at most 12; label at most 4; button a verb plus at most 2 words.
2. **Where explanations go.** Tooltip (one sentence, hover and keyboard focus) for a label or a disabled control; Popover (short paragraph + action) from an info icon; Dialog for decisions and anything long; Help topic for reference. Progress and logs are collapsed Accordions except the current step. Every info icon links to a Help topic id.
3. **The safety gate stays visible but terse**: identity, destroyed/changed/preserved as names and sizes, three confirmations, typed phrase.
4. **Copy module.** All strings live in one module per surface: `Copy { id, short, tip, help: Option<TopicId> }`. `short` is what is drawn, `tip` is the Tooltip (one sentence, at most 24 words), `help` is a topic id (bodies at most 150 words, section 7) or a Dialog id (section 4). Tests: per-screen word budget (sum of `short` of the screen plus shell), title/subtitle/label/button limits, `tip` single sentence, every `help` id exists, every topic at most 150 words, no string outside the module.
5. **Fonts**: Inter for the interface (Montserrat for the wordmark only), JetBrains Mono for code, logs, hashes, commands; bundled with their OFL licences. **Icons**: Lucide only. **Scrollbars**: themed (thin; theme tokens `scrollbar`, `scrollbar_thumb`, `scrollbar_thumb_hover`; light and dark).
6. **Accessibility baseline (stated in the UI)**: keyboard, contrast and scalable text are guaranteed; **screen readers are not guaranteed in the native app**. The documented CLI (`synofurb install ...`) is the equivalent path and is named in the first-launch Dialog, Help, and About. The web UI is semantic HTML and is screen-reader friendly. Status is never colour alone (Tag has icon + text).
7. **Layout**: 960x640 minimum; no horizontal scrolling; the footer with the primary action is pinned outside the scroll area; visible focus on every control.

## 1. App shells

### 1.1 Desktop shell (wizard screens D1 to D11, D13 to D15)

| # | Element | Visible text | Widget | Words |
|---|---|---|---|---|
| 1 | TitleBar: logo mark + wordmark | synofurb | `TitleBar` + SVG logo (custom asset) | 1 |
| 2 | TitleBar actions | (icons only: circle-question-mark = Help, settings = Settings, menu = About / Check for updates) | `Button` icon + `Tooltip`, `DropdownMenu` | 0 |
| 3 | Stepper | Find, Connect, Inspect, Goal, Plan, System, Confirm, Install, Done | `Stepper` + `StepperItem` (horizontal, click only completed steps) | 9 |
| 4 | StatusBar left | Risk Tag, at most 2 words: Read-only, Plan only, Flash changed, Disks changed, Failed safely, Debian running | `StatusBar` + `Tag` (icon + text) | 2 |
| 5 | StatusBar center | {DS918+ · 192.168.1.40} | `StatusBar`, `Label` (mono data) | 0 |
| 6 | StatusBar right | Way back | `Button` (link) opening a `Popover` with the one-line fallback for the current state | 2 |

**Shell words: 14.** Risk Tag states (derived from facts, never from step number): *Read-only* (before Confirm, before any write); *Plan only* (plan prepared, not applied); *Flash changed* (confirmed; boot flash written, no disk yet); *Disks changed* (step 3 reached); *Failed safely* (failed with disks untouched; the boot flash may still hold our entries); *Debian running*. The Way back Popover (one sentence each): Read-only/Plan only "Nothing to undo."; Flash changed "Cancel or power-cycle: DSM boots."; Disks changed "Rescue shell 15 min, then the DSM loader; no data promise."; Failed safely "DSM boots again by itself."; Debian running "Recovery stays under Manage."

Manage (D15) hides the Stepper (data-driven); Help (D16) is a Sheet over the current screen; About (D17) and Settings (D12) are pages reached from the TitleBar and replace the content area (Stepper hidden).

### 1.2 Web shell (all pages except Login)

| Element | Visible text | Widget (web) | Words |
|---|---|---|---|
| Brand | synofurb (+ {host}) | `header` + inline SVG logo | 1 |
| Navigation | Dashboard, Storage, Mounts, Network, Users, Boot, System, Audit, Help, About | `nav` (sidebar; `aria-current=page`), Lucide icons | 10 |
| Status strip | Tag "Needs attention" (worst health) + {RAID sync 24 % · up 2 h 14 min} | `role=status` strip, Tag | 2 |
| Rollback banner | only while a network change is pending (see W6) | Alert, `role=alert` | conditional |

**Shell words: 13.** Phones: nav collapses to a bottom tab bar (Home, Storage, Network, Users, More) with 44 px targets; the strip and rollback banner stay. Web "widgets" are named after the gpui-component equivalents so both surfaces share vocabulary: Tooltip = `role=tooltip` on hover and focus; Popover = `popover` attribute; Dialog = `<dialog>`; Sheet = `<dialog>` docked right; Accordion = `<details>`; Stepper = `<ol aria-current=step>`; Tag = `span.tag` with icon; Alert = `role=alert|status`; Toast = `role=status` live region.

## 2. Widget catalogue (gpui-component 0.8.1, read from `src/`)

**Exists and is used** (module: use): `sidebar` (Sidebar, SidebarMenu: not used on desktop, wizard navigation is the Stepper; Settings uses it), `stepper` (Stepper, StepperItem: wizard + install phases popover), `tab` (TabBar, Tab: Manage, Help), `table` (DataTable: inventory, disks), `tag` (Tag with Primary/Secondary/Danger/Success/Warning/Info variants: all statuses), `badge` (Badge: unread/update dot on the Settings icon), `alert` (Alert Default/Info/Success/Warning/Error: all inline problems), `tooltip` (Tooltip: every `tip`), `popover` (Popover: info icons, Way back), `hover_card` (HoverCard: names and sizes behind summary tags), `dialog` (Dialog, AlertDialog: confirmations, long text, host key), `sheet` (Sheet: Help centre), `accordion`/`collapsible` (Accordion, AccordionItem, Collapsible: steps, log, credits), `breadcrumb` (Breadcrumb: Manage sub-pages), `menu` (DropdownMenu, PopupMenu, ContextMenu: TitleBar menu, row actions), `kbd` (Kbd: shortcut list, tooltips), `button` (Button, ButtonGroup, DropdownButton, Toggle, ToggleGroup: segmented choices), `input` (Input incl. `mask_toggle` and search, NumberInput, OtpInput unused), `checkbox`, `radio`, `switch`, `slider`, `select`/`combobox`, `progress` (Progress, ProgressCircle), `spinner`, `skeleton`, `empty` (Empty: all empty states), `description_list` (DescriptionList: facts, logins), `group_box` (GroupBox: Goal cards, Destroyed/Changed/Preserved), `form` (Form, Field: all forms), `label`, `link`, `clipboard` (Clipboard: copy buttons), `notification` (toasts), `status_bar` (StatusBar), `title_bar` (TitleBar), `setting` (Settings, SettingPage, SettingGroup, SettingItem: Settings screen), `list` + `virtual_list` (List, VirtualList: topics, recovery rows, log lines), `text` (TextView::markdown: Help bodies, licence texts), `command` (Command palette: not used, search is an Input), `scroll` (Scrollbar via `.overflow_y_scrollbar()`; themed through the three scrollbar tokens), `separator`, `resizable` (not used), `tree`, `pagination`, `avatar`, `plot`/`chart` (not used).

**Icons**: `gpui-kit-assets` ships the full Lucide catalogue (1,830 SVGs) but embeds only 101 by default; the app registers its own `AssetSource` for the extra icons it uses (terminal, circle-question-mark, info, copy, triangle-alert, circle-check, circle-x, recycle, settings, search, book-open, download, wifi-off, hard-drive, life-buoy, external-link, refresh-cw, folder-open, key-round, clock, lock, server, shield, power) and does not embed `AllAssets`.

**Missing widgets and exceptions (each must stay listed in code with a comment):**

| Need | Why no widget | Exception |
|---|---|---|
| Disk / RAID / LVM layout diagram | No chart widget draws partition stacks | One custom element built from `div` segments inside a `GroupBox`; its legend is a `Tooltip` |
| Countdown (60 s, 15 min, 90 s) | No timer widget | `Label` (JetBrains Mono) driven by an app-side tick; announced as a timer |
| Logo / wordmark | Brand asset | SVG via `img`/`Icon` custom path; Montserrat wordmark baked into the SVG outlines |
| Log viewer | No log widget | `VirtualList` of `Label` rows (mono) + `Input` search + `ToggleGroup` levels |
| Timeline (Manage, History) | No timeline widget | `List` rows with a Lucide icon and a time `Label` |
| Card | No Card widget | `GroupBox` (nearest) |
| Banner (top of window) | No banner widget | `Alert` placed in a full-width container |
| Typed-phrase match indicator | None needed | `Input` + inline `Tag` |
| Dialog/Sheet/Notification host | Needs `Root` | The window root must be `gpui_component::Root` |

## 3. Flows (minimal UI, all new features included)

**F1 First install (Untested path, boot-flash unit).** D1 first launch Dialog (once) -> D1 select unit -> D2 Connect (host-key Dialog on first contact) -> D3 Inspect (Tag *Untested*) -> D4 Goal (Boot manager marked *Recommended*; Install Debian one click away) -> D5 Plan (Wipe; Keep only behind *Advanced*) -> D6 System (**Kernel** Stable | Newest; **Source** Online | Offline; Boot via *Boot flash*) -> D7 Confirm -> D8 Install (steps 1 to 12; **No return** at step 3) -> D10 Done (copy the password to enable Finish) -> D15 Manage.

**F2 Online / Offline.** D6 Source toggle. *Online*: the NAS downloads packages (needs Internet; said in the *About 25 min* popover). *Offline*: Select a bundle (architecture-matched); if none, an inline error "No bundle" with Button *Download* that opens Settings > Data; the app needs Internet only to download the bundle. Offline also disables update check, list refresh and report-back (Tooltips explain). StatusBar shows *Offline* when the app itself is offline.

**F3 Kernel channel.** D6 Kernel toggle: *Stable* (Debian 6.12 LTS, default) | *Newest* (7.x from trixie-backports, falls back to Stable). One Popover; the choice is part of the Copy command output.

**F4 CLI equivalent.** *Copy command* (terminal icon, tooltip only) on D5, D6 and D7 copies `synofurb install --host <ip> --user <admin> ...` for the current choices. On D7 the three confirmation flags `--have-backup --accept-data-loss --confirm` are included **only after** the three confirmations and the phrase are satisfied; before that the copied command is the `plan` form (read-only). Shortcut `Ctrl+Shift+C`.

**F5 Failure.** Before step 3: D9 with 60 s countdown ("DSM returns in"), *Disks untouched*, primary *Wait for DSM*, no rescue shell. After step 3: D9 with 15 min countdown ("Rescue shell closes in"), *Disks changed*, primary *Open rescue shell*, secondary *Export diagnostics* (first in the Dialog text, because the NAS is unreachable after the clock), Wipe and Keep variants differ only in the *What happens next* Dialog. Recovery options -> D14.

**F6 Boot manager.** D4 -> D13 -> D8-style progress (Boot flash accordion only) -> D15 Manage; Boot page (web W8) for next boot. Power-cycling is always the way back from another system.

**F7 Update.** D11 banner and Dialog; never during an install.

**F8 Web day-2.** W1 Login -> W2 Dashboard (Needs attention first) -> W3/W4 storage -> W6 Network (safe apply) -> W8 Boot.

## 4. Dialog texts (opened on purpose; each at most 90 words, 2 to 4 short paragraphs)

| Dialog | Opened from | Buttons | Words | Text |
|---|---|---|---|---|
| How recovery works | D7 Confirm | Close | 71 | DSM stays the default boot entry. Before installer step 3 (the point of no return), a failure or cancel reboots the NAS into DSM within 60 seconds. From step 3, a failure leaves a rescue shell open for 15 minutes, then the NAS reboots into the DSM loader, which shows Synology's install page. DSM's own system may be gone by then; reinstall it from Synology. We promise nothing about your data. |
| Keep existing volumes | D5 Plan (Advanced) | Use Keep / Cancel | 51 | Keep mode reuses only DSM's system and swap area, about 10 GiB per disk; the system is capped near 9.5 GiB. DSM, its settings, users and packages are erased. Your data volume is left alone on a best-effort basis, with no guarantee. It is not mounted automatically; import it read-only later. |
| What happens next (before step 3) | D9 Install failed | Close | 45 | No disk was changed. The NAS sets DSM as the default and reboots in 60 seconds, then DSM starts as before. If DSM is not back after 10 minutes, power-cycle the NAS. Our entries stay on the boot flash until you remove them under Recovery. |
| What happens next (after step 3, Wipe) | D9 Install failed | Close | 51 | The first disk was changed and DSM's own system may be gone. Export diagnostics first, then open the rescue shell. At zero DSM becomes the default and the NAS reboots into the DSM loader, which shows Synology's install page. Reinstall DSM from Synology. A Wipe install leaves no data to keep. |
| What happens next (after step 3, Keep) | D9 Install failed | Close | 63 | The first disk was changed. Export diagnostics, then open the rescue shell. At zero the NAS reboots into the DSM loader and Synology's install page. Your data volume was not part of any write command and should be intact unless the failure was a hardware problem. Synology documents a reinstall that can keep data volumes; check its current documentation. We cannot guarantee it. |
| Trust this NAS? | D2 Connect | Trust / Cancel | 36 | First connection to {192.168.1.40}. Compare this fingerprint with DSM (Control Panel, Terminal and SNMP). If you did not expect a new key, cancel. A changed key shows the same dialog in danger style and offers only Cancel. |
| Reinstall DSM | D14 Recovery | Open find.synology.com / Close | 46 | The NAS boots Synology's loader, which shows an install page. Open find.synology.com or Synology Assistant, find the unit and install DSM with the .pat file for your model. Synology documents a reinstall that can keep data volumes; check its current documentation. This project cannot guarantee it. |
| Restore boot flash? | D14 Recovery | Restore / Cancel | 29 | This writes the saved image {dom-backups/....img} back to the boot flash. It needs a working Linux on the unit, such as the agent or Debian. Type RESTORE FLASH to continue. |
| Report a result | D10 Done | Open issue / Cancel | 32 | Opens a pre-filled GitHub issue with: model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click. |
| Before you start (first launch) | D1 Inventory | Got it / Open Help | 31 | synofurb only reads until you confirm. Make a tested backup first. Screen readers are not supported in this window; the same steps work from the command line (see Help, Command line). |
| Apply with rollback? (web) | W6 Network | Apply / Cancel | 36 | After applying, open {https://192.168.1.45:8443}. The browser warns again; the certificate is the same: {SHA-256 5E:91:C4:0B:7A:D2:38:F6 ... 2D:83:3C:A1}. Passkeys do not work on an IP address. You have 90 seconds; you can also confirm over SSH {ssh andreas@192.168.1.45 sudo synofurb-net confirm}. Do nothing and it rolls back. |
| Enable write? (web) | W5 Mounts | Enable write / Cancel | 24 | DSM btrfs volumes can use features Debian handles poorly. Read-only is the safe default. Write only if you have a copy of the files. |

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

**Word budget:** screen 24 + shell 14 = **38** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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
| First launch | Dialog (once) | Before you start | Read-only until you confirm. | Three lines: read-only until Confirm; make a backup; screen readers are not supported in this window, use the CLI (help: cli). Button Got it, link Open Help. |

**Primary:** Connect (selected row). **Secondary:** Rescan, Add by IP, Manage.

**Keyboard:** `Ctrl+F` filter; `Up`/`Down` row; `Enter` Connect; `R` rescan; `A` add; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Empty: "No NAS found" + buttons Scan again, Add by IP, link Help; *error*: A failing discovery method is a warning Tag in the Scan details popover; no page error; *loading*: Skeleton rows + Spinner on Rescan; rows stream in; *offline*: StatusBar Tag "Offline"; compatibility column uses the cached list.

### D2 Connect

**Purpose:** Sign in to the NAS over SSH, read-only.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Connect | Label (title) | 1 |
| 2 | Sign in over SSH. Nothing changes. | Label (subtitle) | 6 |
| 3 | {DS918+ · 192.168.1.40 · SN 1930PDN123456} | Label + Tag (compat) identity line | data |
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

**Word budget:** screen 23 + shell 14 = **37** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 24 + shell 14 = **38** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 32 + shell 14 = **46** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Boot manager (i) | Popover | Boot manager | Adds a small SSH agent to the boot flash. | Flash changes, disks untouched, one-shot, 30 min watchdog; help: boot-manager |
| Install Debian (i) | Popover | Install Debian | Erases disks; needs your backup. | Wipe or advanced Keep, about 25 minutes; help: first-install |
| Restore (i) | Popover | Restore / Recover | Reads the unit's state before offering anything. | help: recovery |
| Recommended tag | Tooltip | Recommended | No one has reported this model; try the agent first. | help: compat-status |

**Primary:** Boot manager: Set up (Untested); otherwise Plan install. **Secondary:** the other two cards.

**Keyboard:** `Tab` between card buttons; `Enter` activates; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Plan install disabled for Unsupported units; Tooltip "Debian 13 cannot run on this hardware"; *loading*: n/a.

### D5 Plan

**Purpose:** Shape the storage layout and see the consequence.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Storage plan | Label (title) | 2 |
| 2 | Nothing is written yet. | Label (subtitle) | 4 |
| 3 | Mode | Field + ToggleGroup | 1 |
| 4 | Wipe all | ToggleGroup option (only one visible) | 2 |
| 5 | Advanced | Link (opens Dialog) | 1 |
| 6 | System size | Field + Slider + value label | 2 |
| 7 | Mirror copies | Field + ToggleGroup 2 / 3 / 4 | 2 |
| 8 | Data RAID | Field + Select | 2 |
| 9 | Free space | Field + Slider | 2 |
| 10 | {disk / RAID / LVM layout} | Custom diagram (exception, section 2), legend as Tooltip | data |
| 11 | Destroyed | Tag (danger) + data size | 1 |
| 12 | Changed | Tag (warning) + data | 1 |
| 13 | Preserved | Tag (secondary) + data | 1 |
| 14 | Disk warning | Alert (warning) + data line | 2 |
| 15 | Next: System | Button (primary) | 2 |
| 16 | Back | Button (ghost) | 1 |
| 17 | Copy command | Button (icon: terminal, tooltip only) | cond. |

**Word budget:** screen 26 + shell 14 = **40** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Mode (i) | Tooltip | Mode | Wipe erases all four disks. | help: backups |
| Advanced | Dialog | Keep existing volumes | Best effort; no guarantee. | Dialog (3 short paragraphs): caps the system near 9.5 GiB; what is gone (DSM, settings, users, packages); volumes are not mounted automatically and import read-only. Buttons Use Keep / Cancel |
| System size (i) | Tooltip | System size | Mirrored on every copy; 32 GiB is recommended. | help: first-install |
| Mirror copies (i) | Tooltip | Mirror copies | More copies survive more disk failures. | help: recovery |
| Data RAID (i) | Tooltip | Data RAID | Auto picks the level from the disk count. | help: first-install |
| Free space (i) | Tooltip | Free space | Left unallocated for growth and snapshots. | - |
| Summary tags | HoverCard | Destroyed / Changed / Preserved | Hover for names and sizes. | Names and sizes of disks, volumes and the boot flash |
| Disk warning | Tooltip | Disk warning | A failing disk during the first sync is risky. | help: troubleshooting |
| Copy command | Tooltip | Copy command | Copies the equivalent synofurb command. | help: cli |
| Blocking error | Alert (error) | Size does not fit | - | Inline fix Button "Set to 9 GiB" (secondary); Next stays disabled with Tooltip "Fix the error first" |

**Primary:** Next: System. **Secondary:** Back, Copy command.

**Keyboard:** `Left`/`Right` adjust a focused Slider; `Tab` through controls; `Ctrl+Shift+C` Copy command; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Blocking: Alert error + inline fix; non-blocking: Alert warning; Next disabled only on blocking; *loading*: Layout recomputes instantly; no loading state.

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
| 19 | Boot via | Field + Tag (read-only) "Boot flash" or "Installer kit" | 2 |
| 20 | Next: Confirm | Button (primary) | 2 |
| 21 | Back | Button (ghost) | 1 |
| 22 | Bundle | Field + Select + Button "Download" when Offline | cond. |
| 23 | Copy command | Button (icon: terminal) | cond. |

**Word budget:** screen 29 + shell 14 = **43** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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
| Copy command | Tooltip | Copy command | Copies the equivalent synofurb command. | help: cli |

**Primary:** Next: Confirm. **Secondary:** Back, Copy command.

**Keyboard:** `Tab` order = reading order; `Space` toggles Switch/Checkbox; `Enter` Next; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Defaults filled; Password generated on entry; *error*: Field-level: danger border + Tooltip on focus; console auto-detect failed -> Input becomes editable; *loading*: n/a.

### D7 Confirm

**Purpose:** Final gate: identity, consequences, three confirmations, typed phrase.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Confirm erase | Label (title) | 2 |
| 2 | Last step before changes. | Label (subtitle) | 4 |
| 3 | {DS918+ · 192.168.1.40 · SN 1930PDN123456} | Label (mono) + Tag (compat) identity line | data |
| 4 | Destroyed | GroupBox title (danger border) | 1 |
| 5 | {sda sdb sdc sdd · 14.5 TiB · volume1 · 10.9 TiB · DSM} | List (names + sizes only) | data |
| 6 | Changed | GroupBox title (warning border) | 1 |
| 7 | {Boot flash · backed up} | List | data |
| 8 | Preserved | GroupBox title (success border) | 1 |
| 9 | {Nothing on the disks} | List | data |
| 10 | Disk warning | Tag (warning) | 2 |
| 11 | Untested | Tag (info) | 1 |
| 12 | About 25 min | Tag (secondary) | 3 |
| 13 | Backup is tested | Checkbox | 3 |
| 14 | I accept data loss | Checkbox | 4 |
| 15 | This is the unit | Checkbox | 4 |
| 16 | Type phrase | Field + Input (paste allowed); phrase shown as code chip {ERASE DS918+} | 2 |
| 17 | Fallback: DSM stays default. | Label (one line) | 4 |
| 18 | How recovery works | Link (opens Dialog) | 3 |
| 19 | Erase and install | Button (danger); disabled until all four conditions hold | 3 |
| 20 | Back | Button (ghost) | 1 |
| 21 | Copy command | Button (icon: terminal) | cond. |

**Word budget:** screen 39 + shell 14 = **53** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Disk warning tag | Tooltip | Disk warning | {sdc: 8 reallocated sectors}; consider replacing it first. | help: troubleshooting |
| Untested tag | Tooltip | Untested | No one has reported this model; your install is the test. | help: compat-status |
| About 25 min tag | Popover | Before you start | Keep this computer awake and the NAS online. | Checklist: stay awake and on the network; NAS needs Internet unless Source is Offline; unplug USB and eSATA disks; the first RAID sync takes about 9 hours; help: first-install |
| Changed list | HoverCard | Boot flash | Entries and files are added; the whole flash is backed up first. | Backup file name (mono) + help: backups |
| Preserved list | Tooltip | Preserved | Wipe mode preserves nothing on the disks. | help: backups |
| Twin warning | Alert (warning, conditional) | Same model nearby | {Another DS918+ at 192.168.1.58, serial ...557710.} | Shown only if a same-model unit exists |
| Erase disabled | Tooltip | Erase and install | Missing: {the unticked items or the phrase}. | - |
| Phrase match | Tag inline (success/danger) | Matches / No match | - | - |
| How recovery works | Dialog | How recovery works | DSM is the default; failures return to it. | See dialog text, section 4 |
| Copy command | Tooltip | Copy command | Includes the confirmation flags only after you confirm here. | help: cli |

**Primary:** Erase and install (danger). **Secondary:** Back, Copy command.

**Keyboard:** Tab order: Checkboxes (`Space`), phrase Input, Erase button. `Enter` never triggers Erase; `Space` on the focused button does. `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: All controls cleared, Erase disabled; *error*: Phrase mismatch: inline Tag "No match" (no expected-value echo); *loading*: n/a.

### D8 Install

**Purpose:** Show progress; make the point of no return and cancel rules obvious.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Installing | Label (title) | 1 |
| 2 | Keep this window open. | Label (subtitle) | 4 |
| 3 | {Phase 3 of 7} | Progress (value) + Label | data |
| 4 | Staging | Tag (info) current phase; opens Popover with the 7 phases | 1 |
| 5 | Boot flash | AccordionItem (open until finished): flash backup, agent, reboot, installer send | 2 |
| 6 | Steps | AccordionItem (current step open) | 1 |
| 7 | {1-12 engine step names} | List rows with Icon status; step 3 carries a Tag | data |
| 8 | No return | Tag (danger) on step 3 | 2 |
| 9 | Log | AccordionItem (collapsed): Input search, ToggleGroup level, VirtualList | 1 |
| 10 | Cancel | Button (secondary); disabled after step 3 | 1 |
| 11 | Connection lost | Alert (warning, conditional) + Button "Retry" | cond. |

**Word budget:** screen 13 + shell 14 = **27** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Phase Tag | Popover | Phases | Seven phases from connecting to verifying. | List with current phase marked; help: first-install |
| No return Tag | Tooltip | No return | From step 3 the first disk changes; cancel is free before. | help: recovery |
| Cancel disabled | Tooltip | Cancel | Not possible after step 3. | help: recovery |
| Cancel | AlertDialog | Cancel install? | The NAS returns to DSM within 60 seconds. | Buttons Stop install / Keep going |
| Boot flash | Tooltip | Boot flash | Backed up and verified before the first write. | help: backups |
| Open log file (icon) | Tooltip | Open log file | Opens the log in your editor. | - |

**Primary:** none while running (Cancel is secondary). **Secondary:** Cancel, Open log file (icon).

**Keyboard:** `Ctrl+F` search log when open; `Space` toggles Accordion items; `Esc` never cancels; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Failure replaces this screen with D9; connection loss shows the Alert and keeps polling; *loading*: Progress indeterminate during staging until the first byte count.

### D9 Install failed

**Purpose:** Say what happened, whether disks changed, the clock, and the next action.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Install stopped | Label (title) | 2 |
| 2 | {Step 11 of 12 · Configure packages, users, boot} | Label (subtitle, engine text) | data |
| 3 | Disks changed | Tag (danger) or "Disks untouched" Tag (success) | 2 |
| 4 | Flash restored | Tag (warning) or "Flash changed" / "Flash untouched" | 2 |
| 5 | {boot entry failed read-back; original restored} | Alert (error) body, engine text | data |
| 6 | Rescue shell closes in | Label + mono countdown (custom timer, exception) | 4 |
| 7 | Open rescue shell | Button (primary, pinned in the footer) | 3 |
| 8 | Export diagnostics | Button (secondary) | 2 |
| 9 | Recovery options | Button (secondary) | 2 |
| 10 | What happens next | Link (opens Dialog) | 3 |
| 11 | Log excerpt | AccordionItem (collapsed, mono) | 2 |

**Word budget:** screen 22 + shell 14 = **36** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Countdown | Tooltip | Rescue shell | Then DSM becomes the default and the NAS reboots. | help: recovery |
| Disks changed tag | Tooltip | Disks changed | Repartitioned at {time}; DSM's own system may be gone. | help: recovery |
| Flash tag | Tooltip | Boot flash | Entries are on it; the full backup is on this computer. | help: backups |
| What happens next | Dialog | What happens next | Order of actions and what survives. | Dialog text per variant, section 4 (after PONR Wipe, after PONR Keep, before PONR) |
| Export diagnostics | Tooltip | Export diagnostics | Do this before the clock ends; secrets are removed. | help: troubleshooting |

**Primary:** Open rescue shell (after PONR) / Wait for DSM (before PONR). **Secondary:** Export diagnostics, Recovery options.

**Keyboard:** Footer button pinned and focusable first; `Tab` order: primary, secondary, links; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Variants: before PONR (60 s clock, no rescue shell), after PONR Wipe, after PONR Keep (volume untouched unless hardware failed); *loading*: Waiting for DSM shows Spinner and elapsed time.

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

**Word budget:** screen 23 + shell 14 = **37** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 10 + shell 14 = **24** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 28 + shell 14 = **42** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Purpose:** Install the one-shot SSH agent on the boot flash.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Boot manager | Label (title) | 2 |
| 2 | SSH agent on the boot flash. | Label (subtitle) | 6 |
| 3 | Flash changes | Tag (warning) | 2 |
| 4 | Disks untouched | Tag (success) | 2 |
| 5 | SSH key | Field + ToggleGroup | 2 |
| 6 | Generate | ToggleGroup option | 1 |
| 7 | Paste | ToggleGroup option | 1 |
| 8 | Login timeout | Field + Input (read-only, 30 min) | 2 |
| 9 | Other OS: power cycle returns. | Alert (warning) | 5 |
| 10 | Install agent | Button (primary) | 2 |
| 11 | Back | Button (ghost) | 1 |

**Word budget:** screen 26 + shell 14 = **40** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Flash changes | Popover | Flash changes | Entries and files are added; DSM's entry stays. | Whole flash saved to this computer first and verified; help: boot-manager |
| Disks untouched | Tooltip | Disks untouched | No partition table is written. | help: boot-manager |
| Login timeout | Tooltip | Login timeout | The agent reboots to DSM after 30 minutes without a login. | help: boot-manager |
| One-shot | Popover from the title (i) | One-shot agent | It resets the default to DSM when it starts. | kexec starts Linux kernels only; help: boot-manager |
| Alert | Tooltip | Other OS | After kexec nothing of ours runs; power-cycling returns to the default. | help: boot-manager |

**Primary:** Install agent. **Secondary:** Back.

**Keyboard:** `Enter` Install agent; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Flash not editable: Alert error, button disabled with reason Tooltip; *loading*: Install shows the D8 pattern: Boot flash accordion only.

### D14 Recovery

**Purpose:** Read the unit's real state and offer only what is possible.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Recovery | Label (title) | 1 |
| 2 | State read from the unit. | Label (subtitle) | 5 |
| 3 | {Flash: ours present · Default: DSM · Disks: modified 14:25} | Tags (state facts) | data |
| 4 | Boot DSM next | List row label | 3 |
| 5 | Open agent | List row label | 2 |
| 6 | Restore flash | List row label | 2 |
| 7 | Remove our entries | List row label | 3 |
| 8 | Reinstall DSM | List row label | 2 |
| 9 | Run | Button per row (disabled rows have a reason Tooltip) | 1 |
| 10 | Saved here | AccordionItem (collapsed): file paths + Button "Open folder" | 2 |
| 11 | Open folder | Button | 2 |

**Word budget:** screen 23 + shell 14 = **37** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Each row (i) | Tooltip | Row name | One sentence per action, from RECOVERY.md. | help: recovery |
| Disabled row | Tooltip | Unavailable | The reason is stated, for example that our entries are not on the flash. | - |
| Restore flash | AlertDialog (typed phrase) | Restore boot flash? | Needs a working Linux on the unit. | Type RESTORE FLASH; backup file name shown |
| Reinstall DSM | Dialog | Reinstall DSM | Use Synology's page; no data promise. | Opens find.synology.com; text per section 4 |
| Saved here | Tooltip | Disk metadata backup | First and last 1 MiB of changed disks; not your files. | help: backups |

**Primary:** Run (first enabled row). **Secondary:** other rows.

**Keyboard:** `Up`/`Down` row; `Enter` Run; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: Unit unreachable: Alert "Cannot reach unit" with the manual steps link; *error*: Action failed: Notification error; state re-read; *loading*: Skeleton tags while the state is read.

### D15 Manage

**Purpose:** Unit page for converted units.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | {DS918+} | Label (title, data) | data |
| 2 | Running Debian 13. | Label (subtitle) | 3 |
| 3 | Overview, Logins, Recovery, History | TabBar + Tab (4) | 4 |
| 4 | Address | DescriptionList label | 1 |
| 5 | Default boot | DescriptionList label | 2 |
| 6 | Web UI | DescriptionList label | 2 |
| 7 | Open web UI | Button (primary) | 3 |
| 8 | Diagnostics | Button (secondary, copies diagnostics) | 1 |
| 9 | SSH command | Logins tab: Field + mono + Clipboard | 2 |
| 10 | Password not kept | Tag (secondary) | 3 |
| 11 | {timeline} | History tab: List of events | data |

**Word budget:** screen 21 + shell 14 = **35** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Password not kept | Tooltip | Password | synofurb does not store it; the NAS keeps a hash. | help: first-install |
| Default boot | Tooltip | Default boot | Debian; the DSM loader entry remains. | help: recovery |
| Recovery tab | Link to D14 | Recovery | - | - |

**Primary:** Open web UI. **Secondary:** Diagnostics.

**Keyboard:** `Left`/`Right` tabs; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: No history: Empty; *error*: Unit offline: Tag "Offline", Open web UI disabled with Tooltip; *loading*: Skeleton while reading status.

### D16 Help centre

**Purpose:** Searchable short topics, shortcuts and docs links.

| # | Visible element (reading order) | Widget | Words |
|---|---|---|---|
| 1 | Help | Sheet title (opens from the right, 480 px; full-window below 960 px) | 1 |
| 2 | Search help | Input (search) | 2 |
| 3 | Topics, Shortcuts | TabBar | 2 |
| 4 | First install, Status meanings, Backups, Recovery, Boot manager, Serial console, Offline mode, Command line, Troubleshooting, Report a result | List (10 items, one line each) | 18 |
| 5 | Open docs | Link (external-link icon) | 2 |
| 6 | Back | Button (ghost) in topic view | 1 |
| 7 | {topic body ≤150 words} | TextView.markdown + "More" Link | data |

**Word budget:** screen 26 + shell 14 = **40** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 15 + shell 14 = **29** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Copy diagnostics | Tooltip | Copy diagnostics | Version, OS, settings; no secrets. | help: troubleshooting |
| synodeb note | Popover | synodeb | Boot-flash technique documented by synodeb (GPL-2.0). | No synodeb code is used; synofurb is MIT; link to project |
| Licence rows | Link | Licence | - | Opens bundled licence text (OFL, ISC, Apache-2.0, MIT) in a Dialog TextView |

**Primary:** Copy diagnostics. **Secondary:** links.

**Keyboard:** `Tab`; `Enter`; `Esc` back; `Tab`/`Shift+Tab` move; `F1` Help; `Ctrl+,` Settings; `Esc` closes popovers and dialogs (never cancels an install)

**States:** *empty*: n/a; *error*: Clipboard failure: Notification error; *loading*: n/a.

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

**Word budget:** screen 19 + shell 13 = **32** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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
| 2 | Volumes, Arrays, LVM, Disks | Tabs (role=tablist) | 4 |
| 3 | Volume, Mount, Size, Used | table headers | 4 |
| 4 | {data · /srv/data · 8.66 TiB · 16 %} | table rows | data |
| 5 | Create volume | button.primary | 2 |
| 6 | Extend | button.ghost (row) opens Popover | 1 |
| 7 | Delete | button.ghost danger (row) opens Dialog | 1 |
| 8 | Add | Popover: number input (GiB) + button "Extend" | cond. |

**Word budget:** screen 13 + shell 13 = **26** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Extend | Popover | Extend | Grows online; it cannot be shrunk again. | Result preview: new size and free space; help: storage |
| Delete | Dialog | Delete {name}? | Destroys {n} TiB; cannot be undone. | In use by {2 shares, 1 export}; Checkbox "I have a copy" + Input phrase `delete {name}`; button Delete (danger) |
| Array row | Tooltip | Sync | Progress, speed and time left. | help: storage |
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

**Word budget:** screen 14 + shell 13 = **27** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 13 + shell 13 = **26** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 10 + shell 13 = **23** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 18 + shell 13 = **31** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 24 + shell 13 = **37** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Boot agent | Popover | Boot agent | It resets the default to DSM and reboots after 30 idle minutes. | kexec starts Linux kernels only; after kexec nothing of ours runs; help: boot |
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

**Word budget:** screen 17 + shell 13 = **30** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 15 + shell 13 = **28** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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
| 3 | Safe apply, Sign-in, Certificate, Storage, Imports, Boot, Recovery, Report | List of topics (8) | 9 |
| 4 | Shortcuts | Tab | 1 |
| 5 | Open docs | link | 2 |
| 6 | Back | button.ghost in topic view | 1 |

**Word budget:** screen 16 + shell 13 = **29** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

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

**Word budget:** screen 9 + shell 13 = **22** of 60. Counted: titles, subtitles, labels, options, column headers, buttons, prose. Not counted: data (in braces), conditional elements (shown only in the named state).

**Disclosures** (everything that used to be prose):

| Element | Pattern | short (visible) | tip (one sentence) | help |
|---|---|---|---|---|
| Copy diagnostics | Tooltip | Copy diagnostics | Version, host, service state; no secrets. | help: sign-in |
| synodeb note | Popover | synodeb | Boot-flash technique documented by synodeb (GPL-2.0). | No code used; project link |

**Primary:** Copy diagnostics. **Secondary:** links.

**Keyboard:** `Tab`/`Shift+Tab` move; `/` focuses search where present; `?` opens Help; `Esc` closes popovers, dialogs and sheets

**States:** *empty*: n/a; *error*: Clipboard failure: Toast error; *loading*: n/a.

## 7. Help centre content

**Desktop Help centre (D16)**: a `Sheet` from the right (480 px). Top: `Input` search. Tabs: *Topics*, *Shortcuts*. Topic view: `TextView::markdown` body (at most 150 words) + a *More* `Link` to the docs page + *Back*. Opened by `F1` (topic of the focused control), every info icon, and the TitleBar Help button. Search matches titles and bodies. Offline: the topics are bundled; only *More* needs Internet.

| Id | Title | Words | Docs target |
|---|---|---|---|
| first-install | First install | 112 | README.md, docs/INSTALL.md |
| compat-status | Status meanings | 96 | docs/COMPAT.md |
| backups | Backups | 93 | docs/RECOVERY.md |
| recovery | Recovery | 105 | docs/RECOVERY.md |
| boot-manager | Boot manager | 103 | docs/BOOT.md |
| serial-console | Serial console | 83 | docs/BOOT.md, docs/ARM.md |
| offline-mode | Offline mode | 66 | docs/INSTALL.md |
| cli | Command line | 80 | README.md |
| troubleshooting | Troubleshooting | 97 | docs/RECOVERY.md |
| report-result | Report a result | 76 | docs/COMPAT.md |

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

DSM stays the default boot entry; every failure points it back at DSM.
| When | What happens |
|---|---|
| Before the boot flash is changed | Nothing to undo |
| Agent installed | The agent resets the default to DSM first; it reboots to DSM if nobody logs in |
| Before step 3 | A failure reboots to DSM after 60 s; a power cycle also works |
| After step 3 | Rescue shell for 15 minutes, then the DSM loader runs; DSM's own system may be gone, reinstall it from Synology (find.synology.com); no data promise |
| Debian installed | Pick the previous kernel in the boot menu or boot DSM |
Commands: `synofurb recover`, `synofurb boot-manager enter`.

#### boot-manager: Boot manager

x86 units boot through a small internal boot flash. Boot manager adds an SSH agent entry to it. The whole flash is saved to this computer first; disks are untouched; DSM's entry stays and stays the default.
The agent is one-shot: it resets the default to DSM when it starts and reboots to DSM after 30 minutes without a login. From it, `sf-boot kexec <kernel-url> <initrd-url>` starts another Linux kernel. kexec starts Linux kernels only, not Windows or UEFI-only ISOs. Once another system runs, nothing of ours is running: power-cycling returns to the default entry. That system can see and change your disks.

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
**Installer did not come up**: DSM is already the default; wait, then power-cycle.
**Boot flash write failed**: the tool restores the flash and verifies it; use the rescue shell within 15 minutes.
Logs: `~/.local/share/synofurb/logs` (secrets removed). Settings, Logs opens the folder; About copies diagnostics.

#### report-result: Report a result

Reports move a model from Untested towards Verified. After an install, press Report result on the Done screen. It opens a pre-filled GitHub issue (compat report template) with the model, DSM version, synofurb version, result, boot method and a log excerpt without secrets. The serial number is left out. Nothing is sent without your click, and Settings, Privacy has an off switch. A maintainer checks the log and changes the status through a reviewed pull request.

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
| boot | Boot page | 63 |
| recovery | Recovery | 55 |
| report | Report a result | 32 |

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

These are the entries in the boot flash menu: Debian (default), the previous kernel, the one-shot boot agent and the DSM loader. Power-cycling always returns to the default entry. The agent returns to DSM after 30 idle minutes. Once another system is started with kexec, nothing of ours is running. The DSM loader shows Synology's install page; DSM's own system may be gone.

#### web:recovery: Recovery

If something fails: DSM stays the default boot entry. Before the installer's step 3 a failure reboots to DSM in 60 seconds; after it a rescue shell stays open for 15 minutes, then the DSM loader runs and DSM must be reinstalled from Synology. We promise nothing about data. Commands: `synofurb recover` on your computer.

#### web:report: Report a result

Use Report issue on the About page to open the compatibility report template. Include the model, DSM version, synofurb version, what worked and the log. Do not include serial numbers or passwords.

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
| D1 Inventory | 24 | 14 | 38 | ok |
| D2 Connect | 23 | 14 | 37 | ok |
| D3 Inspect | 24 | 14 | 38 | ok |
| D4 Goal | 32 | 14 | 46 | ok |
| D5 Plan | 26 | 14 | 40 | ok |
| D6 System | 29 | 14 | 43 | ok |
| D7 Confirm | 39 | 14 | 53 | ok |
| D8 Install | 13 | 14 | 27 | ok |
| D9 Install failed | 22 | 14 | 36 | ok |
| D10 Done | 23 | 14 | 37 | ok |
| D11 Update | 10 | 14 | 24 | ok |
| D12 Settings | 28 | 14 | 42 | ok |
| D13 Boot manager | 26 | 14 | 40 | ok |
| D14 Recovery | 23 | 14 | 37 | ok |
| D15 Manage | 21 | 14 | 35 | ok |
| D16 Help centre | 26 | 14 | 40 | ok |
| D17 About | 15 | 14 | 29 | ok |
| W1 Login | 9 | 1 | 10 | ok |
| W2 Dashboard | 19 | 13 | 32 | ok |
| W3 Storage | 13 | 13 | 26 | ok |
| W4 Create volume | 14 | 13 | 27 | ok |
| W5 Mounts and imports | 13 | 13 | 26 | ok |
| W6 Network | 10 | 13 | 23 | ok |
| W7 Users | 18 | 13 | 31 | ok |
| W8 Boot | 24 | 13 | 37 | ok |
| W9 System | 17 | 13 | 30 | ok |
| W10 Audit log | 15 | 13 | 28 | ok |
| W11 Help centre | 16 | 13 | 29 | ok |
| W12 About | 9 | 13 | 22 | ok |

Highest desktop screen: 53 words. Highest web screen: 37 words. Dialog and Help bodies are opened on purpose and are bounded separately (dialogs at most 90 words, topics at most 150).

## 10. Review checklist for the real-app screenshots

Proposed `synofurb gui --demo --screen <name>` names: `inventory`, `inventory-offline`, `connect`, `connect-hostkey`, `inspect`, `goal`, `plan`, `plan-keep`, `system`, `system-offline`, `confirm`, `confirm-keep`, `install`, `install-after-ponr`, `failed-pre`, `failed-post`, `failed-keep`, `done`, `done-arm`, `update`, `settings`, `boot-manager`, `recovery`, `manage`, `help`, `help-topic`, `about`, and the dialogs `dlg-recovery`, `dlg-keep`, `dlg-next-pre`, `dlg-next-wipe`, `dlg-next-keep`. For each: light and dark at 960x640; focus ring visible; no horizontal scroll; word count at or under the table above; every element is the widget named here (hand-made imitations are defects); tooltips and popovers captured open for at least one screen per pattern; fonts Inter / JetBrains Mono visible; scrollbars themed.

Placeholders to confirm with engineering (shown as sample values in this spec): the web lockout numbers (5 failures in 10 minutes, 5 minute pause), the SSH confirm command name `synofurb-net confirm`, `synofurb-dom`/`sf-boot` command names as in `docs/BOOT.md` and `docs/RECOVERY.md`, and the `--screen` names above.
