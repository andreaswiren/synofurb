# The desktop interface (GPUI Kit)

`synofurb` (no arguments) and `synofurb gui` open the graphical wizard. It is written with **GPUI Kit** (`gpui-kit` 0.8.1:
GPUI plus the gpui-component widget set), so one Rust codebase runs on Windows, macOS and Linux. The egui version
(`gui.rs`, `eframe`, `rfd`) is gone. All decisions (scanning, inspection, planning, installing, recovery, updates, the
compatibility list) stay in `synofurb-core`; the UI only shows state and forwards clicks.

```
synofurb gui                              # the real thing
synofurb gui --demo --screen confirm      # built-in demo unit, nothing touches the network
```

Demo screen names: `inventory connect inspect goal plan system confirm install failed done update settings bootmgr recover`
(the old `layout` and `account` still work). In demo mode `install`, `failed` and `done` replay a scripted event stream.
Demo-only environment variables (never needed in normal use):

| Variable | Effect |
|---|---|
| `SYNOFURB_DEMO_UNIT=ds918\|ds1621\|rs816` | Which built-in unit is inspected: Intel BIOS (default), AMD with boot flash and the raw `uart8250` console, or ARM U-Boot (shows the "ACTION NEEDED" U-Boot box) |
| `SYNOFURB_THEME=light\|dark` | Theme for a demo run (otherwise follows the OS) |
| `SYNOFURB_DEMO_SCROLL=bottom` | With `--screen confirm`: scroll to the confirmation fields (screenshots) |
| `SYNOFURB_WINDOW=960x640` | Open at a given size (used to test the minimum window size) |

Screenshots are in `design/screenshots/<screen>-<theme>.png` (light for every screen, plus `confirm-dark.png`).

## Architecture

```
crates/synofurb/src/ui/
  mod.rs        run(): window options, assets, theme, key bindings
  theme.rs      ALL colours, sizes, radii and type styles; maps them onto GPUI Kit's Theme
  settings.rs   per-user preferences (theme, text size, update channel) as JSON
  widgets.rs    badge, banner, card, form row, table, choice card, log-palette code box ...
  state.rs      pure state machine pieces: Screen, Step rail, Risk, ConfirmGate, BootChoice, InstallView
  logview.rs    bounded log buffer, level/text filters with live counts, plain-text export
  diagram.rs    Plan -> DiagramModel (pure) -> boxes (disk / partition / RAID / LVM)
  format.rs     sizes (binary units, 2 decimals), percentages, masking
  worker.rs     std::thread workers + the Msg enum (one channel back to the UI)
  demo.rs       built-in units, demo compatibility statuses, scripted install events
  app.rs        the root view `App`: all state, the message pump, actions
  render.rs     window chrome: step rail, safety bar, update banner, pinned action bar, key bindings
  screens/      one module per screen (+ logviewer, update dialog)
  assets.rs     embedded Lucide SVGs (assets/icons/*.svg) on top of the Kit's default bundle
```

**Threads and the UI.** Slow work (scan, SSH inspect, install, recovery, update check/download, compatibility
refresh) runs on plain `std::thread`s exactly as the egui version did. They send `Msg` values into one
`futures::channel::mpsc::unbounded` sender; `App::new` spawns a GPUI task (`cx.spawn_in`) that awaits the receiver and
calls `App::on_msg` on the UI thread, which updates state and `notify`s. The UI thread never blocks and nothing polls.
The core's `Logger` still talks to a std channel; a small forwarder thread moves its events into the UI channel.

**One root view.** `App` owns every input (`InputState` entities), the plan form, the install view and the update
state. Screens are methods `App::page_<name>(&mut self, window, cx) -> Page` in their own files; `render.rs` wraps the
returned `Page` (title, body, action-bar note and buttons) in the chrome. The flow is the linear guided wizard of UX
concept A: nine steps in a persistent rail (Find NAS, Connect, Inspect, Goal, Storage, System, Confirm, Install, Done),
plus Settings, boot manager and recover as side screens. The rail only goes back through completed steps and is locked
while an install runs.

**Safety bar.** The "persistent safety rail" of concept A is a bar above every page, not a column: a four-level risk
state (Reading only, Plan prepared, You confirmed, Disks modified), the sentence that goes with it, the unit name and
address, and "Disks touched: Yes/No". A column did not fit next to the 212 px step rail at 960 px width. Moving it
is a change in `render.rs` only.

**The data-loss gate** (`screens/confirm.rs`, logic in `state::ConfirmGate`). Order: title with the unit, error
banner, reversible/point-of-no-return strip, Will be destroyed and Will be preserved cards, new layout, backup
finding, the fallback plan (wording follows `docs/RECOVERY.md`, per firmware: `state::fallback_lines`), two
checkboxes, unit name, address and serial, the typed phrase `ERASE <MODEL>` (from `session::confirm_phrase`, paste is
allowed, surrounding whitespace is ignored, case matters), live match text. The red button is disabled until every
item passes, with the reason beside it; `App::start_install` re-checks the gate itself. Enter never triggers it.

## Theme mapping

`design/mockups/ui/tokens.json` is the source. `theme.rs` holds the same values (`LIGHT`, `DARK` tables, type styles,
layout constants); a unit test reads `tokens.json` and fails when the two differ, another checks the declared text
pairs for WCAG AA. A design revision is therefore: edit the tables (or regenerate them from the JSON), run
`cargo test -p synofurb`.

`theme::apply(palette, cx)` does two things: it stores the palette in a global (`cx.pal()` in screens) and writes the
palette into GPUI Kit's `Theme` through `Theme::update`, so stock widgets match:

| Our token | GPUI Kit theme field |
|---|---|
| canvas / fg / border / border_strong | background / foreground / border / input |
| accent, accent_hover, accent_pressed, on_accent | primary, primary_hover, primary_active, primary_foreground (+ `button_primary*`) |
| surface, surface_2, surface_3 | secondary / popover / list / table; secondary_hover; secondary_active |
| danger, danger_hover, danger_pressed, on_danger | danger*, `button_danger*` |
| ok / warn / info solid | success / warning / info (+ `button_*`) |
| accent_bg, accent_fg | accent, accent_foreground, selection, list/table active |
| sidebar, ring, scrim | sidebar, ring, overlay (alpha `scrim.alpha`) |
| (not in Kit) ok/warn/err/info `*_fg *_bg *_border`, log colours | read directly: `Palette` / `Tone` in `widgets.rs` |

Theme choice is Follow system / Light / Dark (Settings; `observe_window_appearance` re-applies when the OS changes).
Text size (100 / 115 / 130 %) sets the window's rem size; all type styles are in rems.
The log viewer uses the dark terminal palette in both themes, as the design says.

## How to add a screen

1. Add a variant to `state::Screen` (and to `DEMO_SCREEN_NAMES` / `DemoTarget::parse`, `Screen::step`, `risk_for`).
2. Create `screens/<name>.rs` with `impl App { pub(crate) fn page_<name>(&mut self, window, cx) -> Page }`; register it
   in `screens/mod.rs` and in `App::page_for` (`render.rs`). Use `cx.pal()` and the helpers in `widgets.rs`; no hex
   values or pixel constants in screens (add them to `theme.rs` instead).
3. Navigate with `self.go(Screen::X, window, cx)`; put background work in `worker.rs` with a new `Msg` variant and
   handle it in `App::on_msg`.
4. If the screen holds pure decisions (what is enabled, what a count is), put them in `state.rs` and unit-test them.
5. Add the demo state in `App::start_demo` so `synofurb gui --demo --screen <name>` works.

To add an icon: copy the SVG from `gpui-kit-assets/assets/icons/` into `ui/assets/icons/` and add it to `assets::ICONS`.

## Platform notes

* **Linux**: GPUI needs the X11/Wayland, xkbcommon, freetype and Vulkan (or GL) development packages
  (`libxkbcommon-x11-dev libxcb1-dev libfreetype-dev libvulkan-dev libwayland-dev` and friends). The root
  `Cargo.toml` carries `[patch.crates-io] xattr = { path = "vendor/xattr" }` because GPUI's `xattr` 0.2.3 does not
  compile with current `libc`; keep it (cargo prints "patch was not used" when nothing in the graph needs it any more;
  then it can go).
* **Windows**: MSVC toolchain and the Windows SDK; GPUI draws with DirectX 11 (no extra packages). Not built here.
* **macOS**: Xcode command line tools and a Metal-capable Mac. Not built here. Keybindings use `cmd` instead of `ctrl`
  through `modkey!`.
* No platform-specific code lives outside `#[cfg(unix)]` for file permissions (`0600` on keys and saved logins) and
  `modkey!`. File dialogs are GPUI's `prompt_for_paths` / `prompt_for_new_path`, not `rfd`.
* Fonts: the system UI font and the system monospace font are used. Inter and JetBrains Mono are not bundled yet (the
  design asks for them; it is a decision for the owner, see below).

## Accessibility

GPUI Kit exposes an AccessKit tree (accesskit_unix/AT-SPI on Linux, UI Automation on Windows, NSAccessibility on
macOS); the recommendation's claim that GPUI has none is out of date for this version. What is wired in our code:
roles and labels on the step rail items ("Step 3: Inspect, current step"), the page title (`Heading`), the safety bar
and the typed-phrase match text (`Status`), error banners (`Alert`), the install log (`Log`) and its level buttons,
install steps (`ListItem` with "done/running/pending/failed"), accessible labels on sliders, selects, progress bars
and the destructive button. Stock Kit widgets bring their own roles. Keyboard: Tab order follows layout, Space selects
choice cards, Enter runs the screen's primary action (never the red button), Alt+Left/Right move between completed
steps, Ctrl+, (Cmd+, on macOS) opens Settings, Ctrl+F/A/C work in the log, Esc closes dialogs and never cancels an
install. Reduced motion replaces the spinner by a static glyph plus the word "running".

## Known limitations

* **Verification of accessibility is by construction only.** No screen reader (NVDA, Narrator, VoiceOver, Orca) was run.
* Disabled buttons are not focusable in GPUI Kit, so the reason is shown as text beside them instead of a description.
* The window has native decorations; there is no custom title bar.
* The log viewer virtualises its rows and supports select all and copy of the filtered text, but not selecting a part
  of a line with the mouse.
* Pressed/hover/focus-visible/disabled/loading states come from the Kit's buttons and inputs and from `choice_card`;
  other custom rows (rail steps, log level buttons) have hover only.
* `danger_edge` and `ring_inverse` from the revised tokens are stored but not applied: the Kit's danger button has no
  separate edge colour and filled buttons draw their own focus line.
* Restore disk headers, rescue shell and "remove boot manager" are listed on the Restore screen as unavailable with a
  reason (no core API); "Return to DSM" works through `session::recover_to_dsm` and is limited to boot-flash units.
* Boot manager mode is only enabled for units with an editable boot flash (`session::boot_manager_install` requires it).
* Updates: the dialog and download are real (`updater`), but the release signing key in core is still a placeholder,
  so "Download and verify" is disabled with that explanation until it is configured.
* The demo shows compatibility statuses from `demo.rs` (the shipped list has no Verified or Known-issues model yet),
  labelled "demo data" on the inspect screen.

## Gaps in `synofurb-core` (worked around in the UI layer)

* `discovery::scan` returns only when every method is done: no streaming or cancel. `worker::spawn_scan` repeats its
  orchestration with a partial-result hook (about 40 lines, calls the public per-method scanners).
* No SMART data in `DeviceFacts`, so the "sdc has reallocated sectors" warning of the mockups cannot be shown.
* No SSH host-key trust step before connecting (`fetch_host_fingerprint` exists but nothing records trust).
* No timestamp of the first disk write; the UI only knows "point of no return passed" from the event.
* The installer's point of no return is emitted at the start of step 3, not between steps 4 and 5 as in the mockup; the
  UI follows the script.
* `Event` carries no structured "disks touched" or failure kind; the failure screen derives it from `PointOfNoReturn`.

## What was and was not verified

Verified here (Linux, headless): `cargo fmt`, `cargo clippy -p synofurb --all-targets -- -D warnings`, `cargo test -p
synofurb` (unit tests for theme tokens against `tokens.json`, contrast, formatting, settings, screens/step rail, risk
level, confirm gate, boot choice, serial parsing, install progress, log filtering, diagram model, scripted demo
events, system-form validation); every demo screen renders under Xvfb with software Vulkan (lavapipe); the layout at the
960 x 640 minimum (plan and connect checked).

Not verified: Windows and macOS builds, real hardware, a real scan/inspect/install over SSH, the update download path
(placeholder signing key), IME and password-manager behaviour of the text inputs, screen readers, text scale 130 % on
every screen, high-contrast OS modes, fonts other than the system defaults. UI integration tests with
`gpui_kit::test` (the gate button disabled until the phrase is typed) were not added: the `test-support` feature
rebuilds the whole GPUI graph with different features (several GB of extra build output), which did not fit on this
machine; the gate logic itself is covered by `state::tests`.

## Decisions for the owner

1. Bundle Inter and JetBrains Mono (the design asks for them) or keep the system fonts.
2. Safety rail as a bar (now) or a right-hand column at widths of 1100 px and up.
3. Keep "Boot manager mode" enabled only for boot-flash units, or also offer the disk-partition variant (typed gate).
4. Whether to ship the release signing key now so the update dialog can install.
5. Window decorations: native (now) or a custom title bar.
