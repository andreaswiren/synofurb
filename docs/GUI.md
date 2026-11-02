# The desktop interface (GPUI Kit)

The desktop UI follows the owner-approved v2 mockup (`design/mockups-v2/desktop`, spec `design/v2/UX-SPEC.md`, brief
`design/BRIEF-v2.md`). It uses stock `gpui-component` widgets through GPUI Kit 0.8.1, bundled Inter and JetBrains Mono,
Lucide icons only, themed scrollbars, and at most 60 words of static prose per screen (enforced by tests in `copy.rs`).
The widget map and the defects of the first UI are in `docs/GUI-COMPONENTS.md`; deviations from the mockup are in
`design/v2/DESIGN-SYSTEM-V2.md` section 13 (DEV-UI-1 to DEV-UI-23).

`synofurb` (no arguments) and `synofurb gui` open the wizard. All decisions (scan, inspect, plan, install, recovery,
update, compatibility) stay in `synofurb-core`; the UI shows state and forwards clicks.

```
synofurb gui                              # the real thing
synofurb gui --demo --screen confirm      # built-in demo unit, nothing touches the network
```

Demo screens (59, see `state::DEMO_SCREEN_NAMES`): `inventory connect inspect goal plan plan-keep system confirm install
install-after-ponr failed failed-pre failed-post done update settings bootmgr recover manage manage-* recovery-* help
help-topic about dlg-* welcome welcome-2..welcome-5` (`first-launch` is an alias of `welcome`). `install`, `failed` and `done` replay a scripted event stream.

| Variable | Effect |
|---|---|
| `SYNOFURB_DEMO_UNIT=ds918\|ds1621\|rs816` | Which built-in unit is inspected |
| `SYNOFURB_THEME=light\|dark` | Theme for a demo run |
| `SYNOFURB_DEMO_SCROLL=bottom` | With `--screen confirm`: scroll to the gate |
| `SYNOFURB_DEMO_GATE=ready` | With `--screen confirm`: gate satisfied (checks the enabled Erase button) |
| `SYNOFURB_WINDOW=960x640` | Open at a given size |

Screenshots of the implemented app: `design/screenshots/v2-app-<screen>-<theme>[-1180].png` (Xvfb + lavapipe).

## Architecture

```
crates/synofurb/src/ui/
  mod.rs        run(): window options, assets, fonts, theme, key bindings
  theme.rs      tokens (generated from design/v2/tokens-v2.json) -> gpui-component Theme; accent; scrollbars
  settings.rs   per-user preferences (theme, text size, accent, update channel) as JSON
  kit.rs        small helpers over stock widgets: segmented (ToggleGroup), field, with_reason (disabled + info button)
  disclosure.rs info_tip / info_popover / dialog: the "explanation lives behind an info button" pattern
  copy.rs       every user-visible string, with a kind and a word budget per screen (tests)
  facts.rs      PURE: UnitFacts -> risk Tag, Way back, Recovery rows, Quick setup defaults, phrase suffix, agent timeout
  cli_cmd.rs    PURE: the Copy command text, and `matches(plan)` for the "Matches plan" Tag
  trust.rs      PURE: the worker-to-UI bridge for the SSH host-key question (refuse on no answer), fingerprint lines
  mgmt.rs       PURE: Manage health reading, risk from health, update gate
  raid.rs       PURE: RAID/SHR labels, survives, slices, mdstat parsing
  state.rs      Screen, six-step Step, ConfirmGate, BootChoice, InstallView, demo targets and fixtures
  app.rs        root view `App`: state, message pump, actions
  render.rs     title bar, Stepper, unit header, status bar, update banner, action bar, key bindings
  welcome.rs    PURE: the Welcome guide's step machine (five steps, Back/Next/Skip, what "seen" means) and its content table
  overlays.rs   Help sheet, Welcome guide, Keep/Next/Add/Slices dialogs, Copy command, Quick setup
  screens/      one module per screen
  worker.rs     std::thread workers + Msg; demo.rs scripted units and events; assets.rs embedded Lucide icons
```

**Welcome guide** (UX spec D1b, `welcome.rs` + `screens/welcome.rs`). A five-step dialog on the first launch: what
synofurb does, the six steps, before you start (disks erased, tested backup, NAS on and online), the way back, ready.
Each step has one Lucide icon, one headline and at most two short lines; details sit behind an info button (the
screen-reader note and "only reads until you confirm" on step 3, Boot manager and Recovery on step 4). It replaces the old
"Before you start" dialog and keeps its safety statements. Back / Next / Skip, a dots row, Left and Right arrow keys, Esc
and the close button; the last step has "Don't show again" (ticked) and Start. `UiSettings.welcome_done` stores "seen"
(old files with `first_launch_done` still count). Skip, or Esc on steps 1 to 4, dismisses it; on the last step Start and Esc
follow the box. Reopen it from Help (first row of the topic list) or the title-bar menu; replaying never clears the flag.
Demo: `--screen welcome` and `welcome-2` to `welcome-5`; screenshots `design/screenshots/v2-app-welcome-<n>-<theme>[-largest].png`.

**Six steps.** Find, Connect (with Inspect), Goal (with Quick setup), Setup (Storage and System tabs), Confirm, Install
(with Done). Settings, About, Help, Manage, Boot manager and Recovery are side screens. The Stepper is the widget from
GPUI Kit; the unit header under it always shows the unit and a risk Tag.

**Risk Tag and Way back** are pure functions of the unit facts (`facts::risk`, `Risk::way_back`): the same state always
shows the same Tag and the same way back; the status bar repeats it. **Recovery** rows come from `facts::recovery_rows`;
when the boot agent never answered, "Manual recovery" is the primary.

**The data-loss gate** (`screens/confirm.rs`, `state::ConfirmGate`): backup alert, Destroyed / Changed / Preserved
triad, the After line, then the gate card: three checkboxes (tested backup, accept data loss, this is the unit) and the
typed phrase `ERASE <MODEL>` (twin units get a serial suffix, `facts::confirm_phrase_for`). Erase stays a real disabled
`Button` until everything passes; the reason is on a sibling info button with a Popover. `App::start_install` re-checks
the gate. **Enter does not trigger Erase** (a `capture_key_down` wrapper stops it); Space and click do. Copy command opens
a dialog with the whole plan as a CLI line and a "Matches plan" Tag computed by `cli_cmd::matches`.

**Disabled-reason pattern.** A disabled `Button` is not focusable (stock behaviour), so the reason sits on a focusable
info button next to it (`kit::with_reason`). Tooltips appear only on focusable controls and never carry the only copy of
safety text (DEV-UI-1).

**Threads and the UI.** Slow work runs on plain `std::thread`s and sends `Msg` values into one unbounded channel; a GPUI
task calls `App::on_msg` on the UI thread. The UI thread never blocks.

## Theme mapping

The sources are `design/v2/tokens-v2.json` (tokens, type scale, layout) and `design/v2/synofurb-theme.json` (the designer's
GPUI ThemeSet, copied byte for byte to `assets/theme/synofurb-theme.json`). The theme is **not hand-mapped**:

* `theme::apply(dark, scale, accent, cx)` loads the ThemeSet with `Theme::apply_config`, patches the chosen accent
  (Purple, Blue, Green, Amber, Rose, each with a tested contrast) into the Kit's colours, sets the rem size from the text
  scale (100 / 115 / 130 %) and styles the scrollbars (`style_scrollbars`: 6 px thumb, 4 px inset, transparent track).
* `tokens.rs` is generated from the JSON by `assets/theme/gen_tokens.py`; `cx.pal()` returns the `Palette` for the few tone
  colours the Kit has no field for (ok / warn / err / info fg and bg, log colours).
* Tests: the ThemeSet file equals the design copy, the type styles equal `tokens-v2.json`, every declared text pair meets
  WCAG AA, every accent is at least 4.5:1 on its surface.

Theme choice is Follow system / Light / Dark (Settings; the window appearance observer re-applies it). The log viewer
uses the dark terminal palette in both themes, as the design says.

## How to add a screen

1. Add a variant to `state::Screen` (and to `DEMO_SCREEN_NAMES` / `DemoTarget::parse`, `Screen::step`, `risk_for`).
2. Create `screens/<name>.rs` with `impl App { pub(crate) fn page_<name>(&mut self, window, cx) -> Page }`; register it
   in `screens/mod.rs` and in `App::page_for` (`render.rs`). Use `cx.pal()` and the helpers in `kit.rs`; no hex
   values in screens; spacing uses the design scale (2, 4, 6, 8, 12, 16, 20, 24, 32), sizes that are fixed by the design
   live in `theme::layout`.
3. Navigate with `self.go(Screen::X, window, cx)`; put background work in `worker.rs` with a new `Msg` variant and
   handle it in `App::on_msg`.
4. If the screen holds pure decisions (what is enabled, what a count is), put them in `state.rs` and unit-test them.
5. Add the demo state in `App::start_demo` so `synofurb gui --demo --screen <name>` works.

To add an icon: copy the SVG from `gpui-kit-assets/assets/icons/` into `ui/assets/icons/` and add it to `assets::ICONS`.

## Platform notes

* **Linux**: GPUI needs the X11/Wayland, xkbcommon, freetype and Vulkan (or GL) development packages
  (`libxkbcommon-x11-dev libxcb1-dev libfreetype-dev libvulkan-dev libwayland-dev` and friends). GPUI's HTTP client pulls `xattr` 0.2.3, which does not compile on Linux with current `libc` (`ENOATTR`); this app
  does not enable that part, so nothing is needed today. If a future GPUI update does pull it in, patch `xattr`
  locally (on Linux `ENOATTR` is `ENODATA`).
* **Windows**: MSVC toolchain and the Windows SDK; GPUI draws with DirectX 11 (no extra packages). Not built here.
* **macOS**: Xcode command line tools and a Metal-capable Mac. Not built here. Keybindings use `cmd` instead of `ctrl`
  through `modkey!`.
* No platform-specific code lives outside `#[cfg(unix)]` for file permissions (`0600` on keys and saved logins) and
  `modkey!`. File dialogs are GPUI's `prompt_for_paths` / `prompt_for_new_path`, not `rfd`.
* Fonts: Inter (Regular, SemiBold, Bold) and JetBrains Mono (Regular, Bold) are bundled (`assets/fonts/`, OFL texts next
  to them) and registered at start-up; only weights 400, 600 and 700 ship.

## Accessibility

GPUI Kit exposes an AccessKit tree; stock widgets bring their roles. Wired here: labels on info buttons (the word, not
the icon), page title as heading, step labels, the typed-phrase match text, error alerts. Keyboard: Tab follows layout;
Space or Enter activates the focused control; Enter on the page itself runs the primary action (never Erase); Alt+Left
goes back, Ctrl+, opens Settings, F1 or ? opens Help at the current topic, Ctrl+Shift+C opens Copy command; Esc closes
dialogs and never cancels an install. Text size (Default, Large, Largest) scales the rem; type scale 12/14/16/24 px.

## Known limitations

* Tooltips are pointer-only in gpui-component (DEV-UI-1); keyboard users get the info button and its Popover.
* Management update in the demo is simulated; the desktop Storage tab is read-only (DEV-UI-13).
* The recovery actions run the engine but have never run against a real unit (DEV-UI-12); SSH host-key trust is a dialog over the engine's TOFU callback (DEV-UI-19), also unverified on a real unit.
* The failure clocks are rules, not live counters (DEV-UI-11). No OS keychain (DEV-UI-14). No rescue shell exists in the installer, so the Failed page has no rescue panel (DEV-UI-18).
* Native window decorations are replaced by the Kit title bar; Windows and macOS are untested.
* The release signing key in core is a placeholder, so "Download and verify" is disabled with that explanation.

## What was and was not verified

Verified (Linux, headless): per-file rustfmt, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (copy budgets and
the tie between the copy and `synofurb_core::timing`, tokens vs JSON, accent contrast, risk Tag, Way back, recovery rows, Quick setup
defaults and its "Use these" state, phrase gates, command-vs-plan, Manage binding to one unit, per-unit saved images, host-key bridge,
Find column widths, management health and update gate, RAID helpers). Every demo screen renders under Xvfb with software Vulkan;
`crates/synofurb/assets/tools/gui-walk.sh` renders every demo screen (pages, dialogs, the Help sheet) at a text size and fails when content
reaches the page gutter or a horizontal scrollbar appears; it also scrolls each page by one and two viewports and walks the log
viewer, the Copy command and Cancel install dialogs and the disabled-reason Popover. The `gui-walk` CI job runs it at Largest on
ubuntu-24.04 (Xvfb, lavapipe); that job has not run yet. A local run at Largest (light, 960 x 640, three viewports per screen) found no overflow on all 55 demo screens (the five Welcome guide screens were walked separately at default and Largest, light and dark: clean) and the 4 extra states. The Help sheet is excluded from the gutter test because it is a panel over
the page; content hidden by `overflow: hidden` without a scrollbar cannot be seen by it. Enter/Space behaviour of Erase and the Command button were checked with injected XTest keys.

Screenshots: `design/screenshots/v2-app-*.png`. Each file shows what its name says; the states that need an input were produced with
injected XTest events (mouse motion, button down, Tab, key combinations), and two files with different names are never byte-identical
(checked by hash). Light 960 x 640 for every demo screen (`failed-pre` is an alias of `failed` and `manage-health-ok` of `manage`: one file each), 73
files; dark 45, Largest text 27 (pages and the dialogs Manual recovery, host-key, changed key, next-step, welcome guide (five steps, light and dark, plus Largest), update,
Keep, Quick setup and Copy command), 1180 x 760 for 8 pages. Dialogs and states: Copy command, Cancel install, Report, Add by IP,
Keep, the seven "What happens next" variants (reboot or halt, before or after the point of no return, Wipe or Keep, agent), host-key
(waiting, ended and changed), Quick setup, Manual recovery, Restore flash (typed phrase), Remove entries (typed phrase), Open agent on
Debian, Boot DSM (and its outcome), Confirm with Erase enabled, Keep and twin, Find empty and scanning, Manage sign-in and every
Manage state, the log viewer. Interaction states: a keyboard focus line on a checkbox and a primary button (light and dark), hover and
pressed of the primary button (light and dark), the disabled-reason Popover.
Not captured: hover and pressed for other control classes, the focus line on text inputs and on segmented choices (the segmented
group showed no focus under Tab in the headless run: unverified), a recorded Tab order, Restore flash and Open agent outcomes (the typed
phrase cannot be typed through XTest keysyms), dark for every dialog, Largest in dark except Confirm.

**Window controls and the black strip in captures.** With no window manager (Xvfb) GPUI falls back to server-side decorations on X11
and `TitleBar` then draws no minimise, maximise or close buttons (the window manager is expected to); a window manager supplies them.
The window also sits 2 px right of its requested origin without one, so the 2 px black strip down the left edge of every capture is
the X root window, not part of the app (a capture with a larger root shows it outside the window; nothing is drawn at x = 0 and 1
inside the app). Captures with real window controls need a desktop session; they cannot be produced headless here (a fake
compositing-manager selection did not make GPUI switch to client decorations).

Not verified: Windows and macOS, real hardware (scan, SSH, recovery actions, Manage update, host-key trust against a real unit), the
update download path, screen readers, IME and password managers, Popover focus return, Select placement, RadioGroup/ToggleGroup roving
focus, high-contrast OS modes. UI integration tests with `gpui_kit::test` were not added (the `test-support` feature rebuilds the GPUI
graph and does not fit the disk here); the logic is covered by unit tests.

## Decisions for the owner

1. Keep "Boot manager mode" only for boot-flash units, or also offer the disk-partition variant (typed gate).
2. Ship the release signing key so the update dialog can install.
3. Wire the Manage update and rollback to the engine's `push_update` / `push_rollback`.
4. Add an OS keychain for saved passwords (needs core support).
