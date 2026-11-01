# synofurb design system (recommended: Direction 1, "ForgeVault family")

One visual language for both surfaces: the native desktop installer (Rust + GPUI) and the NAS management web UI on `https://<ip>:8443`.
It is deliberately the ForgeVault look (shadcn-style neutrals, a single violet accent, hairline borders, 8-12 px radii, lucide-style strokes) with four additions that this product needs: a **status system that never relies on colour alone**, a **data-loss language** (destroyed vs preserved), a **log/severity language**, and a **storage-diagram language**.

Live reference: `direction-1/index.html` (all 17 screens, light and dark). Machine-readable tokens: `tokens.json`.

## 1. Principles

1. **Calm by default, loud only for data loss.** 95% of the UI is neutral. Red fills are reserved for the destructive button, the data-loss banner and errors. Accent violet means "primary action / selected / progress", never "status".
2. **Never colour alone.** Every status is colour + icon + word, and uses a distinct shape or border style where it matters (Untested is dashed, Unsupported has a ban glyph, errors carry a 3 px edge or a bold border).
3. **Say what will happen to the data.** Every action that writes names its effect up front (Erases / Keeps / Depends; "Will be destroyed" next to "Will be preserved").
4. **Same tokens on both surfaces.** The web UI is the desktop app's design with a sidebar instead of a step rail. A token change is one JSON edit.
5. **Cheap to build in GPUI.** Flat fills, 1 px borders, small radii, one soft shadow, monochrome SVG icons. No blur, no inset shadows, no gradients in the base set.

## 2. Colour tokens

Opaque sRGB hex values (no alpha tricks) so they map 1:1 to GPUI `rgb(0x......)` and to CSS custom properties. Source values are OKLCH, derived from ForgeVault's `app/globals.css` (neutral ramp `oklch(L 0 0)`; accent hue 295). Deltas from ForgeVault: accent darkened from `oklch(0.55 0.22 295)` to `oklch(0.50 0.21 295)` (white label ratio 6.67 instead of 5.38, leaves room for a visible hover), and the purple-tinted `accent_fg` / `accent_bg` pair added for selected rows. The accent can be swapped for ForgeVault's blue / green / amber / rose in the **web** plane; only violet has been contrast-verified here (re-run the check, section 3, for any other accent).

| Token | Light | Dark | Role |
|---|---|---|---|
| `canvas` | `#FAFAFA` | `#0A0A0A` | Window / page background |
| `surface` | `#FFFFFF` | `#171717` | Cards, dialogs, inputs, popovers |
| `surface_2` | `#F3F3F3` | `#202020` | Sunken areas: table header, code chips, hover |
| `surface_3` | `#E9E9E9` | `#2B2B2B` | Track fills (progress, slider), pressed, stage |
| `border` | `#E5E5E5` | `#2A2A2A` | Decorative hairlines (cards, row dividers) |
| `border_strong` | `#868686` | `#747474` | Interactive boundaries: inputs, checkboxes, outline buttons (>= 3:1) |
| `fg` | `#0A0A0A` | `#FAFAFA` | Primary text |
| `fg_2` | `#525252` | `#ABABAB` | Secondary text, labels |
| `fg_3` | `#7A7A7A` | `#808080` | Placeholder, disabled hints (never essential info) |
| `sidebar` | `#F7F7F7` | `#101010` | Step rail / sidebar background |
| `overlay` | `#0A0A0A` | `#000000` | Dialog scrim base (used at 55% alpha) |
| `accent` | `#7139CB` | `#A37BFE` | Primary button fill, selection edge, progress, focus ring |
| `accent_hover` | `#622CB5` | `#B091FE` | Primary button hover |
| `on_accent` | `#FFFFFF` | `#0E0C15` | Text/icon on accent |
| `accent_bg` | `#F1EDFE` | `#31254D` | Selected row / card tint, step current |
| `accent_fg` | `#5C29AA` | `#C3AFFE` | Accent-coloured text and links |
| `ring` | `#7139CB` | `#A37BFE` | Focus ring (2 px, 2 px offset) |
| `danger` | `#C60014` | `#E23439` | Destructive button fill (only the data-loss actions) |
| `danger_hover` | `#AC0311` | `#F14D4C` | Destructive hover |
| `on_danger` | `#FFFFFF` | `#0C0403` | Text/icon on danger |
| `ok_solid` | `#03A14A` | `#37BB62` | Success icon / bar / dot |
| `ok_fg` | `#005725` | `#6ED889` | Success text |
| `ok_bg` | `#DEF8E2` | `#0F3118` | Success badge / banner fill |
| `warn_solid` | `#C68206` | `#F0B135` | Warning icon / bar / dot |
| `warn_fg` | `#6F4606` | `#F1C45E` | Warning text |
| `warn_bg` | `#FFF3D3` | `#3D2A02` | Warning badge / banner fill |
| `err_solid` | `#E30117` | `#F4514F` | Error icon / bar / dot |
| `err_fg` | `#A70311` | `#FFA09C` | Error text |
| `err_bg` | `#FEEFEF` | `#4F1A18` | Error badge / banner fill |
| `info_solid` | `#3280DD` | `#539AF2` | Info icon / bar / dot |
| `info_fg` | `#004B96` | `#91C1FF` | Info text |
| `info_bg` | `#E7F1FF` | `#132946` | Info badge / banner fill |
| `log_bg` | `#121114` | `#060607` | Log viewer background |
| `log_fg` | `#DEDEDE` | `#DEDEDE` | Log message text |
| `log_dim` | `#9E9E9E` | `#989898` | Log timestamps, DEBUG message text |
| `log_dbg` | `#9DA5B1` | `#979FAB` | DBG level label |
| `log_inf` | `#89C3FE` | `#89C3FE` | INF level label |
| `log_wrn` | `#F7C243` | `#F7C243` | WRN level label |
| `log_err` | `#FE8A88` | `#FE8A88` | ERR level label |

Status colour triple (`*_solid`, `*_fg`, `*_bg`): `solid` is for icons, dots and bars (non-text, needs 3:1), `fg` for text on either `bg` or `surface` (needs 4.5:1), `bg` is the tint behind badges and banners. Status mapping:

| Meaning | Triple | Icon | Word | Shape cue |
|---|---|---|---|---|
| Verified / OK / Pass / Clean | `ok` | shield-check / check | Verified, OK, Pass | solid border |
| Community-reported / Info | `info` | users / info | Community-reported | solid border |
| Untested / Off / No cable | neutral (`surface_2`, `fg_2`) | help (dashed circle) / x | Untested | **dashed border** |
| Known issues / Note / Watch | `warn` | alert triangle | Known issues, Note | solid border |
| Unsupported / Fail / Error / Locked | `err` | ban / octagon-x / lock | Unsupported, Fail | solid border, 3 px edge on banners |

## 3. Contrast (WCAG 2.x, computed)

Method: relative luminance per WCAG 2.x (sRGB linearisation, 0.2126/0.7152/0.0722), ratio = (L1 + 0.05) / (L2 + 0.05), computed by `tools/contrast.py` (included; reads `tokens.json`, exits non-zero on failure) from the exact hex values above. Targets: 4.5:1 for text, 3:1 for non-text UI boundaries, focus ring and status glyphs (WCAG 1.4.11). `fg_3` is capped at 3:1 and is only allowed on non-essential hints. **66 of 66 pairs pass in each theme; lowest text ratio 4.63:1, lowest non-text ratio 3.18:1.**

### Light

| Foreground | Background | Ratio | Need | Result | Used for |
|---|---|---:|---:|:---:|---|
| `fg` #0A0A0A | `canvas` #FAFAFA | **18.97** | 4.5 | PASS | Body text on page |
| `fg` #0A0A0A | `surface` #FFFFFF | **19.80** | 4.5 | PASS | Body text on card |
| `fg_2` #525252 | `surface` #FFFFFF | **7.81** | 4.5 | PASS | Secondary text on card |
| `fg_2` #525252 | `surface_2` #F3F3F3 | **7.04** | 4.5 | PASS | Secondary text on sunken/table header |
| `fg_2` #525252 | `canvas` #FAFAFA | **7.49** | 4.5 | PASS | Secondary text on page |
| `fg_2` #525252 | `accent_bg` #F1EDFE | **6.80** | 4.5 | PASS | Secondary text on selected card |
| `fg` #0A0A0A | `accent_bg` #F1EDFE | **17.24** | 4.5 | PASS | Text on selected row |
| `fg_3` #7A7A7A | `surface` #FFFFFF | **4.29** | 3 | PASS | Placeholder / disabled hint (non-essential) |
| `on_accent` #FFFFFF | `accent` #7139CB | **6.67** | 4.5 | PASS | Primary button label |
| `accent_fg` #5C29AA | `surface` #FFFFFF | **8.93** | 4.5 | PASS | Accent text/link on card |
| `accent_fg` #5C29AA | `accent_bg` #F1EDFE | **7.77** | 4.5 | PASS | Accent text on selected row/tint |
| `ok_fg` #005725 | `ok_bg` #DEF8E2 | **7.80** | 4.5 | PASS | Verified / success badge |
| `ok_fg` #005725 | `surface` #FFFFFF | **8.79** | 4.5 | PASS | Success text on card |
| `warn_fg` #6F4606 | `warn_bg` #FFF3D3 | **7.44** | 4.5 | PASS | Warning badge / banner |
| `warn_fg` #6F4606 | `surface` #FFFFFF | **8.22** | 4.5 | PASS | Warning text on card |
| `err_fg` #A70311 | `err_bg` #FEEFEF | **7.06** | 4.5 | PASS | Error badge / banner |
| `err_fg` #A70311 | `surface` #FFFFFF | **7.88** | 4.5 | PASS | Error text on card |
| `info_fg` #004B96 | `info_bg` #E7F1FF | **7.53** | 4.5 | PASS | Info badge / banner |
| `info_fg` #004B96 | `surface` #FFFFFF | **8.58** | 4.5 | PASS | Info text on card |
| `on_danger` #FFFFFF | `danger` #C60014 | **6.16** | 4.5 | PASS | Destructive button label |
| `log_fg` #DEDEDE | `log_bg` #121114 | **13.99** | 4.5 | PASS | Log text |
| `log_dim` #9E9E9E | `log_bg` #121114 | **7.02** | 4.5 | PASS | Log timestamps |
| `log_dbg` #9DA5B1 | `log_bg` #121114 | **7.57** | 4.5 | PASS | Log DEBUG |
| `log_inf` #89C3FE | `log_bg` #121114 | **10.13** | 4.5 | PASS | Log INFO |
| `log_wrn` #F7C243 | `log_bg` #121114 | **11.43** | 4.5 | PASS | Log WARN |
| `log_err` #FE8A88 | `log_bg` #121114 | **8.25** | 4.5 | PASS | Log ERROR |
| `border_strong` #868686 | `surface` #FFFFFF | **3.64** | 3 | PASS | Input / checkbox boundary (WCAG 1.4.11) |
| `ring` #7139CB | `surface` #FFFFFF | **6.67** | 3 | PASS | Focus ring |
| `ring` #7139CB | `canvas` #FAFAFA | **6.39** | 3 | PASS | Focus ring on page |
| `ok_solid` #03A14A | `surface` #FFFFFF | **3.39** | 3 | PASS | Status icon/bar OK |
| `warn_solid` #C68206 | `surface` #FFFFFF | **3.18** | 3 | PASS | Status icon/bar Warn |
| `err_solid` #E30117 | `surface` #FFFFFF | **4.90** | 3 | PASS | Status icon/bar Error |
| `info_solid` #3280DD | `surface` #FFFFFF | **3.99** | 3 | PASS | Status icon/bar Info |

### Dark

| Foreground | Background | Ratio | Need | Result | Used for |
|---|---|---:|---:|:---:|---|
| `fg` #FAFAFA | `canvas` #0A0A0A | **18.97** | 4.5 | PASS | Body text on page |
| `fg` #FAFAFA | `surface` #171717 | **17.18** | 4.5 | PASS | Body text on card |
| `fg_2` #ABABAB | `surface` #171717 | **7.81** | 4.5 | PASS | Secondary text on card |
| `fg_2` #ABABAB | `surface_2` #202020 | **7.10** | 4.5 | PASS | Secondary text on sunken/table header |
| `fg_2` #ABABAB | `canvas` #0A0A0A | **8.62** | 4.5 | PASS | Secondary text on page |
| `fg_2` #ABABAB | `accent_bg` #31254D | **6.09** | 4.5 | PASS | Secondary text on selected card |
| `fg` #FAFAFA | `accent_bg` #31254D | **13.39** | 4.5 | PASS | Text on selected row |
| `fg_3` #808080 | `surface` #171717 | **4.54** | 3 | PASS | Placeholder / disabled hint (non-essential) |
| `on_accent` #0E0C15 | `accent` #A37BFE | **6.30** | 4.5 | PASS | Primary button label |
| `accent_fg` #C3AFFE | `surface` #171717 | **9.29** | 4.5 | PASS | Accent text/link on card |
| `accent_fg` #C3AFFE | `accent_bg` #31254D | **7.24** | 4.5 | PASS | Accent text on selected row/tint |
| `ok_fg` #6ED889 | `ok_bg` #0F3118 | **8.04** | 4.5 | PASS | Verified / success badge |
| `ok_fg` #6ED889 | `surface` #171717 | **10.11** | 4.5 | PASS | Success text on card |
| `warn_fg` #F1C45E | `warn_bg` #3D2A02 | **8.36** | 4.5 | PASS | Warning badge / banner |
| `warn_fg` #F1C45E | `surface` #171717 | **10.93** | 4.5 | PASS | Warning text on card |
| `err_fg` #FFA09C | `err_bg` #4F1A18 | **7.21** | 4.5 | PASS | Error badge / banner |
| `err_fg` #FFA09C | `surface` #171717 | **9.19** | 4.5 | PASS | Error text on card |
| `info_fg` #91C1FF | `info_bg` #132946 | **7.87** | 4.5 | PASS | Info badge / banner |
| `info_fg` #91C1FF | `surface` #171717 | **9.63** | 4.5 | PASS | Info text on card |
| `on_danger` #0C0403 | `danger` #E23439 | **4.63** | 4.5 | PASS | Destructive button label |
| `log_fg` #DEDEDE | `log_bg` #060607 | **15.05** | 4.5 | PASS | Log text |
| `log_dim` #989898 | `log_bg` #060607 | **7.02** | 4.5 | PASS | Log timestamps |
| `log_dbg` #979FAB | `log_bg` #060607 | **7.58** | 4.5 | PASS | Log DEBUG |
| `log_inf` #89C3FE | `log_bg` #060607 | **10.90** | 4.5 | PASS | Log INFO |
| `log_wrn` #F7C243 | `log_bg` #060607 | **12.30** | 4.5 | PASS | Log WARN |
| `log_err` #FE8A88 | `log_bg` #060607 | **8.88** | 4.5 | PASS | Log ERROR |
| `border_strong` #747474 | `surface` #171717 | **3.84** | 3 | PASS | Input / checkbox boundary (WCAG 1.4.11) |
| `ring` #A37BFE | `surface` #171717 | **5.82** | 3 | PASS | Focus ring |
| `ring` #A37BFE | `canvas` #0A0A0A | **6.43** | 3 | PASS | Focus ring on page |
| `ok_solid` #37BB62 | `surface` #171717 | **7.21** | 3 | PASS | Status icon/bar OK |
| `warn_solid` #F0B135 | `surface` #171717 | **9.43** | 3 | PASS | Status icon/bar Warn |
| `err_solid` #F4514F | `surface` #171717 | **5.24** | 3 | PASS | Status icon/bar Error |
| `info_solid` #539AF2 | `surface` #171717 | **6.21** | 3 | PASS | Status icon/bar Info |

Not covered by the table (check in implementation): text over the 55% dialog scrim (dialogs are opaque, so unaffected), hover states (`fg` on `surface_2` is 17.84:1 light / 15.61:1 dark), disabled controls (WCAG-exempt, but the `fg_2` label on `surface_2` is still 7.04:1 light / 7.10:1 dark, so they stay legible).

## 4. Typography

Production fonts: **Inter** (variable) for UI text and **JetBrains Mono** (or IBM Plex Mono) for data, both SIL OFL, bundled in the app so GPUI renders identically on Windows, macOS and Linux; the web UI uses the same families with the system stack as fallback. The mockups use system stacks only (they must open from disk without assets).

| Style | Size/line (px) | Weight | Tracking | Used for |
|---|---|---:|---|---|
| `display` | 28/34 | 650 | -0.02em | Metric numbers (tiles), big status words · tabular-nums |
| `h1` | 24/29 | 650 | -0.015em | Screen title |
| `h2` | 16/21 | 620 | 0 | Card / section title, goal card title |
| `body` | 14/21 | 400 | 0 | Default text, descriptions, form values |
| `body_sm` | 13/19 | 400 | 0 | Table cells, helper text, buttons (600) |
| `caption` | 12/16 | 600 | 0 | Badges, field labels, table headers, window title meta |
| `mono` | 13/20 | 400 | 0 | IPs, MACs, device paths, fingerprints, commands · tabular-nums |
| `log` | 12/20 | 400 | 0 | Log viewer lines (line-height 1.65) · tabular-nums |

Rules: sentence case everywhere; no all-caps in Direction 1. Numbers in tiles, tables and logs use tabular figures. Never go below 12 px. Offer a text scale setting (100 / 115 / 130%) that multiplies every size and line height; layouts must reflow at 130% on a 960 x 640 window.

## 5. Spacing, radius, elevation, density

- **Spacing scale (px):** 2 · 4 · 6 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 48. Base unit 4. Card padding 16, dialog padding 20, page padding 28 (desktop) / 24 (web) / 16 (phone), gap between cards 16, gap inside a form 14.
- **Control heights:** small 28, default 36, large 42; table cell padding 10 x 14 (row about 44 single-line, 52 two-line). Phone controls and tab targets are 44 px.
- **Window:** min 960 x 640, step rail 212 px fixed, content max width not capped (tables breathe), action bar pinned to the bottom of the window with content scrolling above it.

| Radius | Value | Used for |
|---|---:|---|
| `sm` | 6 px | Badges, chips, checkbox, code chips |
| `md` | 8 px | Buttons, inputs, banners, rows, rail items |
| `lg` | 12 px | Cards, dialogs, window |
| `pill` | 999 px | Progress bars, avatar, step dots |

| Elevation | CSS / GPUI | Used for |
|---|---|---|
| `e0` | `none` | Flat: border only (inputs, table, banners) |
| `e1` | `0 1px 2px rgb(0 0 0 / 0.05)` | Cards, tiles |
| `e2` | `0 4px 12px -2px rgb(0 0 0 / 0.14), 0 1px 3px rgb(0 0 0 / 0.08)` | Menus, popovers, select lists |
| `e3` | `0 24px 60px -12px rgb(0 0 0 / 0.5)` | Dialogs (over a 55% scrim) |

In dark mode elevation is expressed by one step lighter surface plus the 1 px border; shadows stay but are nearly invisible, which is fine.

## 6. Components

**Buttons** (height 36, radius 8, label 13 px / 600, icon 16 px + 7 px gap). Variants: *primary* (`accent` fill, `on_accent` label; one per screen, bottom right), *secondary* (`surface`, 1 px `border_strong`), *ghost* (no border, `surface_2` on hover), *destructive* (`danger` fill, `on_danger` label, always with a trash icon and a verb + object label such as "Erase 4 disks and install Debian"; never a bare "OK"; sits to the right of, and never replaces, the safe "Back to plan" primary). Disabled = dashed `border_strong` + `surface_2` fill + `fg_2` label; the destructive button is disabled until every gate item passes. States: hover (`accent_hover` / `danger_hover` / `surface_2`), pressed (same fill as hover, no translate), focus (2 px `ring`, 2 px offset).

**Inputs** (height 36, radius 8, `surface`, 1 px `border_strong`, 13 px). Focus: 2 px `ring` outline. Error: 1 px `err_solid` border + `err_fg` helper text with an icon. Monospace for hosts, IPs, device paths, fingerprints. Labels above the field (13 px / 620), helper text below (12 px, `fg_2`). Checkbox 20 px, radio card with 20 px dot and full-card hit area. Segmented control for 2-4 mutually exclusive filters (log level, next-boot scope).

**Badges** (height 22, radius 6, 12 px / 620, 14 px icon, 1 px border at 50% of the solid colour, tinted `*_bg` fill, `*_fg` text). Compatibility set (exact mapping):

| Status | Icon | Style | Meaning |
|---|---|---|---|
| Verified | shield-check | ok, solid border | Maintainer-tested end to end on this model |
| Community-reported | users | info, solid border | Users report success, not maintainer-tested |
| Untested | help-circle (dashed) | neutral, **dashed border** | No data; read-only checks run first |
| Known issues | alert-triangle | warn, solid border | Works with documented limits |
| Unsupported | ban | err, solid border | Installer refuses to start |

Vendor chips (Intel / AMD / ARM): neutral outline chip, bold vendor name + mono architecture tag (`x86_64`, `armv7`). No vendor logos or brand colours (trademarks, and colour would collide with status).

**Cards and tiles** (`surface`, 1 px `border`, radius 12, `e1`). Card header 12 x 16 padding with a 1 px bottom border, title 14 px / 620, optional status badge right-aligned. Metric tile: caption label + icon (top), 28 px display value with a 13 px unit, 12 px sub-line, optional sparkline (46 px high, 1.8 px stroke, 12% area).

**Tables** (13 px; header `surface_2`, 12 px / 620 `fg_2`; 1 px row dividers; numbers right-aligned, tabular, mono for IDs). Selected row = `accent_bg` fill + 3 px `accent` left edge (not colour alone: the edge is a shape and the row also shows a "selected" summary in the action bar). Max 7 columns on the desktop app; below 960 px the web tables become stacked cards.

**Progress and stepper.** Linear bar 8 px, pill, `surface_3` track, `accent` fill (`ok` when done, `warn` for degraded such as RAID resync, `err` on failure), percentage as text next to it, `role=progressbar`. Step rail (desktop): 22 px numbered circle - done = filled accent with check, current = 2 px accent ring + `accent_bg` row + bold label, upcoming = hollow. The 12-step install list uses the same circles, grouped under four phase captions (Prepare, Install, Configure, Verify), with the **point of no return** as a dashed `err_solid` rule with a flag icon between step 4 and 5; once passed it reads "passed" and the Cancel button becomes disabled with the reason in the action bar.

**Banners** (radius 8, 1 px border at 50% solid, `*_bg` fill, `*_fg` text, 18 px icon, bold title + one sentence, actions right). Kinds: info, ok, warn, err, accent. `role=alert` for warn/err. The data-loss banner (err) is the only banner allowed to be above the fold on the Confirm screen and is paired with a two-segment "reversible / point of no return" strip.

**Dialogs** (520 px, radius 12, `surface`, `e3`, 55% `overlay` scrim). Header with icon + title + one-line context, body, footer on `surface_2` with the ghost "dismiss" left and primary right. Escape closes unless the dialog is a data-loss gate (the gate is a full screen, not a dialog, on purpose: it needs room for two lists).

**Data-loss gate pattern** (Confirm screen). Order is fixed: (1) title with the target device name, (2) err banner, (3) two cards side by side, **Will be destroyed** (err top edge 4 px, trash icons) and **Will be preserved** (ok top edge 4 px, shield icons), each with item + one-line detail, (4) backup finding as a warn banner, (5) two checkboxes, (6) typed hostname with live character count (`role=status`, `aria-live=polite`, paste disabled), (7) action bar: safe primary "Back to plan", secondary "Cancel", destructive disabled until 4 + 5 + 6 are satisfied. The typed text must equal the NAS hostname exactly.

**Log viewer.** `log_bg` surface, radius 8, 12 px mono, line-height 1.65, grid columns 70 px time / 50 px level / message. Level is a **glyph + 3-letter word + colour**: `· DBG`, `● INF`, `▲ WRN`, `✖ ERR`. Warn lines get a 12% `log_wrn` wash and a 3 px left edge; error lines the same with `log_err`. Debug message text is dimmed (`log_dim`). Filter = segmented control "Debug / Info / Warn / Error" with live counts (minimum level, cumulative). Footer: follow-output checkbox, line and warning counts. Copy and Save buttons live in the card header. Always dark in Direction 1, in both themes (terminal convention; contrast verified against the dark `log_bg`).

**Disk diagram.** Four labelled layers top to bottom: DISKS, PARTITIONS, RAID (mdadm), LVM / FS. Boxes: disk = `surface` + `border_strong`; partition = `surface_3` (small system partitions) or `accent_bg` + `accent` (data); RAID array = `ok_bg` + `ok_solid` for mirrors, `accent_bg` for the data array; LV = `accent_bg`; problems = 2 px `warn_solid` outline (disk with SMART note) or `err_solid` + diagonal hatch (overflowing LV), always with a text label ("+0.40 TiB over VG"). Connectors are 1.2 px `fg_3` lines at 55% opacity. Text 11-12 px; sizes in TiB/GiB with two decimals. Warnings and the blocking error repeat below as banners, so the diagram is never the only carrier of the message.

**Slider.** 6 px track `surface_3`, filled part `accent` (`err_solid` when invalid), 20 px thumb with a 2 px ring, dashed marker for the maximum with its value written above, min and max labels below; paired with a numeric input and unit. Keyboard: arrows 1 step, Shift 10 steps.

## 7. Iconography

lucide-style outline icons (ForgeVault already uses lucide): 24 x 24 grid, 2 px stroke, round caps and joins, `currentColor`. Render sizes 14 (inline in badges), 16 (buttons, rows), 20 (card headers), 28 (feature). Icons never carry meaning alone in status contexts: always next to a word. Status glyph set: shield-check, users, help-circle (dashed), alert-triangle, ban, octagon-x, check, x, info, lock, flag. Logo: rounded square with three drive bays, the first one lit; monochrome in `accent`. No vendor logos. In GPUI ship each icon as its own `.svg` file with `stroke="currentColor"` and recolour through `text_color`.

## 8. Motion

Almost none. Transitions only on colour and opacity, 120 ms, ease-out; progress width 200 ms; dialogs fade 120 ms (no slide, no scale). The only looping animation is the 1.2 s linear spinner on the currently running install step; with reduced motion on (setting in the app, `prefers-reduced-motion` on the web) it is replaced by a static refresh glyph plus the word "running". The rollback countdown ring updates once per second without easing. Nothing animates on the Confirm screen except focus.

| Duration token | ms | Used for |
|---|---:|---|
| `instant` | 0 ms | Focus ring, checkbox state, disabled/enabled swap |
| `fast` | 120 ms | Hover/pressed colour, dialog fade-in |
| `base` | 200 ms | Progress width, banner appear |
| `spinner` | 1200 ms | Only for "running" glyph; linear, 1 turn |

## 9. GPUI implementation map

Tokens become one `Palette` struct stored as a GPUI `Global`, switched at runtime for light / dark / follow-OS. Everything below is generated from `tokens.json` (a 30-line build script or `build.rs` emits `theme.rs`), so designers change JSON and engineers never retype a hex. Sizes are plain `px()` constants (`SPACE_4 = px(4.)`, `RADIUS_MD = px(8.)`); type styles are a small `TextStyleRefinement` table keyed by the names in section 4.

```rust
// crates/synofurb-ui/src/theme.rs  (generated from tokens.json - do not hand-edit)
use gpui::{rgb, Global, Hsla};

fn c(hex: u32) -> Hsla { rgb(hex).into() }     // opaque sRGB hex -> Hsla once, at startup

#[derive(Clone)]
pub struct Palette {
    pub canvas: Hsla,
    pub surface: Hsla,
    pub surface_2: Hsla,
    pub surface_3: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub fg: Hsla,
    pub fg_2: Hsla,
    pub fg_3: Hsla,
    pub sidebar: Hsla,
    pub overlay: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub on_accent: Hsla,
    pub accent_bg: Hsla,
    pub accent_fg: Hsla,
    pub ring: Hsla,
    pub danger: Hsla,
    pub danger_hover: Hsla,
    pub on_danger: Hsla,
    pub ok_solid: Hsla,
    pub ok_fg: Hsla,
    pub ok_bg: Hsla,
    pub warn_solid: Hsla,
    pub warn_fg: Hsla,
    pub warn_bg: Hsla,
    pub err_solid: Hsla,
    pub err_fg: Hsla,
    pub err_bg: Hsla,
    pub info_solid: Hsla,
    pub info_fg: Hsla,
    pub info_bg: Hsla,
    pub log_bg: Hsla,
    pub log_fg: Hsla,
    pub log_dim: Hsla,
    pub log_dbg: Hsla,
    pub log_inf: Hsla,
    pub log_wrn: Hsla,
    pub log_err: Hsla,
}
impl Global for Palette {}

impl Palette {
    pub fn light() -> Self {
        Self {
            canvas: c(0xFAFAFA),
            surface: c(0xFFFFFF),
            surface_2: c(0xF3F3F3),
            surface_3: c(0xE9E9E9),
            border: c(0xE5E5E5),
            border_strong: c(0x868686),
            fg: c(0x0A0A0A),
            fg_2: c(0x525252),
            fg_3: c(0x7A7A7A),
            sidebar: c(0xF7F7F7),
            overlay: c(0x0A0A0A),
            accent: c(0x7139CB),
            accent_hover: c(0x622CB5),
            on_accent: c(0xFFFFFF),
            accent_bg: c(0xF1EDFE),
            accent_fg: c(0x5C29AA),
            ring: c(0x7139CB),
            danger: c(0xC60014),
            danger_hover: c(0xAC0311),
            on_danger: c(0xFFFFFF),
            ok_solid: c(0x03A14A),
            ok_fg: c(0x005725),
            ok_bg: c(0xDEF8E2),
            warn_solid: c(0xC68206),
            warn_fg: c(0x6F4606),
            warn_bg: c(0xFFF3D3),
            err_solid: c(0xE30117),
            err_fg: c(0xA70311),
            err_bg: c(0xFEEFEF),
            info_solid: c(0x3280DD),
            info_fg: c(0x004B96),
            info_bg: c(0xE7F1FF),
            log_bg: c(0x121114),
            log_fg: c(0xDEDEDE),
            log_dim: c(0x9E9E9E),
            log_dbg: c(0x9DA5B1),
            log_inf: c(0x89C3FE),
            log_wrn: c(0xF7C243),
            log_err: c(0xFE8A88),
        }
    }

    pub fn dark() -> Self {
        Self {
            canvas: c(0x0A0A0A),
            surface: c(0x171717),
            surface_2: c(0x202020),
            surface_3: c(0x2B2B2B),
            border: c(0x2A2A2A),
            border_strong: c(0x747474),
            fg: c(0xFAFAFA),
            fg_2: c(0xABABAB),
            fg_3: c(0x808080),
            sidebar: c(0x101010),
            overlay: c(0x000000),
            accent: c(0xA37BFE),
            accent_hover: c(0xB091FE),
            on_accent: c(0x0E0C15),
            accent_bg: c(0x31254D),
            accent_fg: c(0xC3AFFE),
            ring: c(0xA37BFE),
            danger: c(0xE23439),
            danger_hover: c(0xF14D4C),
            on_danger: c(0x0C0403),
            ok_solid: c(0x37BB62),
            ok_fg: c(0x6ED889),
            ok_bg: c(0x0F3118),
            warn_solid: c(0xF0B135),
            warn_fg: c(0xF1C45E),
            warn_bg: c(0x3D2A02),
            err_solid: c(0xF4514F),
            err_fg: c(0xFFA09C),
            err_bg: c(0x4F1A18),
            info_solid: c(0x539AF2),
            info_fg: c(0x91C1FF),
            info_bg: c(0x132946),
            log_bg: c(0x060607),
            log_fg: c(0xDEDEDE),
            log_dim: c(0x989898),
            log_dbg: c(0x979FAB),
            log_inf: c(0x89C3FE),
            log_wrn: c(0xF7C243),
            log_err: c(0xFE8A88),
        }
    }
}
```

Use: `div().bg(p.surface).border_1().border_color(p.border).rounded(RADIUS_LG).shadow_sm()`, and `cx.global::<Palette>()` in render. Alpha variants (scrim, status borders at 50%, log level wash at 12%) are `color.opacity(x)`, computed at draw time, never stored.

What is simple, medium or costly in GPUI (verify each against the pinned GPUI revision, the API still moves):

| Component | GPUI building blocks | Cost |
|---|---|---|
| Layout shells (window, step rail, cards, tiles, banners, two-column grids) | `div().flex().flex_col().gap_*().p_*()`, `.grid()` / `.flex_1()`, `.rounded(px())`, `.border_1().border_color()`, `.bg()` | Simple |
| Text styles (type scale, mono, tabular nums) | `.text_size(px())`, `.font_weight()`, `.font_family()`, `.line_height()`; fonts registered once with `cx.text_system().add_fonts()` | Simple |
| Buttons (primary / secondary / ghost / destructive, sizes) | `div().id().on_click()` with `.hover(|s| s.bg())`, `.active()`, `.focus_visible()`; disabled = different style, no handler | Simple |
| Badges, chips, compat badges, severity labels | `div().flex().items_center().gap_1().rounded().border_1()` + `svg()` + text | Simple |
| Icons | `svg().path("icons/x.svg").size(px(16.)).text_color(c)` (monochrome SVG assets recoloured by text colour) | Simple |
| Banners, dialogs, scrim | `div().absolute().inset_0().bg(overlay.opacity(0.55))` + centered child; focus trap handled manually | Simple (focus trap: medium) |
| Progress bar, stepper, step list, status dots | Nested `div` with width as a fraction (`relative(0.58)`); `Animation` only for the spinner | Simple |
| Shadows | `.shadow_sm()` / `.shadow(vec![BoxShadow{..}])`. No inset shadows, no blur-behind: the design uses neither | Simple |
| Disabled "hatched" button | Not needed: disabled = dashed border + muted fill (verify `.border_dashed()` in the pinned GPUI). Hatch is a web-only nicety | Simple (web hatch: skip) |
| Tables | Header row + `uniform_list` of rows built from flex cells with fixed/flex-1 widths. No sortable-column widget; selection edge = 3 px child div | Medium |
| Log viewer (virtualised, level filter, follow-tail) | `uniform_list` over a filtered `Vec<LogLine>`; mono font; per-line `div` with fixed-width timestamp and level columns; auto-scroll via `ScrollHandle`. Select/copy-by-drag is not provided by GPUI labels, so "Copy log" and "Save log" buttons are first-class | Medium |
| Sparklines | `canvas()` + `PathBuilder` (stroke + 12% fill); or pre-render to an SVG string | Medium |
| Storage diagram (disk > partition > RAID > LVM) | Boxes are `div`s on a CSS-like grid; connector lines drawn in one `canvas()` layer using measured bounds; overflow hatch = `canvas()` stripes or solid red with an icon | Medium to costly |
| Slider (size) | Custom element: track + thumb `div`s, mouse + keyboard (arrow, shift = 10x, Home/End) handlers, `aria`-style value text exposed in the label next to it | Medium |
| Text input (typed confirmation, host, user, password) | GPUI has no ready-made input widget; build or adopt one (single-line editor: caret, selection, clipboard, IME). The gate relies on it, so budget for it early | Costly |
| Checkbox, radio card, segmented control | `div` states + `on_click`; roving focus for radio groups | Simple |
| Secret reveal / copy | Text swap + `cx.write_to_clipboard()`; auto-hide timer via `cx.spawn` | Simple |
| Countdown ring (web only) | CSS/SVG in the web UI; not needed in GPUI | n/a |
| Window chrome (title bar, min 960 x 640) | `WindowOptions` with custom titlebar; controls drawn per OS (left on macOS, right on Windows/Linux) | Medium |
| Screen-reader / accessibility tree | Not provided by GPUI today (no AccessKit bridge at time of writing; verify) | Costly / risk, see RECOMMENDATION.md |

Design decisions made for GPUI: no inset shadows or backdrop blur; selected-row edge is a child element; all tints are opaque tokens; disabled state is dashed + muted (no pattern fill); no text-selection dependence (explicit Copy buttons); one scroll container per screen with a pinned action bar; icon font avoided in favour of SVG assets.

## 10. Web plane specifics

Same tokens, plus: left sidebar 236 px (`sidebar`), top bar 54 px with breadcrumb, content padding 24, tiles in a 4-column grid collapsing to 2 below 900 px and to a single column below 480 px; phone layout with a bottom tab bar (44 px targets, 5 destinations). The safe-apply banner (warn, 16 px padding) holds a 60 px countdown ring with seconds in mono, a sentence naming both the new address and the automatic rollback time, and two buttons: "Roll back now" (secondary) and "Keep these settings" (primary); the page is served from the new address, so seeing it is itself the confirmation. All accent options from ForgeVault's accent switcher may stay for the web UI.
