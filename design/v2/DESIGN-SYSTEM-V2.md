# synofurb design system v2

Supersedes `design/mockups/ui/DESIGN-SYSTEM.md` (the first-round HTML mockups were rejected). The review artifact is now the real application: `synofurb gui --demo --screen <name>` and the web UI. This file therefore specifies **tokens, sizes and rules for the real `gpui-component` 0.8.1 widgets**, not screens. The Rust theme is **generated** from `tokens-v2.json`; nothing is eyeballed.

Files: `tokens-v2.json` (flat tokens plus `map.*`), `synofurb-theme.json` (a gpui-component `ThemeSet`, generated), `build_v2.py` (tokens and theme), `build_doc.py` (this file), `tools/contrast_v2.py` (the gate). Brand: `assets/brand/`.

## 1. Rules (from BRIEF-v2)

1. **Widget first.** Every element is a gpui-component widget through gpui-kit unless none exists; hand-made `div` copies of Sidebar, Tabs, Table, Tag, Alert, Tooltip, Popover, Dialog, Sheet, Accordion, Breadcrumb, Stepper, Menu or Kbd are defects. Exceptions are listed in section 12.
2. **Text budget.** At most 60 words of static prose per screen besides data; page title at most 4 words; subtitle one line (about 12 words); labels at most 4 words; buttons a verb plus at most 2 words. Strings live in one copy module per surface with `short` / `tip` / `help`; tests enforce the budgets.
3. **Disclosure.** Tooltip = one sentence (at most 14 words) on hover and keyboard focus; Popover = a short paragraph (at most 40 words) with one action, opened by an info icon; Dialog = full explanation (title, 2 to 4 short paragraphs, one primary action); Help topic = reference, in a Sheet. Logs and progress are collapsible, collapsed except the current step.
4. **Safety-critical content stays visible but terse**: destroyed / changed / preserved lists (names and sizes only), unit identity (model, IP, serial), the typed phrase.
5. Keyboard path, visible focus ring, light and dark, never colour-only status, 960 x 640 minimum, no horizontal scrolling, scalable text.

## 2. Fonts

**Inter** for the interface, **JetBrains Mono** for code, logs, hashes, serials and IPs; both bundled with the application (`assets/brand/fonts/`: Inter Regular, SemiBold, Bold; JetBrains Mono Regular, Bold; OFL 1.1 licence texts alongside), never "whatever the system has". Montserrat is allowed for the wordmark only; the shipped logo outlines Inter SemiBold, so no Montserrat file is needed. Weights bundled: 400, 600, 700 only (the type scale uses nothing else).

Loading (to verify against the pinned GPUI revision): `cx.text_system().add_fonts(vec![Inter-Regular, Inter-SemiBold, Inter-Bold, JetBrainsMono-Regular, JetBrainsMono-Bold])` once at startup, before the theme is applied; the theme names them (`font.family` = "Inter", `mono_font.family` = "JetBrains Mono"). gpui-component sets the window rem size to the theme `font.size`; we keep **16**, so the widgets' own sizes hold: `text_sm` = 14, `text_xs` = 12, `text_base` = 16, `text_2xl` = 24. **Never shrink `font.size` to make text smaller**: it would push `text_xs` to 10.5 px. Tabular numerals (`tnum`) on every UI text style (counters, countdowns, sizes, tile values must not jitter). Floor: **12 px**; do not use `Tag` or `Badge` with `Size::Small` or `Size::XSmall` (they draw 10 px text).

| Style | px / line | Weight | GPUI | Used for |
|---|---|---:|---|---|
| title | 24 / 32 | 700 | `text_2xl` | Page title, at most 4 words |
| section | 16 / 24 | 600 | `text_base + font_semibold` | Card, dialog, drawer title |
| body | 14 / 20 | 400 | `text_sm` | Default UI text, buttons, table cells, tooltip, alert |
| caption | 12 / 16 | 400 | `text_xs` | Tags, kbd, helper text (the floor) |
| mono | 13 / 20 | 400 | `mono_font.size` | Hashes, IPs, serials, commands |
| log | 12 / 18 | 400 | `text_xs + font_family(mono)` | Log lines |

Text scale: a user setting (100 / 115 / 130%) multiplies `font.size`; layouts are checked at 130% on 960 x 640 (the same rule applies to the web UI through `rem`).

## 3. Colour tokens

Opaque sRGB hex, except `scrim` (black at 0.60). Light and dark are re-derived, not inverted. Status colours come in three jobs because gpui-component uses one colour for several things: **`ok_fg` / `warn_fg` / `err_fg` / `info_fg`** are used as *text and icon colour* (Alert, outline Tag) **and** as the *solid fill* of a filled Tag, whose label is **`on_ok` / `on_warn` / `on_err` / `on_info`**; **`*_solid`** are for dots, bars and charts only.

| Token | Light | Dark |
|---|---|---|
| `canvas` | `#FAFAFA` | `#0A0A0A` |
| `surface` | `#FFFFFF` | `#171717` |
| `surface_2` | `#F3F3F3` | `#202020` |
| `surface_3` | `#E9E9E9` | `#2B2B2B` |
| `sidebar` | `#F7F7F7` | `#101010` |
| `border` | `#E5E5E5` | `#2A2A2A` |
| `border_strong` | `#808080` | `#868686` |
| `fg` | `#0A0A0A` | `#FAFAFA` |
| `fg_2` | `#525252` | `#B1B1B1` |
| `placeholder` | `#696969` | `#9E9E9E` |
| `accent` | `#7139CB` | `#A37BFE` |
| `accent_hover` | `#622CB5` | `#B091FE` |
| `accent_pressed` | `#541EA0` | `#946AEE` |
| `on_accent` | `#FFFFFF` | `#0E0C15` |
| `accent_bg` | `#F1EDFE` | `#31254D` |
| `accent_fg` | `#5C29AA` | `#C8B7FF` |
| `accent_border` | `#B193E4` | `#6A50A6` |
| `ring` | `#7139CB` | `#A37BFE` |
| `selection` | `#D5C5F3` | `#433369` |
| `danger` | `#C60014` | `#E84642` |
| `danger_hover` | `#AC0311` | `#ED6B68` |
| `danger_pressed` | `#94020D` | `#F18C8A` |
| `on_danger` | `#FFFFFF` | `#150A09` |
| `ok_fg` | `#005725` | `#6ED889` |
| `on_ok` | `#FFFFFF` | `#071009` |
| `ok_solid` | `#047E39` | `#37BB62` |
| `ok_bg` | `#DEF8E2` | `#0F3118` |
| `warn_fg` | `#6F4606` | `#F1C45E` |
| `on_warn` | `#FFFFFF` | `#120C04` |
| `warn_solid` | `#A76C01` | `#F0B135` |
| `warn_bg` | `#FFF3D3` | `#3D2A02` |
| `err_fg` | `#A70311` | `#FFA09C` |
| `on_err` | `#FFFFFF` | `#150A09` |
| `err_solid` | `#C60014` | `#FC5855` |
| `err_bg` | `#FEEFEF` | `#4F1A18` |
| `info_fg` | `#004B96` | `#91C1FF` |
| `on_info` | `#FFFFFF` | `#080E16` |
| `info_solid` | `#1570D1` | `#539AF2` |
| `info_bg` | `#E7F1FF` | `#132946` |
| `scrollbar_thumb` | `#808080` | `#868686` |
| `scrollbar_thumb_hover` | `#686868` | `#A9A9A9` |
| `switch_off` | `#808080` | `#868686` |
| `log_bg` | `#121114` | `#060607` |
| `log_fg` | `#DEDEDE` | `#DEDEDE` |
| `log_dim` | `#9E9E9E` | `#989898` |
| `log_dbg` | `#9DA5B1` | `#979FAB` |
| `log_inf` | `#89C3FE` | `#89C3FE` |
| `log_wrn` | `#F7C243` | `#F7C243` |
| `log_err` | `#FE8A88` | `#FE8A88` |
| `ring_inverse` | `#A37BFE` | `#A37BFE` |

Rules that follow from the widgets' code (read in `gpui-component-0.8.1/src`): the **filled destructive button** uses `button.danger.*` (`danger`, a mid red that is also a white-label fill), while **Alert, Tag and outline buttons** use `danger.*` (`err_fg`, a text-grade red); the two are different tokens on purpose. The dark `danger` is `#E84642` with a near-black label `#150A09` (4.99:1; hover `#ED6B68` 6.40, pressed `#F18C8A` 8.21), like the dark primary: a light red can step lighter on hover and pressed in clear steps (1.28:1 each), which a mid red with a white label cannot (UI review B9: the earlier dark pair differed by 1.05 and 1.09:1). Fill against the dark surfaces stays at least 3:1 (4.60 on `surface`, 5.08 on `canvas`). Button borders cannot be themed separately (the Danger variant draws its fill colour as border), so the old light-red outline is dropped. Red stays rare: on the data-loss gate only the enabled erase button and the "Will be destroyed" marker are red.

## 4. ThemeConfig map (generated)

`synofurb-theme.json` is produced from this table by `build_v2.py`; `contrast_v2.py` checks that every key exists in gpui-component's `ThemeConfigColors` and every token exists, and `build_doc.py` that every key maps to a real `ThemeColor` field. Load with `ThemeRegistry::global_mut(cx).load_themes_from_str(include_str!(".../synofurb-theme.json"))` and `Theme::change(mode, ..)` (verify names against the pinned revision). Also set in the theme: `font.size` 16, `font.family` Inter, `mono_font.family` JetBrains Mono, `mono_font.size` 13, `radius` 8, `radius.lg` 12, `shadow` true.

| ThemeConfig key | ThemeColor field | Token | Light | Dark |
|---|---|---|---|---|
| `background` | `background` | `canvas` | `#FAFAFA` | `#0A0A0A` |
| `foreground` | `foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `border` | `border` | `border` | `#E5E5E5` | `#2A2A2A` |
| `input.border` | `input` | `border_strong` | `#808080` | `#868686` |
| `ring` | `ring` | `ring` | `#7139CB` | `#A37BFE` |
| `caret` | `caret` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `selection.background` | `selection` | `selection` | `#D5C5F3` | `#433369` |
| `overlay` | `overlay` | `#000000/0.6` | `#00000099` | `#00000099` |
| `muted.background` | `muted` | `surface_2` | `#F3F3F3` | `#202020` |
| `muted.foreground` | `muted_foreground` | `fg_2` | `#525252` | `#B1B1B1` |
| `accent.background` | `accent` | `surface_2` | `#F3F3F3` | `#202020` |
| `accent.foreground` | `accent_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `popover.background` | `popover` | `surface` | `#FFFFFF` | `#171717` |
| `popover.foreground` | `popover_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `primary.background` | `primary` | `accent` | `#7139CB` | `#A37BFE` |
| `primary.foreground` | `primary_foreground` | `on_accent` | `#FFFFFF` | `#0E0C15` |
| `primary.hover.background` | `primary_hover` | `accent_hover` | `#622CB5` | `#B091FE` |
| `primary.active.background` | `primary_active` | `accent_pressed` | `#541EA0` | `#946AEE` |
| `secondary.background` | `secondary` | `surface_2` | `#F3F3F3` | `#202020` |
| `secondary.foreground` | `secondary_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `secondary.hover.background` | `secondary_hover` | `surface_3` | `#E9E9E9` | `#2B2B2B` |
| `secondary.active.background` | `secondary_active` | `surface_3` | `#E9E9E9` | `#2B2B2B` |
| `button.background` | `button` | `surface` | `#FFFFFF` | `#171717` |
| `button.foreground` | `button_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `button.hover.background` | `button_hover` | `surface_2` | `#F3F3F3` | `#202020` |
| `button.active.background` | `button_active` | `surface_3` | `#E9E9E9` | `#2B2B2B` |
| `button.primary.background` | `button_primary` | `accent` | `#7139CB` | `#A37BFE` |
| `button.primary.foreground` | `button_primary_foreground` | `on_accent` | `#FFFFFF` | `#0E0C15` |
| `button.primary.hover.background` | `button_primary_hover` | `accent_hover` | `#622CB5` | `#B091FE` |
| `button.primary.active.background` | `button_primary_active` | `accent_pressed` | `#541EA0` | `#946AEE` |
| `button.danger.background` | `button_danger` | `danger` | `#C60014` | `#E84642` |
| `button.danger.foreground` | `button_danger_foreground` | `on_danger` | `#FFFFFF` | `#150A09` |
| `button.danger.hover.background` | `button_danger_hover` | `danger_hover` | `#AC0311` | `#ED6B68` |
| `button.danger.active.background` | `button_danger_active` | `danger_pressed` | `#94020D` | `#F18C8A` |
| `danger.background` | `danger` | `err_fg` | `#A70311` | `#FFA09C` |
| `danger.foreground` | `danger_foreground` | `on_err` | `#FFFFFF` | `#150A09` |
| `danger.hover.background` | `danger_hover` | `err_fg_hover` | `#910410` | `#FEADA9` |
| `danger.active.background` | `danger_active` | `err_fg_active` | `#7E050F` | `#FEB7B4` |
| `success.background` | `success` | `ok_fg` | `#005725` | `#6ED889` |
| `success.foreground` | `success_foreground` | `on_ok` | `#FFFFFF` | `#071009` |
| `success.hover.background` | `success_hover` | `ok_fg_hover` | `#014C21` | `#82DD99` |
| `success.active.background` | `success_active` | `ok_fg_active` | `#03431E` | `#92E1A6` |
| `warning.background` | `warning` | `warn_fg` | `#6F4606` | `#F1C45E` |
| `warning.foreground` | `warning_foreground` | `on_warn` | `#FFFFFF` | `#120C04` |
| `warning.hover.background` | `warning_hover` | `warn_fg_hover` | `#613E07` | `#F2CC74` |
| `warning.active.background` | `warning_active` | `warn_fg_active` | `#553607` | `#F3D287` |
| `info.background` | `info` | `info_fg` | `#004B96` | `#91C1FF` |
| `info.foreground` | `info_foreground` | `on_info` | `#FFFFFF` | `#080E16` |
| `info.hover.background` | `info_hover` | `info_fg_hover` | `#014282` | `#A0C9FE` |
| `info.active.background` | `info_active` | `info_fg_active` | `#033A72` | `#ACD0FE` |
| `link` | `link` | `accent_fg` | `#5C29AA` | `#C8B7FF` |
| `link.hover` | `link_hover` | `accent_hover` | `#622CB5` | `#B091FE` |
| `link.active` | `link_active` | `accent_pressed` | `#541EA0` | `#946AEE` |
| `list.background` | `list` | `surface` | `#FFFFFF` | `#171717` |
| `list.hover.background` | `list_hover` | `surface_2` | `#F3F3F3` | `#202020` |
| `list.active.background` | `list_active` | `accent_bg` | `#F1EDFE` | `#31254D` |
| `list.active.border` | `list_active_border` | `accent` | `#7139CB` | `#A37BFE` |
| `list.even.background` | `list_even` | `surface` | `#FFFFFF` | `#171717` |
| `list.head.background` | `list_head` | `surface_2` | `#F3F3F3` | `#202020` |
| `table.background` | `table` | `surface` | `#FFFFFF` | `#171717` |
| `table.hover.background` | `table_hover` | `surface_2` | `#F3F3F3` | `#202020` |
| `table.active.background` | `table_active` | `accent_bg` | `#F1EDFE` | `#31254D` |
| `table.active.border` | `table_active_border` | `accent` | `#7139CB` | `#A37BFE` |
| `table.even.background` | `table_even` | `surface` | `#FFFFFF` | `#171717` |
| `table.head.background` | `table_head` | `surface_2` | `#F3F3F3` | `#202020` |
| `table.head.foreground` | `table_head_foreground` | `fg_2` | `#525252` | `#B1B1B1` |
| `table.foot.background` | `table_foot` | `surface_2` | `#F3F3F3` | `#202020` |
| `table.foot.foreground` | `table_foot_foreground` | `fg_2` | `#525252` | `#B1B1B1` |
| `table.row.border` | `table_row_border` | `border` | `#E5E5E5` | `#2A2A2A` |
| `tab.background` | `tab` | `#00000000` | `#00000000` | `#00000000` |
| `tab.foreground` | `tab_foreground` | `fg_2` | `#525252` | `#B1B1B1` |
| `tab.active.background` | `tab_active` | `surface` | `#FFFFFF` | `#171717` |
| `tab.active.foreground` | `tab_active_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `tab_bar.background` | `tab_bar` | `#00000000` | `#00000000` | `#00000000` |
| `tab_bar.segmented.background` | `tab_bar_segmented` | `surface_2` | `#F3F3F3` | `#202020` |
| `sidebar.background` | `sidebar` | `sidebar` | `#F7F7F7` | `#101010` |
| `sidebar.foreground` | `sidebar_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `sidebar.border` | `sidebar_border` | `border` | `#E5E5E5` | `#2A2A2A` |
| `sidebar.accent.background` | `sidebar_accent` | `accent_bg` | `#F1EDFE` | `#31254D` |
| `sidebar.accent.foreground` | `sidebar_accent_foreground` | `accent_fg` | `#5C29AA` | `#C8B7FF` |
| `sidebar.primary.background` | `sidebar_primary` | `accent` | `#7139CB` | `#A37BFE` |
| `sidebar.primary.foreground` | `sidebar_primary_foreground` | `on_accent` | `#FFFFFF` | `#0E0C15` |
| `scrollbar.background` | `scrollbar` | `#00000000` | `#00000000` | `#00000000` |
| `scrollbar.thumb.background` | `scrollbar_thumb` | `scrollbar_thumb` | `#808080` | `#868686` |
| `scrollbar.thumb.hover.background` | `scrollbar_thumb_hover` | `scrollbar_thumb_hover` | `#686868` | `#A9A9A9` |
| `progress.bar.background` | `progress_bar` | `accent` | `#7139CB` | `#A37BFE` |
| `slider.background` | `slider_bar` | `accent` | `#7139CB` | `#A37BFE` |
| `slider.thumb.background` | `slider_thumb` | `accent` | `#7139CB` | `#A37BFE` |
| `switch.background` | `switch` | `switch_off` | `#808080` | `#868686` |
| `switch.thumb.background` | `switch_thumb` | `surface` | `#FFFFFF` | `#171717` |
| `skeleton.background` | `skeleton` | `surface_3` | `#E9E9E9` | `#2B2B2B` |
| `accordion.background` | `accordion` | `surface` | `#FFFFFF` | `#171717` |
| `group_box.background` | `group_box` | `surface_2` | `#F3F3F3` | `#202020` |
| `group_box.foreground` | `group_box_foreground` | `fg` | `#0A0A0A` | `#FAFAFA` |
| `description_list.label.background` | `description_list_label` | `surface_2` | `#F3F3F3` | `#202020` |
| `description_list.label.foreground` | `description_list_label_foreground` | `fg_2` | `#525252` | `#B1B1B1` |
| `drag.border` | `drag_border` | `accent` | `#7139CB` | `#A37BFE` |
| `drop_target.background` | `drop_target` | `accent_bg` | `#F1EDFE` | `#31254D` |
| `title_bar.background` | `title_bar` | `surface_2` | `#F3F3F3` | `#202020` |
| `title_bar.border` | `title_bar_border` | `border` | `#E5E5E5` | `#2A2A2A` |
| `status_bar.background` | `status_bar` | `surface` | `#FFFFFF` | `#171717` |
| `status_bar.border` | `status_bar_border` | `border` | `#E5E5E5` | `#2A2A2A` |
| `window.border` | `window_border` | `border_strong` | `#808080` | `#868686` |
| `chart.1` | `chart_1` | `accent` | `#7139CB` | `#A37BFE` |
| `chart.2` | `chart_2` | `info_solid` | `#1570D1` | `#539AF2` |
| `chart.3` | `chart_3` | `ok_solid` | `#047E39` | `#37BB62` |
| `chart.4` | `chart_4` | `warn_solid` | `#A76C01` | `#F0B135` |
| `chart.5` | `chart_5` | `err_solid` | `#C60014` | `#FC5855` |
| `chart.bullish` | `chart_bullish` | `ok_solid` | `#047E39` | `#37BB62` |
| `chart.bearish` | `chart_bearish` | `err_solid` | `#C60014` | `#FC5855` |
| `chart.grid` | `chart_grid` | `border` | `#E5E5E5` | `#2A2A2A` |
| `base.red` | `red` | `err_solid` | `#C60014` | `#FC5855` |
| `base.red.light` | `red_light` | `err_bg` | `#FEEFEF` | `#4F1A18` |
| `base.green` | `green` | `ok_solid` | `#047E39` | `#37BB62` |
| `base.green.light` | `green_light` | `ok_bg` | `#DEF8E2` | `#0F3118` |
| `base.blue` | `blue` | `info_solid` | `#1570D1` | `#539AF2` |
| `base.blue.light` | `blue_light` | `info_bg` | `#E7F1FF` | `#132946` |
| `base.yellow` | `yellow` | `warn_solid` | `#A76C01` | `#F0B135` |
| `base.yellow.light` | `yellow_light` | `warn_bg` | `#FFF3D3` | `#3D2A02` |
| `base.magenta` | `magenta` | `accent` | `#7139CB` | `#A37BFE` |
| `base.magenta.light` | `magenta_light` | `accent_bg` | `#F1EDFE` | `#31254D` |
| `base.cyan` | `cyan` | `info_solid` | `#1570D1` | `#539AF2` |
| `base.cyan.light` | `cyan_light` | `info_bg` | `#E7F1FF` | `#132946` |

## 5. Icons (Lucide)

The Lucide set that ships with `gpui-kit-assets` (1,830 icons, ISC licence; `IconName::<PascalCase of the lucide name>`, check the generated enum) in the app, and inline Lucide SVGs in the web UI. 24 grid, 2 px stroke, sizes 14 (inside tags and kbd), 16 (buttons, rows, menus), 20 (card and sheet headers), 24 (stepper, empty states). Colour always inherits text colour. Never meaning by icon alone for status: icon plus word. All names below exist in the shipped catalogue (checked).

**Navigation (web sidebar, app)**

| Role | Lucide name |
|---|---|
| Dashboard | `layout-dashboard` |
| Storage | `hard-drive` |
| Mounts | `folder-tree` |
| Network | `network` |
| Users | `users` |
| Boot | `power` |
| System | `settings` |
| Audit log | `scroll-text` |
| Updates | `package` |
| Help centre | `circle-question-mark` |
| About | `info` |
| Shortcuts | `keyboard` |
| Docs | `book-open` |
| Search | `search` |
| Menu (phone) | `menu` |
| Collapse sidebar | `panel-left` |
| Notifications | `bell` |
| Theme | `sun / moon` |

**Wizard steps (Stepper)**

| Role | Lucide name |
|---|---|
| Find | `radar` |
| Connect | `plug` |
| Inspect | `scan-search` |
| Goal | `target` |
| Storage | `hard-drive` |
| System | `settings-2` |
| Confirm | `shield-alert` |
| Install | `download` |
| Done | `circle-check-big` |

**Status (always with a word)**

| Role | Lucide name |
|---|---|
| OK / passed | `circle-check` |
| Warning / note | `triangle-alert` |
| Error / failed | `circle-x` |
| Blocking / stop | `octagon-x` |
| Info | `info` |
| Unknown | `circle-question-mark` |
| Running (spins) | `loader-circle` |
| Pending | `circle` |
| Skipped | `circle-minus` |
| Locked | `lock` |
| Point of no return | `flag` |
| Will be destroyed | `trash` |
| Will be changed | `triangle-alert` |
| Will be preserved | `shield-check` |
| Waiting / time left | `clock` |

**Compatibility badges (outline Tag + icon + word)**

| Role | Lucide name |
|---|---|
| Verified | `shield-check (Tag success)` |
| Community-reported | `users (Tag info)` |
| Untested | `circle-dashed (Tag secondary)` |
| Known issues | `triangle-alert (Tag warning)` |
| Unsupported | `ban (Tag danger)` |

**Actions**

| Role | Lucide name |
|---|---|
| Rescan / retry | `refresh-cw` |
| Add | `plus` |
| Back / Next | `arrow-left / arrow-right` |
| Copy / copied | `copy / check` |
| Reveal / hide secret | `eye / eye-off` |
| Export / save | `download` |
| Import | `upload` |
| Open link | `external-link` |
| Open folder | `folder-open` |
| Delete | `trash` |
| Edit | `square-pen` |
| Close | `x` |
| More | `ellipsis` |
| Filter | `list-filter` |
| Terminal / rescue shell | `terminal` |
| Key / password | `key-round` |
| Fingerprint | `fingerprint-pattern` |
| Undo / roll back | `undo-2` |
| Continue | `play` |
| Pause | `pause` |
| Reboot | `rotate-ccw` |
| Recovery | `life-buoy` |
| Expand | `chevron-down / chevron-right` |

**Hardware and metrics**

| Role | Lucide name |
|---|---|
| Disk | `hard-drive` |
| Unit / server | `server` |
| CPU | `cpu` |
| Memory | `memory-stick` |
| Temperature | `thermometer` |
| Traffic / activity | `activity` |
| Network interface | `network` |


## 6. Scrollbars (themed everywhere)

gpui-component draws one scrollbar (gpui-base `Scrollbar`) from three theme colours; we set them in the theme and use the widget's own geometry.

**Desktop (exact numbers).** Track `scrollbar.background` = transparent (`#00000000`: thumb only, no gutter fill). Thumb `scrollbar.thumb.background` = `border_strong` (`#808080` light, `#868686` dark: at least 3.44:1 on every surface it sits on). Thumb hover and drag `scrollbar.thumb.hover.background` = `#686868` light, `#A9A9A9` dark (at least 3.38:1 on all surfaces including the always-dark log panel). Geometry from the widget: hit area **16 px** wide, thumb **6 px** idle growing to **8 px** on hover or drag, **4 px** inset from the edge, minimum thumb length **48 px**, corner radius = theme `radius` clamped to half the thumb (so a pill). Mode: `ScrollbarMode::Always` for the log viewer and tables (people must see that more exists), `ScrollbarMode::Scrolling` elsewhere (shows while scrolling, fades after 2 s idle, stays while hovered). Keyboard: PgUp, PgDn, Home, End and arrows scroll the focused region; the scroll container takes the focus ring (2 px, offset 2). One scroll container per screen above the pinned action bar; reserve 16 px at the scrolling edge so the thumb never covers content.

**Web (CSS).** Same colours, same geometry (6 px thumb visible, 16 px hit area through a transparent border):

```css
:root { --sb-thumb: #808080; --sb-thumb-hover: #686868; }
:root[data-theme="dark"] { --sb-thumb: #868686; --sb-thumb-hover: #A9A9A9; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { --sb-thumb: #868686; --sb-thumb-hover: #A9A9A9; } }
@supports not selector(::-webkit-scrollbar) { * { scrollbar-width: thin; scrollbar-color: var(--sb-thumb) transparent; } }   /* Firefox only: in Chromium scrollbar-width would switch the ::-webkit-scrollbar rules off */
*::-webkit-scrollbar { width: 16px; height: 16px; }                                   /* Chromium, Safari */
*::-webkit-scrollbar-track { background: transparent; }
*::-webkit-scrollbar-thumb { background: var(--sb-thumb); border: 5px solid transparent; background-clip: content-box; border-radius: 999px; min-height: 48px; min-width: 48px; }
*::-webkit-scrollbar-thumb:hover, *::-webkit-scrollbar-thumb:active { background: var(--sb-thumb-hover); border-width: 4px; }
*::-webkit-scrollbar-corner { background: transparent; }
```

(`border: 5px` leaves a 6 px thumb, `4px` on hover makes it 8 px, matching the desktop.) The log panel is dark in both themes: give it its own `--sb-thumb` (`#868686`). Chromium cannot overlay a scrollbar: the 16 px bar takes layout space, so use `scrollbar-gutter: stable` on the one scroll container and no `overflow:auto` anywhere else (the desktop mockup uses a 12 px bar with a 6 px thumb for the same reason).

## 7. Widget specifications

Sizes are the widgets' own (read from the source); our job is to choose the variant and set tokens. `Size::Medium` is the default everywhere; use `Small` only in dense tables. Focus: the theme `ring` colour, 2 px, offset 2; parents of edge controls must not clip it.

| Widget (gpui-component) | Variant / use | Size and metrics | Tokens (ThemeColor) | Notes |
|---|---|---|---|---|
| `Button` | primary (`.primary()`) one per screen; default (outline: border = `input`) for Back and Cancel; `.ghost()` tertiary; `.danger()` only for data-loss actions (verb + object label) | xs 20, sm 24, md **32** (text_sm 14), lg 32 with wider padding; px 10 (md); radius = theme radius 8; icon 16, gap 8 | `button*`, `primary*`, `button_danger*`, `input` (border), `ring` | Do not use the `Secondary` variant (its border is the hairline `border`, below 3:1). Disabled = the widget's own `disabled()` style (read from `button.rs`): label `muted_foreground` at 50 %, fill and border the variant colour at 15 % (default: input background and `input` at 50 %); legibility of the label is about 2.2 to 2.9:1, which WCAG exempts for disabled controls, so the reason must never live only in the label: it is a visible info `Button` next to the control (a disabled `Button` is not focusable and tooltips are hover-only) that opens a Popover. Loading = `.loading(true)` with a present-tense label. Danger border equals its fill (not themeable). |
| `Tag` (and `Badge` for counts) | outline Tag + Lucide icon for compat and status; filled Tag for counts and chips | text_xs 12, line-height 1.25 (15 px), px 10, py 4 (25 px high), weight 400, radius = theme radius (use `.rounded_full()` for pills), border 1; **Medium only** | `success`/`warning`/`danger`/`info` (text, border) with `*_foreground` for filled; `secondary`, `border` | Never `Size::Small`/`XSmall` (10 px). Untested uses `Tag::custom(transparent, muted_foreground, border_strong)` with `circle-dashed` (the `Secondary` outline border is the 1.26:1 hairline, and `border-style: dashed` does not exist). Hover = opacity 0.9. |
| `Alert` | info / success / warning / error, `.banner()` for page-level rows | text_sm 14; px 16 py 10 gap 12 (md); radius 8; small px 12 py 8; icon 16; **no action slot** (`alert.rs` has a title, a message and a close button): an action is a sibling `Button` beside the Alert | text and icon = `info`/`success`/`warning`/`danger`; fill = that colour at 4% alpha; border at 30% | At most one title and one sentence; anything longer goes to a Dialog. `role=alert` equivalent for warn and error. |
| `Tooltip` | one sentence, **hover only** (`tooltip.rs` has no focus path); `.tooltip()` / `managed_tooltip` | text_sm 14, px 8, py 2, radius 8, shadow_md, border 1; 150 ms enter; show delay 400 ms (we set it; the widget default is 500 ms) | `popover`, `popover_foreground`, `border` | At most 14 words. Anything needed to act or to stay safe is on a focusable trigger (an info `Button`, or a `Tag` wrapped as a Popover trigger); a disabled button never carries the tooltip. Optional `Kbd` inside for the shortcut. |
| `Popover` | a short paragraph plus one action (button or link), opened by an info icon or button | padding 12 (`p_3`), offset 6, margin 8, radius 8, shadow, 150 ms enter, max width 320 | `popover`, `popover_foreground`, `border`, `link` | At most 40 words. Escape and outside click close it; focus returns to the trigger. |
| `Dialog` | modal: full explanation or decision; confirmation of a destructive action | width **448** default (640 for wide content), radius_lg 12, 250 ms, overlay `overlay` (black 0.60), padding 16 to 24 | `background`, `border`, `overlay` | Title at most 4 words, 2 to 4 short paragraphs (at most 80 words), one primary action, a Cancel that is not styled as primary. The data-loss gate is a full screen, not a dialog. |
| `Sheet` (drawer) | Help topics, About, logs detail; side Right | width **350** (480 for Help), padding 16, shadow_xl, 150 ms | `background`, `border`, `overlay` | Escape closes; title `font_semibold` 16; keep one scroll container inside. |
| `Kbd` | shortcuts in tooltips, Help and menus | text_xs 12, px 4, min width 20, radius = radius / 2 (4), border 1 in the outline form | `muted` (bg), `muted_foreground` (text), `border` | Platform glyphs on macOS (command, option), words elsewhere (Ctrl, Alt), automatic from the key binding. |
| `Sidebar` (web: custom CSS equal to it) | app navigation in the Management UI shell; desktop: not used (the wizard has a Stepper) | width 255 default, we set **240**; collapsed 48; menu item height 32, icon 16, gap 8, radius 8 | `sidebar*` (background, foreground, border, accent, accent_foreground, primary) | Active item = `sidebar.accent` fill + `sidebar.accent.foreground`; hover = the same at 0.8 alpha. Group labels `muted_foreground`, sentence case, text_xs. |
| `Stepper` | the 6-step wizard (Find, Connect, Goal, Setup, Confirm, Install) in the title area | text-centred layout: indicator disc 24 (lg 32) with the number or an icon, gap 4, label `text_sm` below it, connector 2 px in the `border` token (1.26:1, decorative); all six labels fit at 960 px and at 130 % text | indicator: `secondary` for upcoming, `primary` for every step up to and including the checked one (`trigger.rs`); label colour is ours (the label is our child element) | The widget draws current and done identically. We distinguish them with the check icon (done) and a semibold label (current), both our own children; failed = `circle-x` in `danger` with the step name kept. Custom parts: those two icon choices, the semibold current label and the failed disc. Items are not Tab stops (`Alt+Left/Right`). |
| `DataTable` / `Table` | scan results, disks, volumes, users | row height xs 26, sm 30, **md 32**, lg 40; header row 32; cell padding 8 x 12; row border 1 px | `table*` (background, head, head_foreground, hover, active, active_border, row_border) | At most 5 columns at 960 px. Selected row = what `table/state.rs` paints: `table.active.background` fill plus a 1 px border at 50 % of `table.active.border` all round (2.2:1 to 2.8:1, the tint 1.15:1), so the selection is never colour alone: the first cell holds a `Radio` (16 px, `input` border) that is checked on the selected row. Stripes off (even = background). Drag-select does not exist: give copyable cells a Copy button. |
| `Tabs` | page sections (Underline), small filters (Segmented) | Underline md 36 (sm 30, lg 44), Segmented md 32 (sm 24); padding 12; radius_lg for segments | `tab*`, `tab_bar*`, `foreground` | Log level filter = Segmented tabs with cumulative counts ("Info 149"). |
| `Accordion` | collapsible logs, progress groups, advanced options | header text_sm semibold, chevron 16, radius_lg 12, border 1 | `accordion`, `border`, `foreground`, `muted_foreground` | Collapsed by default except the current step. |
| `Progress` | step progress, resync, downloads | height xs 4, sm 6, **md 8**, lg 10, pill radius | `progress.bar.background` (fill; track = fill at 20% alpha) | The track alone is not a 3:1 boundary: wrap the bar in a 1 px `input` border or show the value as text beside it. Colour variants (warning, danger) through `.color(..)`. |
| `Input`, `Select`, `Switch`, `Checkbox`, `Radio`, `Slider` | forms | input md 32, sm 24, lg 44; checkbox and radio 16; slider thumb 16, bar 6 | `input` (border, at least 3:1), `primary` (checked), `switch`, `switch_thumb`, `slider_bar`, `slider_thumb`, `caret`, `selection`, `ring` | The typed phrase uses `Input` (not a div). Placeholder = `muted_foreground`. Error = `danger` border plus a message with an icon. |
| `Breadcrumb`, `Link` | web and Help | text_sm, separator `chevron-right` 14 | `muted_foreground`, `foreground` (last), `link*` | |
| `Notification` (toasts) | non-blocking result ("Copied") | text_sm, 4 s | `popover*`, `border` | Never for errors that need action: use Alert or Dialog. |
| `Skeleton`, `Spinner` | loading | spinner = `loader-circle` 16 | `skeleton`, `muted_foreground` | Static glyph under reduced motion. |

Contrast of the widget pairs on the generated theme (light / dark): primary label on fill 6.67 / 6.30; default button label 19.80 / 17.18; destructive button label 6.16 / 4.99 (hover 7.57 / 6.40, pressed 9.24 / 8.21); destructive fill against the page (non-text) 5.90 / 5.08; filled Tag success 8.79 / 10.90, warning 8.22 / 11.85, danger 7.88 / 9.97, info 8.58 / 10.40; Alert text on its tint: success 7.92 / 10.65, warning 7.42 / 11.49, danger 7.01 / 9.68, info 7.71 / 10.15; tooltip and popover text 19.80 / 17.18; Kbd text 7.49 / 9.23; tab label 7.49 / 9.23; table head 7.04 / 7.60; sidebar item 7.77 / 7.77; link 8.55 / 11.01; text on selection 12.36 / 10.50; input border 3.78 / 5.44; scrollbar thumb 3.78 / 5.44; switch off track 3.78 / 5.44; progress fill vs its 20% track 4.68 / 4.96.

## 8. Gate and numbers

`python3 design/v2/tools/contrast_v2.py` checks **642 pairs on both themes**: 424 token-matrix pairs (every text token on every surface it may sit on, every non-text token on every surface it may touch, and the five web accent sets) and 218 pairs on the *generated* gpui-component theme (what the widgets will paint: button, tag, alert 4% tints, tooltip, popover, kbd, tab, table, sidebar, link, selection, scrollbar thumb, switch, slider, progress track, input and checkbox border, focus ring). It also verifies the ThemeConfig keys against the crate's schema, and that no token font size is under 12 px. **Result: 642 of 642 pass; lowest text ratio 4.74:1, lowest non-text ratio 3.25:1.** It was run against deliberately bad tokens and fails as it should. Not covered: GPUI's real rendering and alpha blending beyond the 4% and 20% blends the widgets are known to use, text drawn over images, and hover/pressed states of text on arbitrary surfaces (links are only allowed on `canvas`, `surface` and the tint).

## 9. Motion

Widget defaults, kept: fast 120 ms (hover, colour), normal 180 ms (progress, accordion), slow 280 ms, dialog 250 ms, sheet 150 ms, tooltip and popover 150 ms ease-out, no slide for dialogs beyond the widget's own. The only loop is the spinner; with reduced motion on (a setting, because GPUI does not expose the OS flag everywhere) it is a static glyph plus the word "Working". Countdowns update once per second without easing and are announced at 10 s and 5 s only.

## 10. Layout metrics

Window minimum 960 x 640. Title bar 38, stepper 48, content one scroll area with 24 padding, pinned action bar 64, safety rail 216 on the right. Control heights: 32 (md), 24 (sm), 44 for lg inputs and for every touch target in the web UI below 700 px. Spacing scale 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48. Radius: 4 (kbd), 8 (controls), 12 (dialogs, cards, tabs), pill.

## 11. Help and About (both surfaces)

Help centre: a Sheet (480 px) with a search `Input` at the top, a list of short topics (`List`), a "Shortcuts" topic using `Kbd`, "How recovery works", "How to report a compatibility result", and a docs link; topics open in the same sheet; every complex control has an info icon (`info`) that opens its topic (Popover for the summary, link "Open in Help" for the topic). About: a page (desktop) or route (web) with version, build, licences and credits (Inter and JetBrains Mono OFL 1.1, Lucide ISC, GPUI and gpui-component Apache 2.0, plus the dependency list), a "Copy diagnostics" button (Copy then check icon), and links. Both use the same strings from the copy module (`help` field).

## 12. Exceptions (no widget exists) and open items

- **Safety rail** (the "Safety" panel with four levels): no widget; build from `Stepper`-like rows inside a `GroupBox`, tokens as in section 3. Justified: it is product-specific.
- **Disk/storage diagram**: custom `canvas()` for connectors, boxes are `div`s with `Tag`-like styling; never scrolls horizontally: each disk is one row (name, a bar whose width is proportional to the disk size, size), the segments share the bar by `flex`, the array chips below wrap onto more lines, and below 40 rem the Now and After cards stack in one column; 12 px mono text.
- **Log viewer**: virtual list of lines (check `VirtualList`/`List`), mono 12; level glyph = Lucide icon + word.
- **Countdown ring** (web only): SVG.
- **Open:** GPUI accessibility tree (not verified; spike pending), font feature `tnum` exposure in the pinned GPUI, exact `IconName` variant spelling, `Sidebar` custom width, `Tabs` underline colour (expected `primary`), real-app screenshots (to be reviewed against this brief).

**Added after the round-1 reviews** (design/reviews/v2/): custom pieces with no widget, each justified in the mockup README and in the report: the Stepper's check/failed/semibold-current parts (above; completed connectors are painted `primary`, as `StepperSeparator` does); *Alert plus action* = Alert with a sibling Button; the *point-of-no-return strip* (12 ticks and a flag, on `Progress` colours) and *countdown* (title style, tabular digits, no 28/36 px display size); the *Select popup* is the widget's own list drawn open in the mockup; *disabled reason* = info Button + Popover. The prototype's text sizes are only 12, 14, 16, 24 (mono 12, 13) and its radii only 4, 8, 12 and pill: `design/mockups-v2/desktop/check_scale.js` enforces it. Text size is a root-size setting (rem): all lengths in the mockup are rem, there is no height media query, and layouts are checked at 100 %, 115 % and 130 % on 960 x 640.

## 13. Deviations register (against BRIEF-v2 and the widgets; each is a decision, not an accident)

| Id | Deviation | Why | Mitigation and enforcement |
|---|---|---|---|
| DEV-UI-1 | **Tooltips are hover only** (BRIEF-v2's disclosure table says "hover + focus"). | `gpui-component` `tooltip.rs` has no focus path; a custom focus tooltip would be a new widget to build and maintain. | A tooltip may sit only on a control that is already focusable and may only repeat what is visible or non-essential. Every sentence a keyboard user needs is visible text or sits on a focusable info `Button` (or a `Tag` wrapped as a Popover trigger). The mockup raises no tooltip on keyboard focus and `check_tips.js` fails on any `data-tip` element that is not focusable. |
| DEV-UI-2 | **Disabled controls use the widget's disabled style** (label about 2.2 to 2.9:1). | Not themeable per `button.rs`; WCAG 1.4.3 exempts disabled controls. | The reason is never the label: a sibling focusable info button ("Why locked", "Why unavailable") opens a Popover. The disabled button itself is not a Tab stop (`tabindex=-1`), its reason button is. |
| DEV-UI-3 | **Some small targets are 24 px**: info buttons, inline links, copy icon buttons inside data rows (Done, Manage), the service Restart chip, popover action buttons and the Find row action. (The DS default control height is 32 px; BRIEF-v2 states no pixel minimum.) | 24 x 24 px is the WCAG 2.2 AA minimum target size (2.5.8) and they sit inside 32 px rows and text lines; `Button` xs/sm are 20 and 24 px in the widget. Row-level actions are 32 px: Recovery rows, Done Copy password, the log Copy and Open file, Settings rows, and everything on the rescue panel (the 15-minute window). Sliders (20 px track) and the Web UI switch (36 x 20 px) keep the widget sizes and meet 2.5.8 through its spacing rule. | They are keyboard reachable; the pinned action bar never uses them. Revisit with real-app measurements. |
| DEV-UI-4 | **Scrollbar**: Chromium reserves 12 px per side (`scrollbar-gutter: stable both-edges`); GPUI overlays a 16 px hit area. | Browsers cannot overlay a classic scrollbar. | Same thumb, hover and colours; content edges at 24 px match. |
| DEV-UI-5 | **Layout is driven by window width through rem container queries** (no viewport-height media queries; GPUI has none, the real app needs explicit layout rules for the same cases). | The Text size setting scales rem, so thresholds are in rem. At 68.75 rem or less: the unit facts hide, the health cards go to two columns, Setup controls narrow to 18 rem and the Confirm gate narrows to 20 rem. At 52 rem or less: Setup, Goal cards and the health cards become one column, and Confirm becomes one column in this order: alerts, **Destroyed / Changed / Preserved summary, then the gate**, then the details (the summary is always before the consent controls). Two Confirm-only rules at 52 rem or less: the unit header is hidden (its IP and serial are in the gate) and the alerts, page gap and padding are compacted. At 40 rem or less the Now and After diagram cards stack. | Documented here so the GPUI implementation can port the same breakpoints as width tests on the window in rem. At Largest text on 960 x 640 the first screen holds the alerts, the Destroyed / Changed / Preserved summary and the After line, in that order, and only the top edge of the ERASE card (it starts about 395 px down a 403 px viewport); the checkboxes and the phrase field need a scroll (the phrase about 255 px) and the Erase button stays pinned with its reason. |
| DEV-UI-6 | Custom pieces without a widget: Stepper parts (check, failed, semibold current), point-of-no-return strip, countdown (title style), disk diagram, log viewer. | See section 12. | Type scale and radii are enforced by `check_scale.js`. |
| DEV-UI-7 | **Dark destructive button**: light red `#E84642` with a near-black label. | A mid red with a white label cannot step visibly on hover and pressed in dark (UI review B9). | Section 3 numbers; `contrast_v2.py` 642 of 642. |
| DEV-UI-8 | **Enter does not activate the focused Erase button** (GPUI `Button` activates on Enter and Space). | The data-loss safeguard. | A key handler on that one button; Space and click work. Needs a deliberate `on_key_down` interceptor in the real app. |
| DEV-UI-9 | **Desktop implementation of DEV-UI-8**: no global `enter` key binding. | A bound `enter` action swallows Enter on every focused `Button` (verified with injected keys: the Command button stopped opening). | The root runs the primary action on Enter only when the root itself is focused; Erase sits in a `capture_key_down` wrapper that stops Enter. Verified under Xvfb with XTest: Enter on Erase does nothing, Space starts the install. Unit-tested part: the phrase and checkbox gate (`state::ConfirmGate`, `facts::confirm_phrase_for`). |
| DEV-UI-10 | **Desktop segmented choices are `ToggleGroup`s, not `RadioGroup`s** (theme, text size, accent, sign-in method, Setup tabs). | A `RadioGroup` in the widget set is vertical and heavy for 2 to 5 short options; the mockup shows segmented controls. | `kit::segmented` wraps `ToggleGroup`; the choice is also in the Help topics. Roving-focus order of `ToggleGroup` is not verified with a screen reader. |
| DEV-UI-11 | **Static state where the mockup animates**: the 60 s and 15 min countdowns, the install progress bar and the rescue window clock show the last value read, not a live tick. | No timer-driven repaint is wired for these in the desktop shell; the engine reports phases, not seconds. | The text states the rule ("DSM returns in 60 s") instead of a ticking value. The agent-did-not-answer state hides the clock. |
| DEV-UI-12 | **Recovery actions that need the unit produce a command instead of acting**: Restore flash and Remove entries ask for a typed phrase and then show the exact command to run; Open agent shows its command; Reinstall DSM opens a dialog with the Synology link. Boot DSM runs. | The engine has no desktop-safe entry for flash restore or entry removal without the session the CLI sets up. | The Manual recovery dialog states this; the row set is a pure function of the unit facts (`facts::recovery_rows`, unit-tested). |
| DEV-UI-13 | **Manage update is simulated in demo and not wired to `push_update` / `push_rollback`**; the desktop Storage tab is read-only (health, arrays, Hybrid layout, Open web UI). | RAID changes belong to the web UI by decision (no RAID mutations on the desktop). | `mgmt::update_gate` decides the gate (blocked when degraded and unconfirmed, allowed with a warning at Critical) and is unit-tested. |
| DEV-UI-14 | **Not built**: OS keychain for saved passwords, host-key trust dialog, a real rescue shell. | Out of scope for this pass; core has no keychain API. | Logged in TODO; Done shows the host-key fingerprint with a copy button. |

**Timing numbers shown by the mockup, and where they come from** (`crates/synofurb-core/src/session.rs`; nothing here is invented): the install flow arms the boot agent with `timeout_min: 30` (about line 330); the app waits `Duration::from_secs(10 * 60)` = 10 minutes for the agent to appear (about line 362; `domboot.rs:318` says "usually 1-3 minutes"); the Boot manager arms it with `timeout_min: 60` (about line 422). The failed-agent clock therefore reads "DSM may return in about 20 min or more": the watchdog counts from when the agent booted (1 to 3 minutes after the reboot command), so 20 plus that many minutes remain when the app gives up, an estimate and not a promise. The engine resets nothing after the wait and cannot tell "the agent ran but is unreachable" (DSM is the default, the watchdog reboots to DSM) from "the agent never ran" (the default is still the agent, no watchdog): the screen shows "Default: not confirmed", the status-bar Tag "Boot unconfirmed" and names the manual recovery (`boot/dom/README.md`, "Fail-safe design" and "Manual recovery"). Before step 3 an installer failure reboots after 60 s and after step 3 the rescue shell closes after 15 minutes (`docs/RECOVERY.md`). `docs/RECOVERY.md` line 22 ("if nobody claims it in N minutes") and `docs/BOOT.md` line 58 ("within the timeout") should state these numbers.
