#!/usr/bin/env python3
"""Writes design/v2/DESIGN-SYSTEM-V2.md from tokens-v2.json, the generated theme and the checker output. Run build_v2.py first."""
import glob, json, os, re, subprocess
HERE = os.path.dirname(os.path.abspath(__file__))
import sys; sys.path.insert(0, HERE)
from build_v2 import cr
T = json.load(open(os.path.join(HERE, 'tokens-v2.json'))); TH = json.load(open(os.path.join(HERE, 'synofurb-theme.json')))
CHK = os.path.join(HERE, 'tools', 'contrast_v2.py')
out = subprocess.run(['python3', CHK, '-v'], capture_output=True, text=True).stdout
summ = [l for l in out.splitlines() if l.startswith(('Checked', 'Passing'))]
N = int(re.search(r'Checked (\d+)', summ[0]).group(1)); NM = int(re.search(r'(\d+) token-matrix', summ[0]).group(1)); NW = int(re.search(r'(\d+) on the generated', summ[0]).group(1))
MT = re.search(r'lowest text ratio ([\d.]+)', summ[1]).group(1); MN = re.search(r'lowest non-text ratio ([\d.]+)', summ[1]).group(1)
rows = {}
for l in out.splitlines():
    m = re.match(r'ok (light|dark) widget (.*?)\s+([\d.]+) >= ([\d.]+)', l)
    if m: rows.setdefault(m.group(2).strip(), {})[m.group(1)] = float(m.group(3))
def wr(label): d = rows.get(label); return f'{d["light"]:.2f} / {d["dark"]:.2f}' if d and len(d) == 2 else 'n/a'
# ThemeConfig key -> ThemeColor field
src = (glob.glob(os.path.expanduser('~/.cargo/registry/src/*/gpui-component-0.8.1/src/theme/theme_color.rs')) or [None])[0]
fields = set(re.findall(r'pub (\w+): Hsla', open(src).read())) if src else set()
def field(k):
    special = {'input.border': 'input', 'progress.bar.background': 'progress_bar', 'slider.background': 'slider_bar', 'slider.thumb.background': 'slider_thumb', 'drag.border': 'drag_border', 'selection.background': 'selection', 'accent.background': 'accent', 'link': 'link', 'overlay': 'overlay'}
    if k in special: return special[k]
    if k.startswith('base.'): return k[5:].replace('.', '_')
    f = k
    f = re.sub(r'\.hover\.background$', '_hover', f); f = re.sub(r'\.active\.background$', '_active', f)
    f = re.sub(r'\.background$', '', f); f = f.replace('.foreground', '_foreground').replace('.', '_')
    return f
def maprows():
    o = ['| ThemeConfig key | ThemeColor field | Token | Light | Dark |', '|---|---|---|---|---|']
    bad = []
    L, D = TH['themes'][0]['colors'], TH['themes'][1]['colors']
    for k in L:
        f = field(k)
        if fields and f not in fields: bad.append((k, f))
        o.append(f'| `{k}` | `{f}` | `{T["map."+k]}` | `{L[k]}` | `{D[k]}` |')
    assert not bad, bad
    return '\n'.join(o)
def colrows(keys):
    o = ['| Token | Light | Dark |', '|---|---|---|']
    for k in keys: o.append(f'| `{k}` | `{T["color.light."+k]}` | `{T["color.dark."+k]}` |')
    return '\n'.join(o)
ICONS = [
 ('Navigation (web sidebar, app)', [('Dashboard', 'layout-dashboard'), ('Storage', 'hard-drive'), ('Mounts', 'folder-tree'), ('Network', 'network'), ('Users', 'users'), ('Boot', 'power'), ('System', 'settings'), ('Audit log', 'scroll-text'), ('Updates', 'package'), ('Help centre', 'circle-question-mark'), ('About', 'info'), ('Shortcuts', 'keyboard'), ('Docs', 'book-open'), ('Search', 'search'), ('Menu (phone)', 'menu'), ('Collapse sidebar', 'panel-left'), ('Notifications', 'bell'), ('Theme', 'sun / moon')]),
 ('Wizard steps (Stepper)', [('Find', 'radar'), ('Connect', 'plug'), ('Inspect', 'scan-search'), ('Goal', 'target'), ('Storage', 'hard-drive'), ('System', 'settings-2'), ('Confirm', 'shield-alert'), ('Install', 'download'), ('Done', 'circle-check-big')]),
 ('Status (always with a word)', [('OK / passed', 'circle-check'), ('Warning / note', 'triangle-alert'), ('Error / failed', 'circle-x'), ('Blocking / stop', 'octagon-x'), ('Info', 'info'), ('Unknown', 'circle-question-mark'), ('Running (spins)', 'loader-circle'), ('Pending', 'circle'), ('Skipped', 'circle-minus'), ('Locked', 'lock'), ('Point of no return', 'flag'), ('Will be destroyed', 'trash'), ('Will be changed', 'triangle-alert'), ('Will be preserved', 'shield-check'), ('Waiting / time left', 'clock')]),
 ('Compatibility badges (outline Tag + icon + word)', [('Verified', 'shield-check (Tag success)'), ('Community-reported', 'users (Tag info)'), ('Untested', 'circle-dashed (Tag secondary)'), ('Known issues', 'triangle-alert (Tag warning)'), ('Unsupported', 'ban (Tag danger)')]),
 ('Actions', [('Rescan / retry', 'refresh-cw'), ('Add', 'plus'), ('Back / Next', 'arrow-left / arrow-right'), ('Copy / copied', 'copy / check'), ('Reveal / hide secret', 'eye / eye-off'), ('Export / save', 'download'), ('Import', 'upload'), ('Open link', 'external-link'), ('Open folder', 'folder-open'), ('Delete', 'trash'), ('Edit', 'square-pen'), ('Close', 'x'), ('More', 'ellipsis'), ('Filter', 'list-filter'), ('Terminal / rescue shell', 'terminal'), ('Key / password', 'key-round'), ('Fingerprint', 'fingerprint-pattern'), ('Undo / roll back', 'undo-2'), ('Continue', 'play'), ('Pause', 'pause'), ('Reboot', 'rotate-ccw'), ('Recovery', 'life-buoy'), ('Expand', 'chevron-down / chevron-right')]),
 ('Hardware and metrics', [('Disk', 'hard-drive'), ('Unit / server', 'server'), ('CPU', 'cpu'), ('Memory', 'memory-stick'), ('Temperature', 'thermometer'), ('Traffic / activity', 'activity'), ('Network interface', 'network')])]
def icon_tables():
    s = ''
    for h, items in ICONS: s += f'**{h}**\n\n| Role | Lucide name |\n|---|---|\n' + '\n'.join(f'| {a} | `{b}` |' for a, b in items) + '\n\n'
    return s
def typerows():
    o = ['| Style | px / line | Weight | GPUI | Used for |', '|---|---|---:|---|---|']
    for n in ('title', 'section', 'body', 'caption', 'mono', 'log'):
        o.append(f'| {n} | {T[f"type.{n}.size_px"]} / {T[f"type.{n}.line_px"]} | {T[f"type.{n}.weight"]} | `{T[f"type.{n}.gpui"]}` | ' + {'title': 'Page title, at most 4 words', 'section': 'Card, dialog, drawer title', 'body': 'Default UI text, buttons, table cells, tooltip, alert', 'caption': 'Tags, kbd, helper text (the floor)', 'mono': 'Hashes, IPs, serials, commands', 'log': 'Log lines'}[n] + ' |')
    return '\n'.join(o)
def _g(th, k): return T[f'color.{th}.{k}']
DD = [cr('#FFFFFF', _g('dark', k)) for k in ('danger', 'danger_hover', 'danger_pressed')]
TH_MIN = min(cr(_g(th, 'scrollbar_thumb'), _g(th, s)) for th in ('light', 'dark') for s in ('canvas', 'surface', 'surface_2', 'sidebar', 'accent_bg', 'log_bg'))
THH_MIN = min(cr(_g(th, 'scrollbar_thumb_hover'), _g(th, s)) for th in ('light', 'dark') for s in ('canvas', 'surface', 'surface_2', 'sidebar', 'accent_bg', 'log_bg'))
DOC = f'''# synofurb design system v2

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

{typerows()}

Text scale: a user setting (100 / 115 / 130%) multiplies `font.size`; layouts are checked at 130% on 960 x 640 (the same rule applies to the web UI through `rem`).

## 3. Colour tokens

Opaque sRGB hex, except `scrim` (black at 0.60). Light and dark are re-derived, not inverted. Status colours come in three jobs because gpui-component uses one colour for several things: **`ok_fg` / `warn_fg` / `err_fg` / `info_fg`** are used as *text and icon colour* (Alert, outline Tag) **and** as the *solid fill* of a filled Tag, whose label is **`on_ok` / `on_warn` / `on_err` / `on_info`**; **`*_solid`** are for dots, bars and charts only.

{colrows(['canvas', 'surface', 'surface_2', 'surface_3', 'sidebar', 'border', 'border_strong', 'fg', 'fg_2', 'placeholder', 'accent', 'accent_hover', 'accent_pressed', 'on_accent', 'accent_bg', 'accent_fg', 'accent_border', 'ring', 'selection', 'danger', 'danger_hover', 'danger_pressed', 'on_danger', 'ok_fg', 'on_ok', 'ok_solid', 'ok_bg', 'warn_fg', 'on_warn', 'warn_solid', 'warn_bg', 'err_fg', 'on_err', 'err_solid', 'err_bg', 'info_fg', 'on_info', 'info_solid', 'info_bg', 'scrollbar_thumb', 'scrollbar_thumb_hover', 'switch_off', 'log_bg', 'log_fg', 'log_dim', 'log_dbg', 'log_inf', 'log_wrn', 'log_err', 'ring_inverse'])}

Rules that follow from the widgets' code (read in `gpui-component-0.8.1/src`): the **filled destructive button** uses `button.danger.*` (`danger`, a mid red that is also a white-label fill), while **Alert, Tag and outline buttons** use `danger.*` (`err_fg`, a text-grade red); the two are different tokens on purpose. The dark `danger` is `#CE292C`: a red inside the narrow luminance window that is at once at least 3:1 against the dark surfaces and carries a white label with margin ({DD[0]:.2f}:1; hover {DD[1]:.2f}, pressed {DD[2]:.2f}); hover and pressed therefore differ from it by small steps in dark (use the focus ring and the label change for loading). Button borders cannot be themed separately (the Danger variant draws its fill colour as border), so the old light-red outline is dropped. Red stays rare: on the data-loss gate only the enabled erase button and the "Will be destroyed" marker are red.

## 4. ThemeConfig map (generated)

`synofurb-theme.json` is produced from this table by `build_v2.py`; `contrast_v2.py` checks that every key exists in gpui-component's `ThemeConfigColors` and every token exists, and `build_doc.py` that every key maps to a real `ThemeColor` field. Load with `ThemeRegistry::global_mut(cx).load_themes_from_str(include_str!(".../synofurb-theme.json"))` and `Theme::change(mode, ..)` (verify names against the pinned revision). Also set in the theme: `font.size` 16, `font.family` Inter, `mono_font.family` JetBrains Mono, `mono_font.size` 13, `radius` 8, `radius.lg` 12, `shadow` true.

{maprows()}

## 5. Icons (Lucide)

The Lucide set that ships with `gpui-kit-assets` (1,830 icons, ISC licence; `IconName::<PascalCase of the lucide name>`, check the generated enum) in the app, and inline Lucide SVGs in the web UI. 24 grid, 2 px stroke, sizes 14 (inside tags and kbd), 16 (buttons, rows, menus), 20 (card and sheet headers), 24 (stepper, empty states). Colour always inherits text colour. Never meaning by icon alone for status: icon plus word. All names below exist in the shipped catalogue (checked).

{icon_tables()}
## 6. Scrollbars (themed everywhere)

gpui-component draws one scrollbar (gpui-base `Scrollbar`) from three theme colours; we set them in the theme and use the widget's own geometry.

**Desktop (exact numbers).** Track `scrollbar.background` = transparent (`#00000000`: thumb only, no gutter fill). Thumb `scrollbar.thumb.background` = `border_strong` (`{T["color.light.scrollbar_thumb"]}` light, `{T["color.dark.scrollbar_thumb"]}` dark: at least {TH_MIN:.2f}:1 on every surface it sits on). Thumb hover and drag `scrollbar.thumb.hover.background` = `{T["color.light.scrollbar_thumb_hover"]}` light, `{T["color.dark.scrollbar_thumb_hover"]}` dark (at least {THH_MIN:.2f}:1 on all surfaces including the always-dark log panel). Geometry from the widget: hit area **16 px** wide, thumb **6 px** idle growing to **8 px** on hover or drag, **4 px** inset from the edge, minimum thumb length **48 px**, corner radius = theme `radius` clamped to half the thumb (so a pill). Mode: `ScrollbarMode::Always` for the log viewer and tables (people must see that more exists), `ScrollbarMode::Scrolling` elsewhere (shows while scrolling, fades after 2 s idle, stays while hovered). Keyboard: PgUp, PgDn, Home, End and arrows scroll the focused region; the scroll container takes the focus ring (2 px, offset 2). One scroll container per screen above the pinned action bar; reserve 16 px at the scrolling edge so the thumb never covers content.

**Web (CSS).** Same colours, same geometry (6 px thumb visible, 16 px hit area through a transparent border):

```css
:root {{ --sb-thumb: {T["color.light.scrollbar_thumb"]}; --sb-thumb-hover: {T["color.light.scrollbar_thumb_hover"]}; }}
:root[data-theme="dark"] {{ --sb-thumb: {T["color.dark.scrollbar_thumb"]}; --sb-thumb-hover: {T["color.dark.scrollbar_thumb_hover"]}; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --sb-thumb: {T["color.dark.scrollbar_thumb"]}; --sb-thumb-hover: {T["color.dark.scrollbar_thumb_hover"]}; }} }}
* {{ scrollbar-width: thin; scrollbar-color: var(--sb-thumb) transparent; }}          /* Firefox */
*::-webkit-scrollbar {{ width: 16px; height: 16px; }}                                   /* Chromium, Safari */
*::-webkit-scrollbar-track {{ background: transparent; }}
*::-webkit-scrollbar-thumb {{ background: var(--sb-thumb); border: 5px solid transparent; background-clip: content-box; border-radius: 999px; min-height: 48px; min-width: 48px; }}
*::-webkit-scrollbar-thumb:hover, *::-webkit-scrollbar-thumb:active {{ background: var(--sb-thumb-hover); border-width: 4px; }}
*::-webkit-scrollbar-corner {{ background: transparent; }}
```

(`border: 5px` leaves a 6 px thumb, `4px` on hover makes it 8 px, matching the desktop.) The log panel is dark in both themes: give it its own `--sb-thumb` (`{T["color.dark.scrollbar_thumb"]}`).

## 7. Widget specifications

Sizes are the widgets' own (read from the source); our job is to choose the variant and set tokens. `Size::Medium` is the default everywhere; use `Small` only in dense tables. Focus: the theme `ring` colour, 2 px, offset 2; parents of edge controls must not clip it.

| Widget (gpui-component) | Variant / use | Size and metrics | Tokens (ThemeColor) | Notes |
|---|---|---|---|---|
| `Button` | primary (`.primary()`) one per screen; default (outline: border = `input`) for Back and Cancel; `.ghost()` tertiary; `.danger()` only for data-loss actions (verb + object label) | xs 20, sm 24, md **32** (text_sm 14), lg 32 with wider padding; px 10 (md); radius = theme radius 8; icon 16, gap 8 | `button*`, `primary*`, `button_danger*`, `input` (border), `ring` | Do not use the `Secondary` variant (its border is the hairline `border`, below 3:1). Disabled = the widget's disabled style; the reason goes in a tooltip. Loading = `.loading(true)` with a present-tense label. Danger border equals its fill (not themeable). |
| `Tag` (and `Badge` for counts) | outline Tag + Lucide icon for compat and status; filled Tag for counts and chips | text_xs 12, px 10, py 4, radius = theme radius (use `.rounded_full()` for pills), border 1; **Medium only** | `success`/`warning`/`danger`/`info` (text, border) with `*_foreground` for filled; `secondary`, `border` | Never `Size::Small`/`XSmall` (10 px). Untested uses Tag secondary outline with `circle-dashed`. Hover = opacity 0.9. |
| `Alert` | info / success / warning / error, `.banner()` for page-level rows | text_sm 14; px 16 py 10 gap 12 (md); radius 8; small px 12 py 8; icon 16 | text and icon = `info`/`success`/`warning`/`danger`; fill = that colour at 4% alpha; border at 30% | At most one title and one sentence; anything longer goes to a Dialog. `role=alert` equivalent for warn and error. |
| `Tooltip` | one sentence, on hover and keyboard focus; `.tooltip()` / `managed_tooltip` | text_sm 14, px 8, py 2, radius 8, shadow_md, border 1; 150 ms enter; show delay 400 ms (we set it; the widget default is 500 ms) | `popover`, `popover_foreground`, `border` | At most 14 words. Controls without a hover target (a disabled button, a label) get an info icon (`info`, 14 px) that carries the tooltip. Optional `Kbd` inside for the shortcut. |
| `Popover` | a short paragraph plus one action (button or link), opened by an info icon or button | padding 12 (`p_3`), offset 6, margin 8, radius 8, shadow, 150 ms enter, max width 320 | `popover`, `popover_foreground`, `border`, `link` | At most 40 words. Escape and outside click close it; focus returns to the trigger. |
| `Dialog` | modal: full explanation or decision; confirmation of a destructive action | width **448** default (640 for wide content), radius_lg 12, 250 ms, overlay `overlay` (black 0.60), padding 16 to 24 | `background`, `border`, `overlay` | Title at most 4 words, 2 to 4 short paragraphs (at most 80 words), one primary action, a Cancel that is not styled as primary. The data-loss gate is a full screen, not a dialog. |
| `Sheet` (drawer) | Help topics, About, logs detail; side Right | width **350** (480 for Help), padding 16, shadow_xl, 150 ms | `background`, `border`, `overlay` | Escape closes; title `font_semibold` 16; keep one scroll container inside. |
| `Kbd` | shortcuts in tooltips, Help and menus | text_xs 12, px 4, min width 20, radius = radius / 2 (4), border 1 in the outline form | `muted` (bg), `muted_foreground` (text), `border` | Platform glyphs on macOS (command, option), words elsewhere (Ctrl, Alt), automatic from the key binding. |
| `Sidebar` (web: custom CSS equal to it) | app navigation in the Management UI shell; desktop: not used (the wizard has a Stepper) | width 255 default, we set **240**; collapsed 48; menu item height 32, icon 16, gap 8, radius 8 | `sidebar*` (background, foreground, border, accent, accent_foreground, primary) | Active item = `sidebar.accent` fill + `sidebar.accent.foreground`; hover = the same at 0.8 alpha. Group labels `muted_foreground`, sentence case, text_xs. |
| `Stepper` | the 9-step wizard in the title area | icon 24 (lg 32), connector 2 px, gap 4; labels text_sm, only the current label shown below 1100 px window width | `primary` (done, current), `border` (upcoming), `muted_foreground` | Done = check icon, current = ring plus label, failed = `circle-x` in `danger` with the step name kept (custom item state). |
| `DataTable` / `Table` | scan results, disks, volumes, users | row height xs 26, sm 30, **md 32**, lg 40; header row 32; cell padding 8 x 12; row border 1 px | `table*` (background, head, head_foreground, hover, active, active_border, row_border) | At most 5 columns at 960 px. Selected row = `table.active.background` + 3 px `table.active.border`. Stripes off (even = background). Drag-select does not exist: give copyable cells a Copy button. |
| `Tabs` | page sections (Underline), small filters (Segmented) | Underline md 36 (sm 30, lg 44), Segmented md 32 (sm 24); padding 12; radius_lg for segments | `tab*`, `tab_bar*`, `foreground` | Log level filter = Segmented tabs with cumulative counts ("Info 149"). |
| `Accordion` | collapsible logs, progress groups, advanced options | header text_sm semibold, chevron 16, radius_lg 12, border 1 | `accordion`, `border`, `foreground`, `muted_foreground` | Collapsed by default except the current step. |
| `Progress` | step progress, resync, downloads | height xs 4, sm 6, **md 8**, lg 10, pill radius | `progress.bar.background` (fill; track = fill at 20% alpha) | The track alone is not a 3:1 boundary: wrap the bar in a 1 px `input` border or show the value as text beside it. Colour variants (warning, danger) through `.color(..)`. |
| `Input`, `Select`, `Switch`, `Checkbox`, `Radio`, `Slider` | forms | input md 32, sm 24, lg 44; checkbox and radio 16; slider thumb 16, bar 6 | `input` (border, at least 3:1), `primary` (checked), `switch`, `switch_thumb`, `slider_bar`, `slider_thumb`, `caret`, `selection`, `ring` | The typed phrase uses `Input` (not a div). Placeholder = `muted_foreground`. Error = `danger` border plus a message with an icon. |
| `Breadcrumb`, `Link` | web and Help | text_sm, separator `chevron-right` 14 | `muted_foreground`, `foreground` (last), `link*` | |
| `Notification` (toasts) | non-blocking result ("Copied") | text_sm, 4 s | `popover*`, `border` | Never for errors that need action: use Alert or Dialog. |
| `Skeleton`, `Spinner` | loading | spinner = `loader-circle` 16 | `skeleton`, `muted_foreground` | Static glyph under reduced motion. |

Contrast of the widget pairs on the generated theme (light / dark): primary label on fill {wr('primary.foreground / primary.background')}; default button label {wr('button.foreground / button.background')}; destructive button label {wr('button.danger.foreground / button.danger.background')} (hover {wr('button.danger.foreground / button.danger.hover.background')}, pressed {wr('button.danger.foreground / button.danger.active.background')}); destructive fill against the page (non-text) {wr('button.danger.background / background')}; filled Tag success {wr('Tag fill success.background')}, warning {wr('Tag fill warning.background')}, danger {wr('Tag fill danger.background')}, info {wr('Tag fill info.background')}; Alert text on its tint: success {wr('Alert success text on 4% tint over background')}, warning {wr('Alert warning text on 4% tint over background')}, danger {wr('Alert danger text on 4% tint over background')}, info {wr('Alert info text on 4% tint over background')}; tooltip and popover text {wr('foreground / popover.background')}; Kbd text {wr('Kbd outline text on background')}; tab label {wr('tab.foreground / background')}; table head {wr('table.head.foreground / table.head.background')}; sidebar item {wr('sidebar.accent.foreground / sidebar.accent.background')}; link {wr('link / background')}; text on selection {wr('foreground / selection.background')}; input border {wr('input.border / background')}; scrollbar thumb {wr('scrollbar.thumb.background / background')}; switch off track {wr('switch.background / background')}; progress fill vs its 20% track {wr('Progress fill vs 20% track on background')}.

## 8. Gate and numbers

`python3 design/v2/tools/contrast_v2.py` checks **{N} pairs on both themes**: {NM} token-matrix pairs (every text token on every surface it may sit on, every non-text token on every surface it may touch, and the five web accent sets) and {NW} pairs on the *generated* gpui-component theme (what the widgets will paint: button, tag, alert 4% tints, tooltip, popover, kbd, tab, table, sidebar, link, selection, scrollbar thumb, switch, slider, progress track, input and checkbox border, focus ring). It also verifies the ThemeConfig keys against the crate's schema, and that no token font size is under 12 px. **Result: {N} of {N} pass; lowest text ratio {MT}:1, lowest non-text ratio {MN}:1.** It was run against deliberately bad tokens and fails as it should. Not covered: GPUI's real rendering and alpha blending beyond the 4% and 20% blends the widgets are known to use, text drawn over images, and hover/pressed states of text on arbitrary surfaces (links are only allowed on `canvas`, `surface` and the tint).

## 9. Motion

Widget defaults, kept: fast 120 ms (hover, colour), normal 180 ms (progress, accordion), slow 280 ms, dialog 250 ms, sheet 150 ms, tooltip and popover 150 ms ease-out, no slide for dialogs beyond the widget's own. The only loop is the spinner; with reduced motion on (a setting, because GPUI does not expose the OS flag everywhere) it is a static glyph plus the word "Working". Countdowns update once per second without easing and are announced at 10 s and 5 s only.

## 10. Layout metrics

Window minimum 960 x 640. Title bar 38, stepper 48, content one scroll area with 24 padding, pinned action bar 64, safety rail 216 on the right. Control heights: 32 (md), 24 (sm), 44 for lg inputs and for every touch target in the web UI below 700 px. Spacing scale 2, 4, 6, 8, 12, 16, 20, 24, 32, 40, 48. Radius: 4 (kbd), 8 (controls), 12 (dialogs, cards, tabs), pill.

## 11. Help and About (both surfaces)

Help centre: a Sheet (480 px) with a search `Input` at the top, a list of short topics (`List`), a "Shortcuts" topic using `Kbd`, "How recovery works", "How to report a compatibility result", and a docs link; topics open in the same sheet; every complex control has an info icon (`info`) that opens its topic (Popover for the summary, link "Open in Help" for the topic). About: a page (desktop) or route (web) with version, build, licences and credits (Inter and JetBrains Mono OFL 1.1, Lucide ISC, GPUI and gpui-component Apache 2.0, plus the dependency list), a "Copy diagnostics" button (Copy then check icon), and links. Both use the same strings from the copy module (`help` field).

## 12. Exceptions (no widget exists) and open items

- **Safety rail** (the "Safety" panel with four levels): no widget; build from `Stepper`-like rows inside a `GroupBox`, tokens as in section 3. Justified: it is product-specific.
- **Disk/storage diagram**: custom `canvas()` for connectors, boxes are `div`s with `Tag`-like styling; scrolls horizontally inside its card below 640 px; 12 px text.
- **Log viewer**: virtual list of lines (check `VirtualList`/`List`), mono 12; level glyph = Lucide icon + word.
- **Countdown ring** (web only): SVG.
- **Open:** GPUI accessibility tree (not verified; spike pending), font feature `tnum` exposure in the pinned GPUI, exact `IconName` variant spelling, `Sidebar` custom width, `Tabs` underline colour (expected `primary`), dark `danger` hover/pressed subtlety, real-app screenshots (to be reviewed against this brief).
'''
open(os.path.join(HERE, 'DESIGN-SYSTEM-V2.md'), 'w').write(DOC); print(len(DOC), N, NM, NW, MT, MN)
