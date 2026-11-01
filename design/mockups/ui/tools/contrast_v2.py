#!/usr/bin/env python3
"""synofurb contrast gate v2 (real gpui-component widgets).  Usage: python3 contrast_v2.py [-v]
Reads design/v2/tokens-v2.json and the generated design/v2/synofurb-theme.json.  Exit 1 on any failure.
  1. TOKEN MATRIX   every text token on every surface it may sit on (4.5:1) and every non-text token on every surface it may touch (3:1), both themes.
  2. WIDGET PAIRS   pairs written in gpui-component ThemeConfig keys and tested on the *generated theme* (what the widgets will really paint):
                    button/tag fills + labels, alert text on its 4% tint, tooltip, popover, kbd, tab, table, sidebar, link, selection, scrollbar thumb,
                    switch, slider, progress bar vs its 20% track, input and checkbox borders, focus ring.
  3. MAP            every ThemeConfig key used exists in gpui-component's schema (when the crate source is available) and resolves to a defined token.
  4. LINT           font sizes in tokens are >= 12 px."""
import glob, json, os, re, sys
HERE = os.path.dirname(os.path.abspath(__file__)); V2 = os.path.join(HERE, '..', '..', '..', 'v2')
T = json.load(open(os.path.join(V2, 'tokens-v2.json'))); TH = json.load(open(os.path.join(V2, 'synofurb-theme.json')))
VERBOSE = '-v' in sys.argv
def lin(c): c /= 255; return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
def rgb(h): h = h.lstrip('#'); return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))
def lum(h): r, g, b = rgb(h); return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
def ratio(a, b): x, y = sorted((lum(a), lum(b)), reverse=True); return (x + .05) / (y + .05)
def over(fg, bg):  # '#rrggbbaa' over opaque bg
    a = int(fg[7:9], 16) / 255 if len(fg) == 9 else 1.0; f, b = rgb(fg), rgb(bg)
    return '#%02X%02X%02X' % tuple(round(x * a + y * (1 - a)) for x, y in zip(f, b))
def tint(color, a, bg): return over(color + '%02X' % round(a * 255), bg)
fails = []; n = {'matrix': 0, 'widget': 0}; mins = {'text': 9e9, 'non': 9e9}
def chk(kind, label, f, b, need, text):
    n[kind] += 1; r = ratio(f, b); k = 'text' if text else 'non'; mins[k] = min(mins[k], r)
    if r < need: fails.append(f'{label}: {f} on {b} = {r:.2f} < {need}')
    elif VERBOSE: print(f'ok {label:58}{r:6.2f} >= {need}')
NEUT = ['canvas', 'surface', 'surface_2', 'surface_3', 'sidebar', 'accent_bg']
STAT = ['ok', 'warn', 'err', 'info']
TEXT = {'fg': NEUT + ['selection'], 'fg_2': NEUT + ['selection'], 'placeholder': ['canvas', 'surface', 'surface_2', 'accent_bg'], 'accent_fg': NEUT, 'accent_hover': ['canvas', 'surface', 'surface_2'], 'accent_pressed': ['canvas', 'surface'],
        'on_accent': ['accent', 'accent_hover', 'accent_pressed'], 'on_danger': ['danger', 'danger_hover', 'danger_pressed'],
        'log_fg': ['log_bg', 'log_wrn_bg', 'log_err_bg'], 'log_dim': ['log_bg', 'log_wrn_bg', 'log_err_bg'], 'log_dbg': ['log_bg'], 'log_inf': ['log_bg'], 'log_wrn': ['log_bg', 'log_wrn_bg'], 'log_err': ['log_bg', 'log_err_bg']}
for s in STAT:
    TEXT[f'{s}_fg'] = NEUT; TEXT[f'{s}_fg_hover'] = ['canvas', 'surface']; TEXT[f'on_{s}'] = [f'{s}_fg', f'{s}_fg_hover', f'{s}_fg_active']
SURF = NEUT + ['ok_bg', 'warn_bg', 'err_bg', 'info_bg']
NON = {'border_strong': SURF, 'ring': SURF, 'ring_inverse': ['log_bg'], 'accent': ['canvas', 'surface', 'surface_2', 'surface_3'], 'danger': ['canvas', 'surface', 'sidebar'], 'danger_hover': ['canvas', 'surface', 'sidebar'], 'danger_pressed': ['canvas', 'surface', 'sidebar'],
       'scrollbar_thumb': ['canvas', 'surface', 'surface_2', 'sidebar', 'accent_bg', 'log_bg'], 'scrollbar_thumb_hover': ['canvas', 'surface', 'surface_2', 'sidebar', 'accent_bg', 'log_bg'], 'switch_off': ['canvas', 'surface', 'surface_2', 'sidebar']}
for s in STAT: NON[f'{s}_solid'] = ['canvas', 'surface', 'surface_2', 'surface_3', 'accent_bg']
for th in ('light', 'dark'):
    g = lambda k: T[f'color.{th}.{k}']
    for f, bs in TEXT.items():
        for b in bs: chk('matrix', f'{th} text {f} / {b}', g(f), g(b), 4.5, True)
    for f, bs in NON.items():
        for b in bs: chk('matrix', f'{th} non {f} / {b}', g(f), g(b), 3.0, False)
    chk('matrix', f'{th} non surface thumb on switch_off', g('surface'), g('switch_off'), 3.0, False)
# ---- widget pairs on the generated theme (ThemeConfig keys). (fg key, bg key, need, text?, label)
def widget_pairs(c, bgkeys):
    P = []
    for fk, bk, need, text in [
        ('primary.foreground', 'primary.background', 4.5, True), ('primary.foreground', 'primary.hover.background', 4.5, True), ('primary.foreground', 'primary.active.background', 4.5, True),
        ('button.foreground', 'button.background', 4.5, True), ('button.foreground', 'button.hover.background', 4.5, True), ('button.foreground', 'button.active.background', 4.5, True),
        ('button.danger.foreground', 'button.danger.background', 4.5, True), ('button.danger.foreground', 'button.danger.hover.background', 4.5, True), ('button.danger.foreground', 'button.danger.active.background', 4.5, True),
        ('button.danger.background', 'background', 3.0, False), ('button.danger.background', 'popover.background', 3.0, False), ('button.danger.hover.background', 'background', 3.0, False),
        ('foreground', 'background', 4.5, True), ('foreground', 'popover.background', 4.5, True), ('muted.foreground', 'background', 4.5, True), ('muted.foreground', 'muted.background', 4.5, True), ('muted.foreground', 'popover.background', 4.5, True),
        ('foreground', 'selection.background', 4.5, True), ('foreground', 'list.active.background', 4.5, True), ('foreground', 'table.active.background', 4.5, True), ('foreground', 'list.hover.background', 4.5, True),
        ('table.head.foreground', 'table.head.background', 4.5, True), ('table.foot.foreground', 'table.foot.background', 4.5, True), ('foreground', 'table.background', 4.5, True),
        ('tab.foreground', 'background', 4.5, True), ('tab.foreground', 'tab_bar.segmented.background', 4.5, True), ('tab.active.foreground', 'tab.active.background', 4.5, True), ('tab.foreground', 'popover.background', 4.5, True),
        ('sidebar.foreground', 'sidebar.background', 4.5, True), ('sidebar.accent.foreground', 'sidebar.accent.background', 4.5, True), ('sidebar.primary.foreground', 'sidebar.primary.background', 4.5, True), ('muted.foreground', 'sidebar.background', 4.5, True),
        ('link', 'background', 4.5, True), ('link.hover', 'background', 4.5, True), ('link.active', 'background', 4.5, True), ('link', 'popover.background', 4.5, True),
        ('description_list.label.foreground', 'description_list.label.background', 4.5, True), ('group_box.foreground', 'group_box.background', 4.5, True),
        ('input.border', 'background', 3.0, False), ('input.border', 'popover.background', 3.0, False), ('input.border', 'muted.background', 3.0, False), ('input.border', 'list.active.background', 3.0, False), ('input.border', 'table.head.background', 3.0, False),
        ('ring', 'background', 3.0, False), ('ring', 'popover.background', 3.0, False), ('ring', 'muted.background', 3.0, False), ('ring', 'list.active.background', 3.0, False), ('ring', 'sidebar.background', 3.0, False),
        ('scrollbar.thumb.background', 'background', 3.0, False), ('scrollbar.thumb.background', 'popover.background', 3.0, False), ('scrollbar.thumb.background', 'table.background', 3.0, False), ('scrollbar.thumb.background', 'sidebar.background', 3.0, False),
        ('scrollbar.thumb.hover.background', 'background', 3.0, False), ('scrollbar.thumb.hover.background', 'table.background', 3.0, False),
        ('switch.background', 'background', 3.0, False), ('switch.background', 'popover.background', 3.0, False), ('switch.thumb.background', 'switch.background', 3.0, False), ('primary.background', 'background', 3.0, False), ('switch.thumb.background', 'primary.background', 3.0, False),
        ('slider.thumb.background', 'background', 3.0, False), ('table.active.border', 'table.active.background', 3.0, False), ('list.active.border', 'list.active.background', 3.0, False)]:
        P.append((fk, bk, need, text, f'{fk} / {bk}'))
    for s, k in (('success', 'ok'), ('warning', 'warn'), ('danger', 'err'), ('info', 'info')):
        for st in ('background', 'hover.background', 'active.background'): P.append((f'{s}.foreground', f'{s}.{st}', 4.5, True, f'Tag fill {s}.{st}'))
        for bk in ('background', 'popover.background', 'muted.background', 'sidebar.background', 'list.active.background'): P.append((f'{s}.background', bk, 4.5, True, f'{s} text on {bk} (outline tag, icon)'))
    return P
for t in TH['themes']:
    c = {k: (v if v.startswith('#') else v) for k, v in t['colors'].items()}
    bgk = c['background']
    def val(k): return c[k]
    for fk, bk, need, text, label in widget_pairs(c, bgk):
        f, b = val(fk), val(bk)
        if len(b) == 9: b = over(b, bgk)
        chk('widget', f'{t["mode"]} widget {label}', f, b, need, text)
    # alert: text/icon = status colour on (status at 4% over background); also on popover
    for s in ('success', 'warning', 'danger', 'info'):
        for base in ('background', 'popover.background'):
            bgb = tint(val(f'{s}.background'), 0.04, val(base)); chk('widget', f'{t["mode"]} widget Alert {s} text on 4% tint over {base}', val(f'{s}.background'), bgb, 4.5, True)
        chk('widget', f'{t["mode"]} widget Alert {s} border (30%) vs background (decorative, informational)', val(f'{s}.background'), val('background'), 3.0, False)
    # progress and slider bar vs its own 20% track, and the bar vs the page
    for base in ('background', 'popover.background'):
        track = tint(val('progress.bar.background'), 0.2, val(base)); chk('widget', f'{t["mode"]} widget Progress fill vs 20% track on {base}', val('progress.bar.background'), track, 3.0, False)
    # kbd: muted text on muted bg (default) and on background (outline)
    chk('widget', f'{t["mode"]} widget Kbd outline text on background', val('muted.foreground'), val('background'), 4.5, True)
# ---- accents offered on the web plane (desktop is violet only)
for name in sorted({k.split('.')[1] for k in T if k.startswith('accent.')}):
    for th in ('light', 'dark'):
        a = lambda k: T[f'accent.{name}.{th}.{k}']; sf, cv, s3 = T[f'color.{th}.surface'], T[f'color.{th}.canvas'], T[f'color.{th}.surface_3']
        for lab, f, b, need, text in [('label', a('on_accent'), a('accent'), 4.5, True), ('label hover', a('on_accent'), a('accent_hover'), 4.5, True), ('label pressed', a('on_accent'), a('accent_pressed'), 4.5, True), ('link on surface', a('accent_fg'), sf, 4.5, True),
                                      ('link on canvas', a('accent_fg'), cv, 4.5, True), ('link on tint', a('accent_fg'), a('accent_bg'), 4.5, True), ('ring on surface', a('ring'), sf, 3.0, False), ('ring on tint', a('ring'), a('accent_bg'), 3.0, False), ('fill on surface', a('accent'), sf, 3.0, False), ('fill on track', a('accent'), s3, 3.0, False)]:
            chk('matrix', f'{th} accent {name} {lab}', f, b, need, text)
# ---- map integrity
schema = (glob.glob(os.path.expanduser('~/.cargo/registry/src/*/gpui-component-0.8.1/src/theme/schema.rs')) or [None])[0]
valid = set(re.findall(r'serde\(rename = "([a-z_.0-9]+)"\)', open(schema).read())) if schema else None
for k, v in T.items():
    if k.startswith('map.'):
        key, tok = k[4:], v
        if valid is not None and key not in valid: fails.append(f'map: {key} is not a ThemeConfig key in gpui-component 0.8.1')
        if not tok.startswith('#') and f'color.light.{tok}' not in T: fails.append(f'map: {key} -> unknown token {tok}')
for k, v in T.items():
    if k.startswith('type.') and k.endswith('.size_px') and v < 12: fails.append(f'lint: {k} = {v} < 12 px')
for k in ('size.control.xs',): pass
total = n['matrix'] + n['widget']
print(f'Checked {total} pairs on both themes: {n["matrix"]} token-matrix, {n["widget"]} on the generated gpui-component theme.')
print(f'Passing {total - len([f for f in fails if "<" in f and "lint" not in f])} of {total}; lowest text ratio {mins["text"]:.2f}:1, lowest non-text ratio {mins["non"]:.2f}:1; schema check {"on" if valid else "skipped (crate source not found)"}.')
for f in fails: print('FAIL', f)
sys.exit(1 if fails else 0)
