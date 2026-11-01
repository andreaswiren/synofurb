# synofurb design system (Direction 1, "ForgeVault family", revised after the UI critic review)

One visual language for the native installer (Rust + GPUI) and the NAS management plane (`https://<ip>:8443`). It is a **derivative** of ForgeVault's tokens (neutral ramp, one violet accent, hairline cards, 8 and 12 px radii, lucide-style strokes, pill badges), not a copy. Live reference: `direction-1/index.html` (all screens, light and dark, 960 x 640 / 1180 x 760 / 1440 x 900 windows, 1180 / 768 / 390 px web viewports, 100% and 130% text) and `direction-1/states.html`. Tokens: `tokens.json`. Gate: `python3 tools/contrast.py`.

Layout comes from the revised UX concept A (stepper across the top, persistent **safety rail** on the right, pinned action bar) and management plane mgmt-A (sidebar shell, status strip, "Needs attention", docked edit panel, persistent safe-apply banner). Screens drawn: Find, Connect, Inspect, Goal, Plan (valid and blocking error), Confirm (gate incomplete and satisfied), Install (staging and after the point of no return), Failed before and after the point of no return, Done, Update dialog, States sheet; Web: Login, Dashboard, Storage, Network (pending), Users, Boot, and the dashboard, network and users pages at 390 px. Not drawn (they reuse the same components): the System step, find-scan / empty / offline, Settings, Boot manager and Restore screens, Manage and About, the edge-state sheet of the UX mock.

## 1. Principles

1. **Calm everywhere, one loud thing.** On the data-loss gate red appears on exactly two things: the *enabled* destructive button and the "Will be destroyed" marker and heading (a 12 px square and a text line). The banner, the unit card, the "changed" and "preserved" lists and the disabled button are neutral. The red budget is measured by script (3 coloured elements when the button is enabled, 2 when it is not, both themes).
2. **Never colour alone.** Every status has colour, a drawn icon, a word and, where it matters, a border style (Untested and disabled controls are dashed).
3. **Say what happens to the data.** Destroyed, changed (backed up first) and preserved are separate lists; the boot flash is never "preserved".
4. **One token set, two surfaces.** Desktop and web read the same `tokens.json`.
5. **Only what GPUI draws cheaply.** Flat fills, 1 px borders, small radii, one soft shadow, monochrome SVG icons. Excluded everywhere (and linted): gradients, patterns, inset shadows, `color-mix`, `backdrop-filter`, all-caps transforms, text under 12 px.

## 2. Colour tokens

Opaque sRGB hex (alpha only for the scrim, 0.60), so every token is one `rgb(0x......)` in GPUI and one custom property in CSS (underscores become hyphens). Light and dark are re-derived, not inverted.

| Token | Light | Dark | Role |
|---|---|---|---|
| `canvas` | `#FAFAFA` | `#0A0A0A` | Window / page background |
| `surface` | `#FFFFFF` | `#171717` | Cards, dialogs, inputs |
| `surface_2` | `#F3F3F3` | `#202020` | Table header, hover, disabled fill, code chips |
| `surface_3` | `#E9E9E9` | `#2B2B2B` | Progress and slider track (with a 1 px border_strong outline), pressed |
| `sidebar` | `#F7F7F7` | `#101010` | Safety rail and web sidebar |
| `border` | `#E5E5E5` | `#2A2A2A` | Decorative hairlines only (carry no meaning) |
| `border_strong` | `#808080` | `#868686` | Control boundaries: inputs, checkboxes, outline buttons, tracks (3:1 on every surface) |
| `fg` | `#0A0A0A` | `#FAFAFA` | Primary text |
| `fg_2` | `#525252` | `#B1B1B1` | Secondary text, labels, captions |
| `placeholder` | `#696969` | `#9E9E9E` | Placeholder text only (4.5:1) |
| `accent` | `#7139CB` | `#A37BFE` | Primary button, selection edge, progress fill, checked box |
| `accent_hover` | `#622CB5` | `#B091FE` | Primary hover |
| `accent_pressed` | `#541EA0` | `#946AEE` | Primary pressed |
| `on_accent` | `#FFFFFF` | `#0E0C15` | Label on accent fills |
| `accent_bg` | `#F1EDFE` | `#31254D` | Selected row/card tint, current step |
| `accent_fg` | `#5C29AA` | `#C8B7FF` | Accent-coloured text and links |
| `accent_border` | `#B193E4` | `#6A50A6` | Opaque border for accent tints |
| `ring` | `#7139CB` | `#A37BFE` | Focus ring (2 px, offset 2 px) |
| `ring_inverse` | `#A37BFE` | `#A37BFE` | Focus ring inside the always-dark log panel |
| `danger` | `#C60014` | `#B7191C` | Destructive button fill (only data-loss actions) |
| `danger_hover` | `#AC0311` | `#A50E15` | Destructive hover |
| `danger_pressed` | `#94020D` | `#8D0B11` | Destructive pressed |
| `danger_edge` | `#C60014` | `#FF9592` | Destructive button 2 px outline (dark theme: a light red that keeps the button visible on dark surfaces) |
| `on_danger` | `#FFFFFF` | `#FFFFFF` | Destructive label (white in both themes) |
| `ok_solid` | `#047E39` | `#37BB62` | Success dots, bars, step discs |
| `ok_fg` | `#005725` | `#6ED889` | Success text |
| `ok_bg` | `#DEF8E2` | `#0F3118` | Success tint |
| `ok_border` | `#66B585` | `#257D41` | Opaque tint border |
| `warn_solid` | `#A76C01` | `#F0B135` | Warning dots, bars |
| `warn_fg` | `#6F4606` | `#F1C45E` | Warning text |
| `warn_bg` | `#FFF3D3` | `#3D2A02` | Warning tint |
| `warn_border` | `#CFA95F` | `#9F741E` | Opaque tint border |
| `err_solid` | `#C60014` | `#FC5855` | Error dots, bars, failed step discs (the same red family as danger) |
| `err_fg` | `#A70311` | `#FFA09C` | Error text |
| `err_bg` | `#FEEFEF` | `#4F1A18` | Error tint |
| `err_border` | `#DF6C77` | `#AE3C3A` | Opaque tint border |
| `info_solid` | `#1570D1` | `#539AF2` | Info dots |
| `info_fg` | `#004B96` | `#91C1FF` | Info text |
| `info_bg` | `#E7F1FF` | `#132946` | Info tint |
| `info_border` | `#73AAE6` | `#3667A5` | Opaque tint border |
| `log_bg` | `#121114` | `#060607` | Log panel (dark in both themes) |
| `log_fg` | `#DEDEDE` | `#DEDEDE` | Log text |
| `log_dim` | `#9E9E9E` | `#989898` | Timestamps, debug text |
| `log_dbg` | `#9DA5B1` | `#979FAB` | DBG label |
| `log_inf` | `#89C3FE` | `#89C3FE` | INF label |
| `log_wrn` | `#F7C243` | `#F7C243` | WRN label and edge |
| `log_err` | `#FE8A88` | `#FE8A88` | ERR label and edge |
| `log_wrn_bg` | `#322A1B` | `#28200F` | Opaque warn line tint |
| `log_err_bg` | `#3C2729` | `#331E1E` | Opaque error line tint |
| `scrim` | `#000000` | `#000000` | Dialog scrim base colour, used at alpha 0.60 |

### Accents (web plane)

ForgeVault's accent switcher ships five accents. Measured on white, their own values are: purple 5.38:1 (white label), blue 4.93, rose 4.79, **green 3.99 (fails)** and **amber 2.78 (fails as button, ring and fill)**. synofurb therefore offers each accent only through the derived sets below (every set has `accent`, `accent_hover`, `accent_pressed`, `on_accent`, `accent_bg`, `accent_fg`, `ring` in `tokens.json` under `accent.<name>.<theme>`), all tested by `contrast.py`: label on fill and hover and pressed 4.5:1, link text on surface, canvas and tint 4.5:1, ring and fill 3:1 on surface, tint and track. Light themes use a white label; dark themes use a near-black label. Amber is a deep amber in light mode (the bright ForgeVault amber cannot carry a label or a ring on white). The desktop installer is violet only.

| Accent | Light: fill / hover / link text / tint | Dark: fill / label / link text | Label (light, dark) |
|---|---|---|---|
| purple (default) | `#7139CB` / `#6327BA` / `#5E1EB3` / `#F1EDFE` | `#A37BFE` / `#0E0C15` / `#C8B7FF` | white, near-black |
| blue | `#036ED4` / `#0260BB` / `#015AB1` / `#E7F1FF` | `#4197FF` / `#080E16` / `#9DC7FE` | white, near-black |
| green | `#0E803B` / `#026F31` / `#03692E` / `#E5F5E7` | `#47AA62` / `#071009` / `#8FD89E` | white, near-black |
| amber | `#9B6200` / `#875503` / `#805002` / `#FBEEDF` | `#CE871B` / `#130C05` / `#F2B772` | white, near-black |
| rose | `#D6224E` / `#C30040` / `#B9003D` / `#FEEBEB` | `#FF5C74` / `#150A0A` / `#FEA9AF` | white, near-black |

## 3. Contrast gate

`tools/contrast.py` (WCAG 2.x relative luminance, ratio = (L1 + 0.05) / (L2 + 0.05)) checks 472 pairs: **228 token-matrix pairs over both themes** (every text token on every surface it may sit on, every non-text token on every surface it may touch), **100 accent-variant pairs**, and **144 pairs extracted from `direction-1/styles.css`** (every colour/background and border/background combination that really occurs in one rule). It then fails if a colour token used in a colour property takes part in no tested pair, and lints `styles.css` and `index.html` for gradient, pattern, inset shadow, `color-mix`, `backdrop-filter`, uppercase transforms and font sizes under 12 px. Result for the files in this folder: **472 of 472 pairs pass, no lint or coverage failure.** Lowest text ratio: **4.78:1** (placeholder on the accent tint, light). Lowest non-text ratio: **3.25:1** (`border_strong` on `surface_3`, light). Text needs 4.5, non-text 3.

What it cannot see: text whose background is an ancestor's (covered by the matrix of allowed surfaces, not by the rule extractor), anything drawn in SVG attributes in the diagram (its fills are the same tokens and were checked by eye), and GPUI rendering. Treat a new token or a new surface as a change to the matrix in `contrast.py`.

### Text (ratios, light and dark)

| Foreground | Background | Light | Dark | Need |
|---|---|---:|---:|---:|
| `accent_fg` | `accent_bg` | 7.77 | 7.77 | 4.5 |
| `accent_fg` | `canvas` | 8.55 | 11.01 | 4.5 |
| `accent_fg` | `sidebar` | 8.33 | 10.58 | 4.5 |
| `accent_fg` | `surface` | 8.93 | 9.97 | 4.5 |
| `accent_fg` | `surface_2` | 8.04 | 9.06 | 4.5 |
| `accent_fg` | `surface_3` | 7.35 | 7.87 | 4.5 |
| `err_fg` | `accent_bg` | 6.86 | 7.16 | 4.5 |
| `err_fg` | `canvas` | 7.55 | 10.14 | 4.5 |
| `err_fg` | `err_bg` | 7.06 | 7.21 | 4.5 |
| `err_fg` | `surface` | 7.88 | 9.19 | 4.5 |
| `err_fg` | `surface_2` | 7.10 | 8.35 | 4.5 |
| `err_fg` | `surface_3` | 6.49 | 7.26 | 4.5 |
| `fg` | `accent_bg` | 17.24 | 13.39 | 4.5 |
| `fg` | `canvas` | 18.97 | 18.97 | 4.5 |
| `fg` | `sidebar` | 18.48 | 18.23 | 4.5 |
| `fg` | `surface` | 19.80 | 17.18 | 4.5 |
| `fg` | `surface_2` | 17.84 | 15.61 | 4.5 |
| `fg` | `surface_3` | 16.31 | 13.57 | 4.5 |
| `fg_2` | `accent_bg` | 6.80 | 6.52 | 4.5 |
| `fg_2` | `canvas` | 7.49 | 9.23 | 4.5 |
| `fg_2` | `sidebar` | 7.29 | 8.87 | 4.5 |
| `fg_2` | `surface` | 7.81 | 8.36 | 4.5 |
| `fg_2` | `surface_2` | 7.04 | 7.60 | 4.5 |
| `fg_2` | `surface_3` | 6.44 | 6.60 | 4.5 |
| `info_fg` | `accent_bg` | 7.47 | 7.51 | 4.5 |
| `info_fg` | `canvas` | 8.22 | 10.63 | 4.5 |
| `info_fg` | `info_bg` | 7.53 | 7.87 | 4.5 |
| `info_fg` | `surface` | 8.58 | 9.63 | 4.5 |
| `info_fg` | `surface_2` | 7.73 | 8.75 | 4.5 |
| `info_fg` | `surface_3` | 7.07 | 7.60 | 4.5 |
| `log_dbg` | `log_bg` | 7.57 | 7.58 | 4.5 |
| `log_dim` | `log_bg` | 7.02 | 7.02 | 4.5 |
| `log_dim` | `log_err_bg` | 5.18 | 5.41 | 4.5 |
| `log_dim` | `log_wrn_bg` | 5.29 | 5.58 | 4.5 |
| `log_err` | `log_bg` | 8.25 | 8.88 | 4.5 |
| `log_err` | `log_err_bg` | 6.08 | 6.84 | 4.5 |
| `log_fg` | `log_bg` | 13.99 | 15.05 | 4.5 |
| `log_fg` | `log_err_bg` | 10.31 | 11.60 | 4.5 |
| `log_fg` | `log_wrn_bg` | 10.53 | 11.97 | 4.5 |
| `log_inf` | `log_bg` | 10.13 | 10.90 | 4.5 |
| `log_wrn` | `log_bg` | 11.43 | 12.30 | 4.5 |
| `log_wrn` | `log_wrn_bg` | 8.60 | 9.78 | 4.5 |
| `ok_fg` | `accent_bg` | 7.65 | 7.89 | 4.5 |
| `ok_fg` | `canvas` | 8.42 | 11.17 | 4.5 |
| `ok_fg` | `ok_bg` | 7.80 | 8.04 | 4.5 |
| `ok_fg` | `surface` | 8.79 | 10.11 | 4.5 |
| `ok_fg` | `surface_2` | 7.92 | 9.19 | 4.5 |
| `ok_fg` | `surface_3` | 7.24 | 7.99 | 4.5 |
| `on_accent` | `accent` | 6.67 | 6.30 | 4.5 |
| `on_accent` | `accent_hover` | 8.25 | 7.70 | 4.5 |
| `on_accent` | `accent_pressed` | 10.14 | 5.13 | 4.5 |
| `on_danger` | `danger` | 6.16 | 6.63 | 4.5 |
| `on_danger` | `danger_hover` | 7.57 | 7.85 | 4.5 |
| `on_danger` | `danger_pressed` | 9.24 | 9.60 | 4.5 |
| `placeholder` | `accent_bg` | 4.78 | 5.22 | 4.5 |
| `placeholder` | `canvas` | 5.26 | 7.39 | 4.5 |
| `placeholder` | `surface` | 5.49 | 6.69 | 4.5 |
| `placeholder` | `surface_2` | 4.95 | 6.08 | 4.5 |
| `warn_fg` | `accent_bg` | 7.16 | 8.52 | 4.5 |
| `warn_fg` | `canvas` | 7.88 | 12.07 | 4.5 |
| `warn_fg` | `surface` | 8.22 | 10.93 | 4.5 |
| `warn_fg` | `surface_2` | 7.41 | 9.93 | 4.5 |
| `warn_fg` | `surface_3` | 6.77 | 8.63 | 4.5 |
| `warn_fg` | `warn_bg` | 7.44 | 8.36 | 4.5 |

### Non-text (ratios, light and dark)

| Foreground | Background | Light | Dark | Need |
|---|---|---:|---:|---:|
| `accent` | `accent_bg` | 5.81 | 4.54 | 3 |
| `accent` | `canvas` | 6.39 | 6.43 | 3 |
| `accent` | `surface` | 6.67 | 5.82 | 3 |
| `accent` | `surface_2` | 6.01 | 5.29 | 3 |
| `accent` | `surface_3` | 5.49 | 4.60 | 3 |
| `border_strong` | `accent_bg` | 3.44 | 3.84 | 3 |
| `border_strong` | `canvas` | 3.78 | 5.44 | 3 |
| `border_strong` | `err_bg` | 3.54 | 3.86 | 3 |
| `border_strong` | `info_bg` | 3.46 | 4.02 | 3 |
| `border_strong` | `ok_bg` | 3.50 | 3.92 | 3 |
| `border_strong` | `sidebar` | 3.69 | 5.23 | 3 |
| `border_strong` | `surface` | 3.95 | 4.92 | 3 |
| `border_strong` | `surface_2` | 3.56 | 4.48 | 3 |
| `border_strong` | `surface_3` | 3.25 | 3.89 | 3 |
| `border_strong` | `warn_bg` | 3.58 | 3.77 | 3 |
| `danger_edge` | `canvas` | 5.90 | 9.40 | 3 |
| `danger_edge` | `sidebar` | 5.75 | 9.03 | 3 |
| `danger_edge` | `surface` | 6.16 | 8.51 | 3 |
| `err_solid` | `accent_bg` | 5.36 | 4.44 | 3 |
| `err_solid` | `canvas` | 5.90 | 6.28 | 3 |
| `err_solid` | `surface` | 6.16 | 5.69 | 3 |
| `err_solid` | `surface_2` | 5.55 | 5.17 | 3 |
| `err_solid` | `surface_3` | 5.07 | 4.49 | 3 |
| `info_solid` | `accent_bg` | 4.28 | 4.84 | 3 |
| `info_solid` | `canvas` | 4.71 | 6.86 | 3 |
| `info_solid` | `surface` | 4.92 | 6.21 | 3 |
| `info_solid` | `surface_2` | 4.43 | 5.64 | 3 |
| `info_solid` | `surface_3` | 4.05 | 4.90 | 3 |
| `ok_solid` | `accent_bg` | 4.52 | 5.62 | 3 |
| `ok_solid` | `canvas` | 4.97 | 7.96 | 3 |
| `ok_solid` | `surface` | 5.19 | 7.21 | 3 |
| `ok_solid` | `surface_2` | 4.67 | 6.55 | 3 |
| `ok_solid` | `surface_3` | 4.27 | 5.70 | 3 |
| `on_accent` | `accent` | 6.67 | 6.30 | 3 |
| `ring` | `accent_bg` | 5.81 | 4.54 | 3 |
| `ring` | `canvas` | 6.39 | 6.43 | 3 |
| `ring` | `err_bg` | 5.97 | 4.57 | 3 |
| `ring` | `info_bg` | 5.85 | 4.76 | 3 |
| `ring` | `ok_bg` | 5.92 | 4.63 | 3 |
| `ring` | `sidebar` | 6.22 | 6.18 | 3 |
| `ring` | `surface` | 6.67 | 5.82 | 3 |
| `ring` | `surface_2` | 6.01 | 5.29 | 3 |
| `ring` | `surface_3` | 5.49 | 4.60 | 3 |
| `ring` | `warn_bg` | 6.04 | 4.46 | 3 |
| `ring_inverse` | `log_bg` | 6.11 | 6.58 | 3 |
| `warn_solid` | `accent_bg` | 3.82 | 7.35 | 3 |
| `warn_solid` | `canvas` | 4.20 | 10.41 | 3 |
| `warn_solid` | `surface` | 4.39 | 9.43 | 3 |
| `warn_solid` | `surface_2` | 3.95 | 8.57 | 3 |
| `warn_solid` | `surface_3` | 3.61 | 7.45 | 3 |

## 4. Typography

One stack, in the mock and in `tokens.json`: `Inter, "Segoe UI Variable Text", "Segoe UI", -apple-system, BlinkMacSystemFont, "SF Pro Text", Roboto, "Helvetica Neue", Arial, sans-serif`; mono: `"JetBrains Mono", "Cascadia Mono", "SF Mono", ui-monospace, Menlo, Consolas, monospace`. Production bundles **Inter** and **JetBrains Mono** (SIL OFL), static weights **400, 600, 700 only**, so GPUI renders identically on Windows, macOS and Linux. The mock renders with whichever of these the viewing machine has, so line breaks are indicative. **Tabular numerals (`tnum`) on every text style.** **Sentence case; no all-caps** (acronyms and codes such as RAID, DSM, SSH, SMART and the log levels DBG, INF, WRN, ERR are the only capitals). **Minimum 12 px**, enforced by lint. Text scale setting 100 / 115 / 130%: every size and line height scales, spacing does not; the layouts above were checked at 130% on 960 x 640.

| Style | Size/line (px) | Weight | Tracking | Used for |
|---|---|---:|---|---|
| `display` | 28/34 | 700 | -0.02em | Metric numbers, big status words |
| `h1` | 24/30 | 700 | -0.015em | Screen title |
| `h2` | 16/21 | 600 | 0 | Card and section titles |
| `body` | 14/21 | 400 | 0 | Default text, descriptions |
| `body_sm` | 13/19 | 400 | 0 | Table cells, helper text, button labels (600) |
| `caption` | 12/16 | 400 | 0 | Badges (600), field hints, table headers (600) |
| `mono` | 13/20 | 400 | 0 | IPs, MACs, device paths, fingerprints, commands |
| `log` | 12/20 | 400 | 0 | Log lines |

## 5. Spacing, radius, elevation, layout

- **Spacing (px):** 2 · 4 · 6 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 48. Card padding 16, page padding 24 (12 on phones), gap between cards 16.
- **Controls:** 28 / 36 / 42 px; 44 px for every control in the web plane below 700 px. Table cell padding 12 x 16. Radii: 6 (chips, checkbox), 8 (buttons, inputs, banners), 12 (cards, dialogs, window), pill (badges, progress, step discs).
- **Elevation:** `e1` `0 1px 2px rgb(0 0 0 / .05)` cards; `e3` `0 24px 48px -12px rgb(0 0 0 / .5)` windows and dialogs; menus use `e2`. No inset shadows.
- **Desktop window (min 960 x 640):** title bar 38 px (with the "Mockup · sample data" tag in the mocks), stepper 48 px, content (one scroll area, 24 px padding), pinned action bar 64 px, safety rail 216 px fixed on the right. Below 1100 px the stepper shows only the current step's label; below 1000 px two-column grids become one column and secondary table columns drop. The Compatibility column of Find never drops.
- **Web plane:** sidebar 224 px, top bar 52 px, status strip 36 px (worst component wins: "Needs attention"), persistent rollback banner under the strip, content scroll area. Below 700 px: bottom tab bar, tiles in two columns, tables as cards, docked panels stacked, controls 44 px. Docked edit panels (320 px) on wide layouts.
- **Scrollbars:** 12 px hit area, thin thumb in `border_strong` on a transparent track; keyboard scrolling must work on every scroll area.
- **OS chrome:** custom title bar; controls on the right for Windows and Linux, on the left (native traffic lights) for macOS. The mock draws neutral stand-ins.

| Radius | px |
|---|---:|
| `sm` | 6 |
| `md` | 8 |
| `lg` | 12 |
| `pill` | 999 |

| Elevation | Value |
|---|---|
| `e0` | `none` |
| `e1` | `0 1px 2px rgb(0 0 0 / 0.05)` |
| `e2` | `0 4px 12px -2px rgb(0 0 0 / 0.14)` |
| `e3` | `0 24px 48px -12px rgb(0 0 0 / 0.5)` |

## 6. Components and states

Every component below has default, hover, pressed, focus-visible, disabled and (where it applies) error and loading drawn in `states.html`. **Focus** is always a 2 px `ring` outline with a 2 px offset (inside the always-dark log it is `ring_inverse`); the parents of edge controls must reserve room for it (no `overflow: hidden` clipping the ring). **Hover** moves one surface step (`surface_2`), **pressed** two (`surface_3`).

- **Buttons** (36 px, radius 8, 13 px / 600). *Primary*: `accent` fill, `on_accent` label; hover `accent_hover`, pressed `accent_pressed`; one per screen. *Secondary*: `surface`, 1 px `border_strong`. *Ghost*: no border. *Destructive*: `danger` fill, `on_danger` label (white in both themes), 2 px `danger_edge` outline, trash icon, verb + object label ("Erase DS918+ and install Debian"), 700 weight; only data-loss actions; hover `danger_hover`, pressed `danger_pressed`. *Disabled* (all variants): `surface_2` fill, dashed `border_strong`, `fg_2` label, no red. *Loading*: busy glyph (static under reduced motion) plus a present-tense label ("Starting installer…"), clicks ignored, `aria-busy`. Back and Cancel next to the destructive button are secondary and ghost; there is no violet fill on the gate.
- **Inputs** (36 px, 1 px `border_strong`, radius 8). Hover: border `fg_2`. Focus: ring. Error: 2 px `err_solid` border + `err_fg` helper with a drawn icon. Match: 2 px `ok_solid` + `ok_fg` helper. Disabled: dashed. Placeholder: `placeholder`. Mono for hosts, IPs, serials, fingerprints, the typed phrase.
- **Typed phrase** (the gate). Phrase `ERASE <MODEL>` (here `ERASE DS918+`), case-sensitive, **paste allowed**, label states what to type; live status line (`role=status`): "N of 12 characters match. Keep typing." (neutral), "Character N differs." (error, never echoes the expected text), "Matches." (ok). The erase button enables only when both boxes are ticked and the phrase matches.
- **Checkbox** 20 px, 2 px `border_strong`, checked = `accent` + check; whole label is the hit area. **Radio card**: 20 px dot, 1 px `border_strong`; selected = `accent` border + `accent_bg`. **Segmented control** and **tabs**: pressed/selected = `accent` fill / underline; hover `surface_2`.
- **Badges** (24 px, pill, 12 px / 600, 14 px drawn icon, 1 px opaque `*_border`). Compatibility set: Verified = shield-check (`ok`), Community-reported = users (`info`), Untested = dashed-circle help (neutral, **dashed border**), Known issues = triangle (`warn`), Unsupported = ban (`err`). Verified appears only in the legend until a model earns it. Vendor chips (Intel, AMD, ARM): outline chip + mono architecture tag, no logos.
- **Cards** (`surface`, 1 px `border`, radius 12, `e1`); header 12 x 16 with a 1 px bottom border. **Tiles**: caption + icon, 28 px value, 12 px sub-line, optional 48 px sparkline. **Tables** (13 px, header `surface_2`); selected row = `accent_bg` + 4 px `accent` child edge; rows are focusable (`tabindex`) with a ring.
- **Safety rail** (216 px, `sidebar`): status box (icon + title + one sentence), four-level meter (Reading only, Plan prepared, Boot flash changed, Disks changed; passed levels are checks), "Can I cancel?", Target (model, IP, serial), Planned, "Your way back". The status box is neutral for read-only and plan states, amber for "Boot flash changed", "Disks modified" and "Failed, disks untouched", green for "Debian running". It never uses red.
- **Progress** (12 px pill with a 1 px `border_strong` outline, fill `accent`, `ok_solid` when done, `warn_solid` while degraded such as a RAID resync, `err_solid` on failure; text value beside it). **Stepper** (24 px discs: done = filled `accent` + check, current = `accent` ring + tint + label, failed = `err` disc "!" with the step name kept). **Phase pills** (Connecting, Inspecting, Staging, Booting installer, Installing, Waiting for reboot, Verifying) and the 12 installer steps share the disc language; the **point of no return** is a dashed rule with a flag, between steps 2 and 3, reading "passed hh:mm:ss" afterwards.
- **Banners** (radius 8, tint + opaque border, drawn icon, bold title + sentence, actions right; `role=alert` for warn and err). The data-loss banner on the gate is the neutral kind (`surface_2`, `border_strong` outline, no red); unit identity sits in a 2 px `fg` outlined card.
- **Dialog** (520 px, radius 12, `e3`, scrim black at 0.60). **Log viewer** (always dark; 12 px mono; 72 px time, 64 px level, message; level = drawn glyph + word + colour; warn and error lines get opaque tinted rows and a 3 px edge; segmented filter with cumulative counts; Copy and Open log file buttons). **Slider** (8 px outlined track, 20 px thumb, dashed maximum marker with its value above, numeric field and error tag beside it, keyboard arrows). **Disk diagram** (layers: Disks, Partitions, RAID, LVM and file systems; 12 px text; problems are an outline and a drawn icon plus a text label, never a hatch; scrolls horizontally inside its card below 640 px).
- **Secrets** (Done): hidden by default, "Show" reveals for 30 s, "Copy" always; Finish needs "I have copied the password".

## 7. Iconography

lucide-style outline icons, 24 grid, 2 px stroke, round caps. Sizes 12 (log), 14 (badges), 16, 20, 28. The check and x glyphs use a 3 px stroke inside discs. Status glyphs: shield-check, users, dashed-circle help, triangle, ban, octagon-x, check, x, info, lock, flag. Never meaning by icon alone in status contexts. Production: one `.svg` file per icon, recoloured by text colour. (The mock draws them with CSS masks so the folder needs no assets.)

## 8. Motion

Colour and opacity transitions only, 120 ms; progress width 200 ms; dialogs fade 120 ms, no slide. The only loop is the 1.2 s linear busy glyph; with reduced motion it is static and the word "running" or "Starting installer…" carries the meaning. The countdown updates once per second without easing and is exposed to assistive technology at 10 s and 5 s, not every second.

| Token | ms | Used for |
|---|---:|---|
| `instant` | 0 | Focus, checkbox state |
| `fast` | 120 | Hover, pressed, dialog fade |
| `base` | 200 | Progress, banner appear |
| `spinner` | 1200 | Busy glyph (linear) |

## 9. GPUI implementation map

Tokens become one `Palette` global, switched at runtime (light, dark, follow OS). `tokens.json` is the only place a colour is edited; a `build.rs` emits the struct, and `tools/contrast.py` runs in CI. Alpha: only `scrim` (`black.opacity(SCRIM_ALPHA)`).

```rust
// crates/synofurb-ui/src/theme.rs (generated from tokens.json by build.rs; do not hand-edit)
use gpui::{rgb, Global, Hsla};
fn c(hex: u32) -> Hsla { rgb(hex).into() }          // opaque sRGB hex to Hsla, once at startup
pub const SCRIM_ALPHA: f32 = 0.60;                      // scrim = black.opacity(SCRIM_ALPHA)

#[derive(Clone)]
pub struct Palette {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub sidebar: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub fg: Hsla,
    pub fg_2: Hsla,
    pub placeholder: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_pressed: Hsla,
    pub on_accent: Hsla,
    pub accent_bg: Hsla,
    pub accent_fg: Hsla,
    pub accent_border: Hsla,
    pub ring: Hsla,
    pub ring_inverse: Hsla,
    pub danger: Hsla,
    pub danger_hover: Hsla,
    pub danger_pressed: Hsla,
    pub danger_edge: Hsla,
    pub on_danger: Hsla,
    pub ok_solid: Hsla,
    pub ok_fg: Hsla,
    pub ok_bg: Hsla,
    pub ok_border: Hsla,
    pub warn_solid: Hsla,
    pub warn_fg: Hsla,
    pub warn_bg: Hsla,
    pub warn_border: Hsla,
    pub err_solid: Hsla,
    pub err_fg: Hsla,
    pub err_bg: Hsla,
    pub err_border: Hsla,
    pub info_solid: Hsla,
    pub info_fg: Hsla,
    pub info_bg: Hsla,
    pub info_border: Hsla,
    pub log_bg: Hsla,
    pub log_fg: Hsla,
    pub log_dim: Hsla,
    pub log_dbg: Hsla,
    pub log_inf: Hsla,
    pub log_wrn: Hsla,
    pub log_err: Hsla,
    pub log_wrn_bg: Hsla,
    pub log_err_bg: Hsla,
}
impl Global for Palette {}
impl Palette {
    pub fn light() -> Self {
        Self {
            canvas: c(0xFAFAFA),
            surface: c(0xFFFFFF),
            surface_2: c(0xF3F3F3),
            surface_3: c(0xE9E9E9),
            sidebar: c(0xF7F7F7),
            border: c(0xE5E5E5),
            border_strong: c(0x808080),
            fg: c(0x0A0A0A),
            fg_2: c(0x525252),
            placeholder: c(0x696969),
            accent: c(0x7139CB),
            accent_hover: c(0x622CB5),
            accent_pressed: c(0x541EA0),
            on_accent: c(0xFFFFFF),
            accent_bg: c(0xF1EDFE),
            accent_fg: c(0x5C29AA),
            accent_border: c(0xB193E4),
            ring: c(0x7139CB),
            ring_inverse: c(0xA37BFE),
            danger: c(0xC60014),
            danger_hover: c(0xAC0311),
            danger_pressed: c(0x94020D),
            danger_edge: c(0xC60014),
            on_danger: c(0xFFFFFF),
            ok_solid: c(0x047E39),
            ok_fg: c(0x005725),
            ok_bg: c(0xDEF8E2),
            ok_border: c(0x66B585),
            warn_solid: c(0xA76C01),
            warn_fg: c(0x6F4606),
            warn_bg: c(0xFFF3D3),
            warn_border: c(0xCFA95F),
            err_solid: c(0xC60014),
            err_fg: c(0xA70311),
            err_bg: c(0xFEEFEF),
            err_border: c(0xDF6C77),
            info_solid: c(0x1570D1),
            info_fg: c(0x004B96),
            info_bg: c(0xE7F1FF),
            info_border: c(0x73AAE6),
            log_bg: c(0x121114),
            log_fg: c(0xDEDEDE),
            log_dim: c(0x9E9E9E),
            log_dbg: c(0x9DA5B1),
            log_inf: c(0x89C3FE),
            log_wrn: c(0xF7C243),
            log_err: c(0xFE8A88),
            log_wrn_bg: c(0x322A1B),
            log_err_bg: c(0x3C2729),
        }
    }

    pub fn dark() -> Self {
        Self {
            canvas: c(0x0A0A0A),
            surface: c(0x171717),
            surface_2: c(0x202020),
            surface_3: c(0x2B2B2B),
            sidebar: c(0x101010),
            border: c(0x2A2A2A),
            border_strong: c(0x868686),
            fg: c(0xFAFAFA),
            fg_2: c(0xB1B1B1),
            placeholder: c(0x9E9E9E),
            accent: c(0xA37BFE),
            accent_hover: c(0xB091FE),
            accent_pressed: c(0x946AEE),
            on_accent: c(0x0E0C15),
            accent_bg: c(0x31254D),
            accent_fg: c(0xC8B7FF),
            accent_border: c(0x6A50A6),
            ring: c(0xA37BFE),
            ring_inverse: c(0xA37BFE),
            danger: c(0xB7191C),
            danger_hover: c(0xA50E15),
            danger_pressed: c(0x8D0B11),
            danger_edge: c(0xFF9592),
            on_danger: c(0xFFFFFF),
            ok_solid: c(0x37BB62),
            ok_fg: c(0x6ED889),
            ok_bg: c(0x0F3118),
            ok_border: c(0x257D41),
            warn_solid: c(0xF0B135),
            warn_fg: c(0xF1C45E),
            warn_bg: c(0x3D2A02),
            warn_border: c(0x9F741E),
            err_solid: c(0xFC5855),
            err_fg: c(0xFFA09C),
            err_bg: c(0x4F1A18),
            err_border: c(0xAE3C3A),
            info_solid: c(0x539AF2),
            info_fg: c(0x91C1FF),
            info_bg: c(0x132946),
            info_border: c(0x3667A5),
            log_bg: c(0x060607),
            log_fg: c(0xDEDEDE),
            log_dim: c(0x989898),
            log_dbg: c(0x979FAB),
            log_inf: c(0x89C3FE),
            log_wrn: c(0xF7C243),
            log_err: c(0xFE8A88),
            log_wrn_bg: c(0x28200F),
            log_err_bg: c(0x331E1E),
        }
    }
}
```

Verify every row against the pinned GPUI revision; the API moves.

| Component | GPUI building blocks | Cost |
|---|---|---|
| Window shell: title bar, stepper, content, safety rail, pinned action bar | `div()` flex/grid; one scroll container (`.overflow_y_scroll()` + `ScrollHandle`) per screen above a fixed footer; rail is a second fixed column | Simple |
| Reflow rules (labels hide below 1100 px, columns hide below 1000 px, single column grids) | Read the window width in `render` and branch; no CSS-container equivalent, so these are explicit `if width < px(1100.)` switches | Simple |
| Text styles, tabular numerals, mono | `.text_size()`, `.font_weight()`, `.font_family()`, `.line_height()`; tnum through font features (verify exposure in the pinned GPUI); fonts bundled via `cx.text_system().add_fonts()` | Simple (tnum: verify) |
| Buttons, all variants and states | `div().id().on_click()` with `.hover()`, `.active()`, `.focus_visible()`; disabled = dashed border + `surface_2`, no handler; loading = swap icon and label, ignore clicks | Simple |
| Badges, chips, step discs, phase pills | small `div` with `rounded(px(999.))` + `svg()` + label | Simple |
| Icons | one `.svg` per icon with `stroke="currentColor"`; `svg().path(..).text_color(c)`. The HTML mock uses CSS masks only because it must run from a single folder | Simple |
| Banners, dialog, scrim | `div().absolute().inset_0().bg(black.opacity(0.6))`, centred child; focus trap by hand | Simple (trap: medium) |
| Progress bar, slider track, tracks | nested `div` + `border_1()`; fill width as a fraction | Simple |
| Shadows | `.shadow_sm()` / `BoxShadow`; no inset shadows are used anywhere | Simple |
| Selected-row edge | a 4 px child `div` positioned absolute at the left of the first cell | Simple |
| Tables | header row + `uniform_list`; fixed/flex widths per column; cards below 700 px are a different row renderer | Medium |
| Log viewer: level filter, follow, copy | `uniform_list` over the filtered `Vec<LogLine>`; level icon + word + time columns; no drag-select in labels, so Copy and Open log file are first-class buttons | Medium |
| Sparklines | `canvas()` + `PathBuilder` | Medium |
| Storage diagram | boxes are `div`s; connectors drawn in one `canvas()` using measured bounds; 12 px text minimum so the diagram scrolls horizontally inside its card below 640 px | Medium to costly |
| Slider | custom element (track, thumb, mouse + keyboard: arrows, Shift = 10x, Home/End); value shown in a numeric field next to it | Medium |
| Text input (typed phrase, host, user, password) | no stock widget: build or adopt a single-line editor (caret, selection, clipboard, IME, undo, password variant, paste allowed). The gate depends on it: budget it first | Costly |
| Checkbox, radio card, segmented control, tabs | `div` states + roving focus; whole card is the hit area | Simple |
| Secret reveal and copy | text swap, `cx.write_to_clipboard()`, hide after 30 s via `cx.spawn` | Simple |
| Scrollbars | thin custom thumb in `border_strong`, 12 px hit area; keyboard scroll (PgUp/PgDn/Home/End) wired by hand | Medium |
| Window chrome per OS | custom titlebar; controls right on Windows/Linux, left on macOS (native traffic lights); Linux CSD varies by compositor | Medium |
| Screen-reader tree | not provided by GPUI to my knowledge; **a one-day spike with NVDA, Narrator and VoiceOver on a button, a text field, a checkbox and a live region is still to be done** | Costly / unknown |

Open GPUI risks, unchanged by this revision: the accessibility tree (spike not done), the text input, and text selection (hence Copy buttons everywhere a fingerprint, path, command or log line must be copied).

## 10. Revision log (UI critic review)

| Id | Finding | Change |
|---|---|---|
| B1 | Destructive button never drawn enabled; red appears five times on the gate; dark label 4.63:1 | Drawn in every state (states sheet, both themes) and live on the Confirm screen (7a disabled, 7b enabled, loading on click). Disabled destructive = neutral dashed control, not red. Banner and "preserved/changed" lists are neutral; red appears only on the enabled button (2 px light-red outline in dark) and the 12 px marker + heading of "Will be destroyed": measured with a script, 3 elements on 7b, 2 on 7a, both themes. Dark button is now `#B7191C` with a white label = 6.63:1 (hover 7.85, pressed 9.60), outlined in `#FF9592` (8.51:1 on the card). Back and Cancel are outline and ghost buttons; there is no violet fill on the gate. |
| B2 | 960 x 640 never rendered; Compatibility column clipped; no pinned bar or scroll state | Window is a fixed-size frame (960 x 640, 1180 x 760, 1440 x 900, switchable) with one scroll area, a pinned 64 px action bar and a fixed 216 px safety rail. Find uses three columns at every width; the stepper drops labels below 1100 px; grids stack below 1000 px; no horizontal scroll at any size (measured). Scrolled state: "Scroll to end" button and `?scroll=end`. 130% text: checked on Find and Confirm at 960 x 640. Windows, macOS and Linux chrome switch. |
| B3 | Web plane did not reflow at 390 px; Keep button off screen; 36 px targets | Web frame is now a container: below 700 px the sidebar becomes a bottom tab bar (56 px items), tiles go to two columns, tables become cards, the docked panels stack, forms go single column. The safe-apply banner is sticky under the strip, stacks, and both buttons are full width and 44 px (measured: no control under 44 px at 390). W7 to W9 show the same markup at 390; a viewport switch (1180 / 768 / 390) applies to every web screen. |
| B4 | Placeholder 4.29:1; bars and tracks under 3:1; borders on tints; ring in the log; checker blind | `placeholder` is its own token, 4.78:1 or better on every surface it can sit on (light fg_3 is gone). Tracks have a 1 px `border_strong` outline (3.25:1 or better on every surface); fills are `*_solid` at 3.61:1 or better on the track. `border_strong` is 3.25:1 or better on all ten surfaces including tints. `ring_inverse` is used inside the log panel (6.11:1 and 6.58:1). `tools/contrast.py` now checks 472 pairs (228 token matrix over both themes, 100 accent-variant, 144 extracted from the pairs that really occur in `styles.css` rules), fails on any colour token that takes part in no test, and lints the files. It was run against deliberately bad tokens and fails as it should. |
| B5 | Mock and doc disagreed on gradients, hatch, inset shadows, 12 px floor, all-caps, tabular figures, fonts | One source of truth: the doc. No gradient, pattern, inset shadow, `color-mix` or `backdrop-filter` anywhere (disabled = dashed border and flat fill; overflow = outline and icon; selected-row edge = child element; tints are opaque tokens). Type floor is 12 px (diagram text 12 px, diagram scrolls instead of shrinking); the lint fails on anything smaller. All-caps labels are not allowed (acronyms and log codes only). `tnum` on the whole UI, so counters, the countdown and tile values do not jitter. One font stack, copied into `tokens.json`; weights 400, 600, 700 only. |
| B6 | No states, no real controls, no focus demonstration | Real `<button>`, `<input>` (text, checkbox, radio, range), `tabindex` rows and `:focus-visible`. `states.html` (also embedded in the viewer) shows every component in default, hover, pressed, focus, disabled, error and loading in light and dark side by side, plus the typed phrase in empty, partial, mismatch and match. On the real Confirm screen Tab shows the ring on the checkboxes and the phrase field. Row, nav, radio card, tab and segment hover and pressed states are styled. |
| Accents | ForgeVault accent claim (green and amber fail on white) | ForgeVault values re-measured: purple .55 = 5.38:1 with a white label, blue 4.93, rose 4.79, green 3.99 (fails), amber 2.78 (fails as fill, ring and label). All five accents are offered on the web plane only with the derived sets in the Accents table below (white label in light, near-black label in dark; amber is darkened to a deep amber), each tested in both themes by `contrast.py`. Desktop app: violet only. |
| UX alignment | Match the revised UX mock | Top stepper + fixed safety rail (Reading only, Plan prepared, Boot flash changed, Disks changed); typed phrase `ERASE DS918+`, paste allowed, mismatch hint does not echo the phrase; the gate shows unit, IP, serial, disk serials and the same-model warning; point of no return is installer step 3 of 12; boot flash is "changed, backed up first"; all data is sample data (tag in every title bar); every unit is Untested except DS923+ (Community-reported) and DS211j (Unsupported); Verified appears only in the legend; failure screens follow `docs/RECOVERY.md` (60 s before step 3, 15 minutes after, DSM loader and Synology’s own reinstall, no promise). |

Non-blocking list:

| # | Resolution |
|---|---|
| 1 hierarchy | Not changed: `fg_2` stays about 7.8:1; secondary text is distinguished by size and weight. Revisit after usability tests. |
| 2 two reds | Kept as two roles of one family: `danger*` for the button, `err_*` for icons, bars and text; documented. |
| 3 double ring | One focus ring everywhere: 2 px outline, 2 px offset. |
| 4 paste | Paste allowed (UX decision); the match is exact and case-sensitive. |
| 5 log counts | Filter counts are cumulative minimum levels (Debug = all lines). |
| 6 level glyphs | Drawn SVG glyphs plus the word. |
| 7 failed stepper | The failed step keeps its name and turns red with a "!" disc. |
| 8 goal cards | Dot bullets; nothing preselected. |
| 9 amber objects | Not changed; one amber banner per page is the rule. |
| 10 phone frame | No phone furniture: the real layout at true width. |
| 11 title bar | Windows, macOS (left, neutral dots) and Linux switch; no OS name in the rail. |
| 12 scrollbars | Specified (12 px hit area, `border_strong` thumb) and implemented. |
| 13 slider | Error tag sits next to the field and the slider. |
| 14 dark border on tint | 3.84:1 now. |
| 15 track | Outlined track, solids 3.61:1 or better. |
| 16 accents | See Accents. |
| 17 fonts | One stack, weights 400/600/700, bundling stated. |
| 18 resemblance | Documented as a derivative of ForgeVault tokens: pill badges adopted, heavier input border kept (3:1 boundary) and proposed upstream, focus ring differs (2 px, offset 2). |
| 19 dark dialog | Update dialog checked in dark. |
| 20 mobile labels | 12 px. |

**Still open:** the GPUI accessibility and text-input spike; real Inter / JetBrains Mono metrics (the mock uses installed fonts); Windows and Linux chrome fidelity (only the control position is switched); the screens listed under "Not drawn" at the top; real screen-reader, colour-blindness and user testing.
